use crate::common;

use std::str::FromStr;
use std::sync::Arc;

use serde_json::json;
use tea_control::CancellationScope;
use tea_kernel::{AgentKernel, KernelRunConfig, RunState};
use tea_model::{
    ModelCompletion, ModelEvent, ModelResponseInfo, ModelStreamIndex, ProviderToolCallId,
    ToolCallCompleted, ToolCallStarted,
};
use tea_policy::{
    ActorId, ApprovalResolution, CodingWorkspacePolicy, ExecutionSurface, PolicyEngine,
    PolicyEnvironment, PolicyExecutionTarget,
};
use tea_protocol::{
    CodeChange, CodeChangeKind, NextTurnAction, ProtocolMetadata, SessionRecord, StopReason,
    ToolIdempotency, ToolPresentation,
};
use tea_session::SessionStore;
use tea_testkit::{FakeWriteTool, ScriptedModelResponse};
use tea_tools::{
    ArgumentResourceResolver, BoxToolExecutionStream, ToolConcurrency, ToolEffect,
    ToolExecutionSemantics, ToolExecutor, ToolName, ToolRegistry, ToolResourceAccess,
    ToolRetrySafety, ToolSource, ToolSourceKind, ToolSpec, ToolTimeout, ToolTrust, ToolVersion,
};

use common::{EventCollector, FixedClock, TestIds, provider, session_id, store, timestamp};

fn write_registry(fake: FakeWriteTool) -> ToolRegistry {
    write_registry_with_source(fake, ToolSource::native_product())
}

