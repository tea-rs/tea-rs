#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Shared HTTP client policy for `tea-rs` model provider adapters.

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use futures_util::StreamExt as _;
use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};
use serde_json::Value;

/// Maximum response bytes read while extracting a provider error.
pub const MAX_PROVIDER_ERROR_BODY_BYTES: usize = 8 * 1024;
/// Maximum normalized provider reason included in a user-facing diagnostic.
pub const MAX_PROVIDER_ERROR_REASON_BYTES: usize = 1024;

/// Parses a provider-requested retry delay from HTTP response headers.
///
/// The non-standard `retry-after-ms` header takes precedence over the standard
/// `Retry-After` seconds or HTTP-date forms. Invalid values are ignored and a
/// past date produces a zero delay.
#[must_use]
pub fn retry_after_delay(headers: &HeaderMap, now: SystemTime) -> Option<Duration> {
    if let Some(value) = headers
        .get("retry-after-ms")
        .and_then(|value| value.to_str().ok())
        && let Some(delay) = decimal_duration(value, 1_000.0)
    {
        return Some(delay);
    }

    let value = headers.get(RETRY_AFTER)?.to_str().ok()?;
    if let Some(delay) = decimal_duration(value, 1.0) {
        return Some(delay);
    }
    let requested = DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&Utc);
    let now = DateTime::<Utc>::from(now);
    Some((requested - now).to_std().unwrap_or(Duration::ZERO))
}

fn decimal_duration(value: &str, units_per_second: f64) -> Option<Duration> {
    let value = value.parse::<f64>().ok()?;
    if !value.is_finite() {
        return None;
    }
    Duration::try_from_secs_f64(value.max(0.0) / units_per_second).ok()
}

/// Provider-neutral HTTP client settings supplied by the embedding application.
#[derive(Clone, Debug, Default)]
pub struct ProviderHttpConfig {
    user_agent: Option<UserAgent>,
}

impl ProviderHttpConfig {
    /// Creates an HTTP configuration with no application identity headers.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the application identity sent with provider requests.
    #[must_use]
    pub fn with_user_agent(mut self, user_agent: UserAgent) -> Self {
        self.user_agent = Some(user_agent);
        self
    }

    /// Builds a reusable HTTP client with the shared policy and provider timeout.
    ///
    /// # Errors
    ///
    /// Returns an error when `reqwest` cannot construct the client.
    pub fn build_client(&self, timeout: Duration) -> Result<reqwest::Client, reqwest::Error> {
        self.client_builder(timeout).build()
    }

