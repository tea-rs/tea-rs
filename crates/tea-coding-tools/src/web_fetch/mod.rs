use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use futures_util::stream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tea_control::CancellationScope;
use tea_protocol::{
    ContentBlock, ProtocolMetadata, ToolIdempotency, ToolPresentation, WebFetchPresentation,
    WebFetchRedirect, WebFetchTruncation,
};
use tea_tools::{
    BoxToolExecutionStream, ToolConcurrency, ToolEffect, ToolExecutionEvent, ToolExecutionFailure,
    ToolExecutionSemantics, ToolExecutor, ToolName, ToolResult, ToolRetrySafety, ToolSpec,
    ToolSpecError, ToolTimeout, ToolVersion, ValidatedToolInvocation,
};
use thiserror::Error;

mod address_policy;
mod body;
mod cache;
mod http;
mod provider;
mod url_policy;

pub use address_policy::{FetchAddressPolicy, FetchAddressPolicyError, ValidatedFetchAddresses};
pub use body::{
    DEFAULT_FETCH_DECODED_BYTES, DEFAULT_FETCH_HTML_BYTES, DEFAULT_FETCH_HTML_ELEMENTS,
    DecodedFetchBody, FetchBodyDecoder, FetchBodyLimits, FetchContentKind, MAX_FETCH_DECODED_BYTES,
};
pub use cache::{
    DEFAULT_FETCH_CACHE_ENTRIES, DEFAULT_FETCH_CACHE_ENTRY_BYTES, DEFAULT_FETCH_CACHE_TOTAL_BYTES,
    DEFAULT_FETCH_CACHE_TTL, FetchCacheConfig, FetchCacheScope, FetchCacheStats, FetchResultCache,
    MAX_FETCH_CACHE_ENTRIES, MAX_FETCH_CACHE_ENTRY_BYTES, MAX_FETCH_CACHE_TOTAL_BYTES,
    MAX_FETCH_CACHE_TTL,
};
pub use http::{
    DEFAULT_FETCH_RESPONSE_BYTES, FetchDnsResolver, FetchHttpConfig, FetchHttpHeaders,
    FetchHttpLimits, FetchHttpResponse, FetchHttpTimeouts, FetchHttpTransport, FetchResolveFuture,
    MAX_FETCH_RESPONSE_BYTES, SystemFetchDnsResolver,
};
pub use provider::{FETCH_SECURITY_POLICY_VERSION, HttpFetchProvider};
pub use url_policy::{FetchUrlPolicy, FetchUrlPolicyError, FetchUrlScheme, ValidatedFetchUrl};

/// Maximum UTF-8 bytes accepted in a fetch URL.
pub const MAX_FETCH_URL_BYTES: usize = 2 * 1024;
/// Default number of body characters returned by the fetch tool.
pub const DEFAULT_FETCH_MAX_CHARS: usize = 20_000;
/// Maximum body characters retained by one fetch result.
pub const MAX_FETCH_MAX_CHARS: usize = 100_000;
/// Maximum title bytes retained by one fetch result.
pub const MAX_FETCH_TITLE_BYTES: usize = 4 * 1024;
/// Maximum MIME type bytes retained by one fetch result.
pub const MAX_FETCH_MIME_BYTES: usize = 128;
/// Maximum redirect records retained by one fetch result.
pub const MAX_FETCH_REDIRECTS: usize = 10;

/// Invalid provider-neutral fetch request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum FetchRequestError {
    /// URL is empty, oversized, malformed, or outside the initial scheme contract.
    #[error("web fetch URL is invalid")]
    InvalidUrl,
    /// Requested body character limit is outside the bounded contract.
    #[error("web fetch character limit is invalid")]
    InvalidMaxChars,
    /// The request contains an unsupported field or shape.
    #[error("web fetch request shape is invalid")]
    InvalidShape,
}

