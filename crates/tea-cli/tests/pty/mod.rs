use std::collections::BTreeMap;
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::str::FromStr as _;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use clap::Parser as _;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tea_cli::args::CliArgs;
use tea_cli::tui::{CrosstermDriver, TerminalGuard, TerminalOptions, TerminalTitle, ViewportMode};
use tea_cli::{BootstrapEnvironment, CliBootstrap};
use tea_model::{
    ModelCapabilities, ModelCompletion, ModelDisplayName, ModelEvent, ModelResponseInfo, ModelSpec,
    ModelStreamIndex, ProviderId, ProviderToolCallId, ToolCallCompleted, ToolCallStarted,
};
use tea_protocol::{ModelId, StopReason, TokenCount};
use tea_provider_openai::env_file::load_env_file;
use tea_testkit::{ScriptStep, ScriptedModelProvider, ScriptedModelResponse};

mod cleanup;
mod interactive;
mod resize;

const CHILD_ENV: &str = "TEA_PTY_CHILD_SCENARIO";
const CHILD_TEST: &str = "pty::pty::pty_child_process";
const DEFAULT_SIZE: PtySize = PtySize {
    rows: 30,
    cols: 100,
    pixel_width: 0,
    pixel_height: 0,
};
const IO_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) struct PtyHarness {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn std::io::Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: Receiver<Vec<u8>>,
    parser: vt100::Parser,
    raw: Vec<u8>,
    query_scan: usize,
    query_responses: usize,
}

impl PtyHarness {
    pub(super) fn spawn(scenario: &str) -> Self {
        Self::spawn_with_term(scenario, "xterm-256color")
    }

    pub(super) fn spawn_with_term(scenario: &str, term: &str) -> Self {
        Self::spawn_with_environment(scenario, term, BTreeMap::new())
    }

    pub(super) fn spawn_live(scenario: &str) -> Self {
        let dotenv = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env");
        let environment = load_env_file(&dotenv).expect("live PTY test requires a valid .env");
        Self::spawn_with_environment(scenario, "xterm-256color", environment)
    }

    fn spawn_with_environment(
        scenario: &str,
        term: &str,
        environment: BTreeMap<String, String>,
    ) -> Self {
        let pair = native_pty_system().openpty(DEFAULT_SIZE).unwrap();
        let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
        command.args(["--exact", CHILD_TEST, "--nocapture"]);
        command.env(CHILD_ENV, scenario);
        command.env("RUST_BACKTRACE", "0");
        command.env("TERM", term);
        for (key, value) in environment {
            command.env(key, value);
        }
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let (sender, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(length) => {
                        if sender.send(buffer[..length].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            master: pair.master,
            writer,
            child,
            output,
            parser: vt100::Parser::new(DEFAULT_SIZE.rows, DEFAULT_SIZE.cols, 256),
            raw: Vec::new(),
            query_scan: 0,
            query_responses: 0,
        }
    }

    pub(super) fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }

    pub(super) fn paste(&mut self, text: &str) {
        self.send(b"\x1b[200~");
        self.send(text.as_bytes());
        self.send(b"\x1b[201~");
    }

    pub(super) fn resize(&mut self, rows: u16, cols: u16) {
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        self.master.resize(size).unwrap();
        self.parser.screen_mut().set_size(rows, cols);
    }

    pub(super) fn wait_for_raw(&mut self, needle: &[u8]) {
        self.wait_until(|harness| contains(&harness.raw, needle));
    }

    pub(super) fn wait_for_raw_occurrences(&mut self, needle: &[u8], count: usize) {
        self.wait_until(|harness| occurrences(&harness.raw, needle) >= count);
    }

    pub(super) fn wait_for_screen(&mut self, needle: &str) {
        self.wait_until(|harness| harness.screen().contains(needle));
    }

    pub(super) fn wait_for_screen_timeout(&mut self, needle: &str, timeout: Duration) {
        self.wait_until_timeout(timeout, |harness| harness.screen().contains(needle));
    }

    fn wait_until(&mut self, predicate: impl Fn(&Self) -> bool) {
        self.wait_until_timeout(IO_TIMEOUT, predicate);
    }

    fn wait_until_timeout(&mut self, timeout: Duration, predicate: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.read_once(Duration::from_millis(25));
            if predicate(self) {
                return;
            }
            if self.child.try_wait().unwrap().is_some() {
                break;
            }
        }
        panic!(
            "PTY condition timed out; screen={:?}; raw_tail={:?}",
            self.screen(),
            String::from_utf8_lossy(&self.raw[self.raw.len().saturating_sub(512)..])
        );
    }

