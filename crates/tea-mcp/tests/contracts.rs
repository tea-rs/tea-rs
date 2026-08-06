use std::{error::Error, str::FromStr};

use tea_mcp::{
    MAX_MCP_RESTART_COUNT, McpDescriptorDigest, McpError, McpErrorCode, McpManager,
    McpManagerShutdownReport, McpProtocolVersion, McpRemoteToolDescriptor, McpServerConfig,
    McpServerHealth, McpServerId, McpServerLaunch, McpServerSnapshot, McpServerState,
    McpToolBinding, McpToolCatalog,
};
use tea_protocol::ProtocolTimestamp;

#[test]
fn error_codes_are_stable_and_errors_are_safe_trait_objects() {
    let cases = [
        (McpErrorCode::Configuration, "configuration"),
        (McpErrorCode::Startup, "startup"),
        (McpErrorCode::Handshake, "handshake"),
        (McpErrorCode::Descriptor, "descriptor"),
        (McpErrorCode::Identity, "identity"),
        (McpErrorCode::Schema, "schema"),
        (McpErrorCode::PolicyDeclaration, "policy_declaration"),
        (McpErrorCode::Transport, "transport"),
        (McpErrorCode::Protocol, "protocol"),
        (McpErrorCode::Execution, "execution"),
        (McpErrorCode::InvalidResult, "invalid_result"),
        (McpErrorCode::Timeout, "timeout"),
        (McpErrorCode::Cancellation, "cancellation"),
        (McpErrorCode::StaleCatalog, "stale_catalog"),
        (McpErrorCode::Unavailable, "unavailable"),
        (McpErrorCode::OutputBound, "output_bound"),
        (McpErrorCode::ServerExit, "server_exit"),
        (McpErrorCode::Shutdown, "shutdown"),
    ];

    for (code, expected) in cases {
        assert_eq!(
            serde_json::to_string(&code).expect("encode"),
            format!(r#""{expected}""#)
        );
        assert_eq!(
            serde_json::from_str::<McpErrorCode>(&format!(r#""{expected}""#)).expect("decode"),
            code
        );
    }

    let error = McpError::new(McpErrorCode::Transport);
    let object: &(dyn Error + Send + Sync) = &error;
    assert!(!object.to_string().is_empty());
    assert_eq!(error.code(), McpErrorCode::Transport);
}

#[test]
fn health_is_strict_bounded_serializable_and_secret_free() {
    let health = McpServerHealth::new(
        McpServerId::from_str("fixture").expect("server id"),
        McpServerState::Ready,
        None,
        Some(McpDescriptorDigest::from_str(&"a".repeat(64)).expect("descriptor digest")),
        1,
        ProtocolTimestamp::from_str("2026-07-25T10:00:00.000Z").expect("timestamp"),
    )
    .expect("health");

    let encoded = serde_json::to_string(&health).expect("encode");
    assert!(encoded.contains(r#""serverId":"fixture""#));
    assert!(encoded.contains(r#""state":"ready""#));
    assert!(!encoded.contains("argv"));
    assert!(!encoded.contains("environment"));
    assert!(!encoded.contains("stderr"));
    assert_eq!(
        serde_json::from_str::<McpServerHealth>(&encoded).expect("decode"),
        health
    );

    let mut value = serde_json::to_value(&health).expect("value");
    value.as_object_mut().expect("object").insert(
        "rawError".to_owned(),
        serde_json::json!("tea-mcp-sentinel-secret"),
    );
    assert!(serde_json::from_value::<McpServerHealth>(value).is_err());
}

#[test]
fn health_rejects_invalid_digests_and_restart_counts() {
    assert!(McpDescriptorDigest::from_str(&"A".repeat(64)).is_err());
    assert!(McpDescriptorDigest::from_str("short").is_err());

    assert!(
        McpServerHealth::new(
            McpServerId::from_str("fixture").expect("server id"),
            McpServerState::Unhealthy,
            Some(McpErrorCode::ServerExit),
            None,
            MAX_MCP_RESTART_COUNT + 1,
            ProtocolTimestamp::from_str("2026-07-25T10:00:00.000Z").expect("timestamp"),
        )
        .is_err()
    );
}

#[test]
fn public_contracts_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<McpServerConfig>();
    assert_send_sync::<McpServerHealth>();
    assert_send_sync::<McpRemoteToolDescriptor>();
    assert_send_sync::<McpToolBinding>();
    assert_send_sync::<McpToolCatalog>();
    assert_send_sync::<McpProtocolVersion>();
    assert_send_sync::<McpServerSnapshot>();
    assert_send_sync::<McpServerLaunch>();
    assert_send_sync::<McpManager>();
    assert_send_sync::<McpManagerShutdownReport>();
    assert_send_sync::<McpError>();
}
