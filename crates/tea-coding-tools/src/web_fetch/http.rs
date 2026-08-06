use std::collections::BTreeSet;
use std::fmt;
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;
use reqwest::header::{
    ACCEPT, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, LOCATION,
};
use tea_control::CancellationScope;
use tea_provider_http::ProviderHttpConfig;

use super::{
    FetchAddressPolicy, FetchProviderError, FetchProviderErrorCode, FetchRedirect, FetchUrlPolicy,
    MAX_FETCH_REDIRECTS, ValidatedFetchAddresses, ValidatedFetchUrl,
};

pub(crate) const FETCH_ACCEPT_VALUE: &str =
    "text/html, application/xhtml+xml, application/json, text/plain";
pub(crate) const FETCH_ACCEPT_ENCODING_VALUE: &str = "identity";

/// Default maximum raw response bytes retained by the fetch transport.
pub const DEFAULT_FETCH_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
/// Absolute maximum raw response bytes accepted by transport configuration.
pub const MAX_FETCH_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

const MAX_RESPONSE_HEADER_VALUE_BYTES: usize = 1024;
const MAX_HTTP_TIMEOUT: Duration = Duration::from_mins(2);

/// Bounded timeout and low-speed policy for one complete fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FetchHttpTimeouts {
    dns: Duration,
    connect: Duration,
    first_byte: Duration,
    total: Duration,
    low_speed_window: Duration,
    low_speed_min_bytes: usize,
}

impl FetchHttpTimeouts {
    /// Creates a complete timeout policy.
    ///
    /// # Errors
    ///
    /// Rejects zero durations, durations above two minutes, a total timeout
    /// shorter than a phase timeout, and a zero low-speed byte threshold.
    pub fn new(
        dns: Duration,
        connect: Duration,
        first_byte: Duration,
        total: Duration,
        low_speed_window: Duration,
        low_speed_min_bytes: usize,
    ) -> Result<Self, FetchProviderError> {
        let durations = [dns, connect, first_byte, total, low_speed_window];
        if durations
            .iter()
            .any(|duration| duration.is_zero() || *duration > MAX_HTTP_TIMEOUT)
            || total < dns
            || total < connect
            || total < first_byte
            || total < low_speed_window
            || low_speed_min_bytes == 0
        {
            return Err(FetchProviderError::new(
                FetchProviderErrorCode::InvalidConfiguration,
            ));
        }
        Ok(Self {
            dns,
            connect,
            first_byte,
            total,
            low_speed_window,
            low_speed_min_bytes,
        })
    }

    /// Returns the DNS resolution timeout.
    #[must_use]
    pub const fn dns(self) -> Duration {
        self.dns
    }

    /// Returns the TCP/TLS connect timeout.
    #[must_use]
    pub const fn connect(self) -> Duration {
        self.connect
    }

    /// Returns the response-header/first-byte timeout.
    #[must_use]
    pub const fn first_byte(self) -> Duration {
        self.first_byte
    }

    /// Returns the complete request timeout across DNS and redirects.
    #[must_use]
    pub const fn total(self) -> Duration {
        self.total
    }

    /// Returns the low-speed measurement window.
    #[must_use]
    pub const fn low_speed_window(self) -> Duration {
        self.low_speed_window
    }

    /// Returns the minimum bytes expected per low-speed window.
    #[must_use]
    pub const fn low_speed_min_bytes(self) -> usize {
        self.low_speed_min_bytes
    }
}

impl Default for FetchHttpTimeouts {
    fn default() -> Self {
        Self {
            dns: Duration::from_secs(5),
            connect: Duration::from_secs(10),
            first_byte: Duration::from_secs(15),
            total: Duration::from_secs(30),
            low_speed_window: Duration::from_secs(5),
            low_speed_min_bytes: 1024,
        }
    }
}

/// Bounded redirect and raw-body limits for the HTTP transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FetchHttpLimits {
    max_redirects: usize,
    max_response_bytes: usize,
}

impl FetchHttpLimits {
    /// Creates HTTP transport limits.
    ///
    /// # Errors
    ///
    /// Rejects more than ten redirects and raw-body limits outside
    /// `1..=8 MiB`.
    pub fn new(
        max_redirects: usize,
        max_response_bytes: usize,
    ) -> Result<Self, FetchProviderError> {
        if max_redirects > MAX_FETCH_REDIRECTS
            || !(1..=MAX_FETCH_RESPONSE_BYTES).contains(&max_response_bytes)
        {
            return Err(FetchProviderError::new(
                FetchProviderErrorCode::InvalidConfiguration,
            ));
        }
        Ok(Self {
            max_redirects,
            max_response_bytes,
        })
    }