/// Bounded provider-neutral request passed to a client fetch backend.
#[derive(Clone, PartialEq, Eq)]
pub struct FetchRequest {
    url: String,
    max_chars: usize,
}

impl FetchRequest {
    /// Creates and validates a fetch request.
    ///
    /// DNS, IP-range checks, and per-hop connection authorization still belong
    /// to the HTTP transport. This constructor freezes the production URL
    /// syntax and scheme contract before the request reaches a provider.
    ///
    /// # Errors
    ///
    /// Rejects non-HTTPS URLs, credentials, unsafe ports, controls, and character
    /// limits outside `1..=100_000`. Fragments are removed from the canonical URL.
    pub fn new(url: impl Into<String>, max_chars: usize) -> Result<Self, FetchRequestError> {
        Self::new_with_policy(url, max_chars, &FetchUrlPolicy::production())
    }

    /// Creates a request under an explicit URL policy.
    ///
    /// This supports transport conformance tests that use loopback fixture
    /// servers. Product tool invocations use [`Self::new`].
    ///
    /// # Errors
    ///
    /// Returns the same bounded errors as [`Self::new`].
    pub fn new_with_policy(
        url: impl Into<String>,
        max_chars: usize,
        policy: &FetchUrlPolicy,
    ) -> Result<Self, FetchRequestError> {
        let raw_url = url.into();
        let url = policy
            .validate(&raw_url)
            .map_err(|_| FetchRequestError::InvalidUrl)?
            .as_str()
            .to_owned();
        if !(1..=MAX_FETCH_MAX_CHARS).contains(&max_chars) {
            return Err(FetchRequestError::InvalidMaxChars);
        }
        Ok(Self { url, max_chars })
    }

    /// Returns the normalized requested URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Returns the requested body character limit.
    #[must_use]
    pub const fn max_chars(&self) -> usize {
        self.max_chars
    }
}

impl fmt::Debug for FetchRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let validated = FetchUrlPolicy::production().validate(&self.url).ok();
        formatter
            .debug_struct("FetchRequest")
            .field("host", &validated.as_ref().map(ValidatedFetchUrl::host))
            .field("url_bytes", &self.url.len())
            .field("max_chars", &self.max_chars)
            .finish()
    }
}

/// Stable failure categories returned by client fetch backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchProviderErrorCode {
    /// Backend or credential configuration is invalid.
    InvalidConfiguration,
    /// The normalized request is invalid.
    InvalidRequest,
    /// The destination is forbidden by URL or address policy.
    ForbiddenDestination,
    /// DNS resolution failed or returned an unsafe answer.
    DnsResolution,
    /// A redirect was invalid, unsafe, cyclic, or exceeded bounds.
    RedirectRejected,
    /// The response exceeded a compressed or decoded byte limit.
    ResponseTooLarge,
    /// The response MIME type or content shape is unsupported.
    UnsupportedMime,
    /// The backend timed out.
    Timeout,
    /// The backend could not be reached or the peer was not the pinned address.
    Transport,
    /// The response was malformed or failed bounded extraction.
    MalformedResponse,
    /// Cooperative cancellation was requested.
    Cancelled,
}

/// Whether an upper layer may explicitly retry a failed fetch operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchRetryDisposition {
    /// Retrying could repeat an unsafe or deterministic failure.
    Never,
    /// A caller may explicitly retry after a pre-response transport failure.
    Explicit,
}

/// Secret-independent client fetch failure.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FetchProviderError {
    code: FetchProviderErrorCode,
    message: &'static str,
    retry: FetchRetryDisposition,
}

impl FetchProviderError {
    /// Creates a failure with a stable, non-sensitive message.
    #[must_use]
    pub const fn new(code: FetchProviderErrorCode) -> Self {
        Self {
            code,
            message: fetch_error_message(code),
            retry: FetchRetryDisposition::Never,
        }
    }

