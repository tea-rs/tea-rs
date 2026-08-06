#![cfg(feature = "fixture-server")]
#![forbid(unsafe_code)]

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use tea_mcp::{
    McpErrorCode, McpLifecyclePolicy, McpLimits, McpManager, McpProtocolVersion,
    McpReconnectPolicy, McpRemoteToolName, McpServerConfig, McpServerId, McpServerLaunch,
    McpServerState, McpToolDeclaration, McpToolPolicy, McpTransportConfig,
};
use tea_protocol::{ProtocolTimestamp, ToolIdempotency};
use tea_tools::{
    ToolConcurrency, ToolEffect, ToolExecutionSemantics, ToolName, ToolRetrySafety, ToolTimeout,
    ToolTrust,
};

const FIXTURE: &str = env!("CARGO_BIN_EXE_tea-mcp-fixture-server");
static PATH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn manager_bounds_concurrent_initialization() {
    let gate = unique_path("startup-gate");
    let markers = (0..3)
        .map(|index| unique_path(&format!("startup-{index}")))
        .collect::<Vec<_>>();
    let launches = markers
        .iter()
        .enumerate()
        .map(|(index, marker)| {
            launch(server(
                &format!("server-{index}"),
                "gated",
                marker,
                Some(&gate),
                McpLifecyclePolicy::default(),
            ))
        })
        .collect::<Vec<_>>();
    let active = (0..3).map(|index| alias(&format!("server-{index}")));

    let startup = tokio::spawn(McpManager::start(launches, active, 2, timestamp()));
    wait_until(Duration::from_secs(2), || {
        markers.iter().filter(|path| path.exists()).count() == 2
    })
    .await;
    assert_eq!(markers.iter().filter(|path| path.exists()).count(), 2);

    fs::write(&gate, b"open").unwrap();
    let manager = startup.await.unwrap().unwrap();
    assert_eq!(manager.health(timestamp()).unwrap().len(), 3);
    assert!(
        manager
            .health(timestamp())
            .unwrap()
            .iter()
            .all(|health| health.state() == McpServerState::Ready)
    );

    drop(manager);
    cleanup(markers.into_iter().chain([gate]));
}

#[tokio::test]
async fn inactive_failure_is_diagnostic_but_active_failure_aborts_bootstrap() {
    let ready_marker = unique_path("inactive-ready");
    let unused_gate = unique_path("inactive-unused-gate");
    let ready = server(
        "ready",
        "ready",
        &ready_marker,
        Some(&unused_gate),
        McpLifecyclePolicy::default(),
    );
    let failed = crashing_server("inactive-failed", McpLifecyclePolicy::default());
    let failed_id = failed.id().clone();
    let manager = McpManager::start(
        [launch(ready), launch(failed)],
        [alias("ready")],
        2,
        timestamp(),
    )
    .await
    .unwrap();

    let health = manager
        .server_health(&failed_id, timestamp())
        .unwrap()
        .unwrap();
    assert_eq!(health.state(), McpServerState::Unhealthy);
    assert_eq!(health.code(), Some(McpErrorCode::ServerExit));
    assert!(manager.server_snapshot(&failed_id).is_none());
    assert!(manager.catalog().binding(alias("ready").as_str()).is_some());
    drop(manager);

    let healthy_marker = unique_path("active-healthy");
    let failed_marker = unique_path("active-failed");
    let gate = unique_path("active-failure-gate");
    let healthy = server(
        "healthy",
        "ready",
        &healthy_marker,
        Some(&gate),
        McpLifecyclePolicy::default(),
    );
    let failed = server(
        "required",
        "fail-after-gate",
        &failed_marker,
        Some(&gate),
        McpLifecyclePolicy::default(),
    );
    let startup = tokio::spawn(McpManager::start(
        [launch(healthy), launch(failed)],
        [alias("healthy"), alias("required")],
        2,
        timestamp(),
    ));
    wait_until(Duration::from_secs(2), || {
        healthy_marker.exists() && failed_marker.exists()
    })
    .await;
    fs::write(&gate, b"open").unwrap();

    let error = startup.await.unwrap().unwrap_err();
    assert_eq!(error.code(), McpErrorCode::ServerExit);
    wait_until(Duration::from_secs(2), || {
        fs::read_to_string(&healthy_marker)
            .unwrap_or_default()
            .contains("stopped")
    })
    .await;

    cleanup([
        ready_marker,
        unused_gate,
        healthy_marker,
        failed_marker,
        gate,
    ]);
}