    /// Returns the maximum redirect hops.
    #[must_use]
    pub const fn max_redirects(self) -> usize {
        self.max_redirects
    }

    /// Returns the maximum raw response bytes.
    #[must_use]
    pub const fn max_response_bytes(self) -> usize {
        self.max_response_bytes
    }
}

impl Default for FetchHttpLimits {
    fn default() -> Self {
        Self {
            max_redirects: 5,
            max_response_bytes: DEFAULT_FETCH_RESPONSE_BYTES,
        }
    }
}

/// Complete immutable policy for the pinned HTTP transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FetchHttpConfig {
    url_policy: FetchUrlPolicy,
    address_policy: FetchAddressPolicy,
    timeouts: FetchHttpTimeouts,
    limits: FetchHttpLimits,
}

impl FetchHttpConfig {
    /// Creates a transport policy from independently validated components.
    #[must_use]
    pub const fn new(
        url_policy: FetchUrlPolicy,
        address_policy: FetchAddressPolicy,
        timeouts: FetchHttpTimeouts,
        limits: FetchHttpLimits,
    ) -> Self {
        Self {
            url_policy,
            address_policy,
            timeouts,
            limits,
        }
    }

    /// Creates the production public-network policy.
    #[must_use]
    pub fn production() -> Self {
        Self::new(
            FetchUrlPolicy::production(),
            FetchAddressPolicy::public_network(),
            FetchHttpTimeouts::default(),
            FetchHttpLimits::default(),
        )
    }

    /// Creates the explicit loopback-fixture policy.
    #[must_use]
    pub fn loopback_tests(timeouts: FetchHttpTimeouts, limits: FetchHttpLimits) -> Self {
        Self::new(
            FetchUrlPolicy::loopback_tests(),
            FetchAddressPolicy::loopback_tests(),
            timeouts,
            limits,
        )
    }

    /// Returns the URL policy.
    #[must_use]
    pub const fn url_policy(self) -> FetchUrlPolicy {
        self.url_policy
    }

    /// Returns the address policy.
    #[must_use]
    pub const fn address_policy(self) -> FetchAddressPolicy {
        self.address_policy
    }

    /// Returns timeout settings.
    #[must_use]
    pub const fn timeouts(self) -> FetchHttpTimeouts {
        self.timeouts
    }

    /// Returns redirect and body limits.
    #[must_use]
    pub const fn limits(self) -> FetchHttpLimits {
        self.limits
    }
}

impl Default for FetchHttpConfig {
    fn default() -> Self {
        Self::production()
    }
}

/// Object-safe DNS future returned by [`FetchDnsResolver`].
pub type FetchResolveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<IpAddr>, FetchProviderError>> + Send + 'a>>;

/// DNS resolver port used so security tests can supply deterministic answers.
pub trait FetchDnsResolver: fmt::Debug + Send + Sync {
    /// Resolves every A/AAAA answer for one validated host.
    fn resolve(&self, host: &str, cancellation: CancellationScope) -> FetchResolveFuture<'_>;
}

/// Tokio system resolver used by the production fetch provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemFetchDnsResolver;

impl FetchDnsResolver for SystemFetchDnsResolver {
    fn resolve(&self, host: &str, cancellation: CancellationScope) -> FetchResolveFuture<'_> {
        let host = host.to_owned();
        Box::pin(async move {
            if let Ok(address) = host.parse::<IpAddr>() {
                return Ok(vec![address]);
            }
            let lookup = tokio::net::lookup_host((host.as_str(), 0));
            let addresses = tokio::select! {
                biased;
                () = cancellation.cancelled() => return Err(FetchProviderError::cancelled()),
                result = lookup => result.map_err(|_| FetchProviderError::new(FetchProviderErrorCode::DnsResolution))?,
            };
            Ok(addresses.map(|address| address.ip()).collect())
        })
    }
}

/// Redacted subset of response headers consumed by the body decoder.
#[derive(Clone, PartialEq, Eq)]
pub struct FetchHttpHeaders {
    mime: Option<String>,
    encoding: Option<String>,
    length: Option<u64>,
}

