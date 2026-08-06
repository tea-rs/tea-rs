use std::{ffi::OsString, path::PathBuf, str::FromStr, time::Duration};

use tea_mcp::{
    MAX_MCP_ARGUMENT_BYTES, MAX_MCP_ARGUMENT_TOTAL_BYTES, MAX_MCP_ARGUMENTS,
    MAX_MCP_DESCRIPTOR_BYTES, MAX_MCP_ENVIRONMENT_NAME_BYTES, MAX_MCP_ENVIRONMENT_TOTAL_BYTES,
    MAX_MCP_ENVIRONMENT_VARIABLES, MAX_MCP_EXECUTABLE_BYTES, MAX_MCP_FRAME_BYTES,
    MAX_MCP_IN_FLIGHT_REQUESTS, MAX_MCP_LIFECYCLE_TIMEOUT, MAX_MCP_NOTIFICATIONS,
    MAX_MCP_PROGRESS_EVENTS, MAX_MCP_RECONNECT_ATTEMPTS, MAX_MCP_RECONNECT_BACKOFF,
    MAX_MCP_RESULT_BYTES, MAX_MCP_STDERR_BYTES, MAX_MCP_TOOLS_PER_SERVER, McpArgumentResource,
    McpErrorCode, McpLifecyclePolicy, McpLimits, McpReconnectPolicy, McpRemoteToolName,
    McpServerConfig, McpServerId, McpToolDeclaration, McpToolPolicy, McpTransportConfig,
};
use tea_protocol::ToolIdempotency;
use tea_tools::{
    ToolConcurrency, ToolEffect, ToolExecutionSemantics, ToolName, ToolResourceAccess,
    ToolRetrySafety, ToolTimeout,
};

fn absolute_executable(name: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!(r"C:\{name}.exe"))
    } else {
        PathBuf::from(format!("/usr/bin/{name}"))
    }
}

fn transport() -> McpTransportConfig {
    McpTransportConfig::stdio(
        absolute_executable("tea-mcp-fixture"),
        Vec::<OsString>::new(),
    )
    .expect("valid transport")
}

fn server_config(
    inherited_environment: Vec<String>,
    tools: Vec<McpToolPolicy>,
) -> Result<McpServerConfig, tea_mcp::McpError> {
    McpServerConfig::new(
        McpServerId::from_str("fixture").expect("server id"),
        transport(),
        inherited_environment,
        tools,
        McpLimits::default(),
        McpLifecyclePolicy::default(),
        McpReconnectPolicy::default(),
    )
}

