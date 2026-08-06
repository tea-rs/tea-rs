use std::borrow::Cow;
use std::fmt;
use std::io::Read as _;

use dom_query::Document;
use encoding_rs::{Encoding, UTF_8};
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};

use super::{
    FetchHttpHeaders, FetchProviderError, FetchProviderErrorCode, FetchTruncationReason,
    MAX_FETCH_MAX_CHARS, MAX_FETCH_TITLE_BYTES,
};

/// Default maximum decoded response bytes accepted by the body pipeline.
pub const DEFAULT_FETCH_DECODED_BYTES: usize = 4 * 1024 * 1024;
/// Absolute maximum decoded response bytes accepted by body configuration.
pub const MAX_FETCH_DECODED_BYTES: usize = 16 * 1024 * 1024;
/// Default maximum HTML input bytes passed to the parser.
pub const DEFAULT_FETCH_HTML_BYTES: usize = 1024 * 1024;
/// Default maximum opening-tag markers accepted before HTML parsing.
pub const DEFAULT_FETCH_HTML_ELEMENTS: usize = 50_000;

const MIME_SNIFF_BYTES: usize = 512;

/// Independent decoded-byte and HTML-parser limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FetchBodyLimits {
    decoded_bytes: usize,
    html_bytes: usize,
    html_elements: usize,
}

impl FetchBodyLimits {
    /// Creates response-body limits.
    ///
    /// # Errors
    ///
    /// Rejects zero values, decoded bounds above 16 MiB, and HTML bounds above
    /// the decoded-byte limit.
    pub fn new(
        max_decoded_bytes: usize,
        max_html_bytes: usize,
        max_html_elements: usize,
    ) -> Result<Self, FetchProviderError> {
        if !(1..=MAX_FETCH_DECODED_BYTES).contains(&max_decoded_bytes)
            || !(1..=max_decoded_bytes).contains(&max_html_bytes)
            || max_html_elements == 0
        {
            return Err(FetchProviderError::new(
                FetchProviderErrorCode::InvalidConfiguration,
            ));
        }
        Ok(Self {
            decoded_bytes: max_decoded_bytes,
            html_bytes: max_html_bytes,
            html_elements: max_html_elements,
        })
    }

    /// Returns the decoded-byte limit.
    #[must_use]
    pub const fn max_decoded_bytes(self) -> usize {
        self.decoded_bytes
    }

    /// Returns the HTML input byte limit.
    #[must_use]
    pub const fn max_html_bytes(self) -> usize {
        self.html_bytes
    }

    /// Returns the HTML element-marker limit.
    #[must_use]
    pub const fn max_html_elements(self) -> usize {
        self.html_elements
    }
}

impl Default for FetchBodyLimits {
    fn default() -> Self {
        Self {
            decoded_bytes: DEFAULT_FETCH_DECODED_BYTES,
            html_bytes: DEFAULT_FETCH_HTML_BYTES,
            html_elements: DEFAULT_FETCH_HTML_ELEMENTS,
        }
    }
}

/// Canonical response content kind retained by the fetch result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchContentKind {
    /// HTML or XHTML converted to bounded visible text.
    Html,
    /// Plain text.
    PlainText,
    /// Valid JSON text.
    Json,
}

impl FetchContentKind {
    /// Returns the canonical MIME type.
    #[must_use]
    pub const fn mime_type(self) -> &'static str {
        match self {
            Self::Html => "text/html; charset=utf-8",
            Self::PlainText => "text/plain; charset=utf-8",
            Self::Json => "application/json; charset=utf-8",
        }
    }
}

/// Normalized body ready for a provider-neutral [`super::FetchResult`].
#[derive(Clone, PartialEq, Eq)]
pub struct DecodedFetchBody {
    kind: FetchContentKind,
    title: Option<String>,
    body: String,
    truncation: Option<FetchTruncationReason>,
    decoded_bytes: usize,
}

impl DecodedFetchBody {
    /// Returns the canonical content kind.
    #[must_use]
    pub const fn kind(&self) -> FetchContentKind {
        self.kind
    }

    /// Returns the canonical MIME type.
    #[must_use]
    pub const fn mime_type(&self) -> &'static str {
        self.kind.mime_type()
    }

    /// Returns the optional bounded HTML title.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Returns normalized bounded body text.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Returns the output truncation reason.
    #[must_use]
    pub const fn truncation(&self) -> Option<FetchTruncationReason> {
        self.truncation
    }

    /// Returns bytes after content-encoding decompression.
    #[must_use]
    pub const fn decoded_bytes(&self) -> usize {
        self.decoded_bytes
    }
}

impl fmt::Debug for DecodedFetchBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecodedFetchBody")
            .field("kind", &self.kind)
            .field("has_title", &self.title.is_some())
            .field("body_chars", &self.body.chars().count())
            .field("truncation", &self.truncation)
            .field("decoded_bytes", &self.decoded_bytes)
            .finish_non_exhaustive()
    }
}

