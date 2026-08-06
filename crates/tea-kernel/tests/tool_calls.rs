use crate::common;

use std::str::FromStr;
use std::sync::Arc;

use futures_util::stream;
use serde_json::json;
use tea_control::CancellationScope;
use tea_kernel::{AgentKernel, KernelRunConfig, RunState};
use tea_model::{
    ModelCompletion, ModelEvent, ModelResponseInfo, ModelStreamIndex, ProviderToolCallId,
    ToolCallCompleted, ToolCallStarted,
};
use tea_policy::{
    ActorId, CodingWorkspacePolicy, ExecutionSurface, PolicyEngine, PolicyEnvironment,
    PolicyExecutionTarget,
};
use tea_protocol::{
    CanonicalMessage, CodeChange, CodeChangeHunk, CodeChangeKind, CodeChangeLine,
    CodeChangeLineKind, ContentBlock, ExecutionTarget, ProtocolMetadata, SessionRecord,
    ToolFailure, ToolIdempotency, ToolPresentation,
};
use tea_testkit::{FakeReadTool, ScriptedModelResponse};
use tea_tools::{
    ArgumentResourceResolver, BoxToolExecutionStream, TOOL_AUDIT_METADATA_NAMESPACE,
    ToolConcurrency, ToolEffect, ToolExecutionEvent, ToolExecutionSemantics, ToolExecutor,
    ToolName, ToolRegistry, ToolResourceAccess, ToolResult, ToolRetrySafety, ToolSource,
    ToolSourceKind, ToolSpec, ToolTimeout, ToolTrust, ToolVersion, ValidatedToolInvocation,
};

use common::{EventCollector, FixedClock, TestIds, provider, session_id, store};

fn read_registry(fake: FakeReadTool) -> ToolRegistry {
    read_registry_with_source(fake, ToolSource::native_product())
}

fn read_registry_with_source(fake: FakeReadTool, source: ToolSource) -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    tools
        .register(
            ToolSpec::new(
                ToolName::from_str("read_file").unwrap(),
                ToolVersion::from_str("1.0.0").unwrap(),
                "Read one workspace file.",
                json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
                json!({"type":"object","properties":{"content":{"type":"string"}},"required":["content"]}),
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
            .with_source(source),
            Arc::new(
                ArgumentResourceResolver::new("path", "file", ToolResourceAccess::Read).unwrap(),
            ),
            Arc::new(fake),
        )
        .unwrap();
    tools
}

#[derive(Debug)]
struct PresentationReadTool;

impl ToolExecutor for PresentationReadTool {
    fn execute(
        &self,
        _invocation: ValidatedToolInvocation,
        _cancellation: CancellationScope,
    ) -> BoxToolExecutionStream {
        let presentation = ToolPresentation::CodeChange(
            CodeChange::new(
                "notes.txt",
                CodeChangeKind::Update,
                vec![
                    CodeChangeHunk::new(
                        1,
                        1,
                        1,
                        1,
                        vec![
                            CodeChangeLine::new(CodeChangeLineKind::Deletion, Some(1), None, "old")
                                .unwrap(),
                            CodeChangeLine::new(CodeChangeLineKind::Addition, None, Some(1), "new")
                                .unwrap(),
                        ],
                    )
                    .unwrap(),
                ],
                false,
                None,
                Some("--- notes.txt\n+++ notes.txt\n".to_owned()),
                Some(1),
            )
            .unwrap(),
        );
        let result = ToolResult::new(
            vec![ContentBlock::text("edited notes").unwrap()],
            json!({"content":"edited notes"}),
        )
        .unwrap()
        .with_presentation(presentation);
        Box::pin(stream::once(
            async move { ToolExecutionEvent::Finished(result) },
        ))
    }
}

