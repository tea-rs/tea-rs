use std::time::Duration;

use tea_coding_tools::{FetchCacheConfig, TavilySearchConfig};
use tea_kernel::ModelRetryPolicy;

use crate::config::{CodingSettings, SettingsLayer};
use crate::mcp_config::{merge_mcp_server_settings, validate_mcp_server_aliases};
use crate::{CodingError, CodingErrorCode};

/// Deterministically resolves defaults < global < trusted project < env < CLI.
///
/// # Errors
///
/// Rejects invalid versions, selectors, bounds, and duplicate/invalid tools.
pub fn merge_settings(
    defaults: CodingSettings,
    global: Option<&SettingsLayer>,
    trusted_project: Option<&SettingsLayer>,
    environment: Option<&SettingsLayer>,
    cli: Option<&SettingsLayer>,
) -> Result<CodingSettings, CodingError> {
    let mut resolved = defaults;
    for layer in [global, trusted_project, environment, cli]
        .into_iter()
        .flatten()
    {
        apply(&mut resolved, layer)?;
    }
    canonicalize_web_search(&mut resolved)?;
    validate(&resolved)?;
    Ok(resolved)
}

fn apply(settings: &mut CodingSettings, layer: &SettingsLayer) -> Result<(), CodingError> {
    if layer
        .schema_version
        .is_some_and(|version| version != crate::config::CODING_SETTINGS_SCHEMA_VERSION)
    {
        return Err(invalid());
    }
    if let Some(value) = &layer.provider {
        settings.provider.clone_from(value);
    }
    if let Some(value) = &layer.model {
        settings.model.clone_from(value);
    }
    if let Some(value) = &layer.thinking {
        settings.thinking.clone_from(value);
    }
    if let Some(value) = &layer.active_tools {
        settings.active_tools.clone_from(value);
    }
    if let Some(value) = &layer.session_database {
        settings.session_database = Some(value.clone());
    }
    if let Some(value) = layer.max_retries {
        settings.max_retries = value;
    }
    if let Some(value) = layer.retry_base_delay_ms {
        settings.retry_base_delay_ms = value;
    }
    if let Some(value) = layer.retry_max_delay_ms {
        settings.retry_max_delay_ms = value;
    }
    if let Some(value) = layer.compaction_enabled {
        settings.compaction_enabled = value;
    }
    if let Some(value) = layer.project_trust {
        settings.project_trust = value;
    }
    if let Some(servers) = &layer.mcp_servers {
        merge_mcp_server_settings(&mut settings.mcp_servers, servers)?;
    }
    if let Some(tui) = &layer.tui {
        if let Some(value) = &tui.viewport {
            settings.tui.viewport.clone_from(value);
        }
        if let Some(value) = tui.collapse_thinking {
            settings.tui.collapse_thinking = value;
        }
        if let Some(value) = tui.reduced_motion {
            settings.tui.reduced_motion = value;
        }
        if let Some(value) = &tui.submit_key {
            settings.tui.submit_key.clone_from(value);
        }
        macro_rules! merge_key {
            ($field:ident) => {
                if let Some(value) = &tui.$field {
                    settings.tui.$field.clone_from(value);
                }
            };
        }
        merge_key!(newline_key);
        merge_key!(abort_key);
        merge_key!(clear_key);
        merge_key!(exit_key);
        merge_key!(model_key);
        merge_key!(toggle_thinking_key);
        merge_key!(toggle_tools_key);
        merge_key!(copy_key);
        merge_key!(steering_key);
        merge_key!(follow_up_key);
        merge_key!(retrieve_queued_key);
    }
    if let Some(resources) = &layer.resources {
        if let Some(value) = resources.context_files {
            settings.resources.context_files = value;
        }
        if let Some(value) = &resources.skill_paths {
            settings.resources.skill_paths.clone_from(value);
        }
        if let Some(value) = resources.prompt_templates {
            settings.resources.prompt_templates = value;
        }
    }
    if let Some(web_search) = &layer.web_search {
        apply_web_search(&mut settings.web_search, web_search);
    }
    if let Some(web_fetch) = &layer.web_fetch {
        apply_web_fetch(&mut settings.web_fetch, web_fetch);
    }
    Ok(())
}

fn apply_web_search(
    settings: &mut crate::config::WebSearchSettings,
    layer: &crate::config::WebSearchSettingsLayer,
) {
    if let Some(value) = layer.route_preference {
        settings.route_preference = value;
    }
    if let Some(value) = &layer.allowed_domains {
        settings.allowed_domains.clone_from(value);
    }
    if let Some(value) = &layer.blocked_domains {
        settings.blocked_domains.clone_from(value);
    }
    if let Some(location) = &layer.location {
        let resolved = settings.location.get_or_insert_with(Default::default);
        if let Some(value) = &location.country {
            resolved.country = Some(value.clone());
        }
        if let Some(value) = &location.city {
            resolved.city = Some(value.clone());
        }
        if let Some(value) = &location.region {
            resolved.region = Some(value.clone());
        }
        if let Some(value) = &location.timezone {
            resolved.timezone = Some(value.clone());
        }
    }
    if let Some(client) = &layer.client {
        if let Some(value) = client.enabled {
            settings.client.enabled = value;
        }
        if let Some(value) = client.backend {
            settings.client.backend = value;
        }
        if let Some(value) = &client.endpoint {
            settings.client.endpoint.clone_from(value);
        }
        if let Some(value) = &client.api_key_environment {
            settings.client.api_key_environment.clone_from(value);
        }
        if let Some(value) = client.timeout_millis {
            settings.client.timeout_millis = value;
        }
    }
}