impl FetchHttpHeaders {
    /// Creates bounded response metadata for a body decoder or fixture.
    ///
    /// # Errors
    ///
    /// Rejects oversized, control-containing header values.
    pub fn new(
        content_type: Option<String>,
        content_encoding: Option<String>,
        content_length: Option<u64>,
    ) -> Result<Self, FetchProviderError> {
        let mime = content_type.map(validate_header_value).transpose()?;
        let encoding = content_encoding.map(validate_header_value).transpose()?;
        Ok(Self {
            mime,
            encoding,
            length: content_length,
        })
    }

    /// Returns the bounded `Content-Type` value.
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.mime.as_deref()
    }

    /// Returns the bounded `Content-Encoding` value.
    #[must_use]
    pub fn content_encoding(&self) -> Option<&str> {
        self.encoding.as_deref()
    }

    /// Returns the parsed `Content-Length` value.
    #[must_use]
    pub const fn content_length(&self) -> Option<u64> {
        self.length
    }
}

impl fmt::Debug for FetchHttpHeaders {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FetchHttpHeaders")
            .field("content_type", &self.mime)
            .field("content_encoding", &self.encoding)
            .field("content_length", &self.length)
            .finish()
    }
}

/// One complete raw HTTP fetch after bounded manual redirects.
#[derive(Clone, PartialEq, Eq)]
pub struct FetchHttpResponse {
    requested_url: String,
    final_url: String,
    status: u16,
    headers: FetchHttpHeaders,
    body: Vec<u8>,
    redirects: Vec<FetchRedirect>,
}

impl FetchHttpResponse {
    /// Returns the canonical originally requested URL.
    #[must_use]
    pub fn requested_url(&self) -> &str {
        &self.requested_url
    }

    /// Returns the canonical final URL.
    #[must_use]
    pub fn final_url(&self) -> &str {
        &self.final_url
    }

    /// Returns the final HTTP status.
    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    /// Returns the bounded response headers.
    #[must_use]
    pub const fn headers(&self) -> &FetchHttpHeaders {
        &self.headers
    }

    /// Returns the bounded raw response body.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Returns normalized redirect metadata.
    #[must_use]
    pub fn redirects(&self) -> &[FetchRedirect] {
        &self.redirects
    }
}

impl fmt::Debug for FetchHttpResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FetchHttpResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("body_bytes", &self.body.len())
            .field("redirect_count", &self.redirects.len())
            .finish_non_exhaustive()
    }
}

/// HTTP transport that validates DNS, pins connections, and handles redirects.
#[derive(Clone)]
pub struct FetchHttpTransport {
    config: FetchHttpConfig,
    resolver: Arc<dyn FetchDnsResolver>,
    http: ProviderHttpConfig,
}

impl FetchHttpTransport {
    /// Creates a transport with an injected resolver and shared HTTP policy.
    #[must_use]
    pub fn new(
        config: FetchHttpConfig,
        resolver: Arc<dyn FetchDnsResolver>,
        http: ProviderHttpConfig,
    ) -> Self {
        Self {
            config,
            resolver,
            http,
        }
    }

    /// Returns the URL policy enforced before every DNS lookup.
    #[must_use]
    pub const fn url_policy(&self) -> FetchUrlPolicy {
        self.config.url_policy()
    }

    /// Returns the complete immutable transport policy used by cache isolation.
    #[must_use]
    pub const fn config(&self) -> FetchHttpConfig {
        self.config
    }

    /// Creates the production transport using the Tokio system resolver.
    #[must_use]
    pub fn production(http: ProviderHttpConfig) -> Self {
        Self::new(
            FetchHttpConfig::production(),
            Arc::new(SystemFetchDnsResolver),
            http,
        )
    }

