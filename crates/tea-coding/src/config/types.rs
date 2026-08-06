use serde::{Deserialize, Serialize};
use tea_coding_tools::{
    DEFAULT_FETCH_CACHE_ENTRIES, DEFAULT_FETCH_CACHE_ENTRY_BYTES, DEFAULT_FETCH_CACHE_TOTAL_BYTES,
    DEFAULT_FETCH_CACHE_TTL, DEFAULT_TAVILY_SEARCH_ENDPOINT,
};
use tea_mcp::McpServerConfig;
use tea_model::{WebSearchLocation, WebSearchOptions};

use crate::mcp_config::McpServerSettings;

/// Current persisted coding settings schema.
pub const CODING_SETTINGS_SCHEMA_VERSION: u32 = 1;
/// Default environment variable consulted by an embedding product for Tavily credentials.
pub const DEFAULT_WEB_SEARCH_API_KEY_ENVIRONMENT: &str = "TAVILY_API_KEY";
/// Default client web-search request timeout.
pub const DEFAULT_WEB_SEARCH_TIMEOUT_MILLIS: u64 = 30_000;
/// Maximum bytes in one configured API-key environment variable name.
pub const MAX_WEB_SEARCH_API_KEY_ENVIRONMENT_BYTES: usize = 128;

/// Behavior when project-local configuration exists without a saved decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProjectTrustDefault {
    /// Interactive products may ask; headless products deny.
    #[default]
    Ask,
    /// Ignore all project-local settings and declarative resources.
    Ignore,
    /// Fail closed when project-local settings or resources are requested.
    Reject,
}

/// Versioned, secret-free resolved settings snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodingSettings {
    /// Persisted schema version.
    pub schema_version: u32,
    /// Provider selector.
    pub provider: String,
    /// Model selector.
    pub model: String,
    /// Thinking mode selector.
    pub thinking: String,
    /// Canonical active tool names.
    pub active_tools: Vec<String>,
    /// Session database override, relative to injected state dir when absent.
    pub session_database: Option<String>,
    /// Maximum model retries.
    pub max_retries: u32,
    /// Base delay before the first model retry, in milliseconds.
    pub retry_base_delay_ms: u64,
    /// Maximum model retry delay, including provider hints, in milliseconds.
    pub retry_max_delay_ms: u64,
    /// Whether automatic compaction is enabled.
    pub compaction_enabled: bool,
    /// TUI settings.
    pub tui: TuiSettings,
    /// Declarative resource settings.
    pub resources: ResourceSettings,
    /// Registered web-search routing and backend controls.
    #[serde(default)]
    pub web_search: WebSearchSettings,
    /// Explicit client web-fetch backend and bounded cache controls.
    #[serde(default)]
    pub web_fetch: WebFetchSettings,
    /// Default project trust behavior.
    pub project_trust: ProjectTrustDefault,
    /// Validated runtime-only MCP server configs, omitted from durable settings.
    #[serde(skip, default)]
    pub mcp_servers: Vec<McpServerConfig>,
}

impl Default for CodingSettings {
    fn default() -> Self {
        Self {
            schema_version: CODING_SETTINGS_SCHEMA_VERSION,
            provider: "openai".to_owned(),
            model: "gpt-5.4".to_owned(),
            thinking: "medium".to_owned(),
            active_tools: vec!["read", "write", "edit", "bash"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            session_database: None,
            max_retries: 3,
            retry_base_delay_ms: 2_000,
            retry_max_delay_ms: 60_000,
            compaction_enabled: false,
            tui: TuiSettings::default(),
            resources: ResourceSettings::default(),
            web_search: WebSearchSettings::default(),
            web_fetch: WebFetchSettings::default(),
            project_trust: ProjectTrustDefault::Ask,
            mcp_servers: Vec::new(),
        }
    }
}

/// Route selected for web search when a real client backend is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchRoutePreference {
    /// Prefer provider-hosted search and fall back to the client on unsupported models.
    #[default]
    PreferHosted,
    /// Always expose the client function route.
    ForceClient,
}