    pub(super) fn wait_for_exit(&mut self) -> portable_pty::ExitStatus {
        let deadline = Instant::now() + IO_TIMEOUT;
        loop {
            self.read_once(Duration::from_millis(25));
            if let Some(status) = self.child.try_wait().unwrap() {
                self.drain_output();
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "PTY child did not exit: {:?}",
                self.screen()
            );
        }
    }

    pub(super) fn screen(&self) -> String {
        self.parser.screen().contents()
    }

    pub(super) fn raw(&self) -> &[u8] {
        &self.raw
    }

    pub(super) fn frame_count(&self) -> usize {
        occurrences(&self.raw, b"\x1b[?2026h")
    }

    pub(super) fn wait_for_frames(&mut self, count: usize) {
        self.wait_until(|harness| harness.frame_count() >= count);
    }

    pub(super) fn raw_occurrences(&self, needle: &[u8]) -> usize {
        occurrences(&self.raw, needle)
    }

    pub(super) fn cursor_query_count(&self) -> usize {
        occurrences(&self.raw, b"\x1b[6n")
    }

    pub(super) fn wait_for_cursor_queries(&mut self, count: usize) {
        self.wait_until(|harness| harness.cursor_query_count() >= count);
    }

    pub(super) const fn query_responses(&self) -> usize {
        self.query_responses
    }

