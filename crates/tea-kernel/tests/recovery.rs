use crate::common;

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_util::stream;
use serde_json::json;
use tea_control::CancellationScope;
use tea_kernel::{AgentKernel, KernelErrorCode, KernelRunConfig};
use tea_model::{
    ModelCompletion, ModelEvent, ModelResponseInfo, ModelStreamIndex, ProviderToolCallId,
    ToolCallCompleted, ToolCallStarted,
};
use tea_policy::{
    ActorId, ApprovalResolution, CodingWorkspacePolicy, ExecutionSurface, PolicyEngine,
    PolicyEnvironment, PolicyExecutionTarget,
};
use tea_protocol::{
    ApprovalDecision, ContentBlock, ProtocolMetadata, SessionId, SessionRecord, StopReason,
    ToolIdempotency,
};
use tea_session::{
    AppendOutcome, AppendTransaction, InMemorySessionStore, SessionSnapshot, SessionStore,
    SessionStoreError, SessionStoreErrorCode, SessionStoreFuture,
};
use tea_testkit::{FakeWriteTool, ScriptedModelResponse};
use tea_tools::{
    ArgumentResourceResolver, BoxToolExecutionStream, StaticResourceResolver, ToolConcurrency,
    ToolEffect, ToolExecutionEvent, ToolExecutionSemantics, ToolExecutor, ToolName, ToolRegistry,
    ToolResourceAccess, ToolResult, ToolRetrySafety, ToolSpec, ToolTimeout, ToolVersion,
    ValidatedToolInvocation,
};

use common::{EventCollector, FixedClock, TestIds, provider, session_id, store, timestamp};

#[derive(Debug)]
struct FailingStore {
    inner: InMemorySessionStore,
    append_count: AtomicUsize,
    fail_at: AtomicUsize,
}

impl FailingStore {
    const fn new(inner: InMemorySessionStore) -> Self {
        Self {
            inner,
            append_count: AtomicUsize::new(0),
            fail_at: AtomicUsize::new(usize::MAX),
        }
    }
    fn fail_next(&self) {
        let next = self.append_count.load(Ordering::SeqCst) + 1;
        self.fail_at.store(next, Ordering::SeqCst);
    }
    fn fail_at(&self, ordinal: usize) {
        self.fail_at.store(ordinal, Ordering::SeqCst);
    }
}

impl SessionStore for FailingStore {
    fn load(&self, session_id: SessionId) -> SessionStoreFuture<'_, SessionSnapshot> {
        self.inner.load(session_id)
    }
    fn append(&self, transaction: AppendTransaction) -> SessionStoreFuture<'_, AppendOutcome> {
        let ordinal = self.append_count.fetch_add(1, Ordering::SeqCst) + 1;
        if self
            .fail_at
            .compare_exchange(ordinal, usize::MAX, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Box::pin(async {
                Err(SessionStoreError::new(
                    SessionStoreErrorCode::TransactionFailed,
                    "injected append failure",
                ))
            })
        } else {
            self.inner.append(transaction)
        }
    }
    fn active_grants_for_actor(
        &self,
        actor_id: tea_policy::ActorId,
    ) -> SessionStoreFuture<'_, Vec<tea_policy::PolicyGrant>> {
        self.inner.active_grants_for_actor(actor_id)
    }
}

