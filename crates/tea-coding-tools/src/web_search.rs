use std::collections::BTreeSet;
use std::fmt::{self, Write as _};
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{StreamExt as _, stream};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tea_control::CancellationScope;
use tea_model::WebSearchOptions;
use tea_protocol::{
    ContentBlock, ExternalSource, MAX_EXTERNAL_SOURCE_TEXT_BYTES, MAX_EXTERNAL_SOURCE_TITLE_BYTES,
    ProtocolMetadata, ToolIdempotency,
};
use tea_provider_http::ProviderHttpConfig;
use tea_tools::{
    BoxToolExecutionStream, ToolConcurrency, ToolEffect, ToolExecutionEvent, ToolExecutionFailure,
    ToolExecutionSemantics, ToolExecutor, ToolName, ToolResource, ToolResourceAccess,
    ToolResourceError, ToolResult, ToolRetrySafety, ToolSpec, ToolSpecError, ToolTimeout,
    ToolVersion, ValidatedToolInvocation,
};
use thiserror::Error;
use url::Url;

/// Default number of client search results requested from a backend.
pub const DEFAULT_WEB_SEARCH_RESULT_LIMIT: usize = 5;
/// Maximum number of client search results retained in one invocation.
pub const MAX_WEB_SEARCH_RESULT_LIMIT: usize = 20;
/// Maximum UTF-8 bytes accepted in one client search query.
pub const MAX_WEB_SEARCH_QUERY_BYTES: usize = 2_048;
/// Maximum encoded response body accepted from a client search backend.
pub const MAX_WEB_SEARCH_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
/// Default production endpoint for the Tavily Search API.
pub const DEFAULT_TAVILY_SEARCH_ENDPOINT: &str = "https://api.tavily.com/search";

const MAX_WEB_SEARCH_SNIPPET_BYTES: usize = 4 * 1024;
const MAX_TAVILY_API_KEY_BYTES: usize = 512;
const MAX_TAVILY_TIMEOUT: Duration = Duration::from_mins(1);

/// Invalid provider-neutral search request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SearchRequestError {
    /// Query text is empty, oversized, or control-containing.
    #[error("web search query is invalid")]
    InvalidQuery,
    /// Domain policy is invalid or internally conflicting.
    #[error("web search domain policy is invalid")]
    InvalidDomains,
    /// Result limit is outside the portable contract.
    #[error("web search result limit is invalid")]
    InvalidLimit,
}

/// Bounded provider-neutral request passed to a client search backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    query: String,
    options: WebSearchOptions,
    limit: usize,
}

impl SearchRequest {
    /// Creates and validates one client search request.
    ///
    /// # Errors
    ///
    /// Rejects invalid queries, conflicting domain filters, and result limits
    /// outside `1..=20`.
    pub fn new<I, S, J, T>(
        query: impl Into<String>,
        allowed_domains: I,
        blocked_domains: J,
        limit: usize,
    ) -> Result<Self, SearchRequestError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
        J: IntoIterator<Item = T>,
        T: Into<String>,
    {
        let query = validate_query_and_limit(query.into(), limit)?;
        let allowed_domains = allowed_domains
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>();
        let blocked_domains = blocked_domains
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>();
        if !allowed_domains.is_empty() && !blocked_domains.is_empty() {
            return Err(SearchRequestError::InvalidDomains);
        }
        let options = if !allowed_domains.is_empty() {
            WebSearchOptions::new()
                .with_allowed_domains(allowed_domains)
                .map_err(|_| SearchRequestError::InvalidDomains)?
        } else if !blocked_domains.is_empty() {
            WebSearchOptions::new()
                .with_blocked_domains(blocked_domains)
                .map_err(|_| SearchRequestError::InvalidDomains)?
        } else {
            WebSearchOptions::new()
        };
        Ok(Self {
            query,
            options,
            limit,
        })
    }