#[tokio::test]
async fn total_startup_deadline_includes_handshake_and_catalog_discovery() {
    let marker = unique_path("deadline");
    let gate = unique_path("deadline-gate");
    let lifecycle = McpLifecyclePolicy::new(
        Duration::from_millis(150),
        Duration::from_secs(2),
        Duration::from_millis(100),
        Duration::from_millis(100),
        Duration::from_millis(100),
        Duration::from_millis(100),
    )
    .unwrap();
    let started_at = Instant::now();

    let error = McpManager::start(
        [launch(server(
            "deadline",
            "gated",
            &marker,
            Some(&gate),
            lifecycle,
        ))],
        [alias("deadline")],
        1,
        timestamp(),
    )
    .await
    .unwrap_err();

    assert_eq!(error.code(), McpErrorCode::Timeout);
    assert!(started_at.elapsed() < Duration::from_secs(1));
    cleanup([marker, gate]);
}

#[tokio::test]
async fn snapshot_freezes_hashed_implementation_protocol_catalog_and_bindings() {
    let first_marker = unique_path("snapshot-v1");
    let second_marker = unique_path("snapshot-v2");
    let gate = unique_path("snapshot-unused-gate");
    let first = McpManager::start(
        [launch(server(
            "snapshot",
            "ready",
            &first_marker,
            Some(&gate),
            McpLifecyclePolicy::default(),
        ))],
        [alias("snapshot")],
        1,
        timestamp(),
    )
    .await
    .unwrap();
    let second = McpManager::start(
        [launch(server(
            "snapshot",
            "identity-v2",
            &second_marker,
            Some(&gate),
            McpLifecyclePolicy::default(),
        ))],
        [alias("snapshot")],
        1,
        timestamp(),
    )
    .await
    .unwrap();

    let server_id = McpServerId::from_str("snapshot").unwrap();
    let first_snapshot = first.server_snapshot(&server_id).unwrap();
    let second_snapshot = second.server_snapshot(&server_id).unwrap();
    assert_eq!(
        first_snapshot.protocol_version(),
        &McpProtocolVersion::from_str("2025-11-25").unwrap()
    );
    assert_ne!(
        first_snapshot.implementation_digest(),
        second_snapshot.implementation_digest()
    );
    assert_eq!(
        first_snapshot.catalog_digest(),
        second_snapshot.catalog_digest()
    );
    assert_eq!(
        first_snapshot.binding_digest(&alias("snapshot")),
        first
            .catalog()
            .binding(alias("snapshot").as_str())
            .map(|binding| binding.spec().source().descriptor_digest())
    );

    let encoded = serde_json::to_string(first_snapshot).unwrap();
    assert!(!encoded.contains("tea-mcp-fixture"));
    assert!(!encoded.contains("0.1.0"));
    assert_eq!(
        serde_json::from_str::<tea_mcp::McpServerSnapshot>(&encoded).unwrap(),
        *first_snapshot
    );
    let mut tampered = serde_json::to_value(first_snapshot).unwrap();
    tampered["catalogDigest"] = serde_json::json!("f".repeat(64));
    assert!(serde_json::from_value::<tea_mcp::McpServerSnapshot>(tampered).is_err());

    drop(first);
    drop(second);
    cleanup([first_marker, second_marker, gate]);
}