fn presentation_registry() -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    tools
        .register(
            ToolSpec::new(
                ToolName::from_str("read_file").unwrap(),
                ToolVersion::from_str("1.0.0").unwrap(),
                "Read one workspace file.",
                json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
                json!({"type":"object","properties":{"content":{"type":"string"}},"required":["content"]}),
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
            .with_source(ToolSource::native_product()),
            Arc::new(
                ArgumentResourceResolver::new("path", "file", ToolResourceAccess::Read).unwrap(),
            ),
            Arc::new(PresentationReadTool),
        )
        .unwrap();
    tools
}

fn mcp_source() -> ToolSource {
    ToolSource::new(
        ToolSourceKind::Mcp,
        "workspace.files",
        ToolTrust::Workspace,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .unwrap()
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

fn tool_script(name: &str) -> ScriptedModelResponse {
    let index = ModelStreamIndex::new(0).unwrap();
    let provider_id = ProviderToolCallId::from_str("provider-call-1").unwrap();
    ScriptedModelResponse::events([
        ModelEvent::Started(ModelResponseInfo::new()),
        ModelEvent::ToolCallStarted(
            ToolCallStarted::new(index, provider_id.clone(), name).unwrap(),
        ),
        ModelEvent::ToolCallCompleted(
            ToolCallCompleted::new(index, provider_id, name, json!({"path":"/notes.txt"})).unwrap(),
        ),
        ModelEvent::Completed(ModelCompletion::new(tea_protocol::StopReason::ToolUse).unwrap()),
    ])
}

#[tokio::test]
async fn allowed_read_executes_and_feeds_the_next_model_turn() {
    let provider = provider([
        tool_script("read_file"),
        ScriptedModelResponse::text(["done"]),
    ]);
    let store = store().await;
    let tools = read_registry(FakeReadTool::new([(
        "/notes.txt".to_owned(),
        "hello".to_owned(),
    )]));
    let mut policy = PolicyEngine::new();
    policy.add_rule(CodingWorkspacePolicy).unwrap();
    let ids = TestIds::default();
    let events = EventCollector::default();
    let kernel = AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &ids,
        &events,
    );

    let outcome = kernel
        .run(session_id(), &config(), CancellationScope::new())
        .await
        .unwrap();
    assert_eq!(outcome.state(), RunState::Completed);
    let requests = provider.captured_requests().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].messages().len(), 3);
    assert!(requests[1].messages().iter().any(|message| {
        matches!(
            message,
            CanonicalMessage::Assistant { content, .. }
                if content.iter().any(|block| matches!(
                    block,
                    ContentBlock::ToolCall {
                        provider_call_id: Some(provider_call_id),
                        ..
                    } if provider_call_id == "provider-call-1"
                ))
        )
    }));
    let snapshot = tea_session::SessionStore::load(&store, session_id())
        .await
        .unwrap();
    let kinds = snapshot
        .records()
        .iter()
        .map(tea_protocol::RecordEnvelope::record_type)
        .collect::<Vec<_>>();
    assert!(kinds.windows(4).any(|window| {
        window
            == [
                tea_protocol::SessionRecordType::PolicyDecisionRecorded,
                tea_protocol::SessionRecordType::ToolExecutionStarted,
                tea_protocol::SessionRecordType::ToolExecutionFinished,
                tea_protocol::SessionRecordType::MessageCommitted,
            ]
    }));
}