fn apply_web_fetch(
    settings: &mut crate::config::WebFetchSettings,
    layer: &crate::config::WebFetchSettingsLayer,
) {
    if let Some(value) = layer.enabled {
        settings.enabled = value;
    }
    if let Some(value) = layer.backend {
        settings.backend = value;
    }
    if let Some(cache) = &layer.cache {
        if let Some(value) = cache.ttl_seconds {
            settings.cache.ttl_seconds = value;
        }
        if let Some(value) = cache.max_entries {
            settings.cache.max_entries = value;
        }
        if let Some(value) = cache.max_total_bytes {
            settings.cache.max_total_bytes = value;
        }
        if let Some(value) = cache.max_entry_bytes {
            settings.cache.max_entry_bytes = value;
        }
    }
}

pub(crate) fn validate(settings: &CodingSettings) -> Result<(), CodingError> {
    let retry_policy_valid = ModelRetryPolicy::new(
        settings.max_retries.saturating_add(1),
        Duration::from_millis(settings.retry_base_delay_ms),
        Duration::from_millis(settings.retry_max_delay_ms),
    )
    .is_ok();
    let valid = settings.schema_version == crate::config::CODING_SETTINGS_SCHEMA_VERSION
        && valid_text(&settings.provider, 128)
        && valid_text(&settings.model, 256)
        && matches!(
            settings.thinking.as_str(),
            "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
        )
        && retry_policy_valid
        && matches!(settings.tui.viewport.as_str(), "fullscreen" | "inline")
        && tui_keys(&settings.tui).into_iter().all(valid_key_binding)
        && settings.active_tools.len() <= 64
        && settings.resources.skill_paths.len() <= 32
        && settings
            .active_tools
            .iter()
            .all(|value| valid_text(value, 128))
        && settings
            .resources
            .skill_paths
            .iter()
            .all(|value| valid_text(value, 4096))
        && valid_web_search(&settings.web_search)
        && valid_web_fetch(&settings.web_fetch);
    if valid {
        let mut tools = settings.active_tools.clone();
        tools.sort();
        tools.dedup();
        if tools.len() == settings.active_tools.len() {
            return validate_mcp_server_aliases(&settings.mcp_servers);
        }
    }
    Err(invalid())
}

fn canonicalize_web_search(settings: &mut CodingSettings) -> Result<(), CodingError> {
    let options = settings
        .web_search
        .runtime_options()
        .map_err(|_| invalid())?;
    settings
        .web_search
        .allowed_domains
        .clone_from(&options.allowed_domains().to_vec());
    settings
        .web_search
        .blocked_domains
        .clone_from(&options.blocked_domains().to_vec());
    if settings
        .web_search
        .location
        .as_ref()
        .is_some_and(|location| {
            location.country.is_none()
                && location.city.is_none()
                && location.region.is_none()
                && location.timezone.is_none()
        })
    {
        settings.web_search.location = None;
    }
    let client = &mut settings.web_search.client;
    let config = TavilySearchConfig::new(
        &client.endpoint,
        Duration::from_millis(client.timeout_millis),
    )
    .map_err(|_| invalid())?;
    config.endpoint().clone_into(&mut client.endpoint);
    Ok(())
}

fn valid_web_search(settings: &crate::config::WebSearchSettings) -> bool {
    let client = &settings.client;
    settings.runtime_options().is_ok()
        && client.backend == crate::config::WebSearchClientBackend::Tavily
        && valid_environment_name(&client.api_key_environment)
        && TavilySearchConfig::new(
            &client.endpoint,
            Duration::from_millis(client.timeout_millis),
        )
        .is_ok()
        && (settings.route_preference != crate::config::WebSearchRoutePreference::ForceClient
            || client.enabled)
}

fn valid_web_fetch(settings: &crate::config::WebFetchSettings) -> bool {
    settings.backend == crate::config::WebFetchBackend::Http
        && FetchCacheConfig::new(
            Duration::from_secs(settings.cache.ttl_seconds),
            settings.cache.max_entries,
            settings.cache.max_total_bytes,
            settings.cache.max_entry_bytes,
        )
        .is_ok()
}

fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= crate::config::MAX_WEB_SEARCH_API_KEY_ENVIRONMENT_BYTES
        && bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn tui_keys(settings: &crate::config::TuiSettings) -> [&str; 12] {
    [
        &settings.submit_key,
        &settings.newline_key,
        &settings.abort_key,
        &settings.clear_key,
        &settings.exit_key,
        &settings.model_key,
        &settings.toggle_thinking_key,
        &settings.toggle_tools_key,
        &settings.copy_key,
        &settings.steering_key,
        &settings.follow_up_key,
        &settings.retrieve_queued_key,
    ]
}

fn valid_key_binding(value: &str) -> bool {
    if !valid_text(value, 64) {
        return false;
    }
    let mut key = false;
    for token in value.split('+') {
        match token {
            "ctrl" | "control" | "alt" | "option" | "shift" | "super" | "cmd" | "command"
                if !key => {}
            "enter" | "return" | "esc" | "escape" | "backspace" | "delete" | "tab" | "left"
            | "right" | "up" | "down" | "home" | "end" | "space"
                if !key =>
            {
                key = true;
            }
            token if !key && token.chars().count() == 1 => key = true,
            _ => return false,
        }
    }
    key
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn invalid() -> CodingError {
    CodingError::new(CodingErrorCode::InvalidInput, "coding settings are invalid")
}
