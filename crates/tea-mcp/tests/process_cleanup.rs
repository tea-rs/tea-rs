#![cfg(all(feature = "fixture-server", unix))]
#![forbid(unsafe_code)]

use std::{
    ffi::OsString,
    fs,
    path::PathBuf,
    process::{Command, Stdio},
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
async fn awaited_shutdown_kills_the_owned_process_group_and_descendant() {
    let pid_path = unique_path("shutdown-child");
    let config = server(FIXTURE, &["spawn-child", pid_path.to_str().unwrap()]);
    let client = McpStdioClient::start(&config, empty_environment())
        .await
        .unwrap();
    let pid = wait_for_pid(&pid_path);

    let report = client.shutdown().await.unwrap();
    assert!(report.forced_termination());
    wait_until_dead(pid);
    let _ = fs::remove_file(pid_path);
}

#[tokio::test]
async fn drop_signals_cleanup_for_the_owned_process_group() {
    let pid_path = unique_path("drop-child");
    let config = server(FIXTURE, &["spawn-child", pid_path.to_str().unwrap()]);
    let client = McpStdioClient::start(&config, empty_environment())
        .await
        .unwrap();
    let pid = wait_for_pid(&pid_path);

    drop(client);
    wait_until_dead(pid);
    let _ = fs::remove_file(pid_path);
}

#[tokio::test]
async fn ignore_shutdown_is_escalated_within_the_owned_deadlines() {
    let config = server(FIXTURE, &["ignore-shutdown"]);
    let client = McpStdioClient::start(&config, empty_environment())
        .await
        .unwrap();
    let started = Instant::now();
    let report = client.shutdown().await.unwrap();

    assert!(report.forced_termination());
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn startup_failure_is_stable_and_never_invokes_a_shell() {
    let missing = unique_path("missing-executable");
    let config = server(missing.to_str().unwrap(), &[]);
    let error = McpStdioClient::start(&config, empty_environment())
        .await
        .unwrap_err();
    assert_eq!(error.code(), McpErrorCode::Startup);
}

fn server(executable: &str, arguments: &[&str]) -> McpServerConfig {
    let transport = McpTransportConfig::stdio(
        executable,
        arguments.iter().map(|value| OsString::from(*value)),
    )
    .unwrap();
    McpServerConfig::new(
        McpServerId::from_str("fixture-cleanup").unwrap(),
        transport,
        Vec::new(),
        Vec::new(),
        McpLimits::default(),
        McpLifecyclePolicy::new(
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_millis(250),
            Duration::from_millis(150),
            Duration::from_millis(150),
            Duration::from_secs(1),
        )
        .unwrap(),
        McpReconnectPolicy::default(),
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

fn wait_for_pid(path: &PathBuf) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(contents) = fs::read_to_string(path)
            && let Ok(pid) = contents.parse::<u32>()
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "fixture child PID was not written"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_until_dead(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while process_exists(pid) {
        assert!(
            Instant::now() < deadline,
            "descendant {pid} survived cleanup"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn process_exists(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}