    /// Builds a reusable HTTP client that rejects provider redirects.
    ///
    /// Fixed-destination, credentialed APIs use this policy when the embedding
    /// product must disclose and approve the exact network destination before
    /// execution.
    ///
    /// # Errors
    ///
    /// Returns an error when `reqwest` cannot construct the client.
    pub fn build_client_without_redirects(
        &self,
        timeout: Duration,
    ) -> Result<reqwest::Client, reqwest::Error> {
        self.client_builder(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
    }

    /// Builds a direct client pinned to one previously validated socket address.
    ///
    /// Automatic redirects and ambient proxy discovery are disabled. The
    /// original domain remains in the request URL, `Host` header, and TLS SNI;
    /// only DNS resolution is overridden with `address`.
    ///
    /// Callers must validate the domain and address before invoking this method
    /// and must verify the connected peer reported by the response.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be constructed.
    pub fn build_pinned_client_without_redirects(
        &self,
        timeout: Duration,
        connect_timeout: Duration,
        domain: &str,
        address: SocketAddr,
    ) -> Result<reqwest::Client, reqwest::Error> {
        self.client_builder(timeout)
            .connect_timeout(connect_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve(domain, address)
            .build()
    }

    fn client_builder(&self, timeout: Duration) -> reqwest::ClientBuilder {
        let mut builder = reqwest::Client::builder().timeout(timeout);
        if let Some(user_agent) = &self.user_agent {
            builder = builder.user_agent(user_agent.0.clone());
        }
        builder
    }
}

/// A validated value for the HTTP `User-Agent` request header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserAgent(HeaderValue);

impl UserAgent {
    /// Creates a user agent suitable for use in an HTTP request header.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is empty or not a valid HTTP header value.
    pub fn new(value: impl AsRef<str>) -> Result<Self, InvalidUserAgent> {
        let value = value.as_ref();
        if value.trim().is_empty() {
            return Err(InvalidUserAgent);
        }
        HeaderValue::from_str(value)
            .map(Self)
            .map_err(|_| InvalidUserAgent)
    }
}

/// Indicates that a User-Agent was empty or invalid as an HTTP header value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidUserAgent;

impl Display for InvalidUserAgent {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("user agent is invalid")
    }
}

impl Error for InvalidUserAgent {}

/// Reads at most [`MAX_PROVIDER_ERROR_BODY_BYTES`] from a provider response.
///
/// Provider error bodies are untrusted input. The response is deliberately
/// consumed as a byte stream so a large or malicious error response cannot be
/// loaded into memory by an unbounded `Response::text()` call.
pub async fn read_bounded_error_body(response: reqwest::Response) -> String {
    let mut bytes = Vec::with_capacity(MAX_PROVIDER_ERROR_BODY_BYTES);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else { break };
        let remaining = MAX_PROVIDER_ERROR_BODY_BYTES.saturating_sub(bytes.len());
        if remaining == 0 {
            break;
        }
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Normalizes an HTTP error into a bounded, terminal-safe provider diagnostic.
///
/// This follows Pi's status/body/message normalization shape while avoiding
/// serialization of arbitrary provider JSON. Only known reason fields are
/// extracted from JSON objects; sensitive fields are ignored.
#[must_use]
pub fn normalize_provider_error(status: Option<u16>, body: &str) -> String {
    let reason = match serde_json::from_str::<Value>(body) {
        Ok(value) => extract_json_reason(&value)
            .unwrap_or_else(|| "provider returned a structured error".to_owned()),
        Err(_) => body.to_owned(),
    };
    let reason = truncate_reason(&redact_sensitive_text(&sanitize_reason(&reason)));
    let reason = if reason.is_empty() {
        "provider returned no error details".to_owned()
    } else {
        reason
    };
    match status {
        Some(status) => format!("HTTP {status}: {reason}"),
        None => reason,
    }
}

fn extract_json_reason(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    for key in ["error", "message", "detail", "reason"] {
        let Some(candidate) = object.get(key) else {
            continue;
        };
        if is_sensitive_key(key) {
            continue;
        }
        if let Some(text) = candidate.as_str()
            && !text.trim().is_empty()
        {
            return Some(text.to_owned());
        }
        if let Some(text) = extract_json_reason(candidate) {
            return Some(text);
        }
    }
    for key in ["type", "code"] {
        if let Some(text) = object.get(key).and_then(Value::as_str)
            && !text.trim().is_empty()
            && !is_sensitive_key(key)
        {
            return Some(text.to_owned());
        }
    }
    None
}

fn is_sensitive_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "api_key"
            | "apikey"
            | "authorization"
            | "access_token"
            | "refresh_token"
            | "token"
            | "cookie"
            | "password"
            | "secret"
    )
}

fn sanitize_reason(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut in_escape = false;
    let mut in_csi = false;
    for character in value.chars() {
        if in_csi {
            if character.is_ascii_alphabetic() {
                in_csi = false;
            }
            continue;
        }
        if in_escape {
            if character == '[' {
                in_escape = false;
                in_csi = true;
                continue;
            }
            if character.is_ascii_alphabetic() || matches!(character, '@'..='~') {
                in_escape = false;
            }
            continue;
        }
        if character == '\u{1b}' {
            in_escape = true;
            continue;
        }
        if character.is_control() {
            if matches!(character, '\n' | '\r' | '\t') {
                output.push(' ');
            }
            continue;
        }
        output.push(character);
    }
    output
}

