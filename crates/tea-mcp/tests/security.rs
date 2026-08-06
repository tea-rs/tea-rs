#![cfg(all(feature = "fixture-server", unix))]
#![forbid(unsafe_code)]

use std::{
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt as _,
    path::PathBuf,
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use tea_mcp::{
    McpErrorCode, McpLifecyclePolicy, McpLimits, McpManager, McpReconnectPolicy, McpRemoteToolName,
    McpServerConfig, McpServerId, McpServerLaunch, McpServerState, McpToolDeclaration,
    McpToolPolicy, McpTransportConfig,
};
use tea_protocol::{ProtocolTimestamp, ToolIdempotency};
use tea_tools::{
    ToolConcurrency, ToolEffect, ToolExecutionSemantics, ToolName, ToolRetrySafety, ToolTimeout,
    ToolTrust,
};

const FIXTURE: &str = env!("CARGO_BIN_EXE_tea-mcp-fixture-server");
static PATH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn prepared_launch_rejects_a_replaced_executable_before_spawn() {
    let executable = unique_path("prepared-executable");
    let replacement = unique_path("replacement-executable");
    copy_executable(FIXTURE, &executable);

    let launch = McpServerLaunch::new(
        server(&executable),
        ToolTrust::Workspace,
        std::iter::empty::<(OsString, OsString)>(),
    )
    .unwrap();
    copy_executable(FIXTURE, &replacement);
    fs::rename(&replacement, &executable).unwrap();

    let error = McpManager::start([launch], [alias()], 1, timestamp())
        .await
        .unwrap_err();
    assert_eq!(error.code(), McpErrorCode::Identity);

    let _ = fs::remove_file(executable);
}

#[tokio::test]
async fn inactive_missing_executable_remains_a_safe_health_diagnostic() {
    let missing = unique_path("missing-executable");
    let launch = McpServerLaunch::new(
        server(&missing),
        ToolTrust::Workspace,
        std::iter::empty::<(OsString, OsString)>(),
    )
    .unwrap();

    let manager = McpManager::start([launch], std::iter::empty::<ToolName>(), 1, timestamp())
        .await
        .unwrap();
    let health = manager.health(timestamp()).unwrap();
    assert_eq!(health.len(), 1);
    assert_eq!(health[0].state(), McpServerState::Unhealthy);
    assert_eq!(health[0].code(), Some(McpErrorCode::Startup));
    manager.shutdown().await.unwrap();
}

fn copy_executable(source: &str, destination: &PathBuf) {
    fs::copy(source, destination).unwrap();
    fs::set_permissions(destination, fs::Permissions::from_mode(0o700)).unwrap();
}

fn server(executable: &PathBuf) -> McpServerConfig {
    let transport = McpTransportConfig::stdio(
        executable,
        [
            OsString::from("manager"),
            OsString::from("ready"),
            OsString::from("unused-marker"),
            OsString::from("unused-gate"),
        ],
    )
    .unwrap();
    let execution = ToolExecutionSemantics::new(
        ToolIdempotency::Idempotent,
        ToolRetrySafety::Automatic,
        ToolConcurrency::Parallel,
        ToolTimeout::from_millis(1_000).unwrap(),
    )
    .unwrap();
    let declaration = McpToolDeclaration::new([ToolEffect::FsRead], [], execution).unwrap();
    McpServerConfig::new(
        McpServerId::from_str("fixture").unwrap(),
        transport,
        Vec::new(),
        vec![McpToolPolicy::enabled(
            McpRemoteToolName::new("echo").unwrap(),
            declaration,
        )],
        McpLimits::default(),
        McpLifecyclePolicy::default(),
        McpReconnectPolicy::default(),
    )
    .unwrap()
}

fn alias() -> ToolName {
    ToolName::from_str("mcp.fixture.echo").unwrap()
}

fn timestamp() -> ProtocolTimestamp {
    ProtocolTimestamp::from_str("2026-07-25T12:00:00.000Z").unwrap()
}

fn unique_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "tea-mcp-security-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ))
}
