use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::ToolCallId;
use crate::content::{
    MAX_TOOL_ARGUMENT_BYTES, MAX_TOOL_ARGUMENT_DEPTH, validate_provider_tool_call_id,
    validate_tool_name,
};
use crate::metadata::{ProtocolMetadataError, validate_json_bounds};

/// Maximum UTF-8 bytes in one external source URL.
pub const MAX_EXTERNAL_SOURCE_URL_BYTES: usize = 8 * 1024;
/// Maximum UTF-8 bytes in one external source title.
pub const MAX_EXTERNAL_SOURCE_TITLE_BYTES: usize = 1024;
/// Maximum UTF-8 bytes in a source snippet or cited text.
pub const MAX_EXTERNAL_SOURCE_TEXT_BYTES: usize = 64 * 1024;
/// Maximum normalized sources retained for one hosted tool activity.
pub const MAX_HOSTED_TOOL_SOURCES: usize = 64;
/// Maximum encoded JSON bytes in one opaque provider continuation envelope.
pub const MAX_PROVIDER_CONTINUATION_BYTES: usize = 4 * 1024 * 1024;
/// Maximum nesting depth in provider continuation JSON.
pub const MAX_PROVIDER_CONTINUATION_DEPTH: usize = 64;
/// Maximum UTF-8 bytes in a normalized client web-fetch URL.
pub const MAX_WEB_FETCH_URL_BYTES: usize = 2 * 1024;
/// Maximum UTF-8 bytes in a normalized client web-fetch title.
pub const MAX_WEB_FETCH_TITLE_BYTES: usize = 4 * 1024;
/// Maximum UTF-8 bytes in a normalized client web-fetch MIME type.
pub const MAX_WEB_FETCH_MIME_BYTES: usize = 128;
/// Maximum Unicode scalar values in an extracted client web-fetch body.
pub const MAX_WEB_FETCH_BODY_CHARS: usize = 100_000;
/// Maximum UTF-8 bytes in an extracted client web-fetch body.
pub const MAX_WEB_FETCH_BODY_BYTES: usize = MAX_WEB_FETCH_BODY_CHARS * 4;
/// Maximum redirect records retained by a normalized client web-fetch result.
pub const MAX_WEB_FETCH_REDIRECTS: usize = 10;

const MAX_PROVIDER_CONTINUATION_ID_BYTES: usize = 128;
const MAX_HOSTED_TOOL_ERROR_CODE_BYTES: usize = 128;
const MAX_HOSTED_TOOL_ERROR_MESSAGE_BYTES: usize = 4096;

/// Why a normalized client web-fetch body was truncated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebFetchTruncation {
    /// Compressed bytes reached the transport bound.
    CompressedBytes,
    /// Decoded bytes reached the decoding bound.
    DecodedBytes,
    /// Extracted body characters reached the request bound.
    BodyCharacters,
    /// Content extraction reached its parser-complexity bound.
    ParserComplexity,
}

/// One bounded redirect in a normalized client web-fetch result.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawWebFetchRedirect")]
pub struct WebFetchRedirect {
    from: String,
    to: String,
    status: u16,
}

impl WebFetchRedirect {
    /// Creates a normalized redirect record.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid URLs or a non-redirect status code.
    pub fn new(
        from: impl Into<String>,
        to: impl Into<String>,
        status: u16,
    ) -> Result<Self, ExternalContentError> {
        if !(300..=399).contains(&status) {
            return Err(ExternalContentError::InvalidWebFetchRedirect);
        }
        Ok(Self {
            from: normalize_web_fetch_url(&from.into())?,
            to: normalize_web_fetch_url(&to.into())?,
            status,
        })
    }

    /// Returns the normalized source URL.
    #[must_use]
    pub fn from(&self) -> &str {
        &self.from
    }

    /// Returns the normalized destination URL.
    #[must_use]
    pub fn to(&self) -> &str {
        &self.to
    }

    /// Returns the HTTP redirect status.
    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    fn validate(&self) -> Result<(), ExternalContentError> {
        if normalize_web_fetch_url(&self.from)? != self.from
            || normalize_web_fetch_url(&self.to)? != self.to
            || !(300..=399).contains(&self.status)
        {
            return Err(ExternalContentError::InvalidWebFetchRedirect);
        }
        Ok(())
    }
}