    /// Creates a request from the complete validated portable search options.
    ///
    /// This constructor preserves optional location data for client backends
    /// that can implement location-aware search without changing the provider
    /// port.
    ///
    /// # Errors
    ///
    /// Rejects invalid queries and result limits outside `1..=20`.
    pub fn from_options(
        query: impl Into<String>,
        options: WebSearchOptions,
        limit: usize,
    ) -> Result<Self, SearchRequestError> {
        let query = validate_query_and_limit(query.into(), limit)?;
        Ok(Self {
            query,
            options,
            limit,
        })
    }

    /// Returns the exact validated query.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Returns the deterministic domain allowlist.
    #[must_use]
    pub fn allowed_domains(&self) -> &[String] {
        self.options.allowed_domains()
    }

    /// Returns the deterministic domain blocklist.
    #[must_use]
    pub fn blocked_domains(&self) -> &[String] {
        self.options.blocked_domains()
    }

    /// Returns the optional approximate location for capable client backends.
    #[must_use]
    pub const fn location(&self) -> Option<&tea_model::WebSearchLocation> {
        self.options.location()
    }

    /// Returns the requested result limit.
    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }
}

fn validate_query_and_limit(query: String, limit: usize) -> Result<String, SearchRequestError> {
    if query.trim().is_empty()
        || query.len() > MAX_WEB_SEARCH_QUERY_BYTES
        || query.chars().any(char::is_control)
    {
        return Err(SearchRequestError::InvalidQuery);
    }
    if !(1..=MAX_WEB_SEARCH_RESULT_LIMIT).contains(&limit) {
        return Err(SearchRequestError::InvalidLimit);
    }
    Ok(query)
}

/// Stable failure category returned by a client search backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchProviderErrorCode {
    /// Backend or credential configuration is invalid.
    InvalidConfiguration,
    /// The backend rejected the normalized request.
    InvalidRequest,
    /// Credentials were rejected.
    Authentication,
    /// The backend rate limit was reached.
    RateLimited,
    /// The account quota or plan limit was reached.
    QuotaExceeded,
    /// The backend timed out.
    Timeout,
    /// The backend could not be reached.
    Transport,
    /// The backend was temporarily unavailable.
    Unavailable,
    /// The backend returned an invalid or oversized success response.
    MalformedResponse,
    /// Cooperative cancellation was requested.
    Cancelled,
}

/// Secret-independent client search failure.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SearchProviderError {
    code: SearchProviderErrorCode,
    message: &'static str,
}

impl SearchProviderError {
    /// Creates a failure with a stable, non-sensitive message.
    #[must_use]
    pub const fn new(code: SearchProviderErrorCode) -> Self {
        Self {
            code,
            message: provider_error_message(code),
        }
    }

    /// Creates a cooperative cancellation failure.
    #[must_use]
    pub const fn cancelled() -> Self {
        Self::new(SearchProviderErrorCode::Cancelled)
    }

    /// Returns the stable category.
    #[must_use]
    pub const fn code(&self) -> SearchProviderErrorCode {
        self.code
    }

    /// Returns a bounded secret-independent technical message.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }
}

impl fmt::Debug for SearchProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchProviderError")
            .field("code", &self.code)
            .field("message", &self.message)
            .finish()
    }
}

impl fmt::Display for SearchProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for SearchProviderError {}

const fn provider_error_message(code: SearchProviderErrorCode) -> &'static str {
    match code {
        SearchProviderErrorCode::InvalidConfiguration => {
            "web search backend configuration is invalid"
        }
        SearchProviderErrorCode::InvalidRequest => "web search request is invalid",
        SearchProviderErrorCode::Authentication => "web search credentials were rejected",
        SearchProviderErrorCode::RateLimited => "web search backend rate limit was reached",
        SearchProviderErrorCode::QuotaExceeded => "web search backend quota was exceeded",
        SearchProviderErrorCode::Timeout => "web search backend timed out",
        SearchProviderErrorCode::Transport => "web search backend transport failed",
        SearchProviderErrorCode::Unavailable => "web search backend is unavailable",
        SearchProviderErrorCode::MalformedResponse => "web search backend response is invalid",
        SearchProviderErrorCode::Cancelled => "web search was cancelled",
    }
}

/// One normalized client search result.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    source: ExternalSource,
    score: Option<f64>,
}

