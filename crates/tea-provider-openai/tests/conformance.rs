//! Conformance against the model suite, driven by recorded fixtures.

use crate::support;

use std::str::FromStr;

use support::FixtureProvider;
use tea_model::{ModelCapabilities, ModelDisplayName, ModelSpec, ProviderId};
use tea_protocol::{ModelId, TokenCount};
use tea_testkit::run_model_provider_case;

fn model_spec(id: &str) -> ModelSpec {
    ModelSpec::new(
        ModelId::from_str(id).unwrap(),
        ProviderId::from_str("openai").unwrap(),
        ModelDisplayName::from_str("Test Model").unwrap(),
        TokenCount::new(128_000).unwrap(),
        TokenCount::new(4_000).unwrap(),
        ModelCapabilities::text().with_tools(true),
    )
    .unwrap()
}

fn fixture_provider(fixture: &str, model_id: &str) -> FixtureProvider {
    FixtureProvider::from_fixture_name("openai", model_spec(model_id), fixture)
}

#[tokio::test]
async fn text_fixture_passes_conformance() {
    let provider = fixture_provider("text.sse", "gpt-4o-mini");
    let request = tea_model::ModelRequest::new(
        ModelId::from_str("gpt-4o-mini").unwrap(),
        vec![
            tea_protocol::CanonicalMessage::user(
                tea_protocol::MessageId::from_str("0195a0b1-5e52-74b2-8c25-0aa7aa000025").unwrap(),
                vec![tea_protocol::ContentBlock::text("hi").unwrap()],
                tea_protocol::ProtocolTimestamp::from_str("2026-07-23T09:30:12.125Z").unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let collected = run_model_provider_case(&provider, request).await.unwrap();
    assert!(collected.report().event_count() >= 3);
    assert_eq!(
        collected.report().terminal_kind(),
        tea_testkit::ModelTerminalKind::Completed
    );
}

#[tokio::test]
async fn tool_call_fixture_passes_conformance() {
    let provider = fixture_provider("tool_call.sse", "gpt-4o-mini");
    let request = tea_model::ModelRequest::new(
        ModelId::from_str("gpt-4o-mini").unwrap(),
        vec![
            tea_protocol::CanonicalMessage::user(
                tea_protocol::MessageId::from_str("0195a0b1-5e52-74b2-8c25-0aa7aa000026").unwrap(),
                vec![tea_protocol::ContentBlock::text("read the file").unwrap()],
                tea_protocol::ProtocolTimestamp::from_str("2026-07-23T09:30:12.125Z").unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let collected = run_model_provider_case(&provider, request).await.unwrap();
    assert_eq!(
        collected.report().completed_tool_calls(),
        1,
        "exactly one completed tool call"
    );
    assert!(
        collected
            .events()
            .iter()
            .any(|event| matches!(event, tea_model::ModelEvent::ToolCallCompleted(_)))
    );
}

#[tokio::test]
async fn midstream_error_fixture_passes_conformance() {
    let provider = fixture_provider("midstream_error.sse", "gpt-4o-mini");
    let request = tea_model::ModelRequest::new(
        ModelId::from_str("gpt-4o-mini").unwrap(),
        vec![
            tea_protocol::CanonicalMessage::user(
                tea_protocol::MessageId::from_str("0195a0b1-5e52-74b2-8c25-0aa7aa000027").unwrap(),
                vec![tea_protocol::ContentBlock::text("anything").unwrap()],
                tea_protocol::ProtocolTimestamp::from_str("2026-07-23T09:30:12.125Z").unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let collected = run_model_provider_case(&provider, request).await.unwrap();
    assert_eq!(
        collected.report().terminal_kind(),
        tea_testkit::ModelTerminalKind::Failed
    );
    assert_eq!(
        collected.report().failure_code(),
        Some(tea_model::ModelFailureCode::Unavailable)
    );
}