impl fmt::Debug for WebFetchRedirect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebFetchRedirect")
            .field("status", &self.status)
            .field("from_bytes", &self.from.len())
            .field("to_bytes", &self.to.len())
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawWebFetchRedirect {
    from: String,
    to: String,
    status: u16,
}

impl TryFrom<RawWebFetchRedirect> for WebFetchRedirect {
    type Error = ExternalContentError;

    fn try_from(raw: RawWebFetchRedirect) -> Result<Self, Self::Error> {
        Self::new(raw.from, raw.to, raw.status)
    }
}

/// Bounded provider-neutral presentation of one client web-fetch result.
///
/// This contains only normalized URLs, extracted text, and explicit metadata.
/// It deliberately has no provider-owned continuation or raw-response field.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawWebFetchPresentation")]
pub struct WebFetchPresentation {
    requested_url: String,
    final_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    mime_type: String,
    body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncation: Option<WebFetchTruncation>,
    redirects: Vec<WebFetchRedirect>,
}

impl WebFetchPresentation {
    /// Creates a normalized presentation with bounded metadata and extracted body.
    ///
    /// # Errors
    ///
    /// Returns an error when a URL, MIME type, or body violates durable bounds.
    pub fn new(
        requested_url: impl Into<String>,
        final_url: impl Into<String>,
        mime_type: impl Into<String>,
        body: impl Into<String>,
    ) -> Result<Self, ExternalContentError> {
        let body = body.into();
        validate_web_fetch_body(&body)?;
        Ok(Self {
            requested_url: normalize_web_fetch_url(&requested_url.into())?,
            final_url: normalize_web_fetch_url(&final_url.into())?,
            title: None,
            mime_type: normalize_web_fetch_mime(&mime_type.into())?,
            body,
            truncation: None,
            redirects: Vec::new(),
        })
    }

    /// Adds an optional bounded document title.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty, oversized, or control-containing title.
    pub fn with_title(mut self, title: impl Into<String>) -> Result<Self, ExternalContentError> {
        let title = title.into();
        validate_web_fetch_title(&title)?;
        self.title = Some(title);
        Ok(self)
    }

    /// Records why the extracted body was truncated.
    #[must_use]
    pub const fn with_truncation(mut self, truncation: WebFetchTruncation) -> Self {
        self.truncation = Some(truncation);
        self
    }

    /// Adds bounded normalized redirect metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when more than ten redirects are supplied or one is invalid.
    pub fn with_redirects(
        mut self,
        redirects: Vec<WebFetchRedirect>,
    ) -> Result<Self, ExternalContentError> {
        validate_web_fetch_redirects(&redirects)?;
        self.redirects = redirects;
        Ok(self)
    }

    /// Returns the normalized requested URL.
    #[must_use]
    pub fn requested_url(&self) -> &str {
        &self.requested_url
    }

    /// Returns the normalized final URL after redirects.
    #[must_use]
    pub fn final_url(&self) -> &str {
        &self.final_url
    }

    /// Returns the extracted document title when present.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Returns the normalized MIME type.
    #[must_use]
    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }

    /// Returns the bounded extracted body.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Returns the explicit truncation reason when present.
    #[must_use]
    pub const fn truncation(&self) -> Option<WebFetchTruncation> {
        self.truncation
    }

    /// Returns normalized redirects in request order.
    #[must_use]
    pub fn redirects(&self) -> &[WebFetchRedirect] {
        &self.redirects
    }
}

impl fmt::Debug for WebFetchPresentation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebFetchPresentation")
            .field("mime_type", &self.mime_type)
            .field("body_chars", &self.body.chars().count())
            .field("has_title", &self.title.is_some())
            .field("truncation", &self.truncation)
            .field("redirect_count", &self.redirects.len())
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawWebFetchPresentation {
    requested_url: String,
    final_url: String,
    #[serde(default)]
    title: Option<String>,
    mime_type: String,
    body: String,
    #[serde(default)]
    truncation: Option<WebFetchTruncation>,
    #[serde(default)]
    redirects: Vec<WebFetchRedirect>,
}

impl TryFrom<RawWebFetchPresentation> for WebFetchPresentation {
    type Error = ExternalContentError;