/// Bounded decoder and extractor for raw HTTP response bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchBodyDecoder {
    limits: FetchBodyLimits,
}

impl FetchBodyDecoder {
    /// Creates a decoder with explicit independent limits.
    #[must_use]
    pub const fn new(limits: FetchBodyLimits) -> Self {
        Self { limits }
    }

    /// Returns the immutable decoding and parser limits.
    #[must_use]
    pub const fn limits(self) -> FetchBodyLimits {
        self.limits
    }

    /// Decodes, classifies, parses, and bounds one raw response body.
    ///
    /// # Errors
    ///
    /// Rejects unsupported content encodings/MIME types, invalid charset data,
    /// decompression limit violations, malformed JSON, binary content, and HTML
    /// parser inputs above the configured byte/element bounds.
    pub fn decode(
        &self,
        headers: &FetchHttpHeaders,
        raw: &[u8],
        max_chars: usize,
    ) -> Result<DecodedFetchBody, FetchProviderError> {
        if !(1..=MAX_FETCH_MAX_CHARS).contains(&max_chars) {
            return Err(FetchProviderError::new(
                FetchProviderErrorCode::InvalidRequest,
            ));
        }
        let decoded = decode_content_encoding(
            headers.content_encoding(),
            raw,
            self.limits.max_decoded_bytes(),
        )?;
        let decoded_bytes = decoded.len();
        let declared = declared_content(headers.content_type())?;
        let sniffed = sniff_content(&decoded);
        let kind = select_kind(declared.as_ref().map(|value| value.kind), sniffed)?;
        let encoding = select_encoding(declared.as_ref(), kind, &decoded)?;
        let text = decode_text(&decoded, encoding)?;
        let (title, body) = match kind {
            FetchContentKind::Html => self.extract_html(&text)?,
            FetchContentKind::PlainText => (None, normalize_plain_text(&text)),
            FetchContentKind::Json => {
                serde_json::from_str::<serde_json::Value>(&text).map_err(|_| {
                    FetchProviderError::new(FetchProviderErrorCode::MalformedResponse)
                })?;
                (None, normalize_plain_text(text.trim()))
            }
        };
        let (body, truncation) = truncate_body(&body, max_chars);
        Ok(DecodedFetchBody {
            kind,
            title,
            body,
            truncation,
            decoded_bytes,
        })
    }

    fn extract_html(self, html: &str) -> Result<(Option<String>, String), FetchProviderError> {
        if html.len() > self.limits.max_html_bytes()
            || html.bytes().filter(|byte| *byte == b'<').count() > self.limits.max_html_elements()
        {
            return Err(FetchProviderError::new(
                FetchProviderErrorCode::ResponseTooLarge,
            ));
        }
        let document = Document::from(html);
        document
            .select("script, style, noscript, template, svg")
            .remove();
        let title = bounded_title(&normalize_inline_text(&document.select("title").text()));
        let body_selection = document.select("body");
        let visible_text = if body_selection.exists() {
            body_selection.formatted_text()
        } else {
            document.formatted_text()
        };
        let body = normalize_plain_text(visible_text.trim());
        Ok((title, body))
    }
}

impl Default for FetchBodyDecoder {
    fn default() -> Self {
        Self::new(FetchBodyLimits::default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SniffedContent {
    Html,
    Json,
    Text,
    Binary,
}

#[derive(Debug, Clone, Copy)]
struct DeclaredContent {
    kind: FetchContentKind,
    encoding: Option<&'static Encoding>,
}

fn declared_content(value: Option<&str>) -> Result<Option<DeclaredContent>, FetchProviderError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let parsed = value
        .parse::<mime::Mime>()
        .map_err(|_| FetchProviderError::new(FetchProviderErrorCode::UnsupportedMime))?;
    let kind = if parsed.type_() == mime::TEXT && parsed.subtype() == mime::HTML {
        FetchContentKind::Html
    } else if parsed.type_() == mime::TEXT && parsed.subtype() == mime::PLAIN {
        FetchContentKind::PlainText
    } else if parsed.type_() == mime::APPLICATION
        && (parsed.subtype() == mime::JSON || parsed.suffix() == Some(mime::JSON))
    {
        FetchContentKind::Json
    } else if parsed.type_() == mime::APPLICATION
        && parsed.subtype().as_str() == "xhtml"
        && parsed.suffix() == Some(mime::XML)
    {
        FetchContentKind::Html
    } else {
        return Err(FetchProviderError::new(
            FetchProviderErrorCode::UnsupportedMime,
        ));
    };
    let encoding = parsed
        .get_param(mime::CHARSET)
        .map(|label| {
            Encoding::for_label(label.as_str().as_bytes())
                .ok_or_else(|| FetchProviderError::new(FetchProviderErrorCode::UnsupportedMime))
        })
        .transpose()?;
    Ok(Some(DeclaredContent { kind, encoding }))
}

fn sniff_content(bytes: &[u8]) -> SniffedContent {
    if bytes.contains(&0) {
        return SniffedContent::Binary;
    }
    let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(MIME_SNIFF_BYTES)]);
    let normalized = prefix.trim_start().to_ascii_lowercase();
    if normalized.starts_with("<!doctype html")
        || normalized.starts_with("<html")
        || normalized.starts_with("<head")
        || normalized.starts_with("<body")
    {
        SniffedContent::Html
    } else if normalized.starts_with('{') || normalized.starts_with('[') {
        SniffedContent::Json
    } else {
        SniffedContent::Text
    }
}