impl SearchResult {
    /// Creates a source result with an optional normalized relevance score.
    ///
    /// # Errors
    ///
    /// Rejects non-finite scores or scores outside `0.0..=1.0`.
    pub fn new(source: ExternalSource, score: Option<f64>) -> Result<Self, SearchProviderError> {
        if score.is_some_and(|score| !score.is_finite() || !(0.0..=1.0).contains(&score)) {
            return Err(SearchProviderError::new(
                SearchProviderErrorCode::MalformedResponse,
            ));
        }
        Ok(Self { source, score })
    }

    /// Returns the normalized source.
    #[must_use]
    pub const fn source(&self) -> &ExternalSource {
        &self.source
    }

    /// Returns the optional relevance score.
    #[must_use]
    pub const fn score(&self) -> Option<f64> {
        self.score
    }
}

/// Bounded provider-neutral client search response.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResponse {
    results: Vec<SearchResult>,
    truncated: bool,
}

impl SearchResponse {
    /// Creates a response containing no more than the portable result limit.
    ///
    /// # Errors
    ///
    /// Rejects responses containing more than 20 results.
    pub fn new(results: Vec<SearchResult>, truncated: bool) -> Result<Self, SearchProviderError> {
        if results.len() > MAX_WEB_SEARCH_RESULT_LIMIT {
            return Err(SearchProviderError::new(
                SearchProviderErrorCode::MalformedResponse,
            ));
        }
        Ok(Self { results, truncated })
    }

    /// Returns normalized results in backend relevance order.
    #[must_use]
    pub fn results(&self) -> &[SearchResult] {
        &self.results
    }

    /// Returns whether the backend returned more results than retained.
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}

/// Object-safe future returned by [`SearchProvider`].
pub type SearchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SearchResponse, SearchProviderError>> + Send + 'a>>;

/// Object-safe client web-search backend port.
pub trait SearchProvider: fmt::Debug + Send + Sync {
    /// Returns the configured disclosure destination used for policy resources.
    fn destination(&self) -> &str;

    /// Executes one bounded request with cooperative cancellation.
    fn search(&self, request: SearchRequest, cancellation: CancellationScope) -> SearchFuture<'_>;
}

/// Redacted Tavily API credential.
#[derive(Clone, PartialEq, Eq)]
pub struct TavilyApiKey(String);

impl TavilyApiKey {
    /// Creates a bounded visible-ASCII API key.
    ///
    /// # Errors
    ///
    /// Rejects empty, oversized, whitespace-containing, and control-containing keys.
    pub fn new(value: impl Into<String>) -> Result<Self, SearchProviderError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_TAVILY_API_KEY_BYTES
            || !value.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(SearchProviderError::new(
                SearchProviderErrorCode::InvalidConfiguration,
            ));
        }
        Ok(Self(value))
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for TavilyApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TavilyApiKey(**REDACTED**)")
    }
}

/// Validated Tavily HTTP endpoint and timeout policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TavilySearchConfig {
    endpoint: String,
    timeout: Duration,
}

impl TavilySearchConfig {
    /// Creates a Tavily backend configuration.
    ///
    /// HTTPS is required except for loopback HTTP endpoints used by local
    /// gateways and deterministic contract tests. User information, query
    /// strings, and fragments are rejected so credentials cannot be embedded in
    /// a displayable destination.
    ///
    /// # Errors
    ///
    /// Rejects invalid endpoints and timeouts outside `1ms..=60s`.
    pub fn new(endpoint: impl AsRef<str>, timeout: Duration) -> Result<Self, SearchProviderError> {
        let endpoint = Url::parse(endpoint.as_ref())
            .map_err(|_| SearchProviderError::new(SearchProviderErrorCode::InvalidConfiguration))?;
        let loopback_http =
            endpoint.scheme() == "http" && endpoint.host_str().is_some_and(is_loopback_host);
        let valid = (endpoint.scheme() == "https" || loopback_http)
            && endpoint.host_str().is_some()
            && endpoint.username().is_empty()
            && endpoint.password().is_none()
            && endpoint.query().is_none()
            && endpoint.fragment().is_none()
            && endpoint.path() != "/"
            && !timeout.is_zero()
            && timeout <= MAX_TAVILY_TIMEOUT;
        if !valid {
            return Err(SearchProviderError::new(
                SearchProviderErrorCode::InvalidConfiguration,
            ));
        }
        Ok(Self {
            endpoint: endpoint.to_string(),
            timeout,
        })
    }