    fn try_from(raw: RawWebFetchPresentation) -> Result<Self, Self::Error> {
        let mut presentation =
            Self::new(raw.requested_url, raw.final_url, raw.mime_type, raw.body)?;
        if let Some(title) = raw.title {
            presentation = presentation.with_title(title)?;
        }
        if let Some(truncation) = raw.truncation {
            presentation = presentation.with_truncation(truncation);
        }
        presentation.with_redirects(raw.redirects)
    }
}

/// A normalized external source returned by search or another hosted tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawExternalSource")]
pub struct ExternalSource {
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snippet: Option<String>,
}

impl ExternalSource {
    /// Creates a source with a bounded HTTP(S) URL.
    ///
    /// # Errors
    ///
    /// Returns an error for non-HTTP(S), empty, oversized, or control-containing URLs.
    pub fn new(url: impl Into<String>) -> Result<Self, ExternalContentError> {
        let url = normalize_source_url(&url.into())?;
        Ok(Self {
            url,
            title: None,
            snippet: None,
        })
    }

    /// Adds a bounded, control-free source title.
    ///
    /// # Errors
    ///
    /// Returns an error when the title is empty, oversized, or contains controls.
    pub fn with_title(mut self, title: impl Into<String>) -> Result<Self, ExternalContentError> {
        let title = title.into();
        if title.is_empty()
            || title.len() > MAX_EXTERNAL_SOURCE_TITLE_BYTES
            || title.chars().any(char::is_control)
        {
            return Err(ExternalContentError::InvalidSourceTitle);
        }
        self.title = Some(title);
        Ok(self)
    }

    /// Adds a bounded source snippet.
    ///
    /// # Errors
    ///
    /// Returns an error when the snippet is empty, oversized, or contains a null character.
    pub fn with_snippet(
        mut self,
        snippet: impl Into<String>,
    ) -> Result<Self, ExternalContentError> {
        let snippet = snippet.into();
        validate_source_text(&snippet)?;
        self.snippet = Some(snippet);
        Ok(self)
    }

    /// Returns the source URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Returns the optional source title.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Returns the optional source snippet.
    #[must_use]
    pub fn snippet(&self) -> Option<&str> {
        self.snippet.as_deref()
    }

    pub(crate) fn validate(&self) -> Result<(), ExternalContentError> {
        if normalize_source_url(&self.url)? != self.url {
            return Err(ExternalContentError::InvalidSourceUrl);
        }
        if let Some(title) = self.title.as_deref()
            && (title.is_empty()
                || title.len() > MAX_EXTERNAL_SOURCE_TITLE_BYTES
                || title.chars().any(char::is_control))
        {
            return Err(ExternalContentError::InvalidSourceTitle);
        }
        if let Some(snippet) = self.snippet.as_deref() {
            validate_source_text(snippet)?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawExternalSource {
    url: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
}

impl TryFrom<RawExternalSource> for ExternalSource {
    type Error = ExternalContentError;

    fn try_from(raw: RawExternalSource) -> Result<Self, Self::Error> {
        let mut source = Self::new(raw.url)?;
        if let Some(title) = raw.title {
            source = source.with_title(title)?;
        }
        if let Some(snippet) = raw.snippet {
            source = source.with_snippet(snippet)?;
        }
        Ok(source)
    }
}

/// Bounded provider-owned data needed to reconstruct a later request.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawProviderContinuation")]
pub struct ProviderContinuation {
    provider: String,
    format: String,
    payload: Value,
}

impl ProviderContinuation {
    /// Creates a bounded opaque continuation envelope.
    ///
    /// # Errors
    ///
    /// Returns an error for non-canonical identifiers or JSON outside protocol bounds.
    pub fn new(
        provider: impl Into<String>,
        format: impl Into<String>,
        payload: Value,
    ) -> Result<Self, ExternalContentError> {
        let provider = provider.into();
        let format = format.into();
        validate_continuation_id(&provider)?;
        validate_continuation_id(&format)?;
        validate_json_bounds(
            &payload,
            MAX_PROVIDER_CONTINUATION_BYTES,
            MAX_PROVIDER_CONTINUATION_DEPTH,
        )?;
        Ok(Self {
            provider,
            format,
            payload,
        })
    }

    /// Returns the adapter provider identifier.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Returns the adapter-owned payload format identifier.
    #[must_use]
    pub fn format(&self) -> &str {
        &self.format
    }

    /// Returns the opaque payload for a matching provider adapter.
    #[must_use]
    pub const fn payload(&self) -> &Value {
        &self.payload
    }

    pub(crate) fn validate(&self) -> Result<(), ExternalContentError> {
        validate_continuation_id(&self.provider)?;
        validate_continuation_id(&self.format)?;
        validate_json_bounds(
            &self.payload,
            MAX_PROVIDER_CONTINUATION_BYTES,
            MAX_PROVIDER_CONTINUATION_DEPTH,
        )?;
        Ok(())
    }
}

impl fmt::Debug for ProviderContinuation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderContinuation")
            .field("provider", &self.provider)
            .field("format", &self.format)
            .field("payload", &"**REDACTED**")
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProviderContinuation {
    provider: String,
    format: String,
    payload: Value,
}

impl TryFrom<RawProviderContinuation> for ProviderContinuation {
    type Error = ExternalContentError;

    fn try_from(raw: RawProviderContinuation) -> Result<Self, Self::Error> {
        Self::new(raw.provider, raw.format, raw.payload)
    }
}

/// Provider-reported hosted tool failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawHostedToolError")]
pub struct HostedToolError {
    code: String,
    message: String,
}

impl HostedToolError {
    /// Creates a bounded machine-readable hosted tool error.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-canonical code or invalid message.
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<Self, ExternalContentError> {
        let code = code.into();
        let message = message.into();
        if code.is_empty()
            || code.len() > MAX_HOSTED_TOOL_ERROR_CODE_BYTES
            || !code.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'_' | b'-' | b'.')
            })
        {
            return Err(ExternalContentError::InvalidHostedToolError);
        }
        if message.is_empty()
            || message.len() > MAX_HOSTED_TOOL_ERROR_MESSAGE_BYTES
            || message.contains('\0')
        {
            return Err(ExternalContentError::InvalidHostedToolError);
        }
        Ok(Self { code, message })
    }

