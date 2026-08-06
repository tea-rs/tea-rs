#![cfg(feature = "fixture-server")]
#![forbid(unsafe_code)]

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use tea_control::CancellationScope;
use tea_mcp::{
    McpErrorCode, McpLifecyclePolicy, McpLimits, McpManager, McpReconnectPolicy, McpRemoteToolName,
    McpServerConfig, McpServerId, McpServerLaunch, McpServerState, McpToolDeclaration,
    McpToolPolicy, McpTransportConfig,
};
use tea_protocol::{ProtocolMetadata, ProtocolTimestamp, ToolCallId, ToolIdempotency};
use tea_testkit::{ToolTerminalKind, collect_tool_execution};
use tea_tools::{
    ToolConcurrency, ToolEffect, ToolExecutionEvent, ToolExecutionSemantics, ToolInvocation,
    ToolName, ToolRegistry, ToolRetrySafety, ToolTimeout, ToolTrust,
};

const FIXTURE: &str = env!("CARGO_BIN_EXE_tea-mcp-fixture-server");
const CALL_ID: &str = "0195a0b1-5e45-75be-8284-0aa7aa000041";
static PATH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn list_changed_blocks_new_calls_without_interrupting_an_active_call() {
    let harness = Harness::start("stale-during-call", reconnect_policy(1, 10, 10)).await;
    let registry = registry(&harness.manager);
    let first_stream = registry
        .execute(invocation(), CancellationScope::new())
        .unwrap();
    let first = tokio::spawn(async move { collect_tool_execution(first_stream).await.unwrap() });
    wait_until(Duration::from_secs(2), || {
        marker_text(&harness.marker).contains("called")
    })
    .await;
    wait_for_state(&harness.manager, McpServerState::Stale).await;

    let second = collect_tool_execution(
        registry
            .execute(invocation(), CancellationScope::new())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(second.report().terminal_kind(), ToolTerminalKind::Failed);
    assert_eq!(
        mcp_code(second.events().last().unwrap()),
        McpErrorCode::StaleCatalog
    );
    assert_eq!(
        dispatch_phase(second.events().last().unwrap()),
        "before_dispatch"
    );
    assert_eq!(marker_text(&harness.marker).matches("called").count(), 1);

    fs::write(&harness.gate, b"open").unwrap();
    assert_eq!(
        first.await.unwrap().report().terminal_kind(),
        ToolTerminalKind::Finished
    );
    assert!(
        harness
            .manager
            .catalog()
            .binding(alias().as_str())
            .is_some()
    );
    harness.cleanup();
}

#[tokio::test]
async fn reconnect_requires_the_exact_frozen_snapshot() {
    let matching = Harness::start("stale-match", reconnect_policy(1, 10, 10)).await;
    wait_for_state(&matching.manager, McpServerState::Stale).await;
    let snapshot = matching
        .manager
        .server_snapshot(&server_id())
        .unwrap()
        .clone();
    let spec = matching
        .manager
        .catalog()
        .binding(alias().as_str())
        .unwrap()
        .spec()
        .clone();

    let health = matching
        .manager
        .reconnect(&server_id(), observed(2))
        .await
        .unwrap();
    assert_eq!(health.state(), McpServerState::Ready);
    assert_eq!(health.restart_count(), 1);
    assert_eq!(
        matching.manager.server_snapshot(&server_id()),
        Some(&snapshot)
    );
    assert_eq!(
        matching
            .manager
            .catalog()
            .binding(alias().as_str())
            .unwrap()
            .spec(),
        &spec
    );
    let execution = collect_tool_execution(
        registry(&matching.manager)
            .execute(invocation(), CancellationScope::new())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        execution.report().terminal_kind(),
        ToolTerminalKind::Finished
    );
    matching.cleanup();

    for scenario in ["identity-drift", "protocol-drift", "catalog-drift"] {
        let harness = Harness::start(scenario, reconnect_policy(1, 10, 10)).await;
        wait_for_state(&harness.manager, McpServerState::Stale).await;
        let frozen = harness
            .manager
            .server_snapshot(&server_id())
            .unwrap()
            .clone();

        let error = harness
            .manager
            .reconnect(&server_id(), observed(3))
            .await
            .unwrap_err();

        assert_eq!(error.code(), McpErrorCode::Descriptor, "{scenario}");
        assert_eq!(
            harness.manager.server_snapshot(&server_id()),
            Some(&frozen),
            "{scenario}"
        );
        let health = harness
            .manager
            .server_health(&server_id(), observed(4))
            .unwrap()
            .unwrap();
        assert_eq!(health.state(), McpServerState::Unhealthy, "{scenario}");
        assert_eq!(health.code(), Some(McpErrorCode::Descriptor), "{scenario}");
        harness.cleanup();
    }
}

#[tokio::test]
async fn reconnect_uses_capped_deterministic_backoff() {
    let harness = Harness::start("retry-match", reconnect_policy(4, 10, 20)).await;
    wait_for_state(&harness.manager, McpServerState::Stale).await;
    let started_at = Instant::now();

    let health = harness
        .manager
        .reconnect(&server_id(), observed(5))
        .await
        .unwrap();

    assert!(started_at.elapsed() >= Duration::from_millis(45));
    assert_eq!(health.state(), McpServerState::Ready);
    assert_eq!(health.restart_count(), 4);
    assert_eq!(marker_text(&harness.marker).matches("launch:").count(), 5);
    harness.cleanup();
}

#[tokio::test]
async fn one_caller_owns_reconnect_and_other_work_is_unavailable() {
    let harness = Harness::start("blocked-match", reconnect_policy(1, 10, 10)).await;
    wait_for_state(&harness.manager, McpServerState::Stale).await;
    let manager = Arc::clone(&harness.manager);
    let reconnect = tokio::spawn(async move { manager.reconnect(&server_id(), observed(6)).await });
    wait_until(Duration::from_secs(2), || {
        marker_text(&harness.marker).contains("launch:2")
    })
    .await;

    let second = harness
        .manager
        .reconnect(&server_id(), observed(7))
        .await
        .unwrap_err();
    assert_eq!(second.code(), McpErrorCode::Unavailable);
    let call = collect_tool_execution(
        registry(&harness.manager)
            .execute(invocation(), CancellationScope::new())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        mcp_code(call.events().last().unwrap()),
        McpErrorCode::Unavailable
    );
    assert_eq!(marker_text(&harness.marker).matches("launch:").count(), 2);

    fs::write(&harness.gate, b"open").unwrap();
    assert_eq!(
        reconnect.await.unwrap().unwrap().state(),
        McpServerState::Ready
    );
    assert_eq!(marker_text(&harness.marker).matches("launch:").count(), 2);
    harness.cleanup();
}

#[tokio::test]
async fn crash_after_dispatch_is_terminal_and_reconnect_never_replays_it() {
    let harness = Harness::start("crash-during-call", reconnect_policy(1, 10, 10)).await;
    let first = collect_tool_execution(
        registry(&harness.manager)
            .execute(invocation(), CancellationScope::new())
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(first.report().terminal_kind(), ToolTerminalKind::Failed);
    assert_eq!(
        mcp_code(first.events().last().unwrap()),
        McpErrorCode::ServerExit
    );
    assert_eq!(
        dispatch_phase(first.events().last().unwrap()),
        "after_dispatch"
    );
    assert_eq!(marker_text(&harness.marker).matches("called").count(), 1);
    wait_for_state(&harness.manager, McpServerState::Unhealthy).await;

    harness
        .manager
        .reconnect(&server_id(), observed(9))
        .await
        .unwrap();
    assert_eq!(marker_text(&harness.marker).matches("called").count(), 1);

    let second = collect_tool_execution(
        registry(&harness.manager)
            .execute(invocation(), CancellationScope::new())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(second.report().terminal_kind(), ToolTerminalKind::Finished);
    assert_eq!(marker_text(&harness.marker).matches("called").count(), 2);
    harness.manager.shutdown().await.unwrap();
    harness.cleanup();
}

#[tokio::test]
async fn awaited_shutdown_stops_new_work_and_drains_an_active_call() {
    let harness = Harness::start("shutdown-during-call", reconnect_policy(1, 10, 10)).await;
    let registry = registry(&harness.manager);
    let stream = registry
        .execute(invocation(), CancellationScope::new())
        .unwrap();
    let active = tokio::spawn(async move { collect_tool_execution(stream).await.unwrap() });
    wait_until(Duration::from_secs(2), || {
        marker_text(&harness.marker).contains("called")
    })
    .await;

    let report = harness.manager.shutdown().await.unwrap();
    assert_eq!(report.server_count(), 1);
    assert_eq!(report.client_shutdown_count(), 1);
    assert_eq!(report.failed_shutdown_count(), 0);
    assert_eq!(report.forced_termination_count(), 1);
    assert_eq!(report.undrained_server_count(), 0);
    assert_eq!(
        active.await.unwrap().report().terminal_kind(),
        ToolTerminalKind::Failed
    );
    assert_eq!(marker_text(&harness.marker).matches("called").count(), 1);
    assert_eq!(
        harness
            .manager
            .server_health(&server_id(), observed(10))
            .unwrap()
            .unwrap()
            .state(),
        McpServerState::Stopped
    );

    let rejected = collect_tool_execution(
        registry
            .execute(invocation(), CancellationScope::new())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        mcp_code(rejected.events().last().unwrap()),
        McpErrorCode::Unavailable
    );
    assert_eq!(marker_text(&harness.marker).matches("called").count(), 1);
    assert_eq!(
        harness.manager.shutdown().await.unwrap_err().code(),
        McpErrorCode::Unavailable
    );
    harness.cleanup();
}

#[tokio::test]
async fn shutdown_cancels_and_awaits_a_pending_reconnect_candidate() {
    let harness = Harness::start("blocked-match", reconnect_policy(1, 10, 10)).await;
    wait_for_state(&harness.manager, McpServerState::Stale).await;
    let reconnect_manager = Arc::clone(&harness.manager);
    let reconnect = tokio::spawn(async move {
        reconnect_manager
            .reconnect(&server_id(), observed(11))
            .await
    });
    wait_until(Duration::from_secs(2), || {
        marker_text(&harness.marker).contains("launch:2")
    })
    .await;

    let shutdown_manager = Arc::clone(&harness.manager);
    let started_at = Instant::now();
    let shutdown = tokio::spawn(async move { shutdown_manager.shutdown().await });

    assert_eq!(
        reconnect.await.unwrap().unwrap_err().code(),
        McpErrorCode::Cancellation
    );
    let report = shutdown.await.unwrap().unwrap();
    assert!(started_at.elapsed() < Duration::from_secs(1));
    assert_eq!(report.server_count(), 1);
    assert_eq!(report.failed_shutdown_count(), 0);
    assert_eq!(marker_text(&harness.marker).matches("launch:").count(), 2);
    assert_eq!(
        harness
            .manager
            .server_health(&server_id(), observed(12))
            .unwrap()
            .unwrap()
            .state(),
        McpServerState::Stopped
    );
    harness.cleanup();
}

struct Harness {
    manager: Arc<McpManager>,
    state: PathBuf,
    marker: PathBuf,
    gate: PathBuf,
}

impl Harness {
    async fn start(scenario: &str, reconnect: McpReconnectPolicy) -> Self {
        let state = unique_path(&format!("{scenario}-state"));
        let marker = unique_path(&format!("{scenario}-marker"));
        let gate = unique_path(&format!("{scenario}-gate"));
        let config = server(scenario, &state, &marker, &gate, reconnect);
        let launch = McpServerLaunch::new(
            config,
            ToolTrust::Workspace,
            Vec::<(OsString, OsString)>::new(),
        )
        .unwrap();
        let manager = McpManager::start([launch], [alias()], 1, observed(1))
            .await
            .unwrap();
        Self {
            manager: Arc::new(manager),
            state,
            marker,
            gate,
        }
    }

    fn cleanup(self) {
        drop(self.manager);
        cleanup([self.state, self.marker, self.gate]);
    }
}

fn registry(manager: &McpManager) -> ToolRegistry {
    let binding = manager.catalog().binding(alias().as_str()).unwrap();
    let executor = manager.tool_executor(&alias()).unwrap();
    let mut registry = ToolRegistry::new();
    registry
        .register(
            binding.spec().clone(),
            Arc::new(binding.clone()),
            Arc::new(executor),
        )
        .unwrap();
    registry
}

fn invocation() -> ToolInvocation {
    ToolInvocation::new(
        ToolCallId::from_str(CALL_ID).unwrap(),
        alias(),
        serde_json::json!({"value":"hello"}),
        ProtocolMetadata::default(),
    )
    .unwrap()
}

fn server(
    scenario: &str,
    state: &Path,
    marker: &Path,
    gate: &Path,
    reconnect: McpReconnectPolicy,
) -> McpServerConfig {
    let transport = McpTransportConfig::stdio(
        FIXTURE,
        [
            OsString::from("reconnect"),
            OsString::from(scenario),
            state.as_os_str().to_owned(),
            marker.as_os_str().to_owned(),
            gate.as_os_str().to_owned(),
        ],
    )
    .unwrap();
    let execution = ToolExecutionSemantics::new(
        ToolIdempotency::Idempotent,
        ToolRetrySafety::Automatic,
        ToolConcurrency::Parallel,
        ToolTimeout::from_millis(2_000).unwrap(),
    )
    .unwrap();
    let declaration = McpToolDeclaration::new([ToolEffect::FsRead], Vec::new(), execution).unwrap();
    McpServerConfig::new(
        server_id(),
        transport,
        Vec::new(),
        vec![McpToolPolicy::enabled(
            McpRemoteToolName::new("echo").unwrap(),
            declaration,
        )],
        McpLimits::default(),
        McpLifecyclePolicy::new(
            Duration::from_secs(2),
            Duration::from_secs(1),
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(100),
            Duration::from_secs(1),
        )
        .unwrap(),
        reconnect,
    )
    .unwrap()
}

fn reconnect_policy(attempts: u32, initial_ms: u64, maximum_ms: u64) -> McpReconnectPolicy {
    McpReconnectPolicy::bounded(
        attempts,
        Duration::from_millis(initial_ms),
        Duration::from_millis(maximum_ms),
    )
    .unwrap()
}

fn server_id() -> McpServerId {
    McpServerId::from_str("reconnect").unwrap()
}

fn alias() -> ToolName {
    ToolName::from_str("mcp.reconnect.echo").unwrap()
}

fn observed(second: u8) -> ProtocolTimestamp {
    ProtocolTimestamp::from_str(&format!("2026-07-25T12:00:{second:02}.000Z")).unwrap()
}

async fn wait_for_state(manager: &McpManager, expected: McpServerState) {
    wait_until(Duration::from_secs(2), || {
        manager
            .server_health(&server_id(), observed(8))
            .ok()
            .flatten()
            .is_some_and(|health| health.state() == expected)
    })
    .await;
}

fn mcp_code(event: &ToolExecutionEvent) -> McpErrorCode {
    match event {
        ToolExecutionEvent::Failed(failure) => {
            serde_json::from_value(failure.details()["dev.tea-rs.mcp"]["code"].clone()).unwrap()
        }
        _ => panic!("expected failed MCP event"),
    }
}

fn dispatch_phase(event: &ToolExecutionEvent) -> &str {
    match event {
        ToolExecutionEvent::Failed(failure) => failure.details()["dev.tea-rs.mcp"]["phase"]
            .as_str()
            .unwrap(),
        _ => panic!("expected failed MCP event"),
    }
}

fn marker_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

async fn wait_until(timeout: Duration, predicate: impl Fn() -> bool) {
    tokio::time::timeout(timeout, async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

fn unique_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "tea-mcp-reconnect-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ))
}

fn cleanup(paths: impl IntoIterator<Item = PathBuf>) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}