    /// Creates the production endpoint with a caller-selected timeout.
    ///
    /// # Errors
    ///
    /// Rejects an invalid timeout.
    pub fn production(timeout: Duration) -> Result<Self, SearchProviderError> {
        Self::new(DEFAULT_TAVILY_SEARCH_ENDPOINT, timeout)
    }

    /// Returns the canonical endpoint.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Returns the request timeout.
    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }
}

fn is_loopback_host(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// Production Tavily implementation of [`SearchProvider`].
#[derive(Debug, Clone)]
pub struct TavilySearchProvider {
    config: TavilySearchConfig,
    api_key: TavilyApiKey,
    client: reqwest::Client,
}

impl TavilySearchProvider {
    /// Builds a reusable Tavily client under the shared provider HTTP policy.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be constructed.
    pub fn new(
        config: TavilySearchConfig,
        api_key: TavilyApiKey,
        http: &ProviderHttpConfig,
    ) -> Result<Self, SearchProviderError> {
        let client = http
            .build_client_without_redirects(config.timeout())
            .map_err(|_| SearchProviderError::new(SearchProviderErrorCode::InvalidConfiguration))?;
        Ok(Self {
            config,
            api_key,
            client,
        })
    }

    async fn execute(
        &self,
        request: SearchRequest,
        cancellation: CancellationScope,
    ) -> Result<SearchResponse, SearchProviderError> {
        let mut body = Map::new();
        body.insert("query".to_owned(), json!(request.query()));
        body.insert("search_depth".to_owned(), json!("basic"));
        body.insert("max_results".to_owned(), json!(request.limit()));
        body.insert("include_answer".to_owned(), Value::Bool(false));
        body.insert("include_raw_content".to_owned(), Value::Bool(false));
        body.insert("include_images".to_owned(), Value::Bool(false));
        if !request.allowed_domains().is_empty() {
            body.insert(
                "include_domains".to_owned(),
                json!(request.allowed_domains()),
            );
        }
        if !request.blocked_domains().is_empty() {
            body.insert(
                "exclude_domains".to_owned(),
                json!(request.blocked_domains()),
            );
        }
        let send = self
            .client
            .post(self.config.endpoint())
            .bearer_auth(self.api_key.expose())
            .json(&Value::Object(body))
            .send();
        let response = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(SearchProviderError::cancelled()),
            response = send => response.map_err(|error| map_reqwest_error(&error))?,
        };
        if !response.status().is_success() {
            return Err(SearchProviderError::new(status_code(response.status())));
        }
        let bytes = read_bounded_body(response, &cancellation).await?;
        let raw = serde_json::from_slice::<TavilyResponse>(&bytes)
            .map_err(|_| SearchProviderError::new(SearchProviderErrorCode::MalformedResponse))?;
        normalize_tavily_response(raw, request.limit())
    }
}

impl SearchProvider for TavilySearchProvider {
    fn destination(&self) -> &str {
        self.config.endpoint()
    }

    fn search(&self, request: SearchRequest, cancellation: CancellationScope) -> SearchFuture<'_> {
        Box::pin(self.execute(request, cancellation))
    }
}

fn map_reqwest_error(error: &reqwest::Error) -> SearchProviderError {
    let code = if error.is_timeout() {
        SearchProviderErrorCode::Timeout
    } else {
        SearchProviderErrorCode::Transport
    };
    SearchProviderError::new(code)
}

