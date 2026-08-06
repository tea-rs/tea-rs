#![forbid(unsafe_code)]

use std::{ffi::OsString, str::FromStr};

use serde_json::{Value, json};
use tea_mcp::{
    McpErrorCode, McpLifecyclePolicy, McpLimits, McpReconnectPolicy, McpRemoteToolDescriptor,
    McpRemoteToolName, McpServerConfig, McpServerId, McpToolCatalog, McpToolDeclaration,
    McpToolPolicy, McpTransportConfig,
};
use tea_protocol::ToolIdempotency;
use tea_tools::{
    ToolConcurrency, ToolEffect, ToolExecutionSemantics, ToolRetrySafety, ToolTimeout, ToolTrust,
};

#[test]
fn input_and_output_schemas_compile_offline_before_registration() {
    let descriptor = McpRemoteToolDescriptor::from_value(json!({
        "name":"lookup",
        "description":"Lookup.",
        "inputSchema":{
            "$schema":"https://json-schema.org/draft/2020-12/schema",
            "type":"object",
            "properties":{"query":{"type":"string"}},
            "required":["query"]
        },
        "outputSchema":{
            "type":"object",
            "properties":{"found":{"type":"boolean"}},
            "required":["found"]
        }
    }))
    .unwrap();
    let catalog = McpToolCatalog::freeze(&config(), ToolTrust::User, [descriptor]).unwrap();
    let spec = catalog.specs().next().unwrap();

    assert_eq!(spec.input_schema()["type"], "object");
    assert_eq!(spec.output_schema()["type"], "object");
}

#[test]
fn absent_output_schema_gets_a_bounded_object_contract() {
    let descriptor = McpRemoteToolDescriptor::from_value(json!({
        "name":"lookup",
        "description":"Lookup.",
        "inputSchema":{"type":"object"}
    }))
    .unwrap();
    let catalog = McpToolCatalog::freeze(&config(), ToolTrust::User, [descriptor]).unwrap();

    assert_eq!(
        catalog.specs().next().unwrap().output_schema(),
        &json!({"type":"object"})
    );
}

#[test]
fn invalid_external_non_object_and_deep_schemas_fail_closed() {
    let mut deep = json!({"type":"string"});
    for _ in 0..40 {
        deep = json!({"type":"object","properties":{"nested":deep}});
    }

    for input_schema in [
        json!({"type":7}),
        json!({"type":"object","$ref":"https://example.invalid/schema.json"}),
        json!({"type":"array"}),
    ] {
        let error = McpRemoteToolDescriptor::from_value(remote(&input_schema, None)).unwrap_err();
        assert_eq!(error.code(), McpErrorCode::Schema);
    }

    let error = McpRemoteToolDescriptor::from_value(remote(
        &json!({"type":"object"}),
        Some(json!({"type":"array"})),
    ))
    .unwrap_err();
    assert_eq!(error.code(), McpErrorCode::Schema);

    let error = McpRemoteToolDescriptor::from_value(remote(&deep, None)).unwrap_err();
    assert_eq!(error.code(), McpErrorCode::OutputBound);
}

#[test]
fn task_required_descriptors_are_unsupported() {
    let error = McpRemoteToolDescriptor::from_value(json!({
        "name":"lookup",
        "description":"Lookup.",
        "inputSchema":{"type":"object"},
        "execution":{"taskSupport":"required"}
    }))
    .unwrap_err();

    assert_eq!(error.code(), McpErrorCode::Descriptor);
}

fn remote(input_schema: &Value, output_schema: Option<Value>) -> Value {
    let mut value = json!({
        "name":"lookup",
        "description":"Lookup.",
        "inputSchema":input_schema
    });
    if let Some(output_schema) = output_schema {
        value["outputSchema"] = output_schema;
    }
    value
}

fn config() -> McpServerConfig {
    let execution = ToolExecutionSemantics::new(
        ToolIdempotency::Idempotent,
        ToolRetrySafety::Automatic,
        ToolConcurrency::Parallel,
        ToolTimeout::from_millis(5_000).unwrap(),
    )
    .unwrap();
    let declaration =
        McpToolDeclaration::new([ToolEffect::NetworkRequest], Vec::new(), execution).unwrap();
    McpServerConfig::new(
        McpServerId::from_str("schema").unwrap(),
        McpTransportConfig::stdio(
            if cfg!(windows) { r"C:\mcp.exe" } else { "/mcp" },
            Vec::<OsString>::new(),
        )
        .unwrap(),
        Vec::new(),
        vec![McpToolPolicy::enabled(
            McpRemoteToolName::new("lookup").unwrap(),
            declaration,
        )],
        McpLimits::default(),
        McpLifecyclePolicy::default(),
        McpReconnectPolicy::default(),
    )
    .unwrap()
}
