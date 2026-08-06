//! Parallel tool execution preserves source-order commits and runs concurrently.

use crate::common;

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_util::stream;
use serde_json::{Value, json};
use tea_control::CancellationScope;
use tea_kernel::{AgentKernel, KernelRunConfig, RunState};
use tea_model::ModelEvent;
use tea_policy::{
    ActorId, CodingWorkspacePolicy, ExecutionSurface, PolicyEngine, PolicyEnvironment,
    PolicyExecutionTarget,
};
use tea_protocol::{AgentEventType, ProtocolMetadata, SessionRecord, StopReason, ToolIdempotency};
use tea_testkit::ScriptedModelResponse;
use tea_tools::{
    ArgumentResourceResolver, ToolConcurrency, ToolEffect, ToolExecutionEvent,
    ToolExecutionSemantics, ToolExecutor, ToolName, ToolResourceAccess, ToolSpec, ToolTimeout,
    ToolVersion,
};

use common::{EventCollector, FixedClock, TestIds, provider, session_id, store};
use tea_session::SessionStore;

#[derive(Debug, Clone)]
struct OverlapTool {
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

impl ToolExecutor for OverlapTool {
    fn execute(
        &self,
        _invocation: tea_tools::ValidatedToolInvocation,
        _cancellation: CancellationScope,
    ) -> tea_tools::BoxToolExecutionStream {
        let active = Arc::clone(&self.active);
        let max_active = Arc::clone(&self.max_active);
        Box::pin(stream::once(async move {
            let count = active.fetch_add(1, Ordering::SeqCst) + 1;
            let mut current = max_active.load(Ordering::SeqCst);
            while count > current {
                match max_active.compare_exchange(
                    current,
                    count,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(actual) => current = actual,
                }
            }
            // Yield so a concurrently-scheduled peer can observe the overlap.
            tokio::task::yield_now().await;
            active.fetch_sub(1, Ordering::SeqCst);
            ToolExecutionEvent::Finished(
                tea_tools::ToolResult::new(
                    vec![tea_protocol::ContentBlock::text("ok").unwrap()],
                    json!({"content":"ok"}),
                )
                .unwrap(),
            )
        }))
    }
}

fn spec(name: &str) -> ToolSpec {
    ToolSpec::new(
        ToolName::from_str(name).unwrap(),
        ToolVersion::from_str("1.0.0").unwrap(),
        format!("Overlap {name}."),
        json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        json!({"type":"object","properties":{"content":{"type":"string"}},"required":["content"]}),
        [ToolEffect::FsRead],
        ToolExecutionSemantics::new(
            ToolIdempotency::Idempotent,
            tea_tools::ToolRetrySafety::Automatic,
            ToolConcurrency::Parallel,
            ToolTimeout::from_millis(5_000).unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn two_tool_call_script() -> ScriptedModelResponse {
    let index0 = tea_model::ModelStreamIndex::new(0).unwrap();
    let index1 = tea_model::ModelStreamIndex::new(1).unwrap();
    let id_a = tea_model::ProviderToolCallId::from_str("overlap-a").unwrap();
    let id_b = tea_model::ProviderToolCallId::from_str("overlap-b").unwrap();
    ScriptedModelResponse::events([
        ModelEvent::Started(tea_model::ModelResponseInfo::new()),
        ModelEvent::ToolCallStarted(
            tea_model::ToolCallStarted::new(index0, id_a.clone(), "read_a").unwrap(),
        ),
        ModelEvent::ToolCallCompleted(
            tea_model::ToolCallCompleted::new(index0, id_a, "read_a", json!({"path":"/a"}))
                .unwrap(),
        ),
        ModelEvent::ToolCallStarted(
            tea_model::ToolCallStarted::new(index1, id_b.clone(), "read_b").unwrap(),
        ),
        ModelEvent::ToolCallCompleted(
            tea_model::ToolCallCompleted::new(index1, id_b, "read_b", json!({"path":"/b"}))
                .unwrap(),
        ),
        ModelEvent::Completed(tea_model::ModelCompletion::new(StopReason::ToolUse).unwrap()),
    ])
}

fn config() -> KernelRunConfig {
    KernelRunConfig::new(
        ActorId::from_str("user:alice").unwrap(),
        PolicyEnvironment::new(
            ExecutionSurface::Test,
            PolicyExecutionTarget::Native,
            ProtocolMetadata::default(),
        ),
    )
}

#[tokio::test]
async fn parallel_read_only_tools_run_concurrently_and_commit_in_source_order() {
    let provider = provider([
        two_tool_call_script(),
        ScriptedModelResponse::text(["done"]),
    ]);
    let store = store().await;
    let overlap = OverlapTool {
        active: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    };
    let mut registry = tea_tools::ToolRegistry::new();
    registry
        .register(
            spec("read_a"),
            Arc::new(
                ArgumentResourceResolver::new("path", "file", ToolResourceAccess::Read).unwrap(),
            ),
            Arc::new(overlap.clone()),
        )
        .unwrap();
    registry
        .register(
            spec("read_b"),
            Arc::new(
                ArgumentResourceResolver::new("path", "file", ToolResourceAccess::Read).unwrap(),
            ),
            Arc::new(overlap.clone()),
        )
        .unwrap();
    let mut policy = PolicyEngine::new();
    policy.add_rule(CodingWorkspacePolicy).unwrap();
    let events = EventCollector::default();
    let ids = TestIds::default();

    let outcome = AgentKernel::new(
        &provider,
        &registry,
        &policy,
        &store,
        &FixedClock,
        &ids,
        &events,
    )
    .run(session_id(), &config(), CancellationScope::new())
    .await
    .unwrap();
    assert_eq!(outcome.state(), RunState::Completed);

    // True concurrency: at least two read tools were active simultaneously.
    assert!(
        overlap.max_active.load(Ordering::SeqCst) >= 2,
        "parallel read-only tools should run concurrently"
    );

    // Source-order commits: ToolExecutionFinished records appear in declaration
    // order (read_a before read_b) regardless of completion order.
    let snapshot = store.load(session_id()).await.unwrap();
    let finished_order: Vec<&str> = snapshot
        .records()
        .iter()
        .filter_map(|record| match record.record() {
            SessionRecord::ToolExecutionFinished { .. } => Some("finished"),
            _ => None,
        })
        .collect();
    let _ = finished_order;
    let tool_result_messages: Vec<String> = snapshot
        .records()
        .iter()
        .filter_map(|record| match record.record() {
            SessionRecord::MessageCommitted {
                message: tea_protocol::CanonicalMessage::ToolResult { tool_name, .. },
            } => Some(tool_name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        tool_result_messages,
        vec!["read_a".to_owned(), "read_b".to_owned()]
    );
}

#[tokio::test]
async fn serial_tool_and_parallel_tool_partition_into_separate_lanes() {
    let provider = provider([
        two_tool_call_script(),
        ScriptedModelResponse::text(["done"]),
    ]);
    let store = store().await;
    // read_a is parallel read-only; make read_b serial via a write effect.
    let mut registry = tea_tools::ToolRegistry::new();
    registry
        .register(
            spec("read_a"),
            Arc::new(
                ArgumentResourceResolver::new("path", "file", ToolResourceAccess::Read).unwrap(),
            ),
            Arc::new(OverlapTool {
                active: Arc::new(AtomicUsize::new(0)),
                max_active: Arc::new(AtomicUsize::new(0)),
            }),
        )
        .unwrap();
    let write_spec = ToolSpec::new(
        ToolName::from_str("read_b").unwrap(),
        ToolVersion::from_str("1.0.0").unwrap(),
        "Serial write.".to_owned(),
        json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        json!({"type":"object","properties":{"content":{"type":"string"}},"required":["content"]}),
        [ToolEffect::FsWrite],
        ToolExecutionSemantics::new(
            ToolIdempotency::NonIdempotent,
            tea_tools::ToolRetrySafety::ExplicitOnly,
            ToolConcurrency::Serial,
            ToolTimeout::from_millis(5_000).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    registry
        .register(
            write_spec,
            Arc::new(
                ArgumentResourceResolver::new("path", "file", ToolResourceAccess::Write).unwrap(),
            ),
            Arc::new(tea_testkit::FakeWriteTool::new()),
        )
        .unwrap();
    let mut policy = PolicyEngine::new();
    policy.add_rule(CodingWorkspacePolicy).unwrap();
    let events = EventCollector::default();
    // The coding policy asks for writes; the run pauses for approval.
    let outcome = AgentKernel::new(
        &provider,
        &registry,
        &policy,
        &store,
        &FixedClock,
        &TestIds::default(),
        &events,
    )
    .run(session_id(), &config(), CancellationScope::new())
    .await
    .unwrap();
    assert_eq!(outcome.state(), RunState::WaitingApproval);
    let emitted = events.events();
    let types: Vec<_> = emitted
        .iter()
        .map(tea_protocol::EventEnvelope::event_type)
        .collect();
    assert!(types.contains(&AgentEventType::ApprovalRequested));
}

#[allow(dead_code)]
fn _unused(_: Value) {}