#[test]
fn server_ids_are_canonical_bounded_and_strictly_serialized() {
    let id = McpServerId::from_str("workspace.tools-1").expect("valid id");
    assert_eq!(id.as_str(), "workspace.tools-1");
    assert_eq!(
        serde_json::to_string(&id).expect("encode"),
        r#""workspace.tools-1""#
    );
    assert_eq!(
        serde_json::from_str::<McpServerId>(r#""workspace.tools-1""#).expect("decode"),
        id
    );

    for invalid in ["", "UPPER", "starts:wrong", "has space", "line\nbreak"] {
        assert!(
            McpServerId::from_str(invalid).is_err(),
            "accepted {invalid:?}"
        );
    }
    assert!(McpServerId::from_str(&"a".repeat(65)).is_err());
}

#[test]
fn duplicate_remote_tools_and_local_aliases_fail_closed() {
    let first = McpToolPolicy::new(McpRemoteToolName::new("read").expect("remote"));
    let duplicate = first.clone();
    assert_eq!(
        server_config(Vec::new(), vec![first, duplicate])
            .expect_err("duplicate remote tool")
            .code(),
        McpErrorCode::Configuration
    );

    let alias = ToolName::from_str("shared_alias").expect("alias");
    let first = McpToolPolicy::new(McpRemoteToolName::new("first").expect("remote"))
        .with_alias(alias.clone());
    let second =
        McpToolPolicy::new(McpRemoteToolName::new("second").expect("remote")).with_alias(alias);
    assert_eq!(
        server_config(Vec::new(), vec![first, second])
            .expect_err("duplicate alias")
            .code(),
        McpErrorCode::Configuration
    );
}

#[test]
fn stdio_requires_an_absolute_bounded_executable_and_argv() {
    assert_eq!(
        McpTransportConfig::stdio("relative/server", Vec::<OsString>::new())
            .expect_err("relative executable")
            .code(),
        McpErrorCode::Configuration
    );

    let too_many = (0..=MAX_MCP_ARGUMENTS)
        .map(|index| OsString::from(index.to_string()))
        .collect::<Vec<_>>();
    assert!(McpTransportConfig::stdio(absolute_executable("server"), too_many).is_err());
    assert!(
        McpTransportConfig::stdio(
            absolute_executable("server"),
            vec![OsString::from("x".repeat(MAX_MCP_ARGUMENT_BYTES + 1))],
        )
        .is_err()
    );

    let oversized_executable = if cfg!(windows) {
        PathBuf::from(format!(r"C:\{}", "x".repeat(MAX_MCP_EXECUTABLE_BYTES)))
    } else {
        PathBuf::from(format!("/{}", "x".repeat(MAX_MCP_EXECUTABLE_BYTES)))
    };
    assert!(McpTransportConfig::stdio(oversized_executable, Vec::<OsString>::new()).is_err());

    let aggregate_overflow = vec![
        OsString::from("x".repeat(MAX_MCP_ARGUMENT_BYTES));
        MAX_MCP_ARGUMENT_TOTAL_BYTES / MAX_MCP_ARGUMENT_BYTES + 1
    ];
    assert!(McpTransportConfig::stdio(absolute_executable("server"), aggregate_overflow).is_err());
}

#[test]
fn inherited_environment_names_are_bounded_ascii_and_unique() {
    let too_many = (0..=MAX_MCP_ENVIRONMENT_VARIABLES)
        .map(|index| format!("TEA_MCP_{index}"))
        .collect();
    assert!(server_config(too_many, Vec::new()).is_err());
    assert!(
        server_config(
            vec![format!("A{}", "B".repeat(MAX_MCP_ENVIRONMENT_NAME_BYTES))],
            Vec::new(),
        )
        .is_err()
    );

    for names in [
        vec!["TOKEN=value".to_owned()],
        vec!["NON_ASCII_\u{00e9}".to_owned()],
        vec!["PATH".to_owned(), "path".to_owned()],
    ] {
        assert!(server_config(names, Vec::new()).is_err());
    }

    let aggregate_overflow = (0..=MAX_MCP_ENVIRONMENT_TOTAL_BYTES / MAX_MCP_ENVIRONMENT_NAME_BYTES)
        .map(|index| {
            let prefix = format!("V{index}_");
            format!(
                "{prefix}{}",
                "X".repeat(MAX_MCP_ENVIRONMENT_NAME_BYTES - prefix.len())
            )
        })
        .collect();
    assert!(server_config(aggregate_overflow, Vec::new()).is_err());
}

#[test]
fn every_numeric_limit_is_nonzero_and_hard_capped() {
    assert!(McpLimits::default().with_max_frame_bytes(0).is_err());
    assert!(
        McpLimits::default()
            .with_max_frame_bytes(MAX_MCP_FRAME_BYTES + 1)
            .is_err()
    );
    assert!(McpLimits::default().with_max_in_flight_requests(0).is_err());
    assert!(
        McpLimits::default()
            .with_max_in_flight_requests(MAX_MCP_IN_FLIGHT_REQUESTS + 1)
            .is_err()
    );

    for invalid in [0, MAX_MCP_DESCRIPTOR_BYTES + 1] {
        assert!(
            McpLimits::default()
                .with_max_descriptor_bytes(invalid)
                .is_err()
        );
    }
    for invalid in [0, MAX_MCP_RESULT_BYTES + 1] {
        assert!(McpLimits::default().with_max_result_bytes(invalid).is_err());
    }
    for invalid in [0, MAX_MCP_STDERR_BYTES + 1] {
        assert!(McpLimits::default().with_max_stderr_bytes(invalid).is_err());
    }
    for invalid in [0, MAX_MCP_TOOLS_PER_SERVER + 1] {
        assert!(McpLimits::default().with_max_tools(invalid).is_err());
    }
    for invalid in [0, MAX_MCP_NOTIFICATIONS + 1] {
        assert!(
            McpLimits::default()
                .with_max_notifications(invalid)
                .is_err()
        );
    }
    for invalid in [0, MAX_MCP_PROGRESS_EVENTS + 1] {
        assert!(
            McpLimits::default()
                .with_max_progress_events(invalid)
                .is_err()
        );
    }
}

#[test]
fn lifecycle_and_reconnect_policies_are_bounded_and_disabled_by_default() {
    assert!(
        McpLifecyclePolicy::new(
            Duration::ZERO,
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .is_err()
    );
    assert!(
        McpLifecyclePolicy::new(
            MAX_MCP_LIFECYCLE_TIMEOUT + Duration::from_millis(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .is_err()
    );

    assert!(!McpReconnectPolicy::default().is_enabled());
    assert!(
        McpReconnectPolicy::bounded(
            MAX_MCP_RECONNECT_ATTEMPTS + 1,
            Duration::from_millis(1),
            Duration::from_millis(2),
        )
        .is_err()
    );
    assert!(
        McpReconnectPolicy::bounded(1, Duration::from_secs(2), Duration::from_secs(1),).is_err()
    );
    assert!(
        McpReconnectPolicy::bounded(
            1,
            Duration::from_millis(1),
            MAX_MCP_RECONNECT_BACKOFF + Duration::from_millis(1),
        )
        .is_err()
    );
}

#[test]
fn enabled_tools_require_complete_canonical_host_declarations() {
    let execution = ToolExecutionSemantics::new(
        ToolIdempotency::Idempotent,
        ToolRetrySafety::Automatic,
        ToolConcurrency::Parallel,
        ToolTimeout::from_millis(5_000).expect("timeout"),
    )
    .expect("execution semantics");
    assert!(McpToolDeclaration::new(Vec::new(), Vec::new(), execution).is_err());
    assert!(
        McpToolDeclaration::new(
            vec![ToolEffect::FsRead, ToolEffect::FsRead],
            Vec::new(),
            execution,
        )
        .is_err()
    );

    let resource =
        McpArgumentResource::new("path", "file", ToolResourceAccess::Read).expect("resource");
    let declaration =
        McpToolDeclaration::new([ToolEffect::FsRead], [resource], execution).expect("declaration");
    let enabled = McpToolPolicy::enabled(
        McpRemoteToolName::new("read\u{2603}").expect("remote"),
        declaration,
    );
    assert_eq!(
        server_config(Vec::new(), vec![enabled])
            .expect_err("noncanonical default alias")
            .code(),
        McpErrorCode::PolicyDeclaration
    );
}

#[test]
fn tools_are_disabled_by_default_and_debug_redacts_process_values() {
    let tool = McpToolPolicy::new(McpRemoteToolName::new("inspect").expect("remote"));
    assert!(!tool.is_enabled());

    let secret = "tea-mcp-sentinel-secret";
    let config = McpServerConfig::new(
        McpServerId::from_str("fixture").expect("server id"),
        McpTransportConfig::stdio(
            absolute_executable(secret),
            vec![OsString::from(secret), OsString::from("--stdio")],
        )
        .expect("transport"),
        vec!["TEA_MCP_TOKEN".to_owned()],
        vec![tool],
        McpLimits::default(),
        McpLifecyclePolicy::default(),
        McpReconnectPolicy::default(),
    )
    .expect("config");

    let debug = format!("{config:?}");
    assert!(!debug.contains(secret));
    assert!(debug.contains("fixture"));
}