fn redact_sensitive_text(value: &str) -> String {
    const MARKERS: [&str; 9] = [
        "authorization",
        "api_key",
        "apikey",
        "access_token",
        "refresh_token",
        "password",
        "cookie",
        "secret",
        "token",
    ];
    let lower = value.to_ascii_lowercase();
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    while cursor < value.len() {
        let next = MARKERS
            .iter()
            .filter_map(|marker| {
                lower[cursor..]
                    .find(marker)
                    .map(|offset| (cursor + offset, *marker))
            })
            .filter(|(index, marker)| {
                let before = index
                    .checked_sub(1)
                    .and_then(|value| lower.as_bytes().get(value))
                    .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_');
                let after = lower
                    .as_bytes()
                    .get(index + marker.len())
                    .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_');
                before && after
            })
            .min_by_key(|(index, _)| *index);
        let Some((start, marker)) = next else {
            output.push_str(&value[cursor..]);
            break;
        };
        output.push_str(&value[cursor..start]);
        let marker_end = start + marker.len();
        let tail = &value[marker_end..];
        let Some(separator) = tail.find([':', '=']) else {
            output.push_str(&value[start..marker_end]);
            cursor = marker_end;
            continue;
        };
        let separator_end = marker_end + separator + 1;
        output.push_str(&value[start..separator_end]);
        let mut secret_start = separator_end;
        while let Some(byte) = value.as_bytes().get(secret_start) {
            if byte.is_ascii_whitespace() || matches!(*byte, b'"' | b'\'') {
                secret_start += 1;
            } else {
                break;
            }
        }
        let spacing_end = secret_start;
        if value[secret_start..]
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("bearer "))
        {
            secret_start += 7;
        }
        output.push_str(&value[separator_end..spacing_end]);
        let secret_end = value[secret_start..]
            .char_indices()
            .find(|(_, character)| {
                character.is_whitespace() || matches!(character, ',' | '}' | ']' | ';' | '"' | '\'')
            })
            .map_or(value.len(), |(index, _)| secret_start + index);
        output.push_str("[redacted]");
        cursor = secret_end;
    }
    redact_secret_prefixes(&output)
}

fn redact_secret_prefixes(value: &str) -> String {
    const PREFIXES: [&str; 5] = ["sk-ant-", "sk-", "gsk_", "xai-", "AIza"];
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    while cursor < value.len() {
        let next = PREFIXES
            .iter()
            .filter_map(|prefix| {
                value[cursor..]
                    .find(prefix)
                    .map(|offset| (cursor + offset, *prefix))
            })
            .filter(|(index, _)| {
                index
                    .checked_sub(1)
                    .and_then(|position| value.as_bytes().get(position))
                    .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
            })
            .min_by_key(|(index, _)| *index);
        let Some((start, prefix)) = next else {
            output.push_str(&value[cursor..]);
            break;
        };
        output.push_str(&value[cursor..start]);
        let secret_start = start + prefix.len();
        let secret_end = value[secret_start..]
            .char_indices()
            .find(|(_, character)| {
                character.is_whitespace() || matches!(character, ',' | '}' | ']' | ';' | '"' | '\'')
            })
            .map_or(value.len(), |(index, _)| secret_start + index);
        output.push_str("[redacted]");
        cursor = secret_end;
    }
    output
}