fn status_code(status: reqwest::StatusCode) -> SearchProviderErrorCode {
    match status.as_u16() {
        400 | 404 | 422 => SearchProviderErrorCode::InvalidRequest,
        401 | 403 => SearchProviderErrorCode::Authentication,
        429 => SearchProviderErrorCode::RateLimited,
        432 | 433 => SearchProviderErrorCode::QuotaExceeded,
        500..=599 => SearchProviderErrorCode::Unavailable,
        _ => SearchProviderErrorCode::Transport,
    }
}

async fn read_bounded_body(
    response: reqwest::Response,
    cancellation: &CancellationScope,
) -> Result<Vec<u8>, SearchProviderError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_WEB_SEARCH_RESPONSE_BYTES as u64)
    {
        return Err(SearchProviderError::new(
            SearchProviderErrorCode::MalformedResponse,
        ));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(SearchProviderError::cancelled()),
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.map_err(|error| map_reqwest_error(&error))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_WEB_SEARCH_RESPONSE_BYTES {
            return Err(SearchProviderError::new(
                SearchProviderErrorCode::MalformedResponse,
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[derive(Debug, Deserialize)]
struct TavilyResponse {
    results: Vec<TavilyResult>,
}

#[derive(Debug, Deserialize)]
struct TavilyResult {
    title: String,
    url: String,
    content: String,
    score: Option<f64>,
}

fn normalize_tavily_response(
    raw: TavilyResponse,
    limit: usize,
) -> Result<SearchResponse, SearchProviderError> {
    let truncated = raw.results.len() > limit;
    let results = raw
        .results
        .into_iter()
        .take(limit)
        .map(|result| {
            let title = truncate_utf8(&result.title, MAX_EXTERNAL_SOURCE_TITLE_BYTES);
            let mut source = ExternalSource::new(result.url)
                .and_then(|source| source.with_title(title))
                .map_err(|_| {
                    SearchProviderError::new(SearchProviderErrorCode::MalformedResponse)
                })?;
            if !result.content.is_empty() {
                let snippet_limit =
                    MAX_WEB_SEARCH_SNIPPET_BYTES.min(MAX_EXTERNAL_SOURCE_TEXT_BYTES);
                source = source
                    .with_snippet(truncate_utf8(&result.content, snippet_limit))
                    .map_err(|_| {
                        SearchProviderError::new(SearchProviderErrorCode::MalformedResponse)
                    })?;
            }
            SearchResult::new(source, result.score)
        })
        .collect::<Result<Vec<_>, _>>()?;
    SearchResponse::new(results, truncated)
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Client executor for the stable `web_search` tool contract.
#[derive(Debug, Clone)]
pub struct WebSearchTool {
    provider: Arc<dyn SearchProvider>,
    configured_options: WebSearchOptions,
}

impl WebSearchTool {
    /// Creates a client search executor with static product domain policy.
    #[must_use]
    pub fn new(provider: Arc<dyn SearchProvider>, configured_options: WebSearchOptions) -> Self {
        Self {
            provider,
            configured_options,
        }
    }

    /// Builds the portable client/hosted search tool contract.
    ///
    /// # Errors
    ///
    /// Returns an error only if the static contract violates tool bounds.
    pub fn spec() -> Result<ToolSpec, ToolSpecError> {
        ToolSpec::new(
            ToolName::from_str("web_search").map_err(|_| ToolSpecError::InvalidDescription)?,
            ToolVersion::from_str("1.0.0").map_err(|_| ToolSpecError::InvalidDescription)?,
            "Search the public web and return bounded normalized sources.",
            json!({
                "type":"object",
                "properties":{
                    "query":{"type":"string","minLength":1,"maxLength":MAX_WEB_SEARCH_QUERY_BYTES},
                    "allowed_domains":{
                        "type":"array","maxItems":tea_model::MAX_WEB_SEARCH_DOMAINS,
                        "uniqueItems":true,"items":{"type":"string","minLength":1,"maxLength":tea_model::MAX_WEB_SEARCH_DOMAIN_BYTES}
                    },
                    "blocked_domains":{
                        "type":"array","maxItems":tea_model::MAX_WEB_SEARCH_DOMAINS,
                        "uniqueItems":true,"items":{"type":"string","minLength":1,"maxLength":tea_model::MAX_WEB_SEARCH_DOMAIN_BYTES}
                    },
                    "limit":{"type":"integer","minimum":1,"maximum":MAX_WEB_SEARCH_RESULT_LIMIT}
                },
                "required":["query"],
                "additionalProperties":false
            }),
            json!({
                "type":"object",
                "properties":{
                    "results":{
                        "type":"array","maxItems":MAX_WEB_SEARCH_RESULT_LIMIT,
                        "items":{
                            "type":"object",
                            "properties":{
                                "url":{"type":"string"},
                                "title":{"type":"string"},
                                "snippet":{"type":"string"},
                                "score":{"type":"number","minimum":0,"maximum":1}
                            },
                            "required":["url"],
                            "additionalProperties":false
                        }
                    },
                    "truncated":{"type":"boolean"}
                },
                "required":["results","truncated"],
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
            "Use web_search for current public information and cite returned source URLs.",
        )
    }

    /// Returns the fixed external destination declared to policy before execution.
    ///
    /// # Errors
    ///
    /// Returns an error when a custom provider exposes an invalid destination.
    pub fn resource(&self) -> Result<ToolResource, ToolResourceError> {
        ToolResource::new("url", self.provider.destination(), ToolResourceAccess::Read)
    }

    fn request(
        &self,
        invocation: &ValidatedToolInvocation,
    ) -> Result<SearchRequest, SearchRequestError> {
        let query = invocation
            .arguments()
            .get("query")
            .and_then(Value::as_str)
            .ok_or(SearchRequestError::InvalidQuery)?;
        let limit = invocation
            .arguments()
            .get("limit")
            .and_then(Value::as_u64)
            .map(usize::try_from)
            .transpose()
            .map_err(|_| SearchRequestError::InvalidLimit)?
            .unwrap_or(DEFAULT_WEB_SEARCH_RESULT_LIMIT);
        let requested_allowed = domain_argument(invocation, "allowed_domains")?;
        let requested_blocked = domain_argument(invocation, "blocked_domains")?;
        let (allowed, blocked) = effective_domains(
            &self.configured_options,
            requested_allowed,
            requested_blocked,
        )?;
        let mut options = if !allowed.is_empty() {
            WebSearchOptions::new()
                .with_allowed_domains(allowed)
                .map_err(|_| SearchRequestError::InvalidDomains)?
        } else if !blocked.is_empty() {
            WebSearchOptions::new()
                .with_blocked_domains(blocked)
                .map_err(|_| SearchRequestError::InvalidDomains)?
        } else {
            WebSearchOptions::new()
        };
        if let Some(location) = self.configured_options.location() {
            options = options.with_location(location.clone());
        }
        SearchRequest::from_options(query, options, limit)
    }

    async fn run(
        &self,
        invocation: &ValidatedToolInvocation,
        cancellation: CancellationScope,
    ) -> ToolExecutionEvent {
        if cancellation.is_cancelled() {
            return ToolExecutionEvent::Failed(ToolExecutionFailure::cancelled());
        }
        let Ok(request) = self.request(invocation) else {
            return provider_failure(SearchProviderError::new(
                SearchProviderErrorCode::InvalidRequest,
            ));
        };
        match self.provider.search(request, cancellation).await {
            Ok(response) => search_success(&response),
            Err(error) => provider_failure(error),
        }
    }
}

impl ToolExecutor for WebSearchTool {
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

fn domain_argument(
    invocation: &ValidatedToolInvocation,
    name: &str,
) -> Result<Vec<String>, SearchRequestError> {
    invocation
        .arguments()
        .get(name)
        .map(|domains| {
            domains
                .as_array()
                .ok_or(SearchRequestError::InvalidDomains)?
                .iter()
                .map(|domain| {
                    domain
                        .as_str()
                        .map(str::to_owned)
                        .ok_or(SearchRequestError::InvalidDomains)
                })
                .collect()
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn effective_domains(
    configured: &WebSearchOptions,
    requested_allowed: Vec<String>,
    requested_blocked: Vec<String>,
) -> Result<(Vec<String>, Vec<String>), SearchRequestError> {
    if !requested_allowed.is_empty() && !requested_blocked.is_empty() {
        return Err(SearchRequestError::InvalidDomains);
    }
    if !configured.allowed_domains().is_empty() {
        if !requested_blocked.is_empty()
            || requested_allowed.iter().any(|domain| {
                !configured
                    .allowed_domains()
                    .iter()
                    .any(|configured| configured == domain)
            })
        {
            return Err(SearchRequestError::InvalidDomains);
        }
        let allowed = if requested_allowed.is_empty() {
            configured.allowed_domains().to_vec()
        } else {
            requested_allowed
        };
        return Ok((allowed, Vec::new()));
    }
    if !configured.blocked_domains().is_empty() {
        if !requested_allowed.is_empty() {
            return Err(SearchRequestError::InvalidDomains);
        }
        let blocked = configured
            .blocked_domains()
            .iter()
            .cloned()
            .chain(requested_blocked)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        return Ok((Vec::new(), blocked));
    }
    Ok((requested_allowed, requested_blocked))
}

fn search_success(response: &SearchResponse) -> ToolExecutionEvent {
    let results = response
        .results()
        .iter()
        .map(|result| {
            let mut value = Map::new();
            value.insert("url".to_owned(), json!(result.source().url()));
            if let Some(title) = result.source().title() {
                value.insert("title".to_owned(), json!(title));
            }
            if let Some(snippet) = result.source().snippet() {
                value.insert("snippet".to_owned(), json!(snippet));
            }
            if let Some(score) = result.score() {
                value.insert("score".to_owned(), json!(score));
            }
            Value::Object(value)
        })
        .collect::<Vec<_>>();
    let mut visible = String::new();
    if results.is_empty() {
        visible.push_str("No web search results found.");
    } else {
        visible.push_str("Web search results:\n");
        for (index, result) in response.results().iter().enumerate() {
            let source = result.source();
            let title = source.title().unwrap_or(source.url());
            write!(visible, "{}. {title}\n   {}\n", index + 1, source.url())
                .expect("writing to a String cannot fail");
            if let Some(snippet) = source.snippet() {
                visible.push_str("   ");
                visible.push_str(snippet);
                visible.push('\n');
            }
        }
    }
    let output = json!({"results":results,"truncated":response.truncated()});
    let Ok(content) = ContentBlock::text(visible) else {
        return ToolExecutionEvent::Failed(ToolExecutionFailure::internal_contract());
    };
    let Ok(result) = ToolResult::new(vec![content], output) else {
        return ToolExecutionEvent::Failed(ToolExecutionFailure::internal_contract());
    };
    ToolExecutionEvent::Finished(result)
}

fn provider_failure(error: SearchProviderError) -> ToolExecutionEvent {
    if error.code() == SearchProviderErrorCode::Cancelled {
        return ToolExecutionEvent::Failed(ToolExecutionFailure::cancelled());
    }
    let details = ProtocolMetadata::from_entries([(
        "dev.tea-rs.web-search",
        json!({"code":provider_error_code(error.code())}),
    )])
    .unwrap_or_default();
    let failure = ToolExecutionFailure::execution(error.message())
        .unwrap_or_else(|_| ToolExecutionFailure::internal_contract())
        .with_details(details);
    ToolExecutionEvent::Failed(failure)
}

const fn provider_error_code(code: SearchProviderErrorCode) -> &'static str {
    match code {
        SearchProviderErrorCode::InvalidConfiguration => "invalid_configuration",
        SearchProviderErrorCode::InvalidRequest => "invalid_request",
        SearchProviderErrorCode::Authentication => "authentication",
        SearchProviderErrorCode::RateLimited => "rate_limited",
        SearchProviderErrorCode::QuotaExceeded => "quota_exceeded",
        SearchProviderErrorCode::Timeout => "timeout",
        SearchProviderErrorCode::Transport => "transport",
        SearchProviderErrorCode::Unavailable => "unavailable",
        SearchProviderErrorCode::MalformedResponse => "malformed_response",
        SearchProviderErrorCode::Cancelled => "cancelled",
    }
}
