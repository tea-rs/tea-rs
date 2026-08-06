//! Fault-injection: storage failures surface as typed kernel errors.

use crate::common;

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};

use tea_control::CancellationScope;
use tea_kernel::{AgentKernel, KernelErrorCode, KernelRunConfig};
use tea_policy::{
    ActorId, CodingWorkspacePolicy, ExecutionSurface, PolicyEngine, PolicyEnvironment,
    PolicyExecutionTarget,
};
use tea_protocol::{ProtocolMetadata, SessionId};
use tea_session::{
    AppendOutcome, AppendTransaction, InMemorySessionStore, SessionSnapshot, SessionStore,
    SessionStoreError, SessionStoreErrorCode, SessionStoreFuture,
};
use tea_testkit::ScriptedModelResponse;
use tea_tools::ToolRegistry;

use common::{EventCollector, FixedClock, TestIds, provider, session_id, store};

#[derive(Debug)]
struct FailingStore {
    inner: InMemorySessionStore,
    fail_append: AtomicBool,
}

impl FailingStore {
    fn new(inner: InMemorySessionStore) -> Self {
        Self {
            inner,
            fail_append: AtomicBool::new(false),
        }
    }
}

impl SessionStore for FailingStore {
    fn load(&self, session_id: SessionId) -> SessionStoreFuture<'_, SessionSnapshot> {
        Box::pin(async move { self.inner.load(session_id).await })
    }
    fn append(&self, transaction: AppendTransaction) -> SessionStoreFuture<'_, AppendOutcome> {
        Box::pin(async move {
            if self.fail_append.load(Ordering::SeqCst) {
                return Err(SessionStoreError::new(
                    SessionStoreErrorCode::StorageUnavailable,
                    "injected storage failure",
                ));
            }
            self.inner.append(transaction).await
        })
    }
    fn active_grants_for_actor(
        &self,
        actor_id: tea_policy::ActorId,
    ) -> SessionStoreFuture<'_, Vec<tea_policy::PolicyGrant>> {
        Box::pin(async move { self.inner.active_grants_for_actor(actor_id).await })
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

#[tokio::test]
async fn storage_failure_during_run_surfaces_as_session_failure() {
    let provider = provider([ScriptedModelResponse::text(["done"])]);
    // Pre-populate the session through the shared common store helper.
    let failing = FailingStore::new(store().await);
    failing.fail_append.store(true, Ordering::SeqCst);

    let tools = ToolRegistry::new();
    let mut policy = PolicyEngine::new();
    policy.add_rule(CodingWorkspacePolicy).unwrap();
    let events = EventCollector::default();

    let error = AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &failing,
        &FixedClock,
        &TestIds::default(),
        &events,
    )
    .run(session_id(), &config(), CancellationScope::new())
    .await
    .unwrap_err();
    assert_eq!(error.code(), KernelErrorCode::SessionFailure);
}

#[allow(dead_code)]
fn _silence(_: ProtocolMetadata, _: SessionStoreError) {}