/// Client web-search backend selected by the embedding product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchClientBackend {
    /// Tavily's bounded search HTTP API.
    #[default]
    Tavily,
}

/// Secret-free resolved web-search settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSearchSettings {
    /// Route preference applied only when the client backend is enabled and injected.
    pub route_preference: WebSearchRoutePreference,
    /// Canonical domain allowlist shared by hosted and client search.
    pub allowed_domains: Vec<String>,
    /// Canonical domain blocklist shared by hosted and client search.
    pub blocked_domains: Vec<String>,
    /// Optional approximate location disclosed to capable hosted providers.
    pub location: Option<WebSearchLocationSettings>,
    /// Client-backend construction controls for embedding products.
    pub client: ClientWebSearchSettings,
}

impl Default for WebSearchSettings {
    fn default() -> Self {
        Self {
            route_preference: WebSearchRoutePreference::PreferHosted,
            allowed_domains: Vec::new(),
            blocked_domains: Vec::new(),
            location: None,
            client: ClientWebSearchSettings::default(),
        }
    }
}

impl WebSearchSettings {
    pub(crate) fn runtime_options(&self) -> Result<WebSearchOptions, tea_model::ModelRequestError> {
        let mut options = WebSearchOptions::new();
        if !self.allowed_domains.is_empty() {
            options = options.with_allowed_domains(self.allowed_domains.clone())?;
        }
        if !self.blocked_domains.is_empty() {
            options = options.with_blocked_domains(self.blocked_domains.clone())?;
        }
        if let Some(configured) = &self.location {
            let mut location = WebSearchLocation::new();
            if let Some(country) = &configured.country {
                location = location.with_country(country.clone())?;
            }
            if let Some(city) = &configured.city {
                location = location.with_city(city.clone())?;
            }
            if let Some(region) = &configured.region {
                location = location.with_region(region.clone())?;
            }
            if let Some(timezone) = &configured.timezone {
                location = location.with_timezone(timezone.clone())?;
            }
            options = options.with_location(location);
        }
        Ok(options)
    }
}

/// Approximate location fields used by hosted web search.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSearchLocationSettings {
    /// ISO 3166-1 alpha-2 uppercase country code.
    pub country: Option<String>,
    /// Approximate city name.
    pub city: Option<String>,
    /// Approximate region name.
    pub region: Option<String>,
    /// Canonical IANA-style timezone name.
    pub timezone: Option<String>,
}

/// Client web-search backend controls resolved without reading credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ClientWebSearchSettings {
    /// Whether an embedding product may attach a real client backend.
    pub enabled: bool,
    /// Backend configuration contract expected by the injected provider.
    pub backend: WebSearchClientBackend,
    /// Validated Tavily-compatible HTTP endpoint.
    pub endpoint: String,
    /// Environment variable name resolved by the embedding product at construction time.
    pub api_key_environment: String,
    /// Per-request timeout in milliseconds.
    pub timeout_millis: u64,
}

impl Default for ClientWebSearchSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: WebSearchClientBackend::Tavily,
            endpoint: DEFAULT_TAVILY_SEARCH_ENDPOINT.to_owned(),
            api_key_environment: DEFAULT_WEB_SEARCH_API_KEY_ENVIRONMENT.to_owned(),
            timeout_millis: DEFAULT_WEB_SEARCH_TIMEOUT_MILLIS,
        }
    }
}

/// Client web-fetch backend selected by the embedding product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebFetchBackend {
    /// Pinned, redirect-safe HTTP transport with bounded response extraction.
    #[default]
    Http,
}

/// Secret-free resolved web-fetch settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WebFetchSettings {
    /// Whether an embedding product may attach a real fetch backend.
    pub enabled: bool,
    /// Backend configuration contract expected by the injected provider.
    pub backend: WebFetchBackend,
    /// Bounded normalized-result cache controls.
    pub cache: WebFetchCacheSettings,
}

impl Default for WebFetchSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: WebFetchBackend::Http,
            cache: WebFetchCacheSettings::default(),
        }
    }
}

