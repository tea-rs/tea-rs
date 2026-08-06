//! Credential resolution for the OpenAI-compatible adapter.
//!
//! The adapter stores no secrets. A [`CredentialResolver`] returns a bounded
//! [`ApiKey`] and connection config at request time. The crate provides an
//! [`EnvCredentialResolver`] reading the committed `TEA_OPENAI_*` env
//! contract; a test-only `.env` loader (no `dotenv` dependency) populates the
//! process env for the live smoke test.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use tea_model::ProviderId;
use tea_protocol::ModelId;

use crate::error::{OpenAiError, OpenAiErrorCode};

/// Default provider identity advertised by this adapter.
pub const PROVIDER_ID: &str = "openai";

/// `OpenAI` HTTP API used for model requests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OpenAiApiMode {
    /// The legacy `/chat/completions` endpoint.
    #[default]
    ChatCompletions,
    /// The `/responses` endpoint.
    Responses,
}

/// Bounded API key value that never appears in debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Creates a bounded non-empty key.
    ///
    /// # Errors
    ///
    /// Returns an error for empty or oversized values.
    pub fn new(value: impl Into<String>) -> Result<Self, OpenAiError> {
        let value = value.into();
        if value.is_empty() || value.len() > 512 || value.contains('\0') {
            return Err(OpenAiError::new(
                OpenAiErrorCode::Authentication,
                "api key is invalid",
            ));
        }
        Ok(Self(value))
    }

    /// Returns the raw key value (caller must not log it).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiKey(**REDACTED**)")
    }
}

impl FromStr for ApiKey {
    type Err = OpenAiError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

/// Immutable OpenAI-compatible connection configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiConfig {
    provider_id: ProviderId,
    model_id: ModelId,
    base_url: String,
    api_key: ApiKey,
    api_key_header: String,
    api_key_prefix: String,
    api_mode: OpenAiApiMode,
    org_id: Option<String>,
    project_id: Option<String>,
    reasoning_effort: Option<String>,
    vision: bool,
    timeout_millis: u64,
}

impl OpenAiConfig {
    /// Creates an explicit OpenAI-compatible connection configuration.
    ///
    /// The configuration uses the default provider identity, bearer-token
    /// authentication, Chat Completions API mode, and a 60-second timeout.
    ///
    /// # Errors
    ///
    /// Returns an error when the base URL is empty or contains a null byte.
    pub fn new(
        model_id: ModelId,
        base_url: impl Into<String>,
        api_key: ApiKey,
    ) -> Result<Self, OpenAiError> {
        let base_url = base_url.into();
        if base_url.is_empty() || base_url.contains('\0') {
            return Err(OpenAiError::new(
                OpenAiErrorCode::InvalidRequest,
                "base URL is invalid",
            ));
        }
        Ok(Self {
            provider_id: default_provider_id(),
            model_id,
            base_url,
            api_key,
            api_key_header: "Authorization".to_owned(),
            api_key_prefix: "Bearer ".to_owned(),
            api_mode: OpenAiApiMode::ChatCompletions,
            org_id: None,
            project_id: None,
            reasoning_effort: None,
            vision: false,
            timeout_millis: 60_000,
        })
    }

    /// Returns the provider identity.
    #[must_use]
    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }
    /// Returns the configured model used for catalog initialization.
    #[must_use]
    pub const fn model_id(&self) -> &ModelId {
        &self.model_id
    }
    /// Returns the base URL (without `/chat/completions`).
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
    /// Returns the API key.
    #[must_use]
    pub fn api_key(&self) -> &ApiKey {
        &self.api_key
    }
    /// Returns the header name carrying the key.
    #[must_use]
    pub fn api_key_header(&self) -> &str {
        &self.api_key_header
    }
    /// Returns the prefix prepended to the key value.
    #[must_use]
    pub fn api_key_prefix(&self) -> &str {
        &self.api_key_prefix
    }
    /// Returns the configured `OpenAI` API mode.
    #[must_use]
    pub const fn api_mode(&self) -> OpenAiApiMode {
        self.api_mode
    }
    /// Overrides the `OpenAI` API mode.
    #[must_use]
    pub const fn with_api_mode(mut self, api_mode: OpenAiApiMode) -> Self {
        self.api_mode = api_mode;
        self
    }
    /// Returns the optional organization id.
    #[must_use]
    pub fn org_id(&self) -> Option<&str> {
        self.org_id.as_deref()
    }
    /// Returns the optional project id.
    #[must_use]
    pub fn project_id(&self) -> Option<&str> {
        self.project_id.as_deref()
    }
    /// Returns the optional reasoning effort.
    #[must_use]
    pub fn reasoning_effort(&self) -> Option<&str> {
        self.reasoning_effort.as_deref()
    }
    /// Returns whether image input is enabled for the smoke test.
    #[must_use]
    pub const fn vision(&self) -> bool {
        self.vision
    }
    /// Returns the per-request timeout in milliseconds.
    #[must_use]
    pub const fn timeout_millis(&self) -> u64 {
        self.timeout_millis
    }
}

