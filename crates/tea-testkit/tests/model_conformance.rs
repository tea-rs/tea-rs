use std::str::FromStr;

use futures_util::stream;
use tea_model::{
    ModelCapabilities, ModelCompletion, ModelDisplayName, ModelEvent, ModelFailureCode,
    ModelProvider, ModelRequest, ModelResponseInfo, ModelSpec, ProviderId, Utf8Delta,
};
use tea_protocol::{
    CanonicalMessage, ContentBlock, MessageId, ModelId, ProtocolTimestamp, StopReason, TokenCount,
};
use tea_testkit::{
    ModelConformanceError, ModelTerminalKind, ScriptedModelProvider, ScriptedModelResponse,
    collect_model_stream, run_cancelled_model_provider_case, run_model_provider_case,
};

fn model() -> ModelSpec {
    ModelSpec::new(
        ModelId::from_str("fake/model").unwrap(),
        ProviderId::from_str("fake").unwrap(),
        ModelDisplayName::from_str("Fake").unwrap(),
        TokenCount::new(8_000).unwrap(),
        TokenCount::new(2_000).unwrap(),
        ModelCapabilities::text(),
    )
    .unwrap()
}

fn mismatched_provider_model() -> ModelSpec {
    ModelSpec::new(
        ModelId::from_str("fake/mismatched").unwrap(),
        ProviderId::from_str("other-provider").unwrap(),
        ModelDisplayName::from_str("Mismatched").unwrap(),
        TokenCount::new(8_000).unwrap(),
        TokenCount::new(2_000).unwrap(),
        ModelCapabilities::text(),
    )
    .unwrap()
}

fn request(model_id: &str) -> ModelRequest {
    ModelRequest::new(
        ModelId::from_str(model_id).unwrap(),
        vec![
            CanonicalMessage::user(
                MessageId::from_str("0195a0b1-5e3d-73de-b461-0aa7aa000004").unwrap(),
                vec![ContentBlock::text("hello").unwrap()],
                ProtocolTimestamp::from_str("2026-07-23T09:30:12.123Z").unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn collector_returns_events_and_conformance_report() {
    let stream = Box::pin(stream::iter([
        ModelEvent::Started(ModelResponseInfo::new()),
        ModelEvent::TextDelta(Utf8Delta::new("hello").unwrap()),
        ModelEvent::Completed(ModelCompletion::new(StopReason::Completed).unwrap()),
    ]));
    let collected = collect_model_stream(stream).await.unwrap();

    assert_eq!(collected.events().len(), 3);
    assert_eq!(collected.report().event_count(), 3);
    assert_eq!(collected.report().completed_tool_calls(), 0);
    assert_eq!(
        collected.report().terminal_kind(),
        ModelTerminalKind::Completed
    );
    assert_eq!(
        collected.report().stop_reason(),
        Some(&StopReason::Completed)
    );
    assert_eq!(collected.report().failure_code(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn collector_reports_stream_grammar_violations() {
    let stream = Box::pin(stream::iter([
        ModelEvent::TextDelta(Utf8Delta::new("missing start").unwrap()),
        ModelEvent::Completed(ModelCompletion::completed()),
    ]));
    assert!(matches!(
        collect_model_stream(stream).await.unwrap_err(),
        ModelConformanceError::StreamGrammar(_)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn provider_case_validates_advertised_model_and_request() {
    let provider = ScriptedModelProvider::new(
        ProviderId::from_str("fake").unwrap(),
        vec![model()],
        [ScriptedModelResponse::text(["hello"])],
    );
    let collected = run_model_provider_case(&provider, request("fake/model"))
        .await
        .unwrap();
    assert_eq!(
        collected.report().terminal_kind(),
        ModelTerminalKind::Completed
    );

    assert!(matches!(
        run_model_provider_case(&provider, request("missing/model"))
            .await
            .unwrap_err(),
        ModelConformanceError::ModelNotAdvertised
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn conformance_rejects_model_owned_by_a_different_provider() {
    let provider = ScriptedModelProvider::new(
        ProviderId::from_str("fake").unwrap(),
        vec![mismatched_provider_model()],
        [ScriptedModelResponse::text(["must not run"])],
    );
    assert!(matches!(
        run_model_provider_case(&provider, request("fake/mismatched"))
            .await
            .unwrap_err(),
        ModelConformanceError::ProviderModelMismatch
    ));
    assert_eq!(provider.remaining_scripts().unwrap(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_case_polls_then_cancels_and_awaits_terminal_failure() {
    let provider = ScriptedModelProvider::new(
        ProviderId::from_str("fake").unwrap(),
        vec![model()],
        [ScriptedModelResponse::await_cancellation()],
    );
    let collected = run_cancelled_model_provider_case(&provider, request("fake/model"))
        .await
        .unwrap();
    assert_eq!(
        collected.report().terminal_kind(),
        ModelTerminalKind::Failed
    );
    assert_eq!(
        collected.report().failure_code(),
        Some(ModelFailureCode::Cancelled)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn future_adapters_can_use_conformance_functions_through_trait_objects() {
    let provider: Box<dyn ModelProvider> = Box::new(ScriptedModelProvider::new(
        ProviderId::from_str("fake").unwrap(),
        vec![model()],
        [ScriptedModelResponse::text(["trait object"])],
    ));
    let collected = run_model_provider_case(provider.as_ref(), request("fake/model"))
        .await
        .unwrap();
    assert_eq!(collected.report().event_count(), 3);
}
