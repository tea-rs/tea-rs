#![cfg(feature = "fixture-server")]
#![forbid(unsafe_code)]

use std::{
    ffi::OsString,
    str::FromStr,
    time::{Duration, Instant},
};

use tea_mcp::{
    McpErrorCode, McpLifecyclePolicy, McpLimits, McpReconnectPolicy, McpServerConfig, McpServerId,
    McpStdioClient, McpTransportConfig,
};

const FIXTURE: &str = env!("CARGO_BIN_EXE_tea-mcp-fixture-server");

#[tokio::test]
async fn invalid_utf8_frame_fails_closed_within_the_handshake_deadline() {
    let lifecycle = McpLifecyclePolicy::new(
        Duration::from_secs(1),
        Duration::from_millis(200),
        Duration::from_millis(100),
        Duration::from_millis(100),
        Duration::from_millis(100),
        Duration::from_secs(1),
    )
    .unwrap();
    let transport = McpTransportConfig::stdio(
        FIXTURE,
        [OsString::from("malformed"), OsString::from("invalid-utf8")],
    )
    .unwrap();
    let config = McpServerConfig::new(
        McpServerId::from_str("invalid-utf8").unwrap(),
        transport,
        Vec::new(),
        Vec::new(),
        McpLimits::default(),
        lifecycle,
        McpReconnectPolicy::default(),
    )
    .unwrap();

    let started = Instant::now();
    let error = McpStdioClient::start(&config, std::iter::empty::<(OsString, OsString)>())
        .await
        .unwrap_err();
    assert!(matches!(
        error.code(),
        McpErrorCode::Timeout | McpErrorCode::Transport
    ));
    assert!(started.elapsed() < Duration::from_secs(1));
}