/// Bounded in-memory cache controls for normalized fetch results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WebFetchCacheSettings {
    /// Cache lifetime in whole seconds.
    pub ttl_seconds: u64,
    /// Maximum cached result count.
    pub max_entries: usize,
    /// Maximum aggregate cache bytes.
    pub max_total_bytes: usize,
    /// Maximum bytes retained by one entry.
    pub max_entry_bytes: usize,
}

impl Default for WebFetchCacheSettings {
    fn default() -> Self {
        Self {
            ttl_seconds: DEFAULT_FETCH_CACHE_TTL.as_secs(),
            max_entries: DEFAULT_FETCH_CACHE_ENTRIES,
            max_total_bytes: DEFAULT_FETCH_CACHE_TOTAL_BYTES,
            max_entry_bytes: DEFAULT_FETCH_CACHE_ENTRY_BYTES,
        }
    }
}

/// Nested TUI settings merged independently by layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TuiSettings {
    /// Codex-style inline viewport or explicit fullscreen compatibility mode.
    pub viewport: String,
    /// Whether thinking blocks start collapsed.
    pub collapse_thinking: bool,
    /// Whether the terminal UI avoids animated status indicators.
    pub reduced_motion: bool,
    /// Submit key binding.
    pub submit_key: String,
    /// Newline key binding.
    pub newline_key: String,
    /// Active-run abort key binding.
    pub abort_key: String,
    /// Editor clear key binding.
    pub clear_key: String,
    /// Application exit key binding.
    pub exit_key: String,
    /// Model selector key binding.
    pub model_key: String,
    /// Thinking-collapse key binding.
    pub toggle_thinking_key: String,
    /// Tool-collapse key binding.
    pub toggle_tools_key: String,
    /// Copy key binding.
    pub copy_key: String,
    /// Active-run steering key binding.
    pub steering_key: String,
    /// Active-run follow-up key binding.
    pub follow_up_key: String,
    /// Queued-message retrieval key binding.
    pub retrieve_queued_key: String,
}

impl Default for TuiSettings {
    fn default() -> Self {
        Self {
            viewport: "inline".to_owned(),
            collapse_thinking: false,
            reduced_motion: false,
            submit_key: "enter".to_owned(),
            newline_key: "shift+enter".to_owned(),
            abort_key: "escape".to_owned(),
            clear_key: "ctrl+l".to_owned(),
            exit_key: "ctrl+d".to_owned(),
            model_key: "ctrl+o".to_owned(),
            toggle_thinking_key: "ctrl+t".to_owned(),
            toggle_tools_key: "ctrl+g".to_owned(),
            copy_key: "ctrl+y".to_owned(),
            steering_key: "enter".to_owned(),
            follow_up_key: "alt+enter".to_owned(),
            retrieve_queued_key: "ctrl+r".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TuiSettings;

    #[test]
    fn tui_defaults_to_codex_style_inline_viewport() {
        assert_eq!(TuiSettings::default().viewport, "inline");
    }
}

/// Declarative resource discovery settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceSettings {
    /// Whether workspace context files are loaded.
    pub context_files: bool,
    /// Additional bounded skill paths.
    pub skill_paths: Vec<String>,
    /// Whether prompt templates are loaded.
    pub prompt_templates: bool,
}

impl Default for ResourceSettings {
    fn default() -> Self {
        Self {
            context_files: true,
            skill_paths: Vec::new(),
            prompt_templates: true,
        }
    }
}

/// Sparse one-layer settings overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsLayer {
    /// Optional schema version for files.
    pub schema_version: Option<u32>,
    /// Provider override.
    pub provider: Option<String>,
    /// Model override.
    pub model: Option<String>,
    /// Thinking override.
    pub thinking: Option<String>,
    /// Active tools replacement.
    pub active_tools: Option<Vec<String>>,
    /// Session database override.
    pub session_database: Option<String>,
    /// Retry override.
    pub max_retries: Option<u32>,
    /// Base model-retry delay override, in milliseconds.
    pub retry_base_delay_ms: Option<u64>,
    /// Maximum model-retry delay override, in milliseconds.
    pub retry_max_delay_ms: Option<u64>,
    /// Compaction override.
    pub compaction_enabled: Option<bool>,
    /// Nested TUI overlay.
    pub tui: Option<TuiSettingsLayer>,
    /// Nested resource overlay.
    pub resources: Option<ResourceSettingsLayer>,
    /// Nested web-search overlay.
    pub web_search: Option<WebSearchSettingsLayer>,
    /// Nested web-fetch overlay.
    pub web_fetch: Option<WebFetchSettingsLayer>,
    /// Project trust default override.
    pub project_trust: Option<ProjectTrustDefault>,
    /// Full MCP server replacements merged by canonical server ID.
    pub mcp_servers: Option<Vec<McpServerSettings>>,
}