    pub(crate) const fn explicit_retry(code: FetchProviderErrorCode) -> Self {
        Self {
            code,
            message: fetch_error_message(code),
            retry: FetchRetryDisposition::Explicit,
        }
    }

    /// Creates a cooperative cancellation failure.
    #[must_use]
    pub const fn cancelled() -> Self {
        Self::new(FetchProviderErrorCode::Cancelled)
    }

    /// Returns the stable category.
    #[must_use]
    pub const fn code(&self) -> FetchProviderErrorCode {
        self.code
    }

    /// Returns a bounded secret-independent technical message.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }

    /// Returns the conservative retry classification.
    #[must_use]
    pub const fn retry_disposition(&self) -> FetchRetryDisposition {
        self.retry
    }
}

impl fmt::Debug for FetchProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FetchProviderError")
            .field("code", &self.code)
            .field("message", &self.message)
            .field("retry", &self.retry)
            .finish()
    }
}

impl fmt::Display for FetchProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for FetchProviderError {}

const fn fetch_error_message(code: FetchProviderErrorCode) -> &'static str {
    match code {
        FetchProviderErrorCode::InvalidConfiguration => {
            "web fetch backend configuration is invalid"
        }
        FetchProviderErrorCode::InvalidRequest => "web fetch request is invalid",
        FetchProviderErrorCode::ForbiddenDestination => "web fetch destination is forbidden",
        FetchProviderErrorCode::DnsResolution => {
            "web fetch destination could not be resolved safely"
        }
        FetchProviderErrorCode::RedirectRejected => "web fetch redirect was rejected",
        FetchProviderErrorCode::ResponseTooLarge => "web fetch response exceeded its bounds",
        FetchProviderErrorCode::UnsupportedMime => "web fetch response MIME type is unsupported",
        FetchProviderErrorCode::Timeout => "web fetch backend timed out",
        FetchProviderErrorCode::Transport => "web fetch backend transport failed",
        FetchProviderErrorCode::MalformedResponse => "web fetch response is malformed",
        FetchProviderErrorCode::Cancelled => "web fetch was cancelled",
    }
}

/// Why a fetch body was truncated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchTruncationReason {
    /// Compressed bytes exceeded the transport bound.
    CompressedBytes,
    /// Decoded bytes exceeded the transport bound.
    DecodedBytes,
    /// Extracted body characters reached the request limit.
    BodyCharacters,
    /// The content parser reached its complexity or structure bound.
    ParserComplexity,
}

/// One bounded redirect record.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchRedirect {
    from: String,
    to: String,
    status: u16,
}

impl FetchRedirect {
    /// Creates a redirect record.
    ///
    /// # Errors
    ///
    /// Rejects malformed URLs, non-redirect status codes, and oversized values.
    pub fn new(
        from: impl Into<String>,
        to: impl Into<String>,
        status: u16,
    ) -> Result<Self, FetchProviderError> {
        Self::new_with_policy(from, to, status, &FetchUrlPolicy::production())
    }

    /// Creates a redirect record under an explicit URL policy.
    ///
    /// # Errors
    ///
    /// Returns the same bounded errors as [`Self::new`].
    pub fn new_with_policy(
        from: impl Into<String>,
        to: impl Into<String>,
        status: u16,
        policy: &FetchUrlPolicy,
    ) -> Result<Self, FetchProviderError> {
        if !(300..=399).contains(&status) {
            return Err(FetchProviderError::new(
                FetchProviderErrorCode::MalformedResponse,
            ));
        }
        let raw_from = from.into();
        let raw_to = to.into();
        let from = normalize_result_url(&raw_from, *policy)?;
        let to = normalize_result_url(&raw_to, *policy)?;
        Ok(Self { from, to, status })
    }

    /// Returns the source URL.
    #[must_use]
    pub fn from(&self) -> &str {
        &self.from
    }

    /// Returns the redirect target URL.
    #[must_use]
    pub fn to(&self) -> &str {
        &self.to
    }

