//! Live smoke tests for the `OpenAI` provider.
//!
//! These tests are `#[ignore]`d and never run in CI. Run them locally:
//!   cargo test -p tea-provider-openai --features live --test integration `smoke::` -- --ignored
//! They load `.env` (committed template at `.env.example`) via the dependency-
//! free loader and skip with a message when required vars are unset.

#![cfg(feature = "live")]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;

use futures_util::StreamExt;
use tea_control::CancellationScope;
use tea_model::{
    HostedToolKind, HostedToolOptions, ModelCapabilities, ModelDisplayName, ModelEvent,
    ModelProvider, ModelSpec, ModelToolDefinition, WebSearchOptions,
};
use tea_protocol::{
    CanonicalMessage, ContentBlock, MessageId, ModelId, ProtocolMetadata, ProtocolTimestamp,
    TokenCount,
};
use tea_provider_openai::{
    OpenAiApiMode, OpenAiProviderBuilder,
    credential::{CredentialResolver, MapCredentialResolver},
    env_file::load_env_file,
};

fn load_env_map() -> BTreeMap<String, String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.env");
    if path.exists() {
        load_env_file(&path).unwrap_or_default()
    } else {
        BTreeMap::new()
    }
}

#[allow(dead_code)]
fn smoke_provider(api_mode: Option<OpenAiApiMode>) -> Option<OpenAiProviderBuilder> {
    smoke_provider_with_capabilities(api_mode, ModelCapabilities::text().with_tools(true))
}

fn smoke_provider_with_capabilities(
    api_mode: Option<OpenAiApiMode>,
    capabilities: ModelCapabilities,
) -> Option<OpenAiProviderBuilder> {
    let map = load_env_map();
    let api_key = map.get("TEA_OPENAI_API_KEY").filter(|v| !v.is_empty())?;
    let model_text = map
        .get("TEA_OPENAI_MODEL")
        .filter(|v| !v.is_empty())?
        .clone();
    let _ = api_key;
    let resolver = MapCredentialResolver::new(map);
    let mut config = resolver.resolve().ok()?;
    if let Some(api_mode) = api_mode {
        config = config.with_api_mode(api_mode);
    }
    let provider_id = config.provider_id().clone();
    let model_id = ModelId::from_str(&model_text).ok()?;
    let spec = ModelSpec::new(
        model_id,
        provider_id,
        ModelDisplayName::from_str("Smoke Model").unwrap(),
        TokenCount::new(128_000).unwrap(),
        TokenCount::new(4_000).unwrap(),
        capabilities,
    )
    .unwrap();
    Some(
        OpenAiProviderBuilder::new()
            .with_config(std::sync::Arc::new(config))
            .with_catalog(vec![spec]),
    )
}

fn web_search_tool() -> ModelToolDefinition {
    ModelToolDefinition::hosted(
        "Searches the public web and returns cited sources.",
        serde_json::json!({
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"]
        }),
        HostedToolOptions::WebSearch(WebSearchOptions::new()),
    )
    .unwrap()
}

fn user_message(text: &str) -> CanonicalMessage {
    CanonicalMessage::user(
        MessageId::from_str("0195a0b1-5e52-74b2-8c25-0aa7aa000031").unwrap(),
        vec![ContentBlock::text(text).unwrap()],
        ProtocolTimestamp::from_str("2026-07-23T09:30:12.125Z").unwrap(),
    )
    .unwrap()
}

#[tokio::test]
#[ignore = "requires TEA_OPENAI_* env vars and a live network"]
async fn live_text_stream_completes() {
    let Some(builder) = smoke_provider(None) else {
        eprintln!("skipping live smoke: TEA_OPENAI_* not configured");
        return;
    };
    let provider = builder.build().unwrap();
    let model_id = provider.models().first().unwrap().model_id().clone();
    let request = tea_model::ModelRequest::new(
        model_id,
        vec![user_message("Say hello in one short sentence.")],
    )
    .unwrap();
    let mut stream = provider.stream(request, CancellationScope::new());
    let mut text = String::new();
    let mut completed = false;
    while let Some(event) = stream.next().await {
        match event {
            ModelEvent::TextDelta(delta) => text.push_str(delta.as_str()),
            ModelEvent::Completed(completion) => {
                assert!(completion.usage().is_some(), "usage should be reported");
                completed = true;
                break;
            }
            ModelEvent::Failed(failure) => panic!("live stream failed: {failure:?}"),
            _ => {}
        }
    }
    assert!(completed, "stream must reach a terminal completion");
    assert!(!text.is_empty(), "stream must produce visible text");
}

