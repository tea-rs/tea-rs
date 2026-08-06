#![allow(dead_code)]

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use tea_control::CancellationScope;
use tea_mcp::{
    McpLifecyclePolicy, McpLimits, McpReconnectPolicy, McpRemoteToolName, McpServerConfig,
    McpServerId, McpStdioClient, McpToolDeclaration, McpToolExecutor, McpToolPolicy,
    McpTransportConfig,
};
use tea_protocol::{ProtocolMetadata, ToolCallId, ToolIdempotency};
use tea_tools::{
    ToolConcurrency, ToolEffect, ToolExecutionSemantics, ToolInvocation, ToolName, ToolRegistry,
    ToolRetrySafety, ToolTimeout, ToolTrust,
};

const FIXTURE: &str = env!("CARGO_BIN_EXE_tea-mcp-fixture-server");
const CALL_ID: &str = "0195a0b1-5e45-75be-8284-0aa7aa000031";
pub const ALIAS: &str = "mcp.fixture.echo";
static PATH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct Harness {
    pub client: McpStdioClient,
    pub registry: ToolRegistry,
    pub marker: PathBuf,
}

impl Harness {
    pub async fn start(scenario: &str, limits: McpLimits, timeout_ms: u64) -> Self {
        let marker = unique_path(scenario);
        let config = server(scenario, &marker, limits, timeout_ms);
        let client = McpStdioClient::start(&config, empty_environment())
            .await
            .unwrap();
        let catalog = client
            .discover_catalog(&config, ToolTrust::Workspace)
            .await
            .unwrap();
        let binding = catalog.binding(ALIAS).unwrap();
        let executor = McpToolExecutor::new(&client, binding).unwrap();
        let mut registry = ToolRegistry::new();
        registry
            .register(
                binding.spec().clone(),
                Arc::new(binding.clone()),
                Arc::new(executor),
            )
            .unwrap();
        Self {
            client,
            registry,
            marker,
        }
    }

    pub fn invocation(arguments: Value) -> ToolInvocation {
        ToolInvocation::new(
            ToolCallId::from_str(CALL_ID).unwrap(),
            ToolName::from_str(ALIAS).unwrap(),
            arguments,
            ProtocolMetadata::default(),
        )
        .unwrap()
    }

    pub fn execute(&self, cancellation: CancellationScope) -> tea_tools::BoxToolExecutionStream {
        self.registry
            .execute(Self::invocation(json!({"value":"hello"})), cancellation)
            .unwrap()
    }

    pub fn marker_text(&self) -> String {
        fs::read_to_string(&self.marker).unwrap_or_default()
    }

    pub async fn shutdown(self) {
        self.client.shutdown().await.unwrap();
        let _ = fs::remove_file(self.marker);
    }
}

fn server(scenario: &str, marker: &Path, limits: McpLimits, timeout_ms: u64) -> McpServerConfig {
    let transport = McpTransportConfig::stdio(
        FIXTURE,
        [
            OsString::from("execute"),
            OsString::from(scenario),
            marker.as_os_str().to_owned(),
        ],
    )
    .unwrap();
    let execution = ToolExecutionSemantics::new(
        ToolIdempotency::Idempotent,
        ToolRetrySafety::Automatic,
        ToolConcurrency::Parallel,
        ToolTimeout::from_millis(timeout_ms).unwrap(),
    )
    .unwrap();
    let declaration = McpToolDeclaration::new([ToolEffect::FsRead], Vec::new(), execution).unwrap();
    McpServerConfig::new(
        McpServerId::from_str("fixture").unwrap(),
        transport,
        Vec::new(),
        vec![McpToolPolicy::enabled(
            McpRemoteToolName::new("echo").unwrap(),
            declaration,
        )],
        limits,
        McpLifecyclePolicy::new(
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_millis(250),
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap(),
        McpReconnectPolicy::default(),
    )
    .unwrap()
}

fn unique_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "tea-mcp-{label}-{}-{nonce}-{sequence}.marker",
        std::process::id()
    ))
}

fn empty_environment() -> Vec<(OsString, OsString)> {
    Vec::new()
}