    /// Returns the HTTP redirect status.
    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }
}

impl fmt::Debug for FetchRedirect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FetchRedirect")
            .field("status", &self.status)
            .field("from_bytes", &self.from.len())
            .field("to_bytes", &self.to.len())
            .finish()
    }
}

/// Bounded normalized result returned by a client fetch backend.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchResult {
    requested_url: String,
    final_url: String,
    title: Option<String>,
    mime_type: String,
    body: String,
    truncated: Option<FetchTruncationReason>,
    redirects: Vec<FetchRedirect>,
}

impl FetchResult {
    /// Creates a result with bounded URLs, MIME, and body content.
    ///
    /// # Errors
    ///
    /// Rejects malformed URLs, unsupported MIME syntax, oversized body/title
    /// values, and more than ten redirect records.
    pub fn new(
        requested_url: impl Into<String>,
        final_url: impl Into<String>,
        mime_type: impl Into<String>,
        body: impl Into<String>,
    ) -> Result<Self, FetchProviderError> {
        Self::new_with_policy(
            requested_url,
            final_url,
            mime_type,
            body,
            &FetchUrlPolicy::production(),
        )
    }

    /// Creates a result under an explicit URL policy.
    ///
    /// This supports deterministic loopback transport tests without weakening
    /// the production constructor.
    ///
    /// # Errors
    ///
    /// Returns the same bounded errors as [`Self::new`].
    pub fn new_with_policy(
        requested_url: impl Into<String>,
        final_url: impl Into<String>,
        mime_type: impl Into<String>,
        body: impl Into<String>,
        policy: &FetchUrlPolicy,
    ) -> Result<Self, FetchProviderError> {
        let raw_requested_url = requested_url.into();
        let raw_final_url = final_url.into();
        let requested_url = normalize_result_url(&raw_requested_url, *policy)?;
        let final_url = normalize_result_url(&raw_final_url, *policy)?;
        let raw_mime_type = mime_type.into();
        let mime_type = validate_mime_type(&raw_mime_type)?;
        let body = body.into();
        if body.chars().count() > MAX_FETCH_MAX_CHARS || body.contains('\0') {
            return Err(FetchProviderError::new(
                FetchProviderErrorCode::ResponseTooLarge,
            ));
        }
        Ok(Self {
            requested_url,
            final_url,
            title: None,
            mime_type,
            body,
            truncated: None,
            redirects: Vec::new(),
        })
    }

    /// Adds an optional bounded document title.
    ///
    /// # Errors
    ///
    /// Rejects control characters or titles over 4 KiB.
    pub fn with_title(mut self, title: impl Into<String>) -> Result<Self, FetchProviderError> {
        let title = title.into();
        if title.is_empty()
            || title.len() > MAX_FETCH_TITLE_BYTES
            || title.chars().any(char::is_control)
        {
            return Err(FetchProviderError::new(
                FetchProviderErrorCode::MalformedResponse,
            ));
        }
        self.title = Some(title);
        Ok(self)
    }

    /// Marks the result as truncated for a bounded reason.
    #[must_use]
    pub const fn with_truncation(mut self, reason: FetchTruncationReason) -> Self {
        self.truncated = Some(reason);
        self
    }

    /// Adds bounded redirect metadata.
    ///
    /// # Errors
    ///
    /// Rejects more than ten records.
    pub fn with_redirects(
        mut self,
        redirects: Vec<FetchRedirect>,
    ) -> Result<Self, FetchProviderError> {
        if redirects.len() > MAX_FETCH_REDIRECTS {
            return Err(FetchProviderError::new(
                FetchProviderErrorCode::MalformedResponse,
            ));
        }
        self.redirects = redirects;
        Ok(self)
    }

    /// Returns the originally requested URL.
    #[must_use]
    pub fn requested_url(&self) -> &str {
        &self.requested_url
    }