    /// Returns the stable provider-neutral error code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the bounded diagnostic message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawHostedToolError {
    code: String,
    message: String,
}

impl TryFrom<RawHostedToolError> for HostedToolError {
    type Error = ExternalContentError;

    fn try_from(raw: RawHostedToolError) -> Result<Self, Self::Error> {
        Self::new(raw.code, raw.message)
    }
}

/// Terminal provider-hosted tool outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "error", rename_all = "snake_case")]
pub enum HostedToolOutcome {
    /// The provider completed the activity.
    Success,
    /// The provider reported a tool-level error, possibly inside HTTP 200.
    Error(HostedToolError),
}

/// One complete provider-hosted tool activity retained in assistant content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawHostedToolActivity")]
pub struct HostedToolActivity {
    tool_call_id: ToolCallId,
    provider_call_id: String,
    tool_name: String,
    arguments: Value,
    outcome: HostedToolOutcome,
    sources: Vec<ExternalSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    continuation: Option<ProviderContinuation>,
}

impl HostedToolActivity {
    /// Creates a validated complete hosted tool activity.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid identity, arguments, source count, or continuation data.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tool_call_id: ToolCallId,
        provider_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        arguments: Value,
        outcome: HostedToolOutcome,
        sources: Vec<ExternalSource>,
        continuation: Option<ProviderContinuation>,
    ) -> Result<Self, ExternalContentError> {
        let provider_call_id = provider_call_id.into();
        let tool_name = tool_name.into();
        validate_provider_tool_call_id(&provider_call_id)
            .map_err(|_| ExternalContentError::InvalidHostedToolIdentity)?;
        validate_tool_name(&tool_name)
            .map_err(|_| ExternalContentError::InvalidHostedToolIdentity)?;
        if !arguments.is_object() {
            return Err(ExternalContentError::HostedToolArgumentsMustBeObject);
        }
        validate_json_bounds(&arguments, MAX_TOOL_ARGUMENT_BYTES, MAX_TOOL_ARGUMENT_DEPTH)?;
        if sources.len() > MAX_HOSTED_TOOL_SOURCES {
            return Err(ExternalContentError::TooManyHostedToolSources);
        }
        for source in &sources {
            source.validate()?;
        }
        if let Some(continuation) = continuation.as_ref() {
            continuation.validate()?;
        }
        Ok(Self {
            tool_call_id,
            provider_call_id,
            tool_name,
            arguments,
            outcome,
            sources,
            continuation,
        })
    }

    /// Returns the canonical activity identifier.
    #[must_use]
    pub const fn tool_call_id(&self) -> ToolCallId {
        self.tool_call_id
    }

    /// Returns the provider-owned activity identifier.
    #[must_use]
    pub fn provider_call_id(&self) -> &str {
        &self.provider_call_id
    }

    /// Returns the stable registered tool name.
    #[must_use]
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    /// Returns normalized provider-neutral arguments.
    #[must_use]
    pub const fn arguments(&self) -> &Value {
        &self.arguments
    }

    /// Returns the terminal hosted outcome.
    #[must_use]
    pub const fn outcome(&self) -> &HostedToolOutcome {
        &self.outcome
    }

    /// Returns normalized sources in provider order.
    #[must_use]
    pub fn sources(&self) -> &[ExternalSource] {
        &self.sources
    }

    /// Returns provider-owned continuation data for matching adapters.
    #[must_use]
    pub const fn continuation(&self) -> Option<&ProviderContinuation> {
        self.continuation.as_ref()
    }

    pub(crate) fn validate(&self) -> Result<(), ExternalContentError> {
        Self::new(
            self.tool_call_id,
            self.provider_call_id.clone(),
            self.tool_name.clone(),
            self.arguments.clone(),
            self.outcome.clone(),
            self.sources.clone(),
            self.continuation.clone(),
        )?;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawHostedToolActivity {
    tool_call_id: ToolCallId,
    provider_call_id: String,
    tool_name: String,
    arguments: Value,
    outcome: HostedToolOutcome,
    sources: Vec<ExternalSource>,
    #[serde(default)]
    continuation: Option<ProviderContinuation>,
}

impl TryFrom<RawHostedToolActivity> for HostedToolActivity {
    type Error = ExternalContentError;

    fn try_from(raw: RawHostedToolActivity) -> Result<Self, Self::Error> {
        Self::new(
            raw.tool_call_id,
            raw.provider_call_id,
            raw.tool_name,
            raw.arguments,
            raw.outcome,
            raw.sources,
            raw.continuation,
        )
    }
}

/// A normalized citation associated with assistant text and an external source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawSourceCitation")]
pub struct SourceCitation {
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<ToolCallId>,
    source: ExternalSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cited_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    continuation: Option<ProviderContinuation>,
}

impl SourceCitation {
    /// Creates a citation with no text range or provider continuation.
    #[must_use]
    pub fn new(source: ExternalSource) -> Self {
        Self {
            tool_call_id: None,
            source,
            start_index: None,
            end_index: None,
            cited_text: None,
            continuation: None,
        }
    }

    /// Links the citation to one hosted tool activity.
    #[must_use]
    pub const fn with_tool_call_id(mut self, tool_call_id: ToolCallId) -> Self {
        self.tool_call_id = Some(tool_call_id);
        self
    }

    /// Adds a half-open UTF-8 byte range in the associated assistant text.
    ///
    /// # Errors
    ///
    /// Returns an error when the range is empty or reversed.
    pub fn with_range(
        mut self,
        start_index: u32,
        end_index: u32,
    ) -> Result<Self, ExternalContentError> {
        if start_index >= end_index {
            return Err(ExternalContentError::InvalidCitationRange);
        }
        self.start_index = Some(start_index);
        self.end_index = Some(end_index);
        Ok(self)
    }

    /// Adds bounded provider-supplied cited text.
    ///
    /// # Errors
    ///
    /// Returns an error for empty, oversized, or null-containing text.
    pub fn with_cited_text(
        mut self,
        cited_text: impl Into<String>,
    ) -> Result<Self, ExternalContentError> {
        let cited_text = cited_text.into();
        validate_source_text(&cited_text)?;
        self.cited_text = Some(cited_text);
        Ok(self)
    }

    /// Adds opaque provider citation state for same-provider continuation.
    #[must_use]
    pub fn with_continuation(mut self, continuation: ProviderContinuation) -> Self {
        self.continuation = Some(continuation);
        self
    }

    /// Returns the linked hosted tool activity, when known.
    #[must_use]
    pub const fn tool_call_id(&self) -> Option<ToolCallId> {
        self.tool_call_id
    }

    /// Returns the normalized cited source.
    #[must_use]
    pub const fn source(&self) -> &ExternalSource {
        &self.source
    }

    /// Returns the optional half-open text range.
    #[must_use]
    pub const fn range(&self) -> Option<(u32, u32)> {
        match (self.start_index, self.end_index) {
            (Some(start), Some(end)) => Some((start, end)),
            _ => None,
        }
    }

    /// Returns optional cited text supplied by the provider.
    #[must_use]
    pub fn cited_text(&self) -> Option<&str> {
        self.cited_text.as_deref()
    }

    /// Returns opaque provider citation state.
    #[must_use]
    pub const fn continuation(&self) -> Option<&ProviderContinuation> {
        self.continuation.as_ref()
    }

    pub(crate) fn validate(&self) -> Result<(), ExternalContentError> {
        self.source.validate()?;
        if matches!(
            (self.start_index, self.end_index),
            (Some(start), Some(end)) if start >= end
        ) || self.start_index.is_some() != self.end_index.is_some()
        {
            return Err(ExternalContentError::InvalidCitationRange);
        }
        if let Some(cited_text) = self.cited_text.as_deref() {
            validate_source_text(cited_text)?;
        }
        if let Some(continuation) = self.continuation.as_ref() {
            continuation.validate()?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSourceCitation {
    #[serde(default)]
    tool_call_id: Option<ToolCallId>,
    source: ExternalSource,
    #[serde(default)]
    start_index: Option<u32>,
    #[serde(default)]
    end_index: Option<u32>,
    #[serde(default)]
    cited_text: Option<String>,
    #[serde(default)]
    continuation: Option<ProviderContinuation>,
}

impl TryFrom<RawSourceCitation> for SourceCitation {
    type Error = ExternalContentError;

    fn try_from(raw: RawSourceCitation) -> Result<Self, Self::Error> {
        let mut citation = Self::new(raw.source);
        if let Some(tool_call_id) = raw.tool_call_id {
            citation = citation.with_tool_call_id(tool_call_id);
        }
        match (raw.start_index, raw.end_index) {
            (Some(start), Some(end)) => citation = citation.with_range(start, end)?,
            (None, None) => {}
            _ => return Err(ExternalContentError::InvalidCitationRange),
        }
        if let Some(cited_text) = raw.cited_text {
            citation = citation.with_cited_text(cited_text)?;
        }
        if let Some(continuation) = raw.continuation {
            citation = citation.with_continuation(continuation);
        }
        citation.validate()?;
        Ok(citation)
    }
}

/// Validation failure for hosted tool, source, citation, or continuation content.
#[derive(Debug, Error)]
pub enum ExternalContentError {
    /// Source URL is not a bounded HTTP(S) URL.
    #[error("external source URL is invalid")]
    InvalidSourceUrl,
    /// Source title is empty, oversized, or contains controls.
    #[error("external source title is invalid")]
    InvalidSourceTitle,
    /// Source snippet or cited text is empty, oversized, or contains a null character.
    #[error("external source text is invalid")]
    InvalidSourceText,
    /// Provider or continuation format identifier is not canonical.
    #[error("provider continuation identifier is invalid")]
    InvalidContinuationIdentifier,
    /// Hosted tool provider id or tool name is invalid.
    #[error("hosted tool identity is invalid")]
    InvalidHostedToolIdentity,
    /// Hosted tool arguments must be a JSON object.
    #[error("hosted tool arguments must be a JSON object")]
    HostedToolArgumentsMustBeObject,
    /// Hosted tool source collection exceeds protocol bounds.
    #[error("hosted tool returned too many sources")]
    TooManyHostedToolSources,
    /// Hosted tool error code or message is invalid.
    #[error("hosted tool error is invalid")]
    InvalidHostedToolError,
    /// Citation range is missing one endpoint, empty, or reversed.
    #[error("source citation range is invalid")]
    InvalidCitationRange,
    /// Client web-fetch metadata, MIME type, or URL is invalid.
    #[error("web fetch presentation metadata is invalid")]
    InvalidWebFetchMetadata,
    /// Client web-fetch title is empty, oversized, or contains controls.
    #[error("web fetch presentation title is invalid")]
    InvalidWebFetchTitle,
    /// Client web-fetch extracted body exceeds durable bounds or contains a null.
    #[error("web fetch presentation body is invalid")]
    InvalidWebFetchBody,
    /// Client web-fetch redirect metadata is invalid or exceeds its count bound.
    #[error("web fetch presentation redirect is invalid")]
    InvalidWebFetchRedirect,
    /// JSON payload exceeds encoded size or nesting limits.
    #[error("external content JSON exceeds protocol bounds: {0}")]
    JsonBounds(#[from] ProtocolMetadataError),
}

fn normalize_source_url(url: &str) -> Result<String, ExternalContentError> {
    if url.is_empty()
        || url.len() > MAX_EXTERNAL_SOURCE_URL_BYTES
        || url.chars().any(char::is_control)
        || url.contains(' ')
    {
        return Err(ExternalContentError::InvalidSourceUrl);
    }
    let parsed = url::Url::parse(url).map_err(|_| ExternalContentError::InvalidSourceUrl)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.cannot_be_a_base()
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(ExternalContentError::InvalidSourceUrl);
    }
    let normalized = parsed.to_string();
    if normalized.len() > MAX_EXTERNAL_SOURCE_URL_BYTES {
        return Err(ExternalContentError::InvalidSourceUrl);
    }
    Ok(normalized)
}

fn normalize_web_fetch_url(url: &str) -> Result<String, ExternalContentError> {
    if url.is_empty()
        || url.len() > MAX_WEB_FETCH_URL_BYTES
        || url.chars().any(char::is_control)
        || url.contains(' ')
    {
        return Err(ExternalContentError::InvalidWebFetchMetadata);
    }
    let mut parsed =
        url::Url::parse(url).map_err(|_| ExternalContentError::InvalidWebFetchMetadata)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.cannot_be_a_base()
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(ExternalContentError::InvalidWebFetchMetadata);
    }
    parsed.set_fragment(None);
    let normalized = parsed.to_string();
    if normalized.len() > MAX_WEB_FETCH_URL_BYTES {
        return Err(ExternalContentError::InvalidWebFetchMetadata);
    }
    Ok(normalized)
}

fn normalize_web_fetch_mime(value: &str) -> Result<String, ExternalContentError> {
    let valid = !value.is_empty()
        && value.len() <= MAX_WEB_FETCH_MIME_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'/' | b'.' | b'+' | b'-' | b';' | b'=' | b' ')
        });
    valid
        .then(|| value.to_ascii_lowercase())
        .ok_or(ExternalContentError::InvalidWebFetchMetadata)
}

