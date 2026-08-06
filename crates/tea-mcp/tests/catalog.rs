#![cfg(feature = "fixture-server")]
#![forbid(unsafe_code)]

use std::{ffi::OsString, str::FromStr, time::Duration};

use tea_mcp::{
    McpErrorCode, McpLifecyclePolicy, McpLimits, McpReconnectPolicy, McpRemoteToolName,
    McpServerConfig, McpServerId, McpStdioClient, McpToolDeclaration, McpToolPolicy,
    McpTransportConfig,
};
use tea_protocol::ToolIdempotency;
use tea_tools::{
    ToolConcurrency, ToolEffect, ToolExecutionSemantics, ToolRetrySafety, ToolTimeout, ToolTrust,
};

const FIXTURE: &str = env!("CARGO_BIN_EXE_tea-mcp-fixture-server");

#[tokio::test]
async fn empty_tools_list_freezes_an_empty_catalog() {
    let config = server("empty", Vec::new(), McpLimits::default());
    let client = McpStdioClient::start(&config, empty_environment())
        .await
        .unwrap();

    let catalog = client
        .discover_catalog(&config, ToolTrust::User)
        .await
        .unwrap();

    assert!(catalog.is_empty());
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn paginated_discovery_is_filtered_and_sorted_by_local_alias() {
    let tools = vec![
        enabled("zeta"),
        enabled("alpha"),
        McpToolPolicy::new(McpRemoteToolName::new("disabled").unwrap()),
    ];
    let config = server("paginated", tools, McpLimits::default());
    let client = McpStdioClient::start(&config, empty_environment())
        .await
        .unwrap();

    let catalog = client
        .discover_catalog(&config, ToolTrust::Workspace)
        .await
        .unwrap();
    let names = catalog
        .specs()
        .map(|spec| spec.name().as_str())
        .collect::<Vec<_>>();

    assert_eq!(names, ["mcp.fixture.alpha", "mcp.fixture.zeta"]);
    assert!(catalog.binding("mcp.fixture.disabled").is_none());
    assert!(catalog.binding("mcp.fixture.undeclared").is_none());
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn duplicate_tools_and_pagination_loops_fail_closed() {
    for scenario in ["duplicate", "loop"] {
        let config = server(scenario, vec![enabled("duplicate")], McpLimits::default());
        let client = McpStdioClient::start(&config, empty_environment())
            .await
            .unwrap();

        let error = client
            .discover_catalog(&config, ToolTrust::User)
            .await
            .unwrap_err();

        assert_eq!(error.code(), McpErrorCode::Descriptor, "{scenario}");
        client.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn discovery_enforces_configured_tool_and_descriptor_bounds() {
    let limits = McpLimits::default()
        .with_max_tools(1)
        .unwrap()
        .with_max_descriptor_bytes(128)
        .unwrap();
    let config = server("paginated", vec![enabled("alpha")], limits);
    let client = McpStdioClient::start(&config, empty_environment())
        .await
        .unwrap();

    let error = client
        .discover_catalog(&config, ToolTrust::User)
        .await
        .unwrap_err();

    assert_eq!(error.code(), McpErrorCode::OutputBound);
    client.shutdown().await.unwrap();
}

fn server(scenario: &str, tools: Vec<McpToolPolicy>, limits: McpLimits) -> McpServerConfig {
    let transport = McpTransportConfig::stdio(
        FIXTURE,
        [OsString::from("catalog"), OsString::from(scenario)],
    )
    .unwrap();
    McpServerConfig::new(
        McpServerId::from_str("fixture").unwrap(),
        transport,
        Vec::new(),
        tools,
        limits,
        McpLifecyclePolicy::new(
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap(),
        McpReconnectPolicy::default(),
    )
    .unwrap()
}

fn enabled(name: &str) -> McpToolPolicy {
    let execution = ToolExecutionSemantics::new(
        ToolIdempotency::Idempotent,
        ToolRetrySafety::Automatic,
        ToolConcurrency::Parallel,
        ToolTimeout::from_millis(5_000).unwrap(),
    )
    .unwrap();
    let declaration = McpToolDeclaration::new([ToolEffect::FsRead], Vec::new(), execution).unwrap();
    McpToolPolicy::enabled(McpRemoteToolName::new(name).unwrap(), declaration)
}

fn empty_environment() -> Vec<(OsString, OsString)> {
    Vec::new()
}
