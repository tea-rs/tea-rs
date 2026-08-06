use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser as _;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use tea_cli::args::CliArgs;
use tea_cli::tui::{FrameSink, InputEvent, MemoryClipboard, TuiState, run_with_channels};
use tea_cli::{BootstrapEnvironment, CliBootstrap};
use tea_coding::CodingAgentService;
use tea_model::{
    ModelCapabilities, ModelCompletion, ModelDisplayName, ModelEvent, ModelResponseInfo, ModelSpec,
    ModelStreamIndex, ProviderId, ProviderToolCallId, ToolCallCompleted, ToolCallStarted,
    Utf8Delta,
};
use tea_protocol::{ModelId, StopReason, TokenCount, Usage};
use tea_session::SessionSnapshot;
use tea_testkit::{ScriptedModelProvider, ScriptedModelResponse};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

const PROMPT: &str = "read the fixture";
const FINAL_TEXT: &str = "Cross-mode final";

#[derive(Debug, PartialEq)]
struct DurableSemantics {
    records: Value,
    transcript: Value,
    policy_decisions: Vec<Value>,
    usage: Value,
    active_branch: Value,
}

#[derive(Default)]
struct Frames {
    count: usize,
    usage_ready: Arc<AtomicBool>,
}

impl FrameSink for Frames {
    fn render(&mut self, state: &TuiState, _cursor_byte: usize) -> io::Result<()> {
        self.count += 1;
        if state.usage().is_some() {
            self.usage_ready.store(true, Ordering::Release);
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
struct SharedOutput(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn model() -> ModelSpec {
    ModelSpec::new(
        ModelId::from_str("fake/model").unwrap(),
        ProviderId::from_str("fake").unwrap(),
        ModelDisplayName::from_str("Cross-mode Fake").unwrap(),
        TokenCount::new(32_000).unwrap(),
        TokenCount::new(4_000).unwrap(),
        ModelCapabilities::text()
            .with_tools(true)
            .with_usage_reporting(),
    )
    .unwrap()
}

fn provider() -> Arc<ScriptedModelProvider> {
    let index = ModelStreamIndex::new(0).unwrap();
    let provider_call = ProviderToolCallId::from_str("read-cross-mode").unwrap();
    Arc::new(ScriptedModelProvider::new(
        ProviderId::from_str("fake").unwrap(),
        vec![model()],
        [
            ScriptedModelResponse::events([
                ModelEvent::Started(ModelResponseInfo::new()),
                ModelEvent::ToolCallStarted(
                    ToolCallStarted::new(index, provider_call.clone(), "read").unwrap(),
                ),
                ModelEvent::ToolCallCompleted(
                    ToolCallCompleted::new(
                        index,
                        provider_call,
                        "read",
                        serde_json::json!({"path":"fixture.txt"}),
                    )
                    .unwrap(),
                ),
                ModelEvent::Completed(ModelCompletion::new(StopReason::ToolUse).unwrap()),
            ]),
            ScriptedModelResponse::events([
                ModelEvent::Started(ModelResponseInfo::new()),
                ModelEvent::TextDelta(Utf8Delta::new(FINAL_TEXT).unwrap()),
                ModelEvent::Completed(
                    ModelCompletion::new(StopReason::Completed)
                        .unwrap()
                        .with_usage(Usage::new(
                            TokenCount::new(11).unwrap(),
                            TokenCount::new(7).unwrap(),
                        )),
                ),
            ]),
        ],
    ))
}

fn expected_usage() -> Value {
    serde_json::to_value(Usage::new(
        TokenCount::new(11).unwrap(),
        TokenCount::new(7).unwrap(),
    ))
    .unwrap()
}

fn args(base: &std::path::Path, mode: &str) -> CliArgs {
    let workspace = base.join("workspace");
    let mode_root = base.join(mode);
    fs::create_dir_all(&mode_root).unwrap();
    let mut values = vec!["tea".to_owned()];
    if mode != "interactive" {
        values.push(format!("--{mode}"));
    }
    values.extend([
        "--new".to_owned(),
        "--provider".to_owned(),
        "fake".to_owned(),
        "--model".to_owned(),
        "fake/model".to_owned(),
        "--tools".to_owned(),
        "read".to_owned(),
        "--trust".to_owned(),
        "ignore".to_owned(),
        "--cwd".to_owned(),
        workspace.display().to_string(),
        "--config-dir".to_owned(),
        mode_root.join("config").display().to_string(),
        "--state-dir".to_owned(),
        mode_root.join("state").display().to_string(),
        "--data-dir".to_owned(),
        mode_root.join("data").display().to_string(),
    ]);
    if matches!(mode, "print" | "json") {
        values.push(PROMPT.to_owned());
    }
    CliArgs::try_parse_from(values).unwrap()
}

fn bootstrap(base: &std::path::Path, model_provider: Arc<ScriptedModelProvider>) -> CliBootstrap {
    let workspace = base.join("workspace");
    CliBootstrap::new(BootstrapEnvironment::new(
        &workspace,
        Some(workspace.clone()),
        BTreeMap::new(),
    ))
    .with_provider(model_provider)
}

async fn snapshot(service: &CodingAgentService) -> SessionSnapshot {
    let sessions = service.list_sessions().await.unwrap();
    assert_eq!(sessions.len(), 1);
    service
        .session_snapshot(sessions[0].session_id())
        .await
        .unwrap()
}

async fn run_interactive(base: &std::path::Path) -> DurableSemantics {
    let args = args(base, "interactive");
    let model_provider = provider();
    let bootstrap = bootstrap(base, Arc::clone(&model_provider));
    let (service, selection) = bootstrap.build(&args).unwrap();
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    let mut frames = Frames::default();
    let usage_ready = Arc::clone(&frames.usage_ready);
    let producer = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !usage_ready.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("interactive projection did not observe final usage");
        sender
            .send(InputEvent::Key(KeyEvent::new(
                KeyCode::Char('d'),
                KeyModifiers::CONTROL,
            )))
            .await
            .unwrap();
    });
    let mut clipboard = MemoryClipboard::default();
    let state = Box::pin(run_with_channels(
        &service,
        selection,
        receiver,
        &mut frames,
        &mut clipboard,
        Some(PROMPT.to_owned()),
    ))
    .await
    .unwrap();
    producer.await.unwrap();
    assert!(
        state
            .messages()
            .iter()
            .any(|message| { serde_json::to_string(message).unwrap().contains(FINAL_TEXT) })
    );
    assert!(frames.count > 1);
    let usage = serde_json::to_value(state.usage().unwrap()).unwrap();
    let result = normalize_snapshot(&snapshot(&service).await, usage);
    service.shutdown().await;
    result
}

async fn run_print(base: &std::path::Path) -> DurableSemantics {
    let args = args(base, "print");
    let bootstrap = bootstrap(base, provider());
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    Box::pin(tea_cli::modes::print::run(
        &args,
        &bootstrap,
        &mut io::empty(),
        true,
        &mut output,
        &mut diagnostics,
    ))
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        format!("{FINAL_TEXT}\n")
    );
    assert!(diagnostics.is_empty());
    let (service, _) = bootstrap.build(&args).unwrap();
    let result = normalize_snapshot(&snapshot(&service).await, expected_usage());
    service.shutdown().await;
    result
}

async fn run_json(base: &std::path::Path) -> DurableSemantics {
    let args = args(base, "json");
    let bootstrap = bootstrap(base, provider());
    let output = SharedOutput::default();
    Box::pin(tea_cli::modes::json::run(
        &args,
        &bootstrap,
        &mut io::empty(),
        true,
        Box::new(output.clone()),
    ))
    .await
    .unwrap();
    let usage = {
        let bytes = output.0.lock().unwrap();
        let lines = String::from_utf8_lossy(&bytes);
        assert!(lines.contains("run_finished"));
        lines
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find_map(|line| {
                let mut usage = Vec::new();
                collect_named(&line, "usage", &mut usage);
                usage.into_iter().next()
            })
            .unwrap()
    };
    let (service, _) = bootstrap.build(&args).unwrap();
    let result = normalize_snapshot(&snapshot(&service).await, usage);
    service.shutdown().await;
    result
}

async fn run_rpc(base: &std::path::Path) -> DurableSemantics {
    let args = args(base, "rpc");
    let bootstrap = bootstrap(base, provider());
    let (service, selection) = bootstrap.build(&args).unwrap();
    let (mut client_input, server_input) = tokio::io::duplex(64 * 1024);
    let (server_output, client_output) = tokio::io::duplex(64 * 1024);
    let server = tea_cli::rpc::run_service(&service, selection, server_input, server_output);
    let client = async {
        let mut output = BufReader::new(client_output);
        let mut line = String::new();
        output.read_line(&mut line).await.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap()["type"],
            "ready"
        );
        client_input
            .write_all(
                format!(
                    "{{\"rpcVersion\":\"1.0\",\"id\":\"prompt\",\"type\":\"prompt\",\"payload\":{{\"text\":{}}}}}\n",
                    serde_json::to_string(PROMPT).unwrap()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        loop {
            line.clear();
            assert!(output.read_line(&mut line).await.unwrap() > 0);
            let value = serde_json::from_str::<Value>(&line).unwrap();
            if value["type"] == "command_finished" {
                break;
            }
        }
        client_input.shutdown().await.unwrap();
        loop {
            line.clear();
            if output.read_line(&mut line).await.unwrap() == 0 {
                break;
            }
            serde_json::from_str::<Value>(&line).unwrap();
        }
        expected_usage()
    };
    let (server_result, usage) = tokio::join!(server, client);
    server_result.unwrap();
    let result = normalize_snapshot(&snapshot(&service).await, usage);
    service.shutdown().await;
    result
}

fn normalize_snapshot(snapshot: &SessionSnapshot, usage: Value) -> DurableSemantics {
    let mut ids = BTreeMap::new();
    let mut records = serde_json::to_value(snapshot.records()).unwrap();
    normalize_value(&mut records, &mut ids);
    let mut transcript = serde_json::to_value(snapshot.state().messages()).unwrap();
    normalize_value(&mut transcript, &mut ids);
    let mut active_branch = serde_json::to_value(snapshot.state().active_branch_id()).unwrap();
    normalize_value(&mut active_branch, &mut ids);
    let mut policy_decisions = Vec::new();
    collect_named(&records, "decision", &mut policy_decisions);
    assert!(
        !policy_decisions.is_empty(),
        "read policy decision was not durable"
    );
    assert_eq!(usage, expected_usage());
    DurableSemantics {
        records,
        transcript,
        policy_decisions,
        usage,
        active_branch,
    }
}

fn normalize_value(value: &mut Value, ids: &mut BTreeMap<String, String>) {
    match value {
        Value::Array(values) => {
            for value in values {
                normalize_value(value, ids);
            }
        }
        Value::Object(object) => {
            for (key, value) in object {
                if key == "timestamp" || key.ends_with("At") {
                    *value = Value::String("<timestamp>".to_owned());
                } else {
                    normalize_value(value, ids);
                }
            }
        }
        Value::String(text) if uuid::Uuid::parse_str(text).is_ok() => {
            let next = format!("<uuid:{}>", ids.len());
            let original = text.clone();
            text.clone_from(ids.entry(original).or_insert(next));
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn collect_named(value: &Value, key: &str, output: &mut Vec<Value>) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_named(value, key, output);
            }
        }
        Value::Object(object) => {
            if let Some(value) = object.get(key) {
                output.push(value.clone());
            }
            for value in object.values() {
                collect_named(value, key, output);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

#[tokio::test(flavor = "current_thread")]
async fn scripted_read_has_equivalent_durable_semantics_in_every_mode() {
    let base = std::env::temp_dir().join(format!(
        "tea-cli-cross-mode-{}",
        uuid::Uuid::now_v7().hyphenated()
    ));
    fs::create_dir_all(base.join("workspace")).unwrap();
    fs::write(base.join("workspace/fixture.txt"), "fixture contents\n").unwrap();

    let interactive = Box::pin(run_interactive(&base)).await;
    let print = Box::pin(run_print(&base)).await;
    let json = Box::pin(run_json(&base)).await;
    let rpc = Box::pin(run_rpc(&base)).await;
    assert_eq!(interactive, print, "interactive and print diverged");
    assert_eq!(interactive, json, "interactive and JSON diverged");
    assert_eq!(interactive, rpc, "interactive and RPC diverged");

    fs::remove_dir_all(base).unwrap();
}