#[tokio::test]
#[ignore = "requires TEA_OPENAI_* env vars, Responses API support, and a live network"]
async fn live_responses_text_stream_completes() {
    let Some(builder) = smoke_provider(Some(OpenAiApiMode::Responses)) else {
        eprintln!("skipping live Responses smoke: TEA_OPENAI_* not configured");
        return;
    };
    let provider = builder.build().unwrap();
    let model_id = provider.models().first().unwrap().model_id().clone();
    let request = tea_model::ModelRequest::new(
        model_id,
        vec![user_message("Say hello in one short sentence.")],
    )
    .unwrap();
    let mut stream = provider.stream(request, CancellationScope::new());
    let mut text = String::new();
    let mut completed = false;
    while let Some(event) = stream.next().await {
        match event {
            ModelEvent::TextDelta(delta) => text.push_str(delta.as_str()),
            ModelEvent::Completed(completion) => {
                assert!(completion.usage().is_some(), "usage should be reported");
                completed = true;
                break;
            }
            ModelEvent::Failed(failure) => panic!("live Responses stream failed: {failure:?}"),
            _ => {}
        }
    }
    assert!(
        completed,
        "Responses stream must reach a terminal completion"
    );
    assert!(
        !text.is_empty(),
        "Responses stream must produce visible text"
    );
}

#[tokio::test]
#[ignore = "requires TEA_OPENAI_* env vars, hosted web search, and a live network"]
async fn live_responses_hosted_web_search_completes_with_sources() {
    let capabilities = ModelCapabilities::text()
        .with_tools(true)
        .with_hosted_tool(HostedToolKind::WebSearch);
    let Some(builder) =
        smoke_provider_with_capabilities(Some(OpenAiApiMode::Responses), capabilities)
    else {
        eprintln!("skipping live Responses hosted-search smoke: TEA_OPENAI_* not configured");
        return;
    };
    let provider = builder.build().unwrap();
    let model_id = provider.models().first().unwrap().model_id().clone();
    let request = tea_model::ModelRequest::new(
        model_id,
        vec![user_message(
            "Use web_search to find the latest official Rust blog post. Reply with its title, date, and source URL.",
        )],
    )
    .unwrap()
    .with_tools(vec![web_search_tool()], false)
    .unwrap();
    let mut stream = provider.stream(request, CancellationScope::new());
    let mut text = String::new();
    let mut replayed_full_text = false;
    let mut hosted_successes = 0;
    let mut source_count = 0;
    let mut completed = false;
    while let Some(event) = stream.next().await {
        match event {
            ModelEvent::TextDelta(delta) => {
                replayed_full_text |= !text.is_empty() && delta.as_str() == text;
                text.push_str(delta.as_str());
            }
            ModelEvent::HostedToolCompleted(activity) => {
                hosted_successes += usize::from(matches!(
                    activity.outcome(),
                    tea_protocol::HostedToolOutcome::Success
                ));
                source_count += activity.sources().len();
            }
            ModelEvent::Completed(_) => {
                completed = true;
                break;
            }
            ModelEvent::Failed(failure) => {
                panic!("live Responses hosted-search stream failed: {failure:?}")
            }
            _ => {}
        }
    }
    assert!(completed, "hosted-search stream must complete");
    assert!(hosted_successes > 0, "hosted search must report success");
    assert!(source_count > 0, "hosted search must return sources");
    assert!(!text.is_empty(), "hosted search must produce visible text");
    assert!(
        !replayed_full_text,
        "terminal snapshot must not replay the complete streamed answer"
    );
}

#[tokio::test]
#[ignore = "requires TEA_OPENAI_* env vars and a live network"]
async fn live_cancellation_closes_stream() {
    let Some(builder) = smoke_provider(None) else {
        eprintln!("skipping live smoke: TEA_OPENAI_* not configured");
        return;
    };
    let provider = builder.build().unwrap();
    let model_id = provider.models().first().unwrap().model_id().clone();
    let request = tea_model::ModelRequest::new(
        model_id,
        vec![user_message("Write a long essay about the ocean.")],
    )
    .unwrap();
    let cancellation = CancellationScope::new();
    let mut stream = provider.stream(request, cancellation.clone());
    cancellation.cancel();
    while let Some(event) = stream.next().await {
        if matches!(event, ModelEvent::Failed(_)) {
            break;
        }
    }
    let _ = ProtocolMetadata::default();
}