    /// Returns the final URL after bounded redirects.
    #[must_use]
    pub fn final_url(&self) -> &str {
        &self.final_url
    }

    /// Returns the optional extracted document title.
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

    /// Returns the truncation reason, if the body was bounded.
    #[must_use]
    pub const fn truncation(&self) -> Option<FetchTruncationReason> {
        self.truncated
    }

    /// Returns normalized redirect records.
    #[must_use]
    pub fn redirects(&self) -> &[FetchRedirect] {
        &self.redirects
    }
}

impl fmt::Debug for FetchResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FetchResult")
            .field("mime_type", &self.mime_type)
            .field("body_chars", &self.body.chars().count())
            .field("has_title", &self.title.is_some())
            .field("truncated", &self.truncated)
            .field("redirect_count", &self.redirects.len())
            .finish_non_exhaustive()
    }
}

fn normalize_result_url(value: &str, policy: FetchUrlPolicy) -> Result<String, FetchProviderError> {
    policy
        .validate(value)
        .map(|validated| validated.as_str().to_owned())
        .map_err(|_| FetchProviderError::new(FetchProviderErrorCode::MalformedResponse))
}

fn validate_mime_type(value: &str) -> Result<String, FetchProviderError> {
    let valid = !value.is_empty()
        && value.len() <= MAX_FETCH_MIME_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'/' | b'.' | b'+' | b'-' | b';' | b'=' | b' ')
        });
    if valid {
        Ok(value.to_ascii_lowercase())
    } else {
        Err(FetchProviderError::new(
            FetchProviderErrorCode::MalformedResponse,
        ))
    }
}

/// Object-safe future returned by [`FetchProvider`].
pub type FetchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<FetchResult, FetchProviderError>> + Send + 'a>>;

/// Object-safe client web-fetch backend port.
pub trait FetchProvider: fmt::Debug + Send + Sync {
    /// Executes one bounded request with cooperative cancellation.
    fn fetch(&self, request: FetchRequest, cancellation: CancellationScope) -> FetchFuture<'_>;
}

/// Client executor for the stable `web_fetch` tool contract.
#[derive(Debug, Clone)]
pub struct WebFetchTool {
    provider: Arc<dyn FetchProvider>,
}

impl WebFetchTool {
    /// Creates a client fetch executor.
    #[must_use]
    pub fn new(provider: Arc<dyn FetchProvider>) -> Self {
        Self { provider }
    }

    /// Builds the strict client fetch tool contract.
    ///
    /// # Errors
    ///
    /// Returns an error only if the static contract violates tool bounds.
    pub fn spec() -> Result<ToolSpec, ToolSpecError> {
        ToolSpec::new(
            ToolName::from_str("web_fetch").map_err(|_| ToolSpecError::InvalidDescription)?,
            ToolVersion::from_str("1.0.0").map_err(|_| ToolSpecError::InvalidDescription)?,
            "Fetch bounded public web content from an explicitly provided HTTPS URL.",
            json!({
                "type":"object",
                "properties":{
                    "url":{"type":"string","minLength":1,"maxLength":MAX_FETCH_URL_BYTES},
                    "max_chars":{"type":"integer","minimum":1,"maximum":MAX_FETCH_MAX_CHARS}
                },
                "required":["url"],
                "additionalProperties":false
            }),
            json!({
                "type":"object",
                "properties":{
                    "requested_url":{"type":"string"},
                    "final_url":{"type":"string"},
                    "title":{"type":["string","null"]},
                    "mime_type":{"type":"string"},
                    "body":{"type":"string"},
                    "truncated":{"type":["string","null"]},
                    "redirects":{"type":"array","maxItems":MAX_FETCH_REDIRECTS}
                },
                "required":["requested_url","final_url","title","mime_type","body","truncated","redirects"],
                "additionalProperties":false
            }),
            [ToolEffect::NetworkRequest],
            ToolExecutionSemantics::new(
                ToolIdempotency::Idempotent,
                ToolRetrySafety::ExplicitOnly,
                ToolConcurrency::Parallel,
                ToolTimeout::from_millis(60_000)?,
            )?,
        )?
        .with_prompt_hint(
            "Use web_fetch only for an explicitly requested public URL; it does not search the web.",
        )
    }