#[derive(Debug, Clone, Default)]
struct CountingExecutor(Arc<AtomicUsize>);
impl CountingExecutor {
    fn calls(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}
impl ToolExecutor for CountingExecutor {
    fn execute(
        &self,
        _invocation: ValidatedToolInvocation,
        _cancellation: CancellationScope,
    ) -> BoxToolExecutionStream {
        self.0.fetch_add(1, Ordering::SeqCst);
        let result = ToolResult::new(
            vec![ContentBlock::text("executed once").unwrap()],
            json!({"value":"ok"}),
        )
        .unwrap();
        Box::pin(stream::iter([ToolExecutionEvent::Finished(result)]))
    }
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

fn tool_script(name: &str, arguments: serde_json::Value) -> ScriptedModelResponse {
    let index = ModelStreamIndex::new(0).unwrap();
    let provider_id = ProviderToolCallId::from_str(&format!("recovery-{name}")).unwrap();
    ScriptedModelResponse::events([
        ModelEvent::Started(ModelResponseInfo::new()),
        ModelEvent::ToolCallStarted(
            ToolCallStarted::new(index, provider_id.clone(), name).unwrap(),
        ),
        ModelEvent::ToolCallCompleted(
            ToolCallCompleted::new(index, provider_id, name, arguments).unwrap(),
        ),
        ModelEvent::Completed(ModelCompletion::new(StopReason::ToolUse).unwrap()),
    ])
}

fn counting_registry(executor: CountingExecutor) -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    tools
        .register(
            ToolSpec::new(
                ToolName::from_str("read_once").unwrap(),
                ToolVersion::from_str("1.0.0").unwrap(),
                "Execute one counted fake operation.",
                json!({"type":"object"}),
                json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}),
                [ToolEffect::FsRead],
                ToolExecutionSemantics::new(
                    ToolIdempotency::NonIdempotent,
                    ToolRetrySafety::Never,
                    ToolConcurrency::Serial,
                    ToolTimeout::from_millis(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
            Arc::new(StaticResourceResolver::new([]).unwrap()),
            Arc::new(executor),
        )
        .unwrap();
    tools
}

fn write_registry(fake: FakeWriteTool) -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    tools
        .register(
            ToolSpec::new(
                ToolName::from_str("write_file").unwrap(),
                ToolVersion::from_str("1.0.0").unwrap(),
                "Write one fake file.",
                json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
                json!({"type":"object","properties":{"path":{"type":"string"},"writtenBytes":{"type":"integer"}},"required":["path","writtenBytes"]}),
                [ToolEffect::FsWrite],
                ToolExecutionSemantics::new(
                    ToolIdempotency::NonIdempotent,
                    ToolRetrySafety::Never,
                    ToolConcurrency::Serial,
                    ToolTimeout::from_millis(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
            Arc::new(
                ArgumentResourceResolver::new("path", "file", ToolResourceAccess::Write).unwrap(),
            ),
            Arc::new(fake),
        )
        .unwrap();
    tools
}

#[tokio::test]
async fn failed_terminal_append_never_replays_non_idempotent_side_effect() {
    let provider = provider([tool_script("read_once", json!({}))]);
    let store = FailingStore::new(store().await);
    let executor = CountingExecutor::default();
    let tools = counting_registry(executor.clone());
    let mut policy = PolicyEngine::new();
    policy.add_rule(CodingWorkspacePolicy).unwrap();
    let events = EventCollector::default();
    let ids = TestIds::default();
    store.fail_at(3);
    let kernel = AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &ids,
        &events,
    );
    let error = kernel
        .run(session_id(), &config(), CancellationScope::new())
        .await
        .unwrap_err();
    assert_eq!(
        error.code(),
        KernelErrorCode::SessionFailure,
        "{}",
        error.message()
    );
    assert_eq!(executor.calls(), 1);
    let snapshot = store.load(session_id()).await.unwrap();
    assert!(matches!(
        snapshot.records().last().unwrap().record(),
        SessionRecord::ToolExecutionStarted { .. }
    ));

    let retry_ids = TestIds::with_start(700);
    let retry_events = EventCollector::default();
    let retry = AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &retry_ids,
        &retry_events,
    );
    let retry_error = retry
        .run(session_id(), &config(), CancellationScope::new())
        .await
        .unwrap_err();
    assert_eq!(retry_error.code(), KernelErrorCode::InvalidState);
    assert_eq!(executor.calls(), 1);
}

#[tokio::test]
async fn failed_resolution_transaction_remains_pending_and_retries_once() {
    let provider = provider([
        tool_script("write_file", json!({"path":"/notes.txt","content":"hello"})),
        ScriptedModelResponse::text(["done"]),
    ]);
    let store = FailingStore::new(store().await);
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
    let paused = store.load(session_id()).await.unwrap();
    let request = match &paused.approval_artifacts()[0] {
        tea_session::ApprovalArtifactEntry::Requested { request, .. } => request.clone(),
        tea_session::ApprovalArtifactEntry::Resolved { .. } => panic!("expected request"),
    };
    let resolution =
        ApprovalResolution::new(&request, ApprovalDecision::AllowOnce, timestamp(), None).unwrap();
    store.fail_next();
    let failed_ids = TestIds::with_start(800);
    let failed = AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &failed_ids,
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
    assert_eq!(failed.code(), KernelErrorCode::SessionFailure);
    assert!(fake.writes().unwrap().is_empty());
    assert_eq!(
        store
            .load(session_id())
            .await
            .unwrap()
            .approval_artifacts()
            .len(),
        1
    );

    let retry_ids = TestIds::with_start(900);
    AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &retry_ids,
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
    assert_eq!(
        fake.writes().unwrap(),
        [("/notes.txt".to_owned(), "hello".to_owned())]
    );
}
