use crate::common;

use std::str::FromStr;
use std::sync::Arc;

use serde_json::json;
use tea_control::CancellationScope;
use tea_kernel::{
    AgentKernel, KernelEventFuture, KernelEventSink, KernelInputQueue, KernelRunConfig,
};
use tea_model::{
    ModelCompletion, ModelEvent, ModelResponseInfo, ModelStreamIndex, ProviderToolCallId,
    ToolCallCompleted, ToolCallStarted,
};
use tea_policy::{
    ActorId, CodingWorkspacePolicy, ExecutionSurface, PolicyEngine, PolicyEnvironment,
    PolicyExecutionTarget,
};
use tea_protocol::{
    AgentEvent, CanonicalMessage, CommandText, ContentBlock, EventEnvelope, MessageId,
    ProtocolMetadata, StopReason, ToolIdempotency,
};
use tea_session::SessionStore;
use tea_testkit::{FakeReadTool, ScriptedModelResponse};
use tea_tools::{
    ArgumentResourceResolver, ToolConcurrency, ToolEffect, ToolExecutionSemantics, ToolName,
    ToolRegistry, ToolResourceAccess, ToolRetrySafety, ToolSpec, ToolTimeout, ToolVersion,
};

use common::{FixedClock, TestIds, provider, session_id, store, timestamp};

fn user(id: &str, text: &str) -> CanonicalMessage {
    CanonicalMessage::user(
        MessageId::from_str(id).unwrap(),
        vec![ContentBlock::text(text).unwrap()],
        timestamp(),
    )
    .unwrap()
}

#[test]
fn queue_bounds_preserve_already_accepted_entries() {
    let queue = KernelInputQueue::new(1, 8).unwrap();
    let first = user("0195a0b1-6100-7000-8000-000000000001", "first");
    queue.enqueue_follow_up(first).unwrap();
    assert!(
        queue
            .enqueue_follow_up(user("0195a0b1-6101-7000-8000-000000000001", "second"))
            .is_err()
    );
    assert_eq!(queue.lengths().unwrap(), (1, 0));

    queue
        .enqueue_steering(CommandText::new("steer").unwrap())
        .unwrap();
    assert!(
        queue
            .enqueue_steering(CommandText::new("more").unwrap())
            .is_err()
    );
    assert_eq!(queue.lengths().unwrap(), (1, 1));
}

#[test]
fn queue_rejects_non_user_follow_up_and_invalid_limits() {
    assert!(KernelInputQueue::new(0, 1).is_err());
    let queue = KernelInputQueue::new(2, 32).unwrap();
    let assistant = CanonicalMessage::assistant(
        MessageId::from_str("0195a0b1-6102-7000-8000-000000000001").unwrap(),
        vec![ContentBlock::text("assistant").unwrap()],
        tea_protocol::StopReason::Completed,
        timestamp(),
    )
    .unwrap();
    assert!(queue.enqueue_follow_up(assistant).is_err());
    assert_eq!(queue.lengths().unwrap(), (0, 0));
}

#[test]
fn queue_and_run_configuration_are_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<KernelInputQueue>();
    assert_send_sync::<KernelRunConfig>();
    let _config = KernelRunConfig::new(
        ActorId::from_str("user:alice").unwrap(),
        PolicyEnvironment::new(
            ExecutionSurface::Test,
            PolicyExecutionTarget::Native,
            ProtocolMetadata::default(),
        ),
    );
}

#[derive(Debug)]
struct QueueingSink<'a> {
    queue: &'a KernelInputQueue,
}

impl KernelEventSink for QueueingSink<'_> {
    fn emit(&self, event: EventEnvelope) -> KernelEventFuture<'_> {
        Box::pin(async move {
            if matches!(event.event(), AgentEvent::ToolCallRequested { .. })
                && self.queue.lengths()? == (0, 0)
            {
                self.queue
                    .enqueue_steering(CommandText::new("steer next").map_err(|error| {
                        tea_kernel::KernelError::new(
                            tea_kernel::KernelErrorCode::InvalidRequest,
                            error.to_string(),
                        )
                    })?)?;
                self.queue.enqueue_follow_up(user(
                    "0195a0b1-6103-7000-8000-000000000001",
                    "follow next",
                ))?;
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn active_request_is_immutable_and_queue_applies_to_next_turn() {
    let index = ModelStreamIndex::new(0).unwrap();
    let call_id = ProviderToolCallId::from_str("queue-read").unwrap();
    let tool_response = ScriptedModelResponse::events([
        ModelEvent::Started(ModelResponseInfo::new()),
        ModelEvent::ToolCallStarted(
            ToolCallStarted::new(index, call_id.clone(), "read_file").unwrap(),
        ),
        ModelEvent::ToolCallCompleted(
            ToolCallCompleted::new(index, call_id, "read_file", json!({"path":"/notes.txt"}))
                .unwrap(),
        ),
        ModelEvent::Completed(ModelCompletion::new(StopReason::ToolUse).unwrap()),
    ]);
    let provider = provider([
        tool_response,
        ScriptedModelResponse::text(["queued input observed"]),
    ]);
    let store = store().await;
    let mut tools = ToolRegistry::new();
    tools
        .register(
            ToolSpec::new(
                ToolName::from_str("read_file").unwrap(),
                ToolVersion::from_str("1.0.0").unwrap(),
                "Read one fake file.",
                json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
                json!({"type":"object","properties":{"content":{"type":"string"}},"required":["content"]}),
                [ToolEffect::FsRead],
                ToolExecutionSemantics::new(
                    ToolIdempotency::Idempotent,
                    ToolRetrySafety::Automatic,
                    ToolConcurrency::Serial,
                    ToolTimeout::from_millis(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
            Arc::new(
                ArgumentResourceResolver::new("path", "file", ToolResourceAccess::Read).unwrap(),
            ),
            Arc::new(FakeReadTool::new([(
                "/notes.txt".to_owned(),
                "hello".to_owned(),
            )])),
        )
        .unwrap();
    let mut policy = PolicyEngine::new();
    policy.add_rule(CodingWorkspacePolicy).unwrap();
    let queue = KernelInputQueue::new(4, 1024).unwrap();
    let sink = QueueingSink { queue: &queue };
    let ids = TestIds::default();
    let config = KernelRunConfig::new(
        ActorId::from_str("user:alice").unwrap(),
        PolicyEnvironment::new(
            ExecutionSurface::Test,
            PolicyExecutionTarget::Native,
            ProtocolMetadata::default(),
        ),
    );
    AgentKernel::new(&provider, &tools, &policy, &store, &FixedClock, &ids, &sink)
        .with_input_queue(&queue)
        .run(session_id(), &config, CancellationScope::new())
        .await
        .unwrap();

    let requests = provider.captured_requests().unwrap();
    assert_eq!(requests[0].messages().len(), 1);
    assert_eq!(requests[1].messages().len(), 5);
    assert_eq!(queue.lengths().unwrap(), (0, 0));
    let snapshot = store.load(session_id()).await.unwrap();
    assert_eq!(snapshot.state().messages().len(), 6);
}