fn write_registry_with_source(fake: FakeWriteTool, source: ToolSource) -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    tools
        .register(
            ToolSpec::new(
                ToolName::from_str("write_file").unwrap(),
                ToolVersion::from_str("1.0.0").unwrap(),
                "Write one workspace file.",
                json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
                json!({"type":"object","properties":{"path":{"type":"string"},"writtenBytes":{"type":"integer"}},"required":["path","writtenBytes"]}),
                [ToolEffect::FsWrite],
                ToolExecutionSemantics::new(
                    ToolIdempotency::NonIdempotent,
                    ToolRetrySafety::ExplicitOnly,
                    ToolConcurrency::Serial,
                    ToolTimeout::from_millis(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap()
            .with_source(source),
            Arc::new(
                ArgumentResourceResolver::new("path", "file", ToolResourceAccess::Write).unwrap(),
            ),
            Arc::new(fake),
        )
        .unwrap();
    tools
}

#[derive(Debug)]
struct PreviewWriteTool;

impl ToolExecutor for PreviewWriteTool {
    fn preview(
        &self,
        _invocation: &tea_tools::ValidatedToolInvocation,
    ) -> Option<ToolPresentation> {
        Some(ToolPresentation::CodeChange(
            CodeChange::new(
                "notes.txt",
                CodeChangeKind::Update,
                Vec::new(),
                false,
                None,
                None,
                None,
            )
            .unwrap(),
        ))
    }

    fn execute(
        &self,
        _invocation: tea_tools::ValidatedToolInvocation,
        _cancellation: CancellationScope,
    ) -> BoxToolExecutionStream {
        panic!("an approval preview must not execute the tool")
    }
}

fn preview_registry() -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    tools
        .register(
            ToolSpec::new(
                ToolName::from_str("write_file").unwrap(),
                ToolVersion::from_str("1.0.0").unwrap(),
                "Write one workspace file.",
                json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
                json!({"type":"object","properties":{"path":{"type":"string"},"writtenBytes":{"type":"integer"}},"required":["path","writtenBytes"]}),
                [ToolEffect::FsWrite],
                ToolExecutionSemantics::new(
                    ToolIdempotency::NonIdempotent,
                    ToolRetrySafety::ExplicitOnly,
                    ToolConcurrency::Serial,
                    ToolTimeout::from_millis(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
            Arc::new(
                ArgumentResourceResolver::new("path", "file", ToolResourceAccess::Write).unwrap(),
            ),
            Arc::new(PreviewWriteTool),
        )
        .unwrap();
    tools
}

fn mcp_source(digest: &str) -> ToolSource {
    mcp_source_for("workspace.files", digest)
}

fn mcp_source_for(source_id: &str, digest: &str) -> ToolSource {
    ToolSource::new(ToolSourceKind::Mcp, source_id, ToolTrust::Workspace, digest).unwrap()
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

fn write_script() -> ScriptedModelResponse {
    let index = ModelStreamIndex::new(0).unwrap();
    let provider_call_id = ProviderToolCallId::from_str("provider-write").unwrap();
    ScriptedModelResponse::events([
        ModelEvent::Started(ModelResponseInfo::new()),
        ModelEvent::ToolCallStarted(
            ToolCallStarted::new(index, provider_call_id.clone(), "write_file").unwrap(),
        ),
        ModelEvent::ToolCallCompleted(
            ToolCallCompleted::new(
                index,
                provider_call_id,
                "write_file",
                json!({"path":"/notes.txt","content":"hello"}),
            )
            .unwrap(),
        ),
        ModelEvent::Completed(ModelCompletion::new(StopReason::ToolUse).unwrap()),
    ])
}

#[tokio::test]
async fn denial_resolution_and_failure_result_commit_atomically() {
    let provider = provider([
        write_script(),
        ScriptedModelResponse::text(["denial handled"]),
    ]);
    let store = store().await;
    let fake = FakeWriteTool::new();
    let tools = write_registry(fake.clone());
    let mut policy = PolicyEngine::new();
    policy.add_rule(CodingWorkspacePolicy).unwrap();
    let events = EventCollector::default();
    let first_ids = TestIds::default();
    AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &first_ids,
        &events,
    )
    .run(session_id(), &config(), CancellationScope::new())
    .await
    .unwrap();
    let snapshot = store.load(session_id()).await.unwrap();
    let request = match &snapshot.approval_artifacts()[0] {
        tea_session::ApprovalArtifactEntry::Requested { request, .. } => request.clone(),
        tea_session::ApprovalArtifactEntry::Resolved { .. } => panic!("expected request"),
    };
    let resolution = ApprovalResolution::new(
        &request,
        tea_protocol::ApprovalDecision::Deny,
        timestamp(),
        None,
    )
    .unwrap();
    let resume_ids = TestIds::with_start(300);
    let outcome = AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &resume_ids,
        &events,
    )
    .resume_approval(
        session_id(),
        &resolution,
        &config(),
        CancellationScope::new(),
    )
    .await
    .unwrap();
    assert_eq!(outcome.state(), RunState::Completed);
    assert!(fake.writes().unwrap().is_empty());
    let snapshot = store.load(session_id()).await.unwrap();
    let resolution_index = snapshot
        .records()
        .iter()
        .position(|record| matches!(record.record(), SessionRecord::ApprovalResolved { .. }))
        .unwrap();
    assert!(matches!(
        snapshot.records()[resolution_index + 1].record(),
        SessionRecord::ToolExecutionFinished { is_error: true, .. }
    ));
    assert!(matches!(
        snapshot.records()[resolution_index + 2].record(),
        SessionRecord::MessageCommitted { .. }
    ));
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn fresh_kernel_resumes_allowed_approval_from_persisted_context() {
    let provider = provider([
        write_script(),
        ScriptedModelResponse::text(["write complete"]),
    ]);
    let store = store().await;
    let fake = FakeWriteTool::new();
    let tools = write_registry(fake.clone());
    let mut policy = PolicyEngine::new();
    policy.add_rule(CodingWorkspacePolicy).unwrap();
    let events = EventCollector::default();
    let first_ids = TestIds::default();
    let waiting = AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &first_ids,
        &events,
    )
    .run(session_id(), &config(), CancellationScope::new())
    .await
    .unwrap();

    let snapshot = store.load(session_id()).await.unwrap();
    let request = match &snapshot.approval_artifacts()[0] {
        tea_session::ApprovalArtifactEntry::Requested { request, .. } => request.clone(),
        tea_session::ApprovalArtifactEntry::Resolved { .. } => {
            panic!("expected pending request")
        }
    };
    let resolution = ApprovalResolution::new(
        &request,
        tea_protocol::ApprovalDecision::AllowOnce,
        timestamp(),
        None,
    )
    .unwrap();
    let mismatch_ids = TestIds::with_start(50);
    let mismatched_config = KernelRunConfig::new(
        ActorId::from_str("user:alice").unwrap(),
        PolicyEnvironment::new(
            ExecutionSurface::Test,
            PolicyExecutionTarget::Remote,
            ProtocolMetadata::default(),
        ),
    );
    let mismatch = AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &mismatch_ids,
        &events,
    )
    .resume_approval(
        session_id(),
        &resolution,
        &mismatched_config,
        CancellationScope::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(mismatch.code(), tea_kernel::KernelErrorCode::PolicyFailure);
    assert!(fake.writes().unwrap().is_empty());

    let resume_ids = TestIds::with_start(100);
    let resumed = AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &resume_ids,
        &events,
    )
    .resume_approval(
        session_id(),
        &resolution,
        &config(),
        CancellationScope::new(),
    )
    .await
    .unwrap();

    assert_eq!(waiting.state(), RunState::WaitingApproval);
    assert_eq!(resumed.state(), RunState::Completed);
    assert_eq!(
        fake.writes().unwrap(),
        [("/notes.txt".to_owned(), "hello".to_owned())]
    );
    let snapshot = store.load(session_id()).await.unwrap();
    assert_eq!(snapshot.approval_artifacts().len(), 2);
    assert!(snapshot.state().pending_approvals().is_empty());
    assert!(
        snapshot
            .records()
            .iter()
            .any(|record| matches!(record.record(), SessionRecord::ToolExecutionStarted { .. }))
    );
}

#[tokio::test]
async fn ask_persists_request_artifact_and_wait_checkpoint_atomically() {
    let provider = provider([write_script()]);
    let store = store().await;
    let fake = FakeWriteTool::new();
    let tools = write_registry(fake.clone());
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
    assert_eq!(outcome.state(), RunState::WaitingApproval);
    let approval_id = outcome.pending_approval_id().unwrap();
    assert!(
        outcome
            .session()
            .pending_approvals()
            .contains_key(&approval_id)
    );
    assert!(fake.writes().unwrap().is_empty());

    let snapshot = store.load(session_id()).await.unwrap();
    assert_eq!(snapshot.approval_artifacts().len(), 1);
    assert_eq!(snapshot.journal_revision(), 1);
    assert_eq!(
        snapshot.state().latest_checkpoint().unwrap().next_action(),
        NextTurnAction::WaitForApproval
    );
    assert!(matches!(
        snapshot.records()[snapshot.records().len() - 3].record(),
        SessionRecord::PolicyDecisionRecorded { .. }
    ));
    assert!(matches!(
        snapshot.records()[snapshot.records().len() - 2].record(),
        SessionRecord::ApprovalRequested { .. }
    ));
    assert!(matches!(
        snapshot.records().last().unwrap().record(),
        SessionRecord::TurnCheckpointed { .. }
    ));
}

#[tokio::test]
async fn approval_preview_is_ephemeral_and_precedes_the_approval_event() {
    let provider = provider([write_script()]);
    let store = store().await;
    let tools = preview_registry();
    let mut policy = PolicyEngine::new();
    policy.add_rule(CodingWorkspacePolicy).unwrap();
    let events = EventCollector::default();

    let outcome = AgentKernel::new(
        &provider,
        &tools,
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
    let events = events.events();
    let preview_index = events
        .iter()
        .position(|event| {
            matches!(
                event.event(),
                tea_protocol::AgentEvent::ToolExecutionPreview {
                    presentation: ToolPresentation::CodeChange(change),
                    ..
                } if change.path() == "notes.txt"
            )
        })
        .expect("preview event must be emitted");
    let approval_index = events
        .iter()
        .position(|event| {
            matches!(
                event.event(),
                tea_protocol::AgentEvent::ApprovalRequested { .. }
            )
        })
        .expect("approval event must be emitted");
    assert!(preview_index < approval_index);

    let snapshot = store.load(session_id()).await.unwrap();
    assert!(snapshot.records().iter().all(|record| !matches!(
        record.record(),
        SessionRecord::ToolExecutionFinished {
            presentation: Some(_),
            ..
        }
    )));
}

#[tokio::test]
async fn fresh_kernel_rejects_approval_after_source_digest_drift() {
    let provider = provider([write_script()]);
    let store = store().await;
    let fake = FakeWriteTool::new();
    let original_source =
        mcp_source("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let original_tools = write_registry_with_source(fake.clone(), original_source.clone());
    let mut policy = PolicyEngine::new();
    policy.add_rule(CodingWorkspacePolicy).unwrap();
    let events = EventCollector::default();
    AgentKernel::new(
        &provider,
        &original_tools,
        &policy,
        &store,
        &FixedClock,
        &TestIds::default(),
        &events,
    )
    .run(session_id(), &config(), CancellationScope::new())
    .await
    .unwrap();

    let snapshot = store.load(session_id()).await.unwrap();
    let request = match &snapshot.approval_artifacts()[0] {
        tea_session::ApprovalArtifactEntry::Requested { request, .. } => request.clone(),
        tea_session::ApprovalArtifactEntry::Resolved { .. } => panic!("expected request"),
    };
    assert_eq!(request.tool_source(), &original_source);
    let resolution = ApprovalResolution::new(
        &request,
        tea_protocol::ApprovalDecision::AllowOnce,
        timestamp(),
        None,
    )
    .unwrap();
    let drifted_tools = write_registry_with_source(
        fake.clone(),
        mcp_source("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
    );
    let error = AgentKernel::new(
        &provider,
        &drifted_tools,
        &policy,
        &store,
        &FixedClock,
        &TestIds::with_start(900),
        &events,
    )
    .resume_approval(
        session_id(),
        &resolution,
        &config(),
        CancellationScope::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), tea_kernel::KernelErrorCode::PolicyFailure);
    assert!(fake.writes().unwrap().is_empty());
}

#[tokio::test]
async fn fresh_kernel_rejects_approval_after_mcp_server_substitution() {
    let provider = provider([write_script()]);
    let store = store().await;
    let fake = FakeWriteTool::new();
    let source = mcp_source("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let original_tools = write_registry_with_source(fake.clone(), source.clone());
    let mut policy = PolicyEngine::new();
    policy.add_rule(CodingWorkspacePolicy).unwrap();
    let events = EventCollector::default();
    AgentKernel::new(
        &provider,
        &original_tools,
        &policy,
        &store,
        &FixedClock,
        &TestIds::default(),
        &events,
    )
    .run(session_id(), &config(), CancellationScope::new())
    .await
    .unwrap();

    let snapshot = store.load(session_id()).await.unwrap();
    let request = match &snapshot.approval_artifacts()[0] {
        tea_session::ApprovalArtifactEntry::Requested { request, .. } => request.clone(),
        tea_session::ApprovalArtifactEntry::Resolved { .. } => panic!("expected request"),
    };
    let resolution = ApprovalResolution::new(
        &request,
        tea_protocol::ApprovalDecision::AllowOnce,
        timestamp(),
        None,
    )
    .unwrap();
    let substituted_tools = write_registry_with_source(
        fake.clone(),
        mcp_source_for(
            "workspace.replacement",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
    );
    let error = AgentKernel::new(
        &provider,
        &substituted_tools,
        &policy,
        &store,
        &FixedClock,
        &TestIds::with_start(900),
        &events,
    )
    .resume_approval(
        session_id(),
        &resolution,
        &config(),
        CancellationScope::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), tea_kernel::KernelErrorCode::PolicyFailure);
    assert!(fake.writes().unwrap().is_empty());
}