#[test]
fn launch_requires_exact_resolved_environment_names() {
    let marker = unique_path("environment");
    let gate = unique_path("environment-gate");
    let config = server_with_environment("environment", &marker, &gate);

    assert!(
        McpServerLaunch::new(
            config.clone(),
            ToolTrust::Workspace,
            [(OsString::from("OTHER"), OsString::from("secret"))],
        )
        .is_err()
    );
    let launch = McpServerLaunch::new(
        config,
        ToolTrust::Workspace,
        [(OsString::from("TOKEN"), OsString::from("secret"))],
    )
    .unwrap();
    let debug = format!("{launch:?}");
    assert!(!debug.contains("secret"));
    assert!(!debug.contains("TOKEN"));
    cleanup([marker, gate]);
}

#[test]
fn protocol_versions_are_bounded_exact_and_strictly_serialized() {
    let version = McpProtocolVersion::from_str("2025-11-25").unwrap();
    let encoded = serde_json::to_string(&version).unwrap();
    assert_eq!(encoded, r#""2025-11-25""#);
    assert_eq!(
        serde_json::from_str::<McpProtocolVersion>(&encoded).unwrap(),
        version
    );
    assert!(McpProtocolVersion::from_str("").is_err());
    assert!(McpProtocolVersion::from_str("2025-11-25\nsecret").is_err());
    assert!(McpProtocolVersion::from_str(&"v".repeat(65)).is_err());
}

fn launch(config: McpServerConfig) -> McpServerLaunch {
    McpServerLaunch::new(
        config,
        ToolTrust::Workspace,
        Vec::<(OsString, OsString)>::new(),
    )
    .unwrap()
}

fn server(
    id: &str,
    scenario: &str,
    marker: &Path,
    gate: Option<&Path>,
    lifecycle: McpLifecyclePolicy,
) -> McpServerConfig {
    let gate = gate.unwrap_or(marker);
    let transport = McpTransportConfig::stdio(
        FIXTURE,
        [
            OsString::from("manager"),
            OsString::from(scenario),
            marker.as_os_str().to_owned(),
            gate.as_os_str().to_owned(),
        ],
    )
    .unwrap();
    configured_server(id, transport, Vec::new(), lifecycle)
}

fn crashing_server(id: &str, lifecycle: McpLifecyclePolicy) -> McpServerConfig {
    let transport = McpTransportConfig::stdio(FIXTURE, [OsString::from("crash")]).unwrap();
    configured_server(id, transport, Vec::new(), lifecycle)
}

fn server_with_environment(id: &str, marker: &Path, gate: &Path) -> McpServerConfig {
    let transport = McpTransportConfig::stdio(
        FIXTURE,
        [
            OsString::from("manager"),
            OsString::from("ready"),
            marker.as_os_str().to_owned(),
            gate.as_os_str().to_owned(),
        ],
    )
    .unwrap();
    configured_server(
        id,
        transport,
        vec!["TOKEN".to_owned()],
        McpLifecyclePolicy::default(),
    )
}

fn configured_server(
    id: &str,
    transport: McpTransportConfig,
    environment: Vec<String>,
    lifecycle: McpLifecyclePolicy,
) -> McpServerConfig {
    let execution = ToolExecutionSemantics::new(
        ToolIdempotency::Idempotent,
        ToolRetrySafety::Automatic,
        ToolConcurrency::Parallel,
        ToolTimeout::from_millis(5_000).unwrap(),
    )
    .unwrap();
    let declaration = McpToolDeclaration::new([ToolEffect::FsRead], Vec::new(), execution).unwrap();
    McpServerConfig::new(
        McpServerId::from_str(id).unwrap(),
        transport,
        environment,
        vec![McpToolPolicy::enabled(
            McpRemoteToolName::new("echo").unwrap(),
            declaration,
        )],
        McpLimits::default(),
        lifecycle,
        McpReconnectPolicy::default(),
    )
    .unwrap()
}

fn alias(server_id: &str) -> ToolName {
    ToolName::from_str(&format!("mcp.{server_id}.echo")).unwrap()
}

fn timestamp() -> ProtocolTimestamp {
    ProtocolTimestamp::from_str("2026-07-25T12:00:00.000Z").unwrap()
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
        "tea-mcp-health-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ))
}

fn cleanup(paths: impl IntoIterator<Item = PathBuf>) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}
