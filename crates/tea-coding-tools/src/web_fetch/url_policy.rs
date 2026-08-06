use std::fmt;
use std::net::IpAddr;

use thiserror::Error;
use url::{Host, Url};

use super::MAX_FETCH_URL_BYTES;

/// URL-policy failure detected before DNS or network I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum FetchUrlPolicyError {
    /// URL syntax, length, host, or credentials are invalid.
    #[error("web fetch URL is invalid")]
    InvalidUrl,
    /// URL scheme is not allowed by the selected policy.
    #[error("web fetch URL scheme is forbidden")]
    ForbiddenScheme,
    /// Explicit port is not allowed by the selected policy.
    #[error("web fetch URL port is forbidden")]
    ForbiddenPort,
}

/// Immutable URL validation policy used before DNS resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FetchUrlPolicy {
    allow_loopback_http: bool,
}

impl FetchUrlPolicy {
    /// Creates the production policy: HTTPS and the default port only.
    #[must_use]
    pub const fn production() -> Self {
        Self {
            allow_loopback_http: false,
        }
    }

    /// Creates an explicit local-fixture policy.
    ///
    /// This policy permits `http` only for literal loopback addresses or
    /// `localhost`; address validation must still permit and verify loopback.
    /// Product configuration must use [`Self::production`].
    #[must_use]
    pub const fn loopback_tests() -> Self {
        Self {
            allow_loopback_http: true,
        }
    }

    /// Normalizes and validates one URL without resolving its host.
    ///
    /// # Errors
    ///
    /// Rejects invalid IDNA hosts, credentials, unsupported schemes, unsafe
    /// ports, controls, whitespace, and oversized values. Fragments are
    /// deliberately removed because they are not part of HTTP requests or
    /// cache keys.
    pub fn validate(&self, value: &str) -> Result<ValidatedFetchUrl, FetchUrlPolicyError> {
        if value.is_empty()
            || value.len() > MAX_FETCH_URL_BYTES
            || value
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(FetchUrlPolicyError::InvalidUrl);
        }
        let mut parsed = Url::parse(value).map_err(|_| FetchUrlPolicyError::InvalidUrl)?;
        if !parsed.username().is_empty() || parsed.password().is_some() || parsed.host().is_none() {
            return Err(FetchUrlPolicyError::InvalidUrl);
        }
        parsed.set_fragment(None);
        if parsed.path().is_empty() {
            parsed.set_path("/");
        }

        let host = canonical_host(&mut parsed)?;
        let loopback_host = is_loopback_host(&host);
        match parsed.scheme() {
            "https" => {
                if parsed.port().is_some_and(|port| port != 443) {
                    return Err(FetchUrlPolicyError::ForbiddenPort);
                }
                if parsed.port() == Some(443) {
                    parsed
                        .set_port(None)
                        .map_err(|()| FetchUrlPolicyError::InvalidUrl)?;
                }
            }
            "http" if self.allow_loopback_http && loopback_host => {
                if parsed.port().is_some_and(is_dangerous_port) {
                    return Err(FetchUrlPolicyError::ForbiddenPort);
                }
            }
            _ => return Err(FetchUrlPolicyError::ForbiddenScheme),
        }

        let port = parsed
            .port_or_known_default()
            .ok_or(FetchUrlPolicyError::InvalidUrl)?;
        let scheme = if parsed.scheme() == "https" {
            FetchUrlScheme::Https
        } else {
            FetchUrlScheme::Http
        };
        Ok(ValidatedFetchUrl {
            canonical: parsed.to_string(),
            parsed,
            host,
            port,
            scheme,
        })
    }
}

impl Default for FetchUrlPolicy {
    fn default() -> Self {
        Self::production()
    }
}

/// Validated HTTP scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FetchUrlScheme {
    /// Plain HTTP, available only to explicit loopback fixture policies.
    Http,
    /// Production HTTPS.
    Https,
}

/// Canonical URL safe to pass into DNS and per-hop transport validation.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ValidatedFetchUrl {
    canonical: String,
    parsed: Url,
    host: String,
    port: u16,
    scheme: FetchUrlScheme,
}

impl ValidatedFetchUrl {
    /// Returns the canonical request and cache-key URL.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.canonical
    }

    /// Returns the normalized ASCII host used for DNS and TLS SNI.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Returns the effective destination port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Returns the validated scheme.
    #[must_use]
    pub const fn scheme(&self) -> FetchUrlScheme {
        self.scheme
    }

    /// Resolves one redirect location against this URL.
    ///
    /// # Errors
    ///
    /// Returns an error when `Location` is malformed or violates the selected
    /// URL policy.
    pub fn resolve_redirect(
        &self,
        location: &str,
        policy: &FetchUrlPolicy,
    ) -> Result<Self, FetchUrlPolicyError> {
        if location.is_empty()
            || location.len() > MAX_FETCH_URL_BYTES
            || location.chars().any(char::is_control)
        {
            return Err(FetchUrlPolicyError::InvalidUrl);
        }
        let resolved = self
            .parsed
            .join(location)
            .map_err(|_| FetchUrlPolicyError::InvalidUrl)?;
        policy.validate(resolved.as_str())
    }

    /// Returns whether the URL uses HTTPS.
    #[must_use]
    pub const fn is_https(&self) -> bool {
        matches!(self.scheme, FetchUrlScheme::Https)
    }
}

impl fmt::Debug for ValidatedFetchUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedFetchUrl")
            .field("scheme", &self.scheme)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("path_bytes", &self.parsed.path().len())
            .field("has_query", &self.parsed.query().is_some())
            .finish_non_exhaustive()
    }
}

fn canonical_host(parsed: &mut Url) -> Result<String, FetchUrlPolicyError> {
    let host = match parsed.host().ok_or(FetchUrlPolicyError::InvalidUrl)? {
        Host::Domain(domain) => {
            let domain = domain.strip_suffix('.').unwrap_or(domain).to_owned();
            validate_domain(&domain)?;
            parsed
                .set_host(Some(&domain))
                .map_err(|_| FetchUrlPolicyError::InvalidUrl)?;
            domain.to_ascii_lowercase()
        }
        Host::Ipv4(address) => address.to_string(),
        Host::Ipv6(address) => address.to_string(),
    };
    Ok(host)
}

fn validate_domain(domain: &str) -> Result<(), FetchUrlPolicyError> {
    if domain.is_empty()
        || domain.len() > 253
        || !domain.is_ascii()
        || domain.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        Err(FetchUrlPolicyError::InvalidUrl)
    } else {
        Ok(())
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn is_dangerous_port(port: u16) -> bool {
    matches!(
        port,
        1 | 7
            | 9
            | 11
            | 13
            | 15
            | 17
            | 19..=23
            | 25
            | 37
            | 42..=43
            | 53
            | 69
            | 77
            | 79
            | 87
            | 95
            | 101..=104
            | 109..=111
            | 113
            | 115
            | 117
            | 119
            | 123
            | 135
            | 137..=139
            | 143
            | 161
            | 179
            | 389
            | 427
            | 465
            | 512..=515
            | 526
            | 530..=532
            | 540
            | 548
            | 554
            | 556
            | 563
            | 587
            | 601
            | 636
            | 989..=990
            | 993
            | 995
            | 1719..=1720
            | 1723
            | 2049
            | 3659
            | 4045
            | 5060..=5061
            | 6000
            | 6566
            | 6665..=6669
            | 6697
            | 10080
    )
}
