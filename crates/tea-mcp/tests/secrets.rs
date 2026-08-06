#![cfg(feature = "fixture-server")]
#![forbid(unsafe_code)]

use std::{ffi::OsString, str::FromStr, time::Duration};

use tea_mcp::{
    McpLifecyclePolicy, McpLimits, McpManager, McpReconnectPolicy, McpServerConfig, McpServerId,
    McpServerLaunch, McpStdioClient, McpTransportConfig,
};
use tea_protocol::ProtocolTimestamp;
use tea_tools::ToolTrust;

const FIXTURE: &str = env!("CARGO_BIN_EXE_tea-mcp-fixture-server");
const SECRET: &str = "tea-mcp-stderr-and-environment-sentinel";

#[tokio::test]
async fn child_credentials_and_stderr_never_enter_host_projections() {
    let config = server();
    assert_secret_absent("server config", &format!("{config:?}"));

    let client = McpStdioClient::start(
        &config,
        [(OsString::from("MCP_SECRET"), OsString::from(SECRET))],
    )
    .await
    .unwrap();
    client.probe(Duration::from_millis(250)).await.unwrap();
    assert_secret_absent("stdio client", &format!("{client:?}"));
    let report = client.shutdown().await.unwrap();
    assert_secret_absent("stdio shutdown", &format!("{report:?}"));

    let launch = McpServerLaunch::new(
        config,
        ToolTrust::Workspace,
        [(OsString::from("MCP_SECRET"), OsString::from(SECRET))],
    )
    .unwrap();
    assert_secret_absent("manager launch", &format!("{launch:?}"));
    let manager = McpManager::start(
        [launch],
        [],
        1,
        ProtocolTimestamp::from_str("2026-07-25T12:00:00.000Z").unwrap(),
    )
    .await
    .unwrap();
    assert_secret_absent("manager", &format!("{manager:?}"));
    assert_secret_absent(
        "health",
        &serde_json::to_string(&manager.health(timestamp()).unwrap()).unwrap(),
    );
    let shutdown = manager.shutdown().await.unwrap();
    assert_secret_absent("manager shutdown", &format!("{shutdown:?}"));
}

fn server() -> McpServerConfig {
    let transport = McpTransportConfig::stdio(
        FIXTURE,
        [OsString::from("secret-stderr"), OsString::from(SECRET)],
    )
    .unwrap();
    McpServerConfig::new(
        McpServerId::from_str("secret-fixture").unwrap(),
        transport,
        vec!["MCP_SECRET".to_owned()],
        Vec::new(),
        McpLimits::default(),
        McpLifecyclePolicy::default(),
        McpReconnectPolicy::default(),
    )
    .unwrap()
}

fn timestamp() -> ProtocolTimestamp {
    ProtocolTimestamp::from_str("2026-07-25T12:00:00.000Z").unwrap()
}

fn assert_secret_absent(label: &str, value: &str) {
    assert!(!value.contains(SECRET), "secret leaked through {label}");
}