fn validate_web_fetch_title(title: &str) -> Result<(), ExternalContentError> {
    if title.is_empty()
        || title.len() > MAX_WEB_FETCH_TITLE_BYTES
        || title.chars().any(char::is_control)
    {
        Err(ExternalContentError::InvalidWebFetchTitle)
    } else {
        Ok(())
    }
}

fn validate_web_fetch_body(body: &str) -> Result<(), ExternalContentError> {
    if body.len() > MAX_WEB_FETCH_BODY_BYTES
        || body.chars().count() > MAX_WEB_FETCH_BODY_CHARS
        || body.contains('\0')
    {
        Err(ExternalContentError::InvalidWebFetchBody)
    } else {
        Ok(())
    }
}

fn validate_web_fetch_redirects(
    redirects: &[WebFetchRedirect],
) -> Result<(), ExternalContentError> {
    if redirects.len() > MAX_WEB_FETCH_REDIRECTS {
        return Err(ExternalContentError::InvalidWebFetchRedirect);
    }
    for redirect in redirects {
        redirect.validate()?;
    }
    Ok(())
}

fn validate_source_text(text: &str) -> Result<(), ExternalContentError> {
    if text.is_empty() || text.len() > MAX_EXTERNAL_SOURCE_TEXT_BYTES || text.contains('\0') {
        Err(ExternalContentError::InvalidSourceText)
    } else {
        Ok(())
    }
}

fn validate_continuation_id(value: &str) -> Result<(), ExternalContentError> {
    let mut bytes = value.bytes();
    if value.is_empty()
        || value.len() > MAX_PROVIDER_CONTINUATION_ID_BYTES
        || !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        || !bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
    {
        Err(ExternalContentError::InvalidContinuationIdentifier)
    } else {
        Ok(())
    }
}
