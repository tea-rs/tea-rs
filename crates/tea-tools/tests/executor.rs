use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use tea_control::CancellationScope;
use tea_protocol::{ContentBlock, ProtocolMetadata, ToolCallId, ToolIdempotency};
use tea_tools::{
    BoxToolExecutionStream, StaticResourceResolver, ToolConcurrency, ToolEffect,
    ToolExecutionEvent, ToolExecutionFailureCode, ToolExecutionSemantics, ToolExecutor,
    ToolInvocation, ToolName, ToolProgress, ToolRegistry, ToolResult, ToolRetrySafety, ToolSpec,
    ToolStreamValidator, ToolStreamViolation, ToolTimeout, ToolVersion,
};

#[derive(Debug, Clone)]
enum Behavior {
    Output(Value),
    AwaitCancellation,
    NoTerminal,
}

#[derive(Debug)]
struct FakeExecutor {
    calls: Arc<AtomicUsize>,
    behavior: Behavior,
}

impl ToolExecutor for FakeExecutor {
    fn execute(
        &self,
        _invocation: tea_tools::ValidatedToolInvocation,
        cancellation: CancellationScope,
    ) -> BoxToolExecutionStream {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.behavior.clone() {
            Behavior::Output(output) => Box::pin(stream::iter([
                ToolExecutionEvent::Progress(ToolProgress::new("working", 1, Some(1)).unwrap()),
                ToolExecutionEvent::Finished(
                    ToolResult::new(vec![ContentBlock::text("done").unwrap()], output).unwrap(),
                ),
            ])),
            Behavior::AwaitCancellation => Box::pin(stream::once(async move {
                cancellation.cancelled().await;
                ToolExecutionEvent::Failed(tea_tools::ToolExecutionFailure::cancelled())
            })),
            Behavior::NoTerminal => Box::pin(stream::iter([ToolExecutionEvent::Progress(
                ToolProgress::new("working", 0, None).unwrap(),
            )])),
        }
    }
}

fn spec() -> ToolSpec {
    ToolSpec::new(
        ToolName::from_str("read_file").unwrap(),
        ToolVersion::from_str("1.0.0").unwrap(),
        "Reads a file.",
        json!({
            "type":"object",
            "properties":{"path":{"type":"string"}},
            "required":["path"],
            "additionalProperties":false
        }),
        json!({
            "type":"object",
            "properties":{"content":{"type":"string"}},
            "required":["content"],
            "additionalProperties":false
        }),
        [ToolEffect::FsRead],
        ToolExecutionSemantics::new(
            ToolIdempotency::Idempotent,
            ToolRetrySafety::Automatic,
            ToolConcurrency::Parallel,
            ToolTimeout::from_millis(1_000).unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn invocation(arguments: Value) -> ToolInvocation {
    ToolInvocation::new(
        ToolCallId::from_str("0195a0b1-5e45-75be-8284-0aa7aa000011").unwrap(),
        ToolName::from_str("read_file").unwrap(),
        arguments,
        ProtocolMetadata::default(),
    )
    .unwrap()
}

fn registry(behavior: Behavior, calls: Arc<AtomicUsize>) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry
        .register(
            spec(),
            Arc::new(StaticResourceResolver::new([]).unwrap()),
            Arc::new(FakeExecutor { calls, behavior }),
        )
        .unwrap();
    registry
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_arguments_cannot_reach_executor() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = registry(
        Behavior::Output(json!({"content":"ok"})),
        Arc::clone(&calls),
    );
    assert!(
        registry
            .execute(invocation(json!({"wrong":true})), CancellationScope::new())
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn registry_preserves_progress_and_valid_terminal_output() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = registry(
        Behavior::Output(json!({"content":"ok"})),
        Arc::clone(&calls),
    );
    let events = registry
        .execute(
            invocation(json!({"path":"notes"})),
            CancellationScope::new(),
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        events.as_slice(),
        [
            ToolExecutionEvent::Progress(_),
            ToolExecutionEvent::Finished(_)
        ]
    ));
    let mut validator = ToolStreamValidator::new();
    for event in &events {
        validator.observe(event).unwrap();
    }
    assert_eq!(validator.finish().unwrap(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_executor_output_becomes_typed_terminal_failure() {
    let registry = registry(
        Behavior::Output(json!({"wrong":true})),
        Arc::new(AtomicUsize::new(0)),
    );
    let events = registry
        .execute(
            invocation(json!({"path":"notes"})),
            CancellationScope::new(),
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(
        events.last(),
        Some(ToolExecutionEvent::Failed(failure))
            if failure.code() == ToolExecutionFailureCode::InvalidOutput
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_terminates_pending_fake_without_spawn_or_sleep() {
    let registry = registry(Behavior::AwaitCancellation, Arc::new(AtomicUsize::new(0)));
    let cancellation = CancellationScope::new();
    let stream = registry
        .execute(invocation(json!({"path":"notes"})), cancellation.clone())
        .unwrap();
    let (events, ()) = futures_util::future::join(stream.collect::<Vec<_>>(), async move {
        cancellation.cancel();
    })
    .await;
    assert!(matches!(
        events.as_slice(),
        [ToolExecutionEvent::Failed(failure)]
            if failure.code() == ToolExecutionFailureCode::Cancelled
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn executor_stream_ending_without_terminal_is_normalized_to_failure() {
    let registry = registry(Behavior::NoTerminal, Arc::new(AtomicUsize::new(0)));
    let events = registry
        .execute(
            invocation(json!({"path":"notes"})),
            CancellationScope::new(),
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(
        events.last(),
        Some(ToolExecutionEvent::Failed(failure))
            if failure.code() == ToolExecutionFailureCode::Internal
    ));
}

#[test]
fn result_failure_and_progress_bounds_fail_closed() {
    assert!(
        ToolResult::new(
            vec![ContentBlock::text("done").unwrap()],
            json!({"data":"x".repeat(256 * 1024)}),
        )
        .is_err()
    );
    assert!(tea_tools::ToolExecutionFailure::execution("x".repeat(4097)).is_err());
    assert!(ToolProgress::new("x".repeat(4097), 0, None).is_err());
    assert!(
        ToolInvocation::new(
            ToolCallId::from_str("0195a0b1-5e45-75be-8284-0aa7aa000011").unwrap(),
            ToolName::from_str("read_file").unwrap(),
            json!("not-an-object"),
            ProtocolMetadata::default(),
        )
        .is_err()
    );
}

#[test]
fn stream_validator_rejects_missing_and_post_terminal_events() {
    let validator = ToolStreamValidator::new();
    assert_eq!(
        validator.finish().unwrap_err(),
        ToolStreamViolation::MissingTerminal
    );

    let mut validator = ToolStreamValidator::new();
    let terminal = ToolExecutionEvent::Failed(tea_tools::ToolExecutionFailure::cancelled());
    validator.observe(&terminal).unwrap();
    assert_eq!(
        validator.observe(&terminal).unwrap_err(),
        ToolStreamViolation::EventAfterTerminal
    );
    assert!(ToolProgress::new("bad", 2, Some(1)).is_err());
}