fn truncate_reason(value: &str) -> String {
    if value.len() <= MAX_PROVIDER_ERROR_REASON_BYTES {
        return value.to_owned();
    }
    let suffix = "... [truncated]";
    let limit = MAX_PROVIDER_ERROR_REASON_BYTES.saturating_sub(suffix.len());
    let boundary = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= limit)
        .last()
        .unwrap_or(0);
    format!("{}{}", &value[..boundary], suffix)
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    use super::*;

    #[test]
    fn parses_retry_after_headers_with_precedence_and_dates() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("9"));
        headers.insert("retry-after-ms", HeaderValue::from_static("1500"));
        assert_eq!(
            retry_after_delay(&headers, SystemTime::UNIX_EPOCH),
            Some(Duration::from_millis(1500))
        );

        headers.remove("retry-after-ms");
        headers.insert(RETRY_AFTER, HeaderValue::from_static("2.5"));
        assert_eq!(
            retry_after_delay(&headers, SystemTime::UNIX_EPOCH),
            Some(Duration::from_millis(2500))
        );

        headers.insert(
            RETRY_AFTER,
            HeaderValue::from_static("Thu, 01 Jan 1970 00:00:05 GMT"),
        );
        assert_eq!(
            retry_after_delay(&headers, SystemTime::UNIX_EPOCH),
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            retry_after_delay(&headers, SystemTime::UNIX_EPOCH + Duration::from_secs(10)),
            Some(Duration::ZERO)
        );

        headers.insert(RETRY_AFTER, HeaderValue::from_static("not-a-delay"));
        assert_eq!(retry_after_delay(&headers, SystemTime::UNIX_EPOCH), None);
    }

    async fn captured_headers(config: &ProviderHttpConfig) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream.read(&mut buffer).await.unwrap();
                assert_ne!(read, 0, "request closed before headers were sent");
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        let response = config
            .build_client(Duration::from_secs(1))
            .unwrap()
            .get(format!("http://{address}/request"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
        server.await.unwrap()
    }

    #[test]
    fn user_agent_rejects_empty_and_invalid_header_values() {
        assert_eq!(UserAgent::new(""), Err(InvalidUserAgent));
        assert_eq!(
            UserAgent::new("tea-cli/1.0\r\ninvalid"),
            Err(InvalidUserAgent)
        );
    }

    #[test]
    fn normalizes_status_and_known_json_reason_without_serializing_secrets() {
        let message = normalize_provider_error(
            Some(403),
            r#"{"error":{"message":"blocked by gateway WAF","api_key":"sk-secret"}}"#,
        );
        assert_eq!(message, "HTTP 403: blocked by gateway WAF");
        assert!(!message.contains("sk-secret"));
    }

    #[test]
    fn normalizes_plain_text_with_bounds_and_terminal_sanitization() {
        let message = normalize_provider_error(Some(500), "\u{1b}[31mserver\nerror\u{1b}[0m");
        assert!(!message.contains('\u{1b}'));
        assert!(!message.contains('\n'));
        assert!(message.contains("server"));

        let long = normalize_provider_error(None, &"x".repeat(2_000));
        assert!(long.len() <= MAX_PROVIDER_ERROR_REASON_BYTES);
        assert!(long.ends_with("... [truncated]"));

        let secret = normalize_provider_error(None, "authorization: Bearer sk-secret");
        assert_eq!(secret, "authorization: [redacted]");

        let unknown_json = normalize_provider_error(None, r#"{"api_key":"sk-secret"}"#);
        assert_eq!(unknown_json, "provider returned a structured error");

        let inline_key = normalize_provider_error(None, r#"{"error":"sk-secret"}"#);
        assert_eq!(inline_key, "[redacted]");
    }

    #[tokio::test]
    async fn response_body_is_bounded_before_conversion_to_text() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            let body = vec![b'x'; MAX_PROVIDER_ERROR_BODY_BYTES * 2];
            let header = format!(
                "HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
        });
        let response = ProviderHttpConfig::new()
            .build_client(Duration::from_secs(1))
            .unwrap()
            .get(format!("http://{address}/error"))
            .send()
            .await
            .unwrap();
        let body = read_bounded_error_body(response).await;
        assert_eq!(body.len(), MAX_PROVIDER_ERROR_BODY_BYTES);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn client_sends_configured_user_agent_and_omits_it_by_default() {
        let configured =
            ProviderHttpConfig::new().with_user_agent(UserAgent::new("tea-cli/0.1.0").unwrap());
        let headers = captured_headers(&configured).await;
        assert!(
            headers
                .lines()
                .any(|header| header.eq_ignore_ascii_case("user-agent: tea-cli/0.1.0"))
        );

        let headers = captured_headers(&ProviderHttpConfig::new()).await;
        assert!(
            !headers
                .lines()
                .any(|header| header.to_ascii_lowercase().starts_with("user-agent:"))
        );
    }

    #[tokio::test]
    async fn fixed_destination_client_does_not_follow_redirects() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/redirected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });

        let response = ProviderHttpConfig::new()
            .build_client_without_redirects(Duration::from_secs(1))
            .unwrap()
            .get(format!("http://{address}/request"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        server.await.unwrap();
    }
}
