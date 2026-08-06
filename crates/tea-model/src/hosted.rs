use std::collections::BTreeSet;

use crate::ModelRequestError;

/// Maximum number of domains in one portable web-search policy.
pub const MAX_WEB_SEARCH_DOMAINS: usize = 100;
/// Maximum bytes in one canonical web-search domain.
pub const MAX_WEB_SEARCH_DOMAIN_BYTES: usize = 253;
/// Maximum bytes in one approximate location field.
pub const MAX_WEB_SEARCH_LOCATION_FIELD_BYTES: usize = 128;

/// Provider-neutral kind of tool executed by the model provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostedToolKind {
    /// Searches the public web inside the provider response lifecycle.
    WebSearch,
}

impl HostedToolKind {
    /// Returns the stable model-visible tool name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::WebSearch => "web_search",
        }
    }
}

/// Bounded approximate user location for hosted web search.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebSearchLocation {
    country: Option<String>,
    city: Option<String>,
    region: Option<String>,
    timezone: Option<String>,
}

impl WebSearchLocation {
    /// Creates an empty location that can be populated through validated builders.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            country: None,
            city: None,
            region: None,
            timezone: None,
        }
    }

    /// Sets an ISO 3166-1 alpha-2 uppercase country code.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is not exactly two uppercase ASCII letters.
    pub fn with_country(mut self, country: impl Into<String>) -> Result<Self, ModelRequestError> {
        let country = country.into();
        if country.len() != 2 || !country.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(ModelRequestError::InvalidWebSearchLocation);
        }
        self.country = Some(country);
        Ok(self)
    }

    /// Sets a bounded city name.
    ///
    /// # Errors
    ///
    /// Returns an error for empty, oversized, or control-containing text.
    pub fn with_city(mut self, city: impl Into<String>) -> Result<Self, ModelRequestError> {
        self.city = Some(validate_location_text(city.into())?);
        Ok(self)
    }

    /// Sets a bounded region name.
    ///
    /// # Errors
    ///
    /// Returns an error for empty, oversized, or control-containing text.
    pub fn with_region(mut self, region: impl Into<String>) -> Result<Self, ModelRequestError> {
        self.region = Some(validate_location_text(region.into())?);
        Ok(self)
    }

    /// Sets a canonical IANA-style timezone name.
    ///
    /// # Errors
    ///
    /// Returns an error for empty, oversized, or non-canonical text.
    pub fn with_timezone(mut self, timezone: impl Into<String>) -> Result<Self, ModelRequestError> {
        let timezone = timezone.into();
        if timezone.is_empty()
            || timezone.len() > MAX_WEB_SEARCH_LOCATION_FIELD_BYTES
            || !timezone.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'+')
            })
        {
            return Err(ModelRequestError::InvalidWebSearchLocation);
        }
        self.timezone = Some(timezone);
        Ok(self)
    }

    /// Returns the optional country code.
    #[must_use]
    pub fn country(&self) -> Option<&str> {
        self.country.as_deref()
    }

    /// Returns the optional city.
    #[must_use]
    pub fn city(&self) -> Option<&str> {
        self.city.as_deref()
    }

    /// Returns the optional region.
    #[must_use]
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// Returns the optional timezone.
    #[must_use]
    pub fn timezone(&self) -> Option<&str> {
        self.timezone.as_deref()
    }

    /// Returns whether no location field is configured.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.country.is_none()
            && self.city.is_none()
            && self.region.is_none()
            && self.timezone.is_none()
    }
}

/// Portable policy shared by hosted web-search adapters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebSearchOptions {
    allowed_domains: Vec<String>,
    blocked_domains: Vec<String>,
    location: Option<WebSearchLocation>,
}

impl WebSearchOptions {
    /// Creates unrestricted web-search options with no location disclosure.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            allowed_domains: Vec::new(),
            blocked_domains: Vec::new(),
            location: None,
        }
    }

    /// Sets a canonical allowlist, replacing any previous allowlist.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid domains, too many domains, or a configured blocklist.
    pub fn with_allowed_domains<I, S>(mut self, domains: I) -> Result<Self, ModelRequestError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if !self.blocked_domains.is_empty() {
            return Err(ModelRequestError::ConflictingWebSearchDomainFilters);
        }
        self.allowed_domains = collect_domains(domains)?;
        Ok(self)
    }

    /// Sets a canonical blocklist, replacing any previous blocklist.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid domains, too many domains, or a configured allowlist.
    pub fn with_blocked_domains<I, S>(mut self, domains: I) -> Result<Self, ModelRequestError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if !self.allowed_domains.is_empty() {
            return Err(ModelRequestError::ConflictingWebSearchDomainFilters);
        }
        self.blocked_domains = collect_domains(domains)?;
        Ok(self)
    }

    /// Adds an approximate location disclosed only after the tool is active.
    #[must_use]
    pub fn with_location(mut self, location: WebSearchLocation) -> Self {
        self.location = (!location.is_empty()).then_some(location);
        self
    }

    /// Returns allowed domains in deterministic canonical order.
    #[must_use]
    pub fn allowed_domains(&self) -> &[String] {
        &self.allowed_domains
    }

    /// Returns blocked domains in deterministic canonical order.
    #[must_use]
    pub fn blocked_domains(&self) -> &[String] {
        &self.blocked_domains
    }

    /// Returns the optional approximate location.
    #[must_use]
    pub const fn location(&self) -> Option<&WebSearchLocation> {
        self.location.as_ref()
    }
}

/// Provider-neutral options for one hosted tool definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostedToolOptions {
    /// Hosted web-search policy.
    WebSearch(WebSearchOptions),
}

impl HostedToolOptions {
    /// Returns the hosted capability kind required by these options.
    #[must_use]
    pub const fn kind(&self) -> HostedToolKind {
        match self {
            Self::WebSearch(_) => HostedToolKind::WebSearch,
        }
    }

    /// Returns web-search options.
    #[must_use]
    pub const fn web_search(&self) -> &WebSearchOptions {
        match self {
            Self::WebSearch(options) => options,
        }
    }
}

fn collect_domains<I, S>(domains: I) -> Result<Vec<String>, ModelRequestError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut values = BTreeSet::new();
    for domain in domains {
        if values.len() == MAX_WEB_SEARCH_DOMAINS {
            return Err(ModelRequestError::TooManyWebSearchDomains);
        }
        let domain = domain.into();
        validate_domain(&domain)?;
        values.insert(domain);
    }
    Ok(values.into_iter().collect())
}

fn validate_domain(domain: &str) -> Result<(), ModelRequestError> {
    if domain.is_empty()
        || domain.len() > MAX_WEB_SEARCH_DOMAIN_BYTES
        || domain.starts_with('.')
        || domain.ends_with('.')
        || !domain.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
        || domain.split('.').any(|label| {
            label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-')
        })
    {
        Err(ModelRequestError::InvalidWebSearchDomain)
    } else {
        Ok(())
    }
}

fn validate_location_text(value: String) -> Result<String, ModelRequestError> {
    if value.is_empty()
        || value.len() > MAX_WEB_SEARCH_LOCATION_FIELD_BYTES
        || value.chars().any(char::is_control)
    {
        Err(ModelRequestError::InvalidWebSearchLocation)
    } else {
        Ok(value)
    }
}