    async fn run(
        &self,
        invocation: &ValidatedToolInvocation,
        cancellation: CancellationScope,
    ) -> ToolExecutionEvent {
        if cancellation.is_cancelled() {
            return ToolExecutionEvent::Failed(ToolExecutionFailure::cancelled());
        }
        let Ok(request) = request_from_invocation(invocation) else {
            return provider_failure(FetchProviderError::new(
                FetchProviderErrorCode::InvalidRequest,
            ));
        };
        match self.provider.fetch(request, cancellation).await {
            Ok(result) => fetch_success(&result),
            Err(error) => provider_failure(error),
        }
    }
}

impl ToolExecutor for WebFetchTool {
    fn execute(
        &self,
        invocation: ValidatedToolInvocation,
        cancellation: CancellationScope,
    ) -> BoxToolExecutionStream {
        let executor = self.clone();
        Box::pin(stream::once(async move {
            executor.run(&invocation, cancellation).await
        }))
    }
}

fn request_from_invocation(
    invocation: &ValidatedToolInvocation,
) -> Result<FetchRequest, FetchRequestError> {
    let url = invocation
        .arguments()
        .get("url")
        .and_then(Value::as_str)
        .ok_or(FetchRequestError::InvalidUrl)?;
    let max_chars = invocation
        .arguments()
        .get("max_chars")
        .and_then(Value::as_u64)
        .map(usize::try_from)
        .transpose()
        .map_err(|_| FetchRequestError::InvalidMaxChars)?
        .unwrap_or(DEFAULT_FETCH_MAX_CHARS);
    FetchRequest::new(url, max_chars)
}

fn fetch_success(result: &FetchResult) -> ToolExecutionEvent {
    let redirects = result
        .redirects()
        .iter()
        .map(|redirect| {
            json!({
                "from": redirect.from(),
                "to": redirect.to(),
                "status": redirect.status(),
            })
        })
        .collect::<Vec<_>>();
    let Ok(presentation) = web_fetch_presentation(result) else {
        return ToolExecutionEvent::Failed(ToolExecutionFailure::internal_contract());
    };
    let visible = web_fetch_visible_text(&presentation);
    let output = json!({
        "requested_url": result.requested_url(),
        "final_url": result.final_url(),
        "title": result.title(),
        "mime_type": result.mime_type(),
        "body": result.body(),
        "truncated": result.truncation().map(truncation_code),
        "redirects": redirects,
    });
    let Ok(content) = ContentBlock::text(visible) else {
        return ToolExecutionEvent::Failed(ToolExecutionFailure::internal_contract());
    };
    let Ok(result) = ToolResult::new(vec![content], output) else {
        return ToolExecutionEvent::Failed(ToolExecutionFailure::internal_contract());
    };
    ToolExecutionEvent::Finished(
        result.with_presentation(ToolPresentation::WebFetch(Box::new(presentation))),
    )
}

fn web_fetch_presentation(
    result: &FetchResult,
) -> Result<WebFetchPresentation, tea_protocol::ExternalContentError> {
    let mut presentation = WebFetchPresentation::new(
        result.requested_url(),
        result.final_url(),
        result.mime_type(),
        result.body(),
    )?;
    if let Some(title) = result.title() {
        presentation = presentation.with_title(title)?;
    }
    if let Some(truncation) = result.truncation() {
        presentation = presentation.with_truncation(match truncation {
            FetchTruncationReason::CompressedBytes => WebFetchTruncation::CompressedBytes,
            FetchTruncationReason::DecodedBytes => WebFetchTruncation::DecodedBytes,
            FetchTruncationReason::BodyCharacters => WebFetchTruncation::BodyCharacters,
            FetchTruncationReason::ParserComplexity => WebFetchTruncation::ParserComplexity,
        });
    }
    let redirects = result
        .redirects()
        .iter()
        .map(|redirect| WebFetchRedirect::new(redirect.from(), redirect.to(), redirect.status()))
        .collect::<Result<Vec<_>, _>>()?;
    presentation.with_redirects(redirects)
}

