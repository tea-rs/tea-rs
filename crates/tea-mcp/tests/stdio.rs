#![cfg(feature = "fixture-server")]
#![forbid(unsafe_code)]

use std::{
    ffi::OsString,
    fs,
    path::PathBuf,
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use tea_mcp::{
    McpErrorCode, McpLifecyclePolicy, McpLimits, McpReconnectPolicy, McpServerConfig, McpServerId,
    McpStdioClient, McpTransportConfig,
};

const FIXTURE: &str = env!("CARGO_BIN_EXE_tea-mcp-fixture-server");

#[tokio::test]
async fn normal_stdio_is_empty_environment_bounded_and_gracefully_owned() {
    let state_path = unique_path("normal-state");
    let limits = McpLimits::default().with_max_stderr_bytes(1_024).unwrap();
    let config = server(
        &["normal", state_path.to_str().unwrap(), "131072"],
        limits,
        lifecycle(Duration::from_secs(2)),
    );

    let client = McpStdioClient::start(&config, empty_environment())
        .await
        .unwrap();
    client.probe(Duration::from_secs(1)).await.unwrap();
    let state = wait_for_file(&state_path);
    assert!(state.contains("initialized=true"));
    assert!(state.contains("env_count=0"));

    let report = client.shutdown().await.unwrap();
    assert_eq!(report.retained_stderr_bytes(), 1_024);
    assert_eq!(report.dropped_stderr_bytes(), 131_072 - 1_024);
    assert!(!report.forced_termination());
    let _ = fs::remove_file(state_path);
}

#[tokio::test]
async fn malformed_oversized_incomplete_and_partial_frames_fail_stably() {
    let cases = [
        (&["malformed"][..], McpErrorCode::Transport),
        (&["malformed", "oversized"][..], McpErrorCode::OutputBound),
        (&["malformed", "incomplete"][..], McpErrorCode::Transport),
        (&["slow", "partial"][..], McpErrorCode::Timeout),
        (&["crash"][..], McpErrorCode::ServerExit),
    ];

    for (arguments, expected) in cases {
        let limits = McpLimits::default().with_max_frame_bytes(512).unwrap();
        let config = server(arguments, limits, lifecycle(Duration::from_millis(150)));
        let error = McpStdioClient::start(&config, empty_environment())
            .await
            .unwrap_err();
        assert_eq!(error.code(), expected, "arguments: {arguments:?}");
    }
}

#[tokio::test]
async fn unknown_duplicate_and_notification_floods_close_the_transport() {
    for arguments in [
        &["malformed", "unknown"][..],
        &["malformed", "duplicate"][..],
        &["flood"][..],
    ] {
        let limits = McpLimits::default().with_max_notifications(8).unwrap();
        let config = server(arguments, limits, lifecycle(Duration::from_secs(2)));
        let client = McpStdioClient::start(&config, empty_environment())
            .await
            .unwrap();
        let first = client.probe(Duration::from_millis(500)).await;
        let result = if first.is_ok() {
            client.probe(Duration::from_millis(500)).await
        } else {
            first
        };
        assert_eq!(result.unwrap_err().code(), McpErrorCode::Transport);
        let _ = client.shutdown().await;
    }
}

#[tokio::test]
async fn request_deadline_sends_cancellation_before_owned_shutdown() {
    let config = server(
        &["ignore-cancel"],
        McpLimits::default(),
        lifecycle(Duration::from_secs(2)),
    );
    let client = McpStdioClient::start(&config, empty_environment())
        .await
        .unwrap();

    let error = client.probe(Duration::from_millis(100)).await.unwrap_err();
    assert_eq!(error.code(), McpErrorCode::Timeout);
    client.shutdown().await.unwrap();
}

fn server(arguments: &[&str], limits: McpLimits, lifecycle: McpLifecyclePolicy) -> McpServerConfig {
    let transport = McpTransportConfig::stdio(
        FIXTURE,
        arguments.iter().map(|value| OsString::from(*value)),
    )
    .unwrap();
    McpServerConfig::new(
        McpServerId::from_str("fixture").unwrap(),
        transport,
        Vec::new(),
        Vec::new(),
        limits,
        lifecycle,
        McpReconnectPolicy::default(),
    )
    .unwrap()
}

fn lifecycle(handshake_timeout: Duration) -> McpLifecyclePolicy {
    McpLifecyclePolicy::new(
        Duration::from_secs(1),
        handshake_timeout,
        Duration::from_millis(250),
        Duration::from_millis(250),
        Duration::from_millis(150),
        Duration::from_secs(1),
    )
    .unwrap()
}

fn empty_environment() -> impl Iterator<Item = (OsString, OsString)> {
    std::iter::empty()
}

fn unique_path(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "tea-mcp-{label}-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn wait_for_file(path: &PathBuf) -> String {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(contents) = fs::read_to_string(path) {
            return contents;
        }
        assert!(Instant::now() < deadline, "fixture state was not written");
        thread::sleep(Duration::from_millis(10));
    }
}