/// Object-safe port that resolves connection configuration at request time.
///
/// Implementations must not retain secrets beyond the returned config.
pub trait CredentialResolver: fmt::Debug + Send + Sync {
    /// Resolves the connection configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when required configuration (api key, base url) is
    /// missing or invalid.
    fn resolve(&self) -> Result<OpenAiConfig, OpenAiError>;
}

/// Resolves [`OpenAiConfig`] from the `TEA_OPENAI_*` environment contract.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvCredentialResolver;

impl EnvCredentialResolver {
    /// Creates the env-backed resolver.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl CredentialResolver for EnvCredentialResolver {
    fn resolve(&self) -> Result<OpenAiConfig, OpenAiError> {
        resolve_config(default_provider_id(), |key| std::env::var(key).ok())
    }
}

/// Resolves [`OpenAiConfig`] from a supplied key-value map (e.g. a parsed
/// `.env` file). Used by the live smoke test to avoid mutating the process
/// environment (and thus avoid `unsafe`).
#[derive(Clone)]
pub struct MapCredentialResolver {
    provider_id: ProviderId,
    values: BTreeMap<String, String>,
}

impl MapCredentialResolver {
    /// Creates a resolver backed by the supplied map.
    #[must_use]
    pub fn new(values: BTreeMap<String, String>) -> Self {
        Self {
            provider_id: default_provider_id(),
            values,
        }
    }

    /// Creates a resolver for one custom OpenAI-compatible provider identity.
    #[must_use]
    pub fn for_provider(provider_id: ProviderId, values: BTreeMap<String, String>) -> Self {
        Self {
            provider_id,
            values,
        }
    }
}

impl Default for MapCredentialResolver {
    fn default() -> Self {
        Self::new(BTreeMap::new())
    }
}

impl fmt::Debug for MapCredentialResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MapCredentialResolver")
            .field("values", &"**REDACTED**")
            .finish()
    }
}

impl CredentialResolver for MapCredentialResolver {
    fn resolve(&self) -> Result<OpenAiConfig, OpenAiError> {
        resolve_config(self.provider_id.clone(), |key| {
            self.values.get(key).cloned()
        })
    }
}

/// Shared configuration builder parameterized by a value lookup.
fn resolve_config(
    provider_id: ProviderId,
    get: impl Fn(&str) -> Option<String>,
) -> Result<OpenAiConfig, OpenAiError> {
    let api_key = get("TEA_OPENAI_API_KEY")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            OpenAiError::new(
                OpenAiErrorCode::Authentication,
                "TEA_OPENAI_API_KEY is not set",
            )
        })?;
    let model_id = get("TEA_OPENAI_MODEL")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            OpenAiError::new(
                OpenAiErrorCode::Authentication,
                "TEA_OPENAI_MODEL is not set",
            )
        })?
        .parse::<ModelId>()
        .map_err(|_| {
            OpenAiError::new(
                OpenAiErrorCode::InvalidRequest,
                "TEA_OPENAI_MODEL is invalid",
            )
        })?;
    let base_url = get("TEA_OPENAI_BASE_URL")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "https://api.openai.com/v1".to_owned());
    let api_key_header = get("TEA_OPENAI_API_KEY_HEADER")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Authorization".to_owned());
    let api_key_prefix = get("TEA_OPENAI_API_KEY_PREFIX").map_or_else(
        || "Bearer ".to_owned(),
        |value| {
            if value == "__NONE__" {
                String::new()
            } else {
                value
            }
        },
    );
    let api_mode = match get("TEA_OPENAI_API_MODE").as_deref() {
        None | Some("" | "chat-completions") => OpenAiApiMode::ChatCompletions,
        Some("responses") => OpenAiApiMode::Responses,
        Some(_) => {
            return Err(OpenAiError::new(
                OpenAiErrorCode::InvalidRequest,
                "TEA_OPENAI_API_MODE must be chat-completions or responses",
            ));
        }
    };
    let org_id = get("TEA_OPENAI_ORG_ID").filter(|value| !value.is_empty());
    let project_id = get("TEA_OPENAI_PROJECT_ID").filter(|value| !value.is_empty());
    let reasoning_effort = get("TEA_OPENAI_REASONING_EFFORT")
        .filter(|value| value.parse::<tea_protocol::ReasoningEffort>().is_ok());
    let vision = get("TEA_OPENAI_VISION")
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
    let timeout_millis = get("TEA_OPENAI_REQUEST_TIMEOUT_MS")
        .and_then(|value| value.parse().ok())
        .filter(|value: &u64| *value > 0)
        .unwrap_or(60_000);
    Ok(OpenAiConfig {
        provider_id,
        model_id,
        base_url,
        api_key: ApiKey::new(api_key)?,
        api_key_header,
        api_key_prefix,
        api_mode,
        org_id,
        project_id,
        reasoning_effort,
        vision,
        timeout_millis,
    })
}

fn default_provider_id() -> ProviderId {
    ProviderId::from_str(PROVIDER_ID).expect("provider id is canonical")
}
