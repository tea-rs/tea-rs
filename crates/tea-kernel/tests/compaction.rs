//! Manual session compaction replaces the compacted prefix with a summary.

use crate::common;

use std::str::FromStr;

use tea_control::CancellationScope;
use tea_kernel::{AgentKernel, KernelRunConfig};
use tea_policy::{
    ActorId, CodingWorkspacePolicy, ExecutionSurface, PolicyEngine, PolicyEnvironment,
    PolicyExecutionTarget,
};
use tea_protocol::{
    CanonicalMessage, ContentBlock, MessageId, ProtocolMetadata, SessionRecord, StopReason,
};
use tea_session::SessionStore;
use tea_testkit::ScriptedModelResponse;
use tea_tools::ToolRegistry;

use common::{EventCollector, FixedClock, TestIds, provider, session_id, store, timestamp};

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

fn summary(text: &str) -> CanonicalMessage {
    CanonicalMessage::assistant(
        MessageId::from_str("0195a0b1-5e90-7000-8000-000000000091").unwrap(),
        vec![ContentBlock::text(text).unwrap()],
        StopReason::Completed,
        timestamp(),
    )
    .unwrap()
}

#[tokio::test]
async fn manual_compact_replaces_prefix_with_summary() {
    let provider = provider([ScriptedModelResponse::text(["first turn"])]); // run that completes
    let store = store().await;
    let tools = ToolRegistry::new();
    let mut policy = PolicyEngine::new();
    policy.add_rule(CodingWorkspacePolicy).unwrap();
    let events = EventCollector::default();
    let ids = TestIds::default();

    // Run a turn so the session has assistant + user transcript records.
    let _ = AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &ids,
        &events,
    )
    .run(session_id(), &config(), CancellationScope::new())
    .await
    .unwrap();

    let before = store.load(session_id()).await.unwrap();
    assert!(before.state().messages().len() >= 2);
    let tail_record_id = before.state().tail_record_id();

    let after = AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &ids,
        &events,
    )
    .compact(session_id(), summary("compacted summary"), tail_record_id)
    .await
    .unwrap();

    // Materialized transcript begins with the summary; original records remain.
    assert_eq!(after.state().messages().len(), 1);
    assert_eq!(after.state().messages()[0], summary("compacted summary"),);
    assert_eq!(
        after
            .state()
            .latest_compaction()
            .unwrap()
            .compacted_through_record_id(),
        tail_record_id,
    );
    // Original records are preserved in the durable log for audit.
    assert!(after.records().len() > before.records().len());
    assert!(matches!(
        after.records().last().unwrap().record(),
        SessionRecord::SessionCompacted { .. }
    ));
}

#[tokio::test]
async fn manual_compact_rejects_non_tail_or_missing_record() {
    let provider = provider([]);
    let store = store().await;
    let tools = ToolRegistry::new();
    let mut policy = PolicyEngine::new();
    policy.add_rule(CodingWorkspacePolicy).unwrap();
    let events = EventCollector::default();
    let ids = TestIds::default();
    let kernel = AgentKernel::new(
        &provider,
        &tools,
        &policy,
        &store,
        &FixedClock,
        &ids,
        &events,
    );

    // A record id that does not exist on the active branch.
    let missing = "0195a0b1-5e90-7000-8000-000000000099".parse().unwrap();
    let result = kernel.compact(session_id(), summary("x"), missing).await;
    assert!(result.is_err(), "compacting a missing record must fail");
}
