use std::str::FromStr;
use std::sync::Arc;

use futures_util::StreamExt;
use serde_json::{Value, json};
use tea_control::CancellationScope;
use tea_protocol::{ProtocolMetadata, ToolCallId, ToolIdempotency};
use tea_testkit::{FakeProcessScript, FakeProcessTool, FakeReadTool, FakeWriteTool};
use tea_tools::{
    ArgumentResourceResolver, StaticResourceResolver, ToolConcurrency, ToolEffect,
    ToolExecutionEvent, ToolExecutionFailureCode, ToolExecutionSemantics, ToolInvocation, ToolName,
    ToolRegistry, ToolResourceAccess, ToolRetrySafety, ToolSpec, ToolTimeout, ToolVersion,
};

fn semantics(idempotency: ToolIdempotency) -> ToolExecutionSemantics {
    ToolExecutionSemantics::new(
        idempotency,
        if matches!(idempotency, ToolIdempotency::Idempotent) {
            ToolRetrySafety::Automatic
        } else {
            ToolRetrySafety::Never
        },
        ToolConcurrency::Parallel,
        ToolTimeout::from_millis(1_000).unwrap(),
    )
    .unwrap()
}

fn read_spec() -> ToolSpec {
    ToolSpec::new(
        ToolName::from_str("read_file").unwrap(),
        ToolVersion::from_str("1.0.0").unwrap(),
        "Reads a fake file.",
        json!({
            "type":"object",
            "properties":{"path":{"type":"string","minLength":1}},
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
        semantics(ToolIdempotency::Idempotent),
    )
    .unwrap()
}

fn write_spec() -> ToolSpec {
    ToolSpec::new(
        ToolName::from_str("write_file").unwrap(),
        ToolVersion::from_str("1.0.0").unwrap(),
        "Writes a fake file.",
        json!({
            "type":"object",
            "properties":{
                "path":{"type":"string","minLength":1},
                "content":{"type":"string"}
            },
            "required":["path","content"],
            "additionalProperties":false
        }),
        json!({
            "type":"object",
            "properties":{
                "path":{"type":"string"},
                "writtenBytes":{"type":"integer","minimum":0}
            },
            "required":["path","writtenBytes"],
            "additionalProperties":false
        }),
        [ToolEffect::FsWrite, ToolEffect::ExternalMutation],
        semantics(ToolIdempotency::Idempotent),
    )
    .unwrap()
}

fn process_spec() -> ToolSpec {
    ToolSpec::new(
        ToolName::from_str("run_process").unwrap(),
        ToolVersion::from_str("1.0.0").unwrap(),
        "Runs a fake process.",
        json!({
            "type":"object",
            "properties":{"command":{"type":"string","minLength":1}},
            "required":["command"],
            "additionalProperties":false
        }),
        json!({
            "type":"object",
            "properties":{
                "stdout":{"type":"string"},
                "exitCode":{"type":"integer"}
            },
            "required":["stdout","exitCode"],
            "additionalProperties":false
        }),
        [ToolEffect::ProcessSpawn],
        semantics(ToolIdempotency::NonIdempotent),
    )
    .unwrap()
}

fn invocation(name: &str, arguments: Value) -> ToolInvocation {
    ToolInvocation::new(
        ToolCallId::from_str("0195a0b1-5e45-75be-8284-0aa7aa000011").unwrap(),
        ToolName::from_str(name).unwrap(),
        arguments,
        ProtocolMetadata::default(),
    )
    .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn fake_read_returns_deterministic_content() {
    let executor = Arc::new(FakeReadTool::new([(
        "/workspace/notes.txt".to_owned(),
        "hello".to_owned(),
    )]));
    let mut registry = ToolRegistry::new();
    registry
        .register(
            read_spec(),
            Arc::new(
                ArgumentResourceResolver::new("path", "file", ToolResourceAccess::Read).unwrap(),
            ),
            executor,
        )
        .unwrap();

    let events = registry
        .execute(
            invocation("read_file", json!({"path":"/workspace/notes.txt"})),
            CancellationScope::new(),
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(
        events.as_slice(),
        [ToolExecutionEvent::Finished(result)]
            if result.output() == &json!({"content":"hello"})
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn fake_write_captures_only_schema_valid_invocations() {
    let executor = Arc::new(FakeWriteTool::new());
    let mut registry = ToolRegistry::new();
    registry
        .register(
            write_spec(),
            Arc::new(
                ArgumentResourceResolver::new("path", "file", ToolResourceAccess::Write).unwrap(),
            ),
            executor.clone(),
        )
        .unwrap();

    assert!(
        registry
            .execute(
                invocation("write_file", json!({"path":"/workspace/a.txt"})),
                CancellationScope::new(),
            )
            .is_err()
    );
    assert!(executor.writes().unwrap().is_empty());

    let events = registry
        .execute(
            invocation(
                "write_file",
                json!({"path":"/workspace/a.txt","content":"hello"}),
            ),
            CancellationScope::new(),
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(
        events.last(),
        Some(ToolExecutionEvent::Finished(_))
    ));
    assert_eq!(
        executor.writes().unwrap(),
        [("/workspace/a.txt".to_owned(), "hello".to_owned())]
    );
}

fn process_registry(script: FakeProcessScript) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry
        .register(
            process_spec(),
            Arc::new(StaticResourceResolver::new([]).unwrap()),
            Arc::new(FakeProcessTool::new(script)),
        )
        .unwrap();
    registry
}

#[tokio::test(flavor = "current_thread")]
async fn fake_process_emits_progress_success_and_failure_without_real_process() {
    let complete = process_registry(FakeProcessScript::Complete {
        stdout: "ok".to_owned(),
    });
    let events = complete
        .execute(
            invocation("run_process", json!({"command":"echo ok"})),
            CancellationScope::new(),
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(
        events.as_slice(),
        [
            ToolExecutionEvent::Progress(_),
            ToolExecutionEvent::Finished(_)
        ]
    ));

    let failed = process_registry(FakeProcessScript::Fail {
        message: "exit 1".to_owned(),
    });
    let events = failed
        .execute(
            invocation("run_process", json!({"command":"false"})),
            CancellationScope::new(),
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(
        events.as_slice(),
        [ToolExecutionEvent::Failed(failure)]
            if failure.code() == ToolExecutionFailureCode::ExecutionFailed
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn fake_process_waits_for_cooperative_cancellation() {
    let registry = process_registry(FakeProcessScript::AwaitCancellation);
    let cancellation = CancellationScope::new();
    let stream = registry
        .execute(
            invocation("run_process", json!({"command":"wait"})),
            cancellation.clone(),
        )
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