fn web_fetch_visible_text(presentation: &WebFetchPresentation) -> String {
    let mut visible = format!(
        "Fetched URL: {}\nContent-Type: {}",
        presentation.final_url(),
        presentation.mime_type()
    );
    if presentation.requested_url() != presentation.final_url() {
        visible.push_str("\nRequested URL: ");
        visible.push_str(presentation.requested_url());
    }
    if let Some(title) = presentation.title() {
        visible.push_str("\nTitle: ");
        visible.push_str(title);
    }
    if let Some(truncation) = presentation.truncation() {
        visible.push_str("\nTruncated: ");
        visible.push_str(match truncation {
            WebFetchTruncation::CompressedBytes => "compressed_bytes",
            WebFetchTruncation::DecodedBytes => "decoded_bytes",
            WebFetchTruncation::BodyCharacters => "body_characters",
            WebFetchTruncation::ParserComplexity => "parser_complexity",
        });
    }
    if !presentation.redirects().is_empty() {
        visible.push_str("\nRedirects: ");
        visible.push_str(&presentation.redirects().len().to_string());
    }
    visible.push_str("\n\n");
    visible.push_str(presentation.body());
    visible
}

fn provider_failure(error: FetchProviderError) -> ToolExecutionEvent {
    if error.code() == FetchProviderErrorCode::Cancelled {
        return ToolExecutionEvent::Failed(ToolExecutionFailure::cancelled());
    }
    let details = ProtocolMetadata::from_entries([(
        "dev.tea-rs.web-fetch",
        json!({
            "code":fetch_error_code(error.code()),
            "retry":fetch_retry_code(error.retry_disposition()),
        }),
    )])
    .unwrap_or_default();
    let failure = ToolExecutionFailure::execution(error.message())
        .unwrap_or_else(|_| ToolExecutionFailure::internal_contract())
        .with_details(details);
    ToolExecutionEvent::Failed(failure)
}

const fn fetch_retry_code(disposition: FetchRetryDisposition) -> &'static str {
    match disposition {
        FetchRetryDisposition::Never => "never",
        FetchRetryDisposition::Explicit => "explicit",
    }
}

const fn fetch_error_code(code: FetchProviderErrorCode) -> &'static str {
    match code {
        FetchProviderErrorCode::InvalidConfiguration => "invalid_configuration",
        FetchProviderErrorCode::InvalidRequest => "invalid_request",
        FetchProviderErrorCode::ForbiddenDestination => "forbidden_destination",
        FetchProviderErrorCode::DnsResolution => "dns_resolution",
        FetchProviderErrorCode::RedirectRejected => "redirect_rejected",
        FetchProviderErrorCode::ResponseTooLarge => "response_too_large",
        FetchProviderErrorCode::UnsupportedMime => "unsupported_mime",
        FetchProviderErrorCode::Timeout => "timeout",
        FetchProviderErrorCode::Transport => "transport",
        FetchProviderErrorCode::MalformedResponse => "malformed_response",
        FetchProviderErrorCode::Cancelled => "cancelled",
    }
}

const fn truncation_code(reason: FetchTruncationReason) -> &'static str {
    match reason {
        FetchTruncationReason::CompressedBytes => "compressed_bytes",
        FetchTruncationReason::DecodedBytes => "decoded_bytes",
        FetchTruncationReason::BodyCharacters => "body_characters",
        FetchTruncationReason::ParserComplexity => "parser_complexity",
    }
}