#[tokio::test]
async fn successful_tool_presentation_is_persisted_but_not_sent_to_the_model() {
    let provider = provider([
        tool_script("read_file"),
        ScriptedModelResponse::text(["done"]),
    ]);
    let store = store().await;
    let tools = presentation_registry();
    let mut policy = PolicyEngine::new();
    policy.add_rule(CodingWorkspacePolicy).unwrap();
    AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &TestIds::default(),
        &EventCollector::default(),
    )
    .run(session_id(), &config(), CancellationScope::new())
    .await
    .unwrap();

    let requests = provider.captured_requests().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].messages().iter().any(|message| {
        matches!(
            message,
            CanonicalMessage::ToolResult { content, .. }
                if matches!(content.as_slice(), [ContentBlock::Text { text }] if text == "edited notes")
        )
    }), "unexpected second request: {:?}", requests[1].messages());
    let snapshot = tea_session::SessionStore::load(&store, session_id())
        .await
        .unwrap();
    assert!(snapshot.records().iter().any(|record| matches!(
        record.record(),
        SessionRecord::ToolExecutionFinished {
            presentation: Some(ToolPresentation::CodeChange(change)),
            ..
        } if change.path() == "notes.txt"
    )));
    assert!(snapshot.state().tool_calls().values().any(|tool| matches!(
        tool.execution(),
        tea_session::ToolExecutionState::Finished {
            presentation: Some(ToolPresentation::CodeChange(change)),
            ..
        } if change.first_changed_line() == Some(1)
    )));
}

#[tokio::test]
async fn unknown_tool_becomes_model_visible_failure_without_execution() {
    let provider = provider([
        tool_script("missing_tool"),
        ScriptedModelResponse::text(["recovered"]),
    ]);
    let store = store().await;
    let tools = ToolRegistry::new();
    let policy = PolicyEngine::new();
    let ids = TestIds::default();
    let events = EventCollector::default();
    let kernel = AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &ids,
        &events,
    );

    kernel
        .run(session_id(), &config(), CancellationScope::new())
        .await
        .unwrap();
    let requests = provider.captured_requests().unwrap();
    assert_eq!(requests.len(), 2);
    match requests[1].messages().last().unwrap() {
        CanonicalMessage::ToolResult {
            is_error,
            error: Some(error),
            content,
            ..
        } => {
            assert!(*is_error);
            assert_eq!(error.code(), "unknown_tool");
            assert!(matches!(content[0], ContentBlock::Text { .. }));
        }
        other => panic!("unexpected tool result: {other:?}"),
    }
    let snapshot = tea_session::SessionStore::load(&store, session_id())
        .await
        .unwrap();
    assert!(snapshot.records().iter().any(|record| matches!(
        record.record(),
        SessionRecord::ToolExecutionFinished {
            error: Some(error),
            ..
        } if error == &ToolFailure::new("unknown_tool", "model requested an unknown tool").unwrap()
    )));
}

#[tokio::test]
async fn mcp_source_is_audited_before_policy_and_selects_mcp_execution() {
    let provider = provider([
        tool_script("read_file"),
        ScriptedModelResponse::text(["done"]),
    ]);
    let store = store().await;
    let tools = read_registry_with_source(
        FakeReadTool::new([("/notes.txt".to_owned(), "hello".to_owned())]),
        mcp_source(),
    );
    let mut policy = PolicyEngine::new();
    policy.add_rule(CodingWorkspacePolicy).unwrap();
    AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &TestIds::default(),
        &EventCollector::default(),
    )
    .run(session_id(), &config(), CancellationScope::new())
    .await
    .unwrap();

    let snapshot = tea_session::SessionStore::load(&store, session_id())
        .await
        .unwrap();
    let requested = snapshot
        .records()
        .iter()
        .find(|record| matches!(record.record(), SessionRecord::ToolCallRequested { .. }))
        .unwrap();
    assert_eq!(
        requested.metadata().get(TOOL_AUDIT_METADATA_NAMESPACE),
        Some(&json!({
            "toolVersion":"1.0.0",
            "source":{
                "kind":"mcp",
                "sourceId":"workspace.files",
                "trust":"workspace",
                "descriptorDigest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            "effects":["fs.read"],
            "resources":[{
                "scheme":"file",
                "redactedPresentation":"file:/notes.txt",
                "access":"read"
            }]
        }))
    );
    assert!(snapshot.records().iter().any(|record| matches!(
        record.record(),
        SessionRecord::ToolExecutionStarted {
            execution_target: ExecutionTarget::Mcp,
            ..
        }
    )));
}