/// Sparse web-search settings overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSearchSettingsLayer {
    /// Route-preference override.
    pub route_preference: Option<WebSearchRoutePreference>,
    /// Domain allowlist replacement.
    pub allowed_domains: Option<Vec<String>>,
    /// Domain blocklist replacement.
    pub blocked_domains: Option<Vec<String>>,
    /// Nested approximate-location overlay.
    pub location: Option<WebSearchLocationSettingsLayer>,
    /// Nested client-backend overlay.
    pub client: Option<ClientWebSearchSettingsLayer>,
}

/// Sparse approximate-location overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSearchLocationSettingsLayer {
    /// Country override.
    pub country: Option<String>,
    /// City override.
    pub city: Option<String>,
    /// Region override.
    pub region: Option<String>,
    /// Timezone override.
    pub timezone: Option<String>,
}

/// Sparse client web-search backend overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClientWebSearchSettingsLayer {
    /// Client-backend enablement override.
    pub enabled: Option<bool>,
    /// Client-backend selector override.
    pub backend: Option<WebSearchClientBackend>,
    /// Endpoint override.
    pub endpoint: Option<String>,
    /// API-key environment variable name override.
    pub api_key_environment: Option<String>,
    /// Request-timeout override in milliseconds.
    pub timeout_millis: Option<u64>,
}

/// Sparse web-fetch settings overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebFetchSettingsLayer {
    /// Backend enablement override.
    pub enabled: Option<bool>,
    /// Backend selector override.
    pub backend: Option<WebFetchBackend>,
    /// Nested bounded-cache overlay.
    pub cache: Option<WebFetchCacheSettingsLayer>,
}

/// Sparse normalized-result cache overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebFetchCacheSettingsLayer {
    /// Cache lifetime override in whole seconds.
    pub ttl_seconds: Option<u64>,
    /// Maximum entry-count override.
    pub max_entries: Option<usize>,
    /// Maximum aggregate-byte override.
    pub max_total_bytes: Option<usize>,
    /// Maximum per-entry byte override.
    pub max_entry_bytes: Option<usize>,
}

/// Sparse TUI overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TuiSettingsLayer {
    /// Viewport override.
    pub viewport: Option<String>,
    /// Collapse override.
    pub collapse_thinking: Option<bool>,
    /// Reduced-motion override.
    pub reduced_motion: Option<bool>,
    /// Submit key override.
    pub submit_key: Option<String>,
    /// Newline key override.
    pub newline_key: Option<String>,
    /// Abort key override.
    pub abort_key: Option<String>,
    /// Clear key override.
    pub clear_key: Option<String>,
    /// Exit key override.
    pub exit_key: Option<String>,
    /// Model selector key override.
    pub model_key: Option<String>,
    /// Thinking toggle key override.
    pub toggle_thinking_key: Option<String>,
    /// Tool toggle key override.
    pub toggle_tools_key: Option<String>,
    /// Copy key override.
    pub copy_key: Option<String>,
    /// Steering key override.
    pub steering_key: Option<String>,
    /// Follow-up key override.
    pub follow_up_key: Option<String>,
    /// Queue retrieval key override.
    pub retrieve_queued_key: Option<String>,
}

/// Sparse resource overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceSettingsLayer {
    /// Context-file override.
    pub context_files: Option<bool>,
    /// Skill paths replacement.
    pub skill_paths: Option<Vec<String>>,
    /// Prompt-template override.
    pub prompt_templates: Option<bool>,
}