    pub(super) fn settle(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            self.read_once(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    pub(super) fn interrupt(&self) {
        use nix::sys::signal::{Signal, killpg};
        use nix::unistd::Pid;

        let process_group = self.master.process_group_leader().unwrap();
        killpg(Pid::from_raw(process_group), Signal::SIGINT).unwrap();
    }

    fn read_once(&mut self, timeout: Duration) {
        match self.output.recv_timeout(timeout) {
            Ok(chunk) => self.ingest(&chunk),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
        }
    }

    fn drain_output(&mut self) {
        while let Ok(chunk) = self.output.try_recv() {
            self.ingest(&chunk);
        }
    }

    fn ingest(&mut self, chunk: &[u8]) {
        self.parser.process(chunk);
        self.raw.extend_from_slice(chunk);
        let start = self.query_scan.saturating_sub(8);
        let pending = &self.raw[start..];
        let cursor_queries = occurrences(pending, b"\x1b[6n");
        let device_queries = occurrences(pending, b"\x1b[c");
        for _ in 0..cursor_queries {
            self.writer.write_all(b"\x1b[1;1R").unwrap();
            self.query_responses += 1;
        }
        for _ in 0..device_queries {
            self.writer.write_all(b"\x1b[?1;2c").unwrap();
            self.query_responses += 1;
        }
        if cursor_queries + device_queries > 0 {
            self.writer.flush().unwrap();
        }
        self.query_scan = self.raw.len();
    }
}

impl Drop for PtyHarness {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn model() -> ModelSpec {
    ModelSpec::new(
        ModelId::from_str("fake/model").unwrap(),
        ProviderId::from_str("fake").unwrap(),
        ModelDisplayName::from_str("PTY Fake").unwrap(),
        TokenCount::new(32_000).unwrap(),
        TokenCount::new(4_000).unwrap(),
        ModelCapabilities::text()
            .with_image_input()
            .with_tools(true),
    )
    .unwrap()
}

fn tui_args(root: &std::path::Path, selection: &str) -> CliArgs {
    tui_args_with_trust(root, selection, "ignore")
}

fn tui_args_with_trust(root: &std::path::Path, selection: &str, trust: &str) -> CliArgs {
    CliArgs::try_parse_from([
        "tea",
        selection,
        "--provider",
        "fake",
        "--model",
        "fake/model",
        "--trust",
        trust,
        "--cwd",
        root.to_str().unwrap(),
        "--config-dir",
        root.join("config").to_str().unwrap(),
        "--state-dir",
        root.join("state").to_str().unwrap(),
        "--data-dir",
        root.join("data").to_str().unwrap(),
    ])
    .unwrap()
}

fn bootstrap(
    root: &std::path::Path,
    responses: impl IntoIterator<Item = ScriptedModelResponse>,
) -> CliBootstrap {
    CliBootstrap::new(BootstrapEnvironment::new(
        root,
        Some(root.to_path_buf()),
        BTreeMap::new(),
    ))
    .with_provider(Arc::new(ScriptedModelProvider::new(
        ProviderId::from_str("fake").unwrap(),
        vec![model()],
        responses,
    )))
}

fn prepare_root(viewport: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "tea-cli-pty-child-{}",
        uuid::Uuid::now_v7().hyphenated()
    ));
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(
        root.join("config/settings.json"),
        format!(r#"{{"schemaVersion":1,"tui":{{"viewport":"{viewport}"}}}}"#),
    )
    .unwrap();
    root
}

async fn run_tui(viewport: &str, reopen: bool) {
    let root = prepare_root(viewport);
    let bootstrap = bootstrap(
        &root,
        [ScriptedModelResponse::text([
            "| Name | Status | Extra | More |\n|---|---|---|---|\n| Tea | Ready | Stable | Safe |\n\n",
            "PTY final: **你好** uses `e\u{301}` and *styled text* with [docs](https://e.test)\n",
            "```rust\nfn main() { println!(\"wide response\"); }\n```",
        ])],
    );
    Box::pin(tea_cli::tui::run(&tui_args(&root, "--new"), &bootstrap))
        .await
        .unwrap();
    if reopen {
        println!("TEA_PTY_REOPEN");
        Box::pin(tea_cli::tui::run(
            &tui_args(&root, "--continue"),
            &bootstrap,
        ))
        .await
        .unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

async fn run_image_tui() {
    let root = prepare_root("inline");
    fs::write(
        root.join("private-source.png"),
        b"\x89PNG\r\n\x1a\nprivate-image",
    )
    .unwrap();
    let bootstrap = bootstrap(&root, [ScriptedModelResponse::text(["PTY image response"])]);
    Box::pin(tea_cli::tui::run(&tui_args(&root, "--new"), &bootstrap))
        .await
        .unwrap();
    println!("TEA_PTY_IMAGE_REOPEN");
    Box::pin(tea_cli::tui::run(
        &tui_args(&root, "--continue"),
        &bootstrap,
    ))
    .await
    .unwrap();
    fs::remove_dir_all(root).unwrap();
}

fn read_tool_response() -> ScriptedModelResponse {
    let index = ModelStreamIndex::new(0).unwrap();
    let provider_call_id = ProviderToolCallId::from_str("pty-read-readme").unwrap();
    ScriptedModelResponse::events([
        ModelEvent::Started(ModelResponseInfo::new()),
        ModelEvent::ToolCallStarted(
            ToolCallStarted::new(index, provider_call_id.clone(), "read").unwrap(),
        ),
        ModelEvent::ToolCallCompleted(
            ToolCallCompleted::new(
                index,
                provider_call_id,
                "read",
                serde_json::json!({"path":"README.md"}),
            )
            .unwrap(),
        ),
        ModelEvent::Completed(ModelCompletion::new(StopReason::ToolUse).unwrap()),
    ])
}

async fn run_read_tui() {
    let root = prepare_root("inline");
    fs::write(
        root.join("README.md"),
        "# Tea PTY fixture\n\nA deterministic README used by the TUI test.\n",
    )
    .unwrap();
    let bootstrap = bootstrap(
        &root,
        [
            read_tool_response(),
            ScriptedModelResponse::text(["README summary complete"]),
        ],
    );
    Box::pin(tea_cli::tui::run(&tui_args(&root, "--new"), &bootstrap))
        .await
        .unwrap();
    fs::remove_dir_all(root).unwrap();
}

async fn run_slow_tui() {
    let root = prepare_root("inline");
    let bootstrap = bootstrap(
        &root,
        [ScriptedModelResponse::new([
            ScriptStep::event(ModelEvent::Started(ModelResponseInfo::new())),
            ScriptStep::AwaitCancellation,
        ])],
    );
    Box::pin(tea_cli::tui::run(&tui_args(&root, "--new"), &bootstrap))
        .await
        .unwrap();
    fs::remove_dir_all(root).unwrap();
}

async fn run_trust_tui(reopen: bool) {
    let root = prepare_root("inline");
    fs::write(root.join("AGENTS.md"), "# Trusted PTY fixture\n").unwrap();
    let bootstrap = bootstrap(&root, []);
    let args = tui_args_with_trust(&root, "--new", "default");
    Box::pin(tea_cli::tui::run(&args, &bootstrap))
        .await
        .unwrap();
    if reopen {
        println!("TEA_PTY_TRUST_REOPEN");
        Box::pin(tea_cli::tui::run(&args, &bootstrap))
            .await
            .unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

async fn run_rejected_trust_tui() {
    let root = prepare_root("inline");
    fs::write(root.join("AGENTS.md"), "# Untrusted PTY fixture\n").unwrap();
    let bootstrap = bootstrap(&root, []);
    let args = tui_args_with_trust(&root, "--new", "default");
    let error = Box::pin(tea_cli::tui::run(&args, &bootstrap))
        .await
        .unwrap_err();
    assert_eq!(error.category(), tea_cli::ExitCategory::TrustOrConfig);
    assert_eq!(
        error.message(),
        "project-local configuration is not trusted"
    );
    assert!(!root.join("state/project-trust.json").exists());
    println!("TEA_PTY_TRUST_REJECTED");
    fs::remove_dir_all(root).unwrap();
}

async fn run_live_read_tui() {
    let root = prepare_root("inline");
    fs::write(
        root.join("README.md"),
        "# Tea live PTY fixture\n\nThis file verifies the live read-tool event sequence.\n",
    )
    .unwrap();
    let model = std::env::var("TEA_OPENAI_MODEL").expect("TEA_OPENAI_MODEL must be configured");
    let args = CliArgs::try_parse_from([
        "tea",
        "--new",
        "--provider",
        "openai",
        "--model",
        &model,
        "--tools",
        "read",
        "--trust",
        "ignore",
        "--cwd",
        root.to_str().unwrap(),
        "--config-dir",
        root.join("config").to_str().unwrap(),
        "--state-dir",
        root.join("state").to_str().unwrap(),
        "--data-dir",
        root.join("data").to_str().unwrap(),
    ])
    .unwrap();
    let environment = BootstrapEnvironment::from_process().unwrap();
    let bootstrap = CliBootstrap::new(environment);
    Box::pin(tea_cli::tui::run(&args, &bootstrap))
        .await
        .unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pty_child_process() {
    let Ok(scenario) = std::env::var(CHILD_ENV) else {
        return;
    };
    match scenario.as_str() {
        "fullscreen" => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run_tui("fullscreen", false)),
        "inline" => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run_tui("inline", false)),
        "inline-stale" => {
            print!("STALE_TERMINAL_CONTENT");
            std::io::stdout().flush().unwrap();
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run_tui("inline", false));
        }
        "inline-quit" => {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run_tui("inline", false));
            print!("TEA_SHELL_PROMPT");
            std::io::stdout().flush().unwrap();
            std::thread::sleep(Duration::from_millis(250));
        }
        "inline-read" => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run_read_tui()),
        "inline-slow" => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run_slow_tui()),
        "trust-reopen" => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run_trust_tui(true)),
        "trust-reject" => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run_rejected_trust_tui()),
        "inline-live-read" => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run_live_read_tui()),
        "reopen" => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run_tui("inline", true)),
        "image-reopen" => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run_image_tui()),
        "panic" => {
            let _guard = TerminalGuard::enter(
                CrosstermDriver::new(std::io::stdout()),
                TerminalOptions {
                    title: Some(TerminalTitle::default()),
                    ..TerminalOptions::default()
                },
            )
            .unwrap();
            panic!("intentional PTY cleanup panic");
        }
        "handoff" => {
            let guard = TerminalGuard::enter(
                CrosstermDriver::new(std::io::stdout()),
                TerminalOptions {
                    title: Some(TerminalTitle::default()),
                    viewport: ViewportMode::Fullscreen,
                    ..TerminalOptions::default()
                },
            )
            .unwrap();
            guard
                .handoff(Duration::from_secs(1), || {
                    println!("HANDOFF_CHILD");
                    Ok(())
                })
                .unwrap();
        }
        other => panic!("unknown PTY child scenario: {other}"),
    }
}