    /// Executes one complete bounded GET request.
    ///
    /// # Errors
    ///
    /// Returns a stable secret-independent error for URL, DNS, peer, redirect,
    /// size, timeout, transport, and cancellation failures.
    pub async fn get(
        &self,
        url: &str,
        cancellation: CancellationScope,
    ) -> Result<FetchHttpResponse, FetchProviderError> {
        if cancellation.is_cancelled() {
            return Err(FetchProviderError::cancelled());
        }
        let request = self
            .config
            .url_policy()
            .validate(url)
            .map_err(|_| FetchProviderError::new(FetchProviderErrorCode::InvalidRequest))?;
        let operation = self.get_inner(request, cancellation.child());
        tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(FetchProviderError::cancelled()),
            result = tokio::time::timeout(self.config.timeouts().total(), operation) => {
                result.unwrap_or_else(|_| Err(FetchProviderError::new(FetchProviderErrorCode::Timeout)))
            }
        }
    }

    async fn get_inner(
        &self,
        requested: ValidatedFetchUrl,
        cancellation: CancellationScope,
    ) -> Result<FetchHttpResponse, FetchProviderError> {
        let mut current = requested.clone();
        let mut visited = BTreeSet::from([requested.as_str().to_owned()]);
        let mut redirects = Vec::new();

        loop {
            let addresses = self.resolve(&current, cancellation.child()).await?;
            let response = self
                .send_to_validated_peer(&current, &addresses, cancellation.child())
                .await?;
            if is_redirect(response.status()) {
                if redirects.len() >= self.config.limits().max_redirects() {
                    return Err(FetchProviderError::new(
                        FetchProviderErrorCode::RedirectRejected,
                    ));
                }
                let location = response
                    .headers()
                    .get(LOCATION)
                    .ok_or_else(|| {
                        FetchProviderError::new(FetchProviderErrorCode::RedirectRejected)
                    })?
                    .to_str()
                    .map_err(|_| {
                        FetchProviderError::new(FetchProviderErrorCode::RedirectRejected)
                    })?;
                let next = current
                    .resolve_redirect(location, &self.config.url_policy())
                    .map_err(|_| {
                        FetchProviderError::new(FetchProviderErrorCode::RedirectRejected)
                    })?;
                if current.is_https() && !next.is_https() {
                    return Err(FetchProviderError::new(
                        FetchProviderErrorCode::RedirectRejected,
                    ));
                }
                if !visited.insert(next.as_str().to_owned()) {
                    return Err(FetchProviderError::new(
                        FetchProviderErrorCode::RedirectRejected,
                    ));
                }
                redirects.push(FetchRedirect::new_with_policy(
                    current.as_str(),
                    next.as_str(),
                    response.status().as_u16(),
                    &self.config.url_policy(),
                )?);
                current = next;
                continue;
            }

            let status = response.status().as_u16();
            let headers = response_headers(&response)?;
            let body = read_raw_body(
                response,
                self.config.timeouts(),
                self.config.limits().max_response_bytes(),
                cancellation.child(),
            )
            .await?;
            return Ok(FetchHttpResponse {
                requested_url: requested.as_str().to_owned(),
                final_url: current.as_str().to_owned(),
                status,
                headers,
                body,
                redirects,
            });
        }
    }

    async fn resolve(
        &self,
        url: &ValidatedFetchUrl,
        cancellation: CancellationScope,
    ) -> Result<ValidatedFetchAddresses, FetchProviderError> {
        let resolution = self.resolver.resolve(url.host(), cancellation.child());
        let addresses = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(FetchProviderError::cancelled()),
            result = tokio::time::timeout(self.config.timeouts().dns(), resolution) => {
                result
                    .map_err(|_| FetchProviderError::new(FetchProviderErrorCode::Timeout))??
            }
        };
        self.config
            .address_policy()
            .validate(addresses)
            .map_err(|_| FetchProviderError::new(FetchProviderErrorCode::DnsResolution))
    }

    async fn send_to_validated_peer(
        &self,
        url: &ValidatedFetchUrl,
        addresses: &ValidatedFetchAddresses,
        cancellation: CancellationScope,
    ) -> Result<reqwest::Response, FetchProviderError> {
        let mut last_error = FetchProviderError::explicit_retry(FetchProviderErrorCode::Transport);
        for address in addresses.socket_addresses(url.port()) {
            let client = self
                .http
                .build_pinned_client_without_redirects(
                    self.config.timeouts().total(),
                    self.config.timeouts().connect(),
                    url.host(),
                    address,
                )
                .map_err(|_| {
                    FetchProviderError::new(FetchProviderErrorCode::InvalidConfiguration)
                })?;
            let send = client
                .get(url.as_str())
                .header(ACCEPT, FETCH_ACCEPT_VALUE)
                .header(ACCEPT_ENCODING, FETCH_ACCEPT_ENCODING_VALUE)
                .send();
            let response = tokio::select! {
                biased;
                () = cancellation.cancelled() => return Err(FetchProviderError::cancelled()),
                result = tokio::time::timeout(self.config.timeouts().first_byte(), send) => {
                    match result {
                        Ok(Ok(response)) => response,
                        Ok(Err(error)) => {
                            last_error = map_pre_response_error(&error);
                            continue;
                        }
                        Err(_) => {
                            last_error = FetchProviderError::explicit_retry(
                                FetchProviderErrorCode::Timeout,
                            );
                            continue;
                        }
                    }
                }
            };
            if response.remote_addr() != Some(address) {
                return Err(FetchProviderError::new(
                    FetchProviderErrorCode::ForbiddenDestination,
                ));
            }
            return Ok(response);
        }
        Err(last_error)
    }
}

