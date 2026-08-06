//! Contract tests for the `OpenAI` provider errors, credential resolver, and env loader.

use std::collections::BTreeMap;
use std::str::FromStr as _;

use tea_model::ProviderId;
use tea_provider_openai::{
    credential::{ApiKey, CredentialResolver, MapCredentialResolver, OpenAiApiMode, OpenAiConfig},
    env_file::load_env_file,
    error::{OpenAiError, OpenAiErrorCode},
};

fn env_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn error_codes_are_stable() {
    for code in [
        OpenAiErrorCode::InvalidRequest,
        OpenAiErrorCode::Authentication,
        OpenAiErrorCode::PermissionDenied,
        OpenAiErrorCode::RateLimited,
        OpenAiErrorCode::Unavailable,
        OpenAiErrorCode::Transport,
        OpenAiErrorCode::MalformedResponse,
        OpenAiErrorCode::ContextOverflow,
        OpenAiErrorCode::Cancelled,
        OpenAiErrorCode::Internal,
    ] {
        let error = OpenAiError::new(code, "example");
        assert_eq!(error.code(), code);
        assert!(!error.message().is_empty());
        assert!(!error.message().contains('\0'));
    }
}

#[test]
fn api_key_is_bounded_and_redacts_in_debug() {
    let key = ApiKey::new("sk-test-1234567890").unwrap();
    assert_eq!(key.as_str(), "sk-test-1234567890");
    assert!(!format!("{key:?}").contains("1234567890"));
}

#[test]
fn explicit_config_uses_safe_connection_defaults() {
    let config = OpenAiConfig::new(
        "example-model".parse().unwrap(),
        "https://gateway.example.test/v1",
        ApiKey::new("test-key").unwrap(),
    )
    .unwrap();

    assert_eq!(config.provider_id().as_str(), "openai");
    assert_eq!(config.model_id().as_str(), "example-model");
    assert_eq!(config.base_url(), "https://gateway.example.test/v1");
    assert_eq!(config.api_key_header(), "Authorization");
    assert_eq!(config.api_key_prefix(), "Bearer ");
    assert_eq!(config.api_mode(), OpenAiApiMode::ChatCompletions);
    assert_eq!(config.timeout_millis(), 60_000);
}

#[test]
fn explicit_config_rejects_an_empty_base_url() {
    let error = OpenAiConfig::new(
        "example-model".parse().unwrap(),
        "",
        ApiKey::new("test-key").unwrap(),
    )
    .unwrap_err();

    assert_eq!(error.code(), OpenAiErrorCode::InvalidRequest);
}

#[test]
fn map_resolver_reads_contract() {
    let map = env_map(&[
        ("TEA_OPENAI_BASE_URL", "https://gateway.example.test/v1"),
        ("TEA_OPENAI_API_KEY", "sk-test-key"),
        ("TEA_OPENAI_MODEL", "gpt-4o-mini"),
        ("TEA_OPENAI_API_KEY_HEADER", "x-api-key"),
        ("TEA_OPENAI_API_KEY_PREFIX", "__NONE__"),
    ]);
    let config = MapCredentialResolver::new(map).resolve().unwrap();
    assert_eq!(config.base_url(), "https://gateway.example.test/v1");
    assert_eq!(config.api_key().as_str(), "sk-test-key");
    assert_eq!(config.api_key_header(), "x-api-key");
    assert_eq!(config.api_key_prefix(), "");
    assert_eq!(config.api_mode(), OpenAiApiMode::ChatCompletions);
    assert_eq!(config.model_id().as_str(), "gpt-4o-mini");
}

#[test]
fn map_resolver_selects_responses_api() {
    let map = env_map(&[
        ("TEA_OPENAI_API_KEY", "sk-key"),
        ("TEA_OPENAI_MODEL", "gpt-4o"),
        ("TEA_OPENAI_API_MODE", "responses"),
    ]);
    let config = MapCredentialResolver::new(map).resolve().unwrap();
    assert_eq!(config.api_mode(), OpenAiApiMode::Responses);
}

#[test]
fn map_resolver_rejects_unknown_api_mode() {
    let map = env_map(&[
        ("TEA_OPENAI_API_KEY", "sk-key"),
        ("TEA_OPENAI_MODEL", "gpt-4o"),
        ("TEA_OPENAI_API_MODE", "legacy"),
    ]);
    let error = MapCredentialResolver::new(map).resolve().unwrap_err();
    assert_eq!(error.code(), OpenAiErrorCode::InvalidRequest);
}

#[test]
fn map_resolver_defaults_openai_shape() {
    let map = env_map(&[
        ("TEA_OPENAI_API_KEY", "sk-key"),
        ("TEA_OPENAI_MODEL", "gpt-4o"),
    ]);
    let config = MapCredentialResolver::new(map).resolve().unwrap();
    assert_eq!(config.base_url(), "https://api.openai.com/v1");
    assert_eq!(config.api_key_header(), "Authorization");
    assert_eq!(config.api_key_prefix(), "Bearer ");
    assert_eq!(config.timeout_millis(), 60_000);
}

#[test]
fn map_resolver_supports_custom_provider_identity() {
    let map = env_map(&[
        ("TEA_OPENAI_API_KEY", "custom-key"),
        ("TEA_OPENAI_MODEL", "custom-model"),
    ]);
    let config =
        MapCredentialResolver::for_provider(ProviderId::from_str("deepseek").unwrap(), map)
            .resolve()
            .unwrap();
    assert_eq!(config.provider_id().as_str(), "deepseek");
}

#[test]
fn map_resolver_requires_key_and_model() {
    let map = env_map(&[("TEA_OPENAI_BASE_URL", "https://x.test/v1")]);
    let err = MapCredentialResolver::new(map).resolve().unwrap_err();
    assert_eq!(err.code(), OpenAiErrorCode::Authentication);
}

#[test]
fn load_env_file_parses_key_value_lines() {
    let path = std::env::temp_dir().join("tea-openai-env-contract.env");
    std::fs::write(
        &path,
        "# a comment\n\nTEA_OPENAI_BASE_URL=https://example.test/v1\nTEA_OPENAI_API_KEY=\"sk-quoted\"\nTEA_OPENAI_MODEL=gpt-4o-mini\n",
    )
    .unwrap();
    let map = load_env_file(&path).unwrap();
    assert_eq!(
        map.get("TEA_OPENAI_BASE_URL"),
        Some(&"https://example.test/v1".to_owned())
    );
    assert_eq!(map.get("TEA_OPENAI_API_KEY"), Some(&"sk-quoted".to_owned()));
    assert_eq!(map.get("TEA_OPENAI_MODEL"), Some(&"gpt-4o-mini".to_owned()));
    let _ = std::fs::remove_file(&path);
}

#[allow(dead_code)]
fn _send_sync<R: CredentialResolver + Send + Sync>(_r: R) {}
