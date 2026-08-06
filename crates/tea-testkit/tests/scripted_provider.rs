use std::str::FromStr;
use std::sync::Arc;

use futures_util::StreamExt;
use serde_json::json;
use tea_model::{
    ModelCancellation, ModelCapabilities, ModelCompletion, ModelDisplayName, ModelEvent,
    ModelFailureCode, ModelProvider, ModelRequest, ModelResponseInfo, ModelSpec, ModelStreamIndex,
    ProviderId, ProviderToolCallId, ToolCallCompleted, ToolCallStarted, Utf8Delta,
};
use tea_protocol::{
    CanonicalMessage, ContentBlock, MessageId, ModelId, ProtocolTimestamp, StopReason, TokenCount,
};
use tea_testkit::{ScriptStep, ScriptedModelProvider, ScriptedModelResponse};

fn model() -> ModelSpec {
    ModelSpec::new(
        ModelId::from_str("fake/model").unwrap(),
        ProviderId::from_str("fake").unwrap(),
        ModelDisplayName::from_str("Fake Model").unwrap(),
        TokenCount::new(16_000).unwrap(),
        TokenCount::new(4_000).unwrap(),
        ModelCapabilities::text().with_reasoning().with_tools(true),
    )
    .unwrap()
}

fn request(text: &str) -> ModelRequest {
    ModelRequest::new(
        ModelId::from_str("fake/model").unwrap(),
        vec![
            CanonicalMessage::user(
                MessageId::from_str("0195a0b1-5e3d-73de-b461-0aa7aa000004").unwrap(),
                vec![ContentBlock::text(text).unwrap()],
                ProtocolTimestamp::from_str("2026-07-23T09:30:12.123Z").unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn fake_consumes_fifo_scripts_and_captures_requests() {
    let provider = ScriptedModelProvider::new(
        ProviderId::from_str("fake").unwrap(),
        vec![model()],
        [
            ScriptedModelResponse::text(["first ", "answer"]),
            ScriptedModelResponse::text(["second"]),
        ],
    );

    let first = provider
        .stream(request("one"), ModelCancellation::new())
        .collect::<Vec<_>>()
        .await;
    let second = provider
        .stream(request("two"), ModelCancellation::new())
        .collect::<Vec<_>>()
        .await;

    assert_eq!(
        first
            .iter()
            .filter_map(ModelEvent::as_text_delta)
            .collect::<String>(),
        "first answer"
    );
    assert_eq!(
        second
            .iter()
            .filter_map(ModelEvent::as_text_delta)
            .collect::<String>(),
        "second"
    );
    assert_eq!(provider.captured_requests().unwrap().len(), 2);
    assert_eq!(provider.remaining_scripts().unwrap(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn fake_emits_one_or_several_complete_tool_calls_deterministically() {
    let index0 = ModelStreamIndex::new(0).unwrap();
    let index1 = ModelStreamIndex::new(1).unwrap();
    let call0 = ProviderToolCallId::from_str("call_0").unwrap();
    let call1 = ProviderToolCallId::from_str("call_1").unwrap();
    let script = ScriptedModelResponse::events([
        ModelEvent::Started(ModelResponseInfo::new()),
        ModelEvent::ToolCallStarted(
            ToolCallStarted::new(index0, call0.clone(), "read_file").unwrap(),
        ),
        ModelEvent::ToolCallCompleted(
            ToolCallCompleted::new(index0, call0, "read_file", json!({"path":"a"})).unwrap(),
        ),
        ModelEvent::ToolCallStarted(
            ToolCallStarted::new(index1, call1.clone(), "write_file").unwrap(),
        ),
        ModelEvent::ToolCallCompleted(
            ToolCallCompleted::new(index1, call1, "write_file", json!({"path":"b"})).unwrap(),
        ),
        ModelEvent::Completed(ModelCompletion::new(StopReason::ToolUse).unwrap()),
    ]);
    let provider = ScriptedModelProvider::new(
        ProviderId::from_str("fake").unwrap(),
        vec![model()],
        [script],
    );

    let events = provider
        .stream(request("tools"), ModelCancellation::new())
        .collect::<Vec<_>>()
        .await;
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ModelEvent::ToolCallCompleted(_)))
            .count(),
        2
    );
}

#[tokio::test(flavor = "current_thread")]
async fn fake_supports_failure_before_and_during_content_and_context_overflow() {
    let provider = ScriptedModelProvider::new(
        ProviderId::from_str("fake").unwrap(),
        vec![model()],
        [
            ScriptedModelResponse::failure(ModelFailureCode::Unavailable, "unavailable"),
            ScriptedModelResponse::events([
                ModelEvent::Started(ModelResponseInfo::new()),
                ModelEvent::TextDelta(Utf8Delta::new("partial").unwrap()),
                ScriptedModelResponse::failure_event(
                    ModelFailureCode::Transport,
                    "stream disconnected",
                ),
            ]),
            ScriptedModelResponse::context_overflow("context limit exceeded"),
        ],
    );

    for expected in [
        ModelFailureCode::Unavailable,
        ModelFailureCode::Transport,
        ModelFailureCode::ContextOverflow,
    ] {
        let events = provider
            .stream(request("fail"), ModelCancellation::new())
            .collect::<Vec<_>>()
            .await;
        assert!(matches!(
            events.last(),
            Some(ModelEvent::Failed(failure)) if failure.code() == expected
        ));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_waits_without_spawn_or_wall_clock_sleep() {
    let provider = Arc::new(ScriptedModelProvider::new(
        ProviderId::from_str("fake").unwrap(),
        vec![model()],
        [ScriptedModelResponse::await_cancellation()],
    ));
    let cancellation = ModelCancellation::new();
    let stream = provider.stream(request("cancel"), cancellation.clone());

    let collect = stream.collect::<Vec<_>>();
    let cancel = async move { cancellation.cancel() };
    let (events, ()) = tokio::join!(collect, cancel);

    assert!(matches!(events.first(), Some(ModelEvent::Started(_))));
    assert!(matches!(
        events.last(),
        Some(ModelEvent::Failed(failure)) if failure.code() == ModelFailureCode::Cancelled
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn missing_script_is_a_terminal_failure_not_a_panic() {
    let provider =
        ScriptedModelProvider::new(ProviderId::from_str("fake").unwrap(), vec![model()], []);
    let events = provider
        .stream(request("missing"), ModelCancellation::new())
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(
        events.as_slice(),
        [ModelEvent::Started(_), ModelEvent::Failed(failure)]
            if failure.code() == ModelFailureCode::Internal
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn pre_cancelled_scope_terminates_before_scripted_content() {
    let provider = ScriptedModelProvider::new(
        ProviderId::from_str("fake").unwrap(),
        vec![model()],
        [ScriptedModelResponse::text(["must not be emitted"])],
    );
    let cancellation = ModelCancellation::new();
    cancellation.cancel();
    let events = provider
        .stream(request("cancelled"), cancellation)
        .collect::<Vec<_>>()
        .await;

    assert!(matches!(events.first(), Some(ModelEvent::Started(_))));
    assert!(matches!(
        events.last(),
        Some(ModelEvent::Failed(failure)) if failure.code() == ModelFailureCode::Cancelled
    ));
    assert!(!events.iter().any(|event| event.as_text_delta().is_some()));
}

#[test]
fn scripts_are_immutable_values_and_steps_are_explicit() {
    let response = ScriptedModelResponse::new([
        ScriptStep::event(ModelEvent::Started(ModelResponseInfo::new())),
        ScriptStep::AwaitCancellation,
    ]);
    assert_eq!(response.steps().len(), 2);
}