impl fmt::Debug for FetchHttpTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FetchHttpTransport")
            .field("config", &self.config)
            .field("resolver", &self.resolver)
            .finish_non_exhaustive()
    }
}

fn response_headers(response: &reqwest::Response) -> Result<FetchHttpHeaders, FetchProviderError> {
    let content_type = bounded_header(response.headers().get(CONTENT_TYPE))?;
    let content_encoding = bounded_header(response.headers().get(CONTENT_ENCODING))?;
    let content_length = response
        .headers()
        .get(CONTENT_LENGTH)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or_else(|| FetchProviderError::new(FetchProviderErrorCode::MalformedResponse))
        })
        .transpose()?;
    FetchHttpHeaders::new(content_type, content_encoding, content_length)
}

fn bounded_header(
    value: Option<&reqwest::header::HeaderValue>,
) -> Result<Option<String>, FetchProviderError> {
    value
        .map(|value| {
            let value = value
                .to_str()
                .map_err(|_| FetchProviderError::new(FetchProviderErrorCode::MalformedResponse))?;
            if value.len() > MAX_RESPONSE_HEADER_VALUE_BYTES {
                return Err(FetchProviderError::new(
                    FetchProviderErrorCode::MalformedResponse,
                ));
            }
            Ok(value.to_owned())
        })
        .transpose()
}

fn validate_header_value(value: String) -> Result<String, FetchProviderError> {
    if value.len() > MAX_RESPONSE_HEADER_VALUE_BYTES || value.chars().any(char::is_control) {
        Err(FetchProviderError::new(
            FetchProviderErrorCode::MalformedResponse,
        ))
    } else {
        Ok(value)
    }
}

async fn read_raw_body(
    response: reqwest::Response,
    timeouts: FetchHttpTimeouts,
    max_bytes: usize,
    cancellation: CancellationScope,
) -> Result<Vec<u8>, FetchProviderError> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(FetchProviderError::new(
            FetchProviderErrorCode::ResponseTooLarge,
        ));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    let mut window_started = tokio::time::Instant::now();
    let mut window_bytes = 0_usize;
    loop {
        let remaining = timeouts
            .low_speed_window()
            .saturating_sub(window_started.elapsed());
        if remaining.is_zero() {
            if window_bytes < timeouts.low_speed_min_bytes() {
                return Err(FetchProviderError::new(FetchProviderErrorCode::Timeout));
            }
            window_started = tokio::time::Instant::now();
            window_bytes = 0;
            continue;
        }
        let next = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(FetchProviderError::cancelled()),
            result = tokio::time::timeout(remaining, stream.next()) => {
                result.map_err(|_| FetchProviderError::new(FetchProviderErrorCode::Timeout))?
            }
        };
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.map_err(|error| map_response_body_error(&error))?;
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(FetchProviderError::new(
                FetchProviderErrorCode::ResponseTooLarge,
            ));
        }
        window_bytes = window_bytes.saturating_add(chunk.len());
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn is_redirect(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

fn map_pre_response_error(error: &reqwest::Error) -> FetchProviderError {
    if error.is_timeout() {
        FetchProviderError::explicit_retry(FetchProviderErrorCode::Timeout)
    } else {
        FetchProviderError::explicit_retry(FetchProviderErrorCode::Transport)
    }
}

fn map_response_body_error(error: &reqwest::Error) -> FetchProviderError {
    if error.is_timeout() {
        FetchProviderError::new(FetchProviderErrorCode::Timeout)
    } else {
        FetchProviderError::new(FetchProviderErrorCode::Transport)
    }
}