fn select_kind(
    declared: Option<FetchContentKind>,
    sniffed: SniffedContent,
) -> Result<FetchContentKind, FetchProviderError> {
    match (declared, sniffed) {
        (_, SniffedContent::Binary) => Err(FetchProviderError::new(
            FetchProviderErrorCode::UnsupportedMime,
        )),
        (Some(FetchContentKind::Json), _) | (None, SniffedContent::Json) => {
            Ok(FetchContentKind::Json)
        }
        (Some(FetchContentKind::Html), _) | (_, SniffedContent::Html) => Ok(FetchContentKind::Html),
        (Some(FetchContentKind::PlainText), _) | (None, SniffedContent::Text) => {
            Ok(FetchContentKind::PlainText)
        }
    }
}

fn select_encoding(
    declared: Option<&DeclaredContent>,
    kind: FetchContentKind,
    bytes: &[u8],
) -> Result<(&'static Encoding, usize), FetchProviderError> {
    if let Some((encoding, bom_bytes)) = Encoding::for_bom(bytes) {
        if kind == FetchContentKind::Json && encoding != UTF_8 {
            return Err(FetchProviderError::new(
                FetchProviderErrorCode::MalformedResponse,
            ));
        }
        return Ok((encoding, bom_bytes));
    }
    let encoding = declared.and_then(|value| value.encoding).unwrap_or(UTF_8);
    if kind == FetchContentKind::Json && encoding != UTF_8 {
        return Err(FetchProviderError::new(
            FetchProviderErrorCode::MalformedResponse,
        ));
    }
    Ok((encoding, 0))
}

fn decode_text<'a>(
    bytes: &'a [u8],
    (encoding, offset): (&'static Encoding, usize),
) -> Result<Cow<'a, str>, FetchProviderError> {
    let (text, _, had_errors) = encoding.decode(&bytes[offset..]);
    if had_errors || text.contains('\0') {
        Err(FetchProviderError::new(
            FetchProviderErrorCode::MalformedResponse,
        ))
    } else {
        Ok(text)
    }
}

fn decode_content_encoding(
    encoding: Option<&str>,
    raw: &[u8],
    max_bytes: usize,
) -> Result<Vec<u8>, FetchProviderError> {
    match encoding
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("" | "identity") => bounded_identity(raw, max_bytes),
        Some("gzip" | "x-gzip") => read_bounded(GzDecoder::new(raw), max_bytes),
        Some("deflate") => read_bounded(ZlibDecoder::new(raw), max_bytes).or_else(|error| {
            if error.code() == FetchProviderErrorCode::MalformedResponse {
                read_bounded(DeflateDecoder::new(raw), max_bytes)
            } else {
                Err(error)
            }
        }),
        Some("br") => read_bounded(brotli::Decompressor::new(raw, 4096), max_bytes),
        Some(_) => Err(FetchProviderError::new(
            FetchProviderErrorCode::UnsupportedMime,
        )),
    }
}

fn bounded_identity(raw: &[u8], max_bytes: usize) -> Result<Vec<u8>, FetchProviderError> {
    if raw.len() > max_bytes {
        Err(FetchProviderError::new(
            FetchProviderErrorCode::ResponseTooLarge,
        ))
    } else {
        Ok(raw.to_vec())
    }
}

fn read_bounded(
    reader: impl std::io::Read,
    max_bytes: usize,
) -> Result<Vec<u8>, FetchProviderError> {
    let mut reader = reader.take(max_bytes as u64 + 1);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|_| FetchProviderError::new(FetchProviderErrorCode::MalformedResponse))?;
    if bytes.len() > max_bytes {
        Err(FetchProviderError::new(
            FetchProviderErrorCode::ResponseTooLarge,
        ))
    } else {
        Ok(bytes)
    }
}

fn bounded_title(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    let mut end = value.len().min(MAX_FETCH_TITLE_BYTES);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    Some(value[..end].to_owned())
}

fn normalize_plain_text(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect::<String>()
        .trim()
        .to_owned()
}

fn normalize_inline_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_body(value: &str, max_chars: usize) -> (String, Option<FetchTruncationReason>) {
    let mut characters = value.chars();
    let truncated = characters.by_ref().take(max_chars).collect::<String>();
    if characters.next().is_some() {
        (truncated, Some(FetchTruncationReason::BodyCharacters))
    } else {
        (truncated, None)
    }
}
