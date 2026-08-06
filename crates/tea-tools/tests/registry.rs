use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use futures_util::stream;
use serde_json::{Map, Value, json};
use tea_control::CancellationScope;
use tea_model::{
    HostedToolKind, HostedToolOptions, ModelCapabilities, ModelDisplayName, ModelSpec, ProviderId,
    WebSearchOptions,
};
use tea_protocol::{ModelId, ProtocolMetadata, TokenCount, ToolCallId, ToolIdempotency};
use tea_tools::{
    BoxToolExecutionStream, SchedulerClass, StaticResourceResolver, ToolConcurrency, ToolEffect,
    ToolExecutionEvent, ToolExecutionFailure, ToolExecutionSemantics, ToolExecutor, ToolInvocation,
    ToolName, ToolRegistry, ToolRegistryError, ToolResource, ToolResourceAccess, ToolRetrySafety,
    ToolRoutePreference, ToolSource, ToolSourceKind, ToolSpec, ToolTimeout, ToolTrust, ToolVersion,
};

const DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn spec(name: &str, version: &str) -> ToolSpec {
    let schema = json!({
        "type":"object",
        "properties":{"path":{"type":"string"}},
        "required":["path"],
        "additionalProperties":false
    });
    ToolSpec::new(
        ToolName::from_str(name).unwrap(),
        ToolVersion::from_str(version).unwrap(),
        "Reads a path.",
        schema.clone(),
        json!({
            "type":"object",
            "properties":{"content":{"type":"string"}},
            "required":["content"],
            "additionalProperties":false
        }),
        [ToolEffect::FsRead],
        ToolExecutionSemantics::new(
            ToolIdempotency::Idempotent,
            ToolRetrySafety::Automatic,
            ToolConcurrency::Parallel,
            ToolTimeout::from_millis(1_000).unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}

#[derive(Debug)]
struct NeverExecutor;

impl ToolExecutor for NeverExecutor {
    fn execute(
        &self,
        _invocation: tea_tools::ValidatedToolInvocation,
        _cancellation: CancellationScope,
    ) -> BoxToolExecutionStream {
        Box::pin(stream::iter([ToolExecutionEvent::Failed(
            ToolExecutionFailure::execution("not executed in registry test").unwrap(),
        )]))
    }
}

fn resolver() -> Arc<StaticResourceResolver> {
    Arc::new(
        StaticResourceResolver::new([ToolResource::new(
            "file",
            "/workspace/notes.txt",
            ToolResourceAccess::Read,
        )
        .unwrap()])
        .unwrap(),
    )
}

fn web_search_options() -> HostedToolOptions {
    HostedToolOptions::WebSearch(WebSearchOptions::new())
}

fn model(capabilities: ModelCapabilities) -> ModelSpec {
    ModelSpec::new(
        ModelId::from_str("test/model").unwrap(),
        ProviderId::from_str("test-provider").unwrap(),
        ModelDisplayName::from_str("Test Model").unwrap(),
        TokenCount::new(32_000).unwrap(),
        TokenCount::new(8_000).unwrap(),
        capabilities,
    )
    .unwrap()
}

fn call_id() -> ToolCallId {
    ToolCallId::from_str("0195a0b1-5e45-75be-8284-0aa7aa000011").unwrap()
}

#[test]
fn resources_are_bounded_canonical_and_deduplicated() {
    let first = ToolResource::new("file", "/workspace/a.txt", ToolResourceAccess::Read).unwrap();
    let duplicate = first.clone();
    let second = ToolResource::new("file", "/workspace/b.txt", ToolResourceAccess::Write).unwrap();
    let resolver = StaticResourceResolver::new([second.clone(), duplicate, first.clone()]).unwrap();
    let resources = resolver.resources();
    assert_eq!(resources, &[first, second]);
    assert!(ToolResource::new("Bad", "/tmp", ToolResourceAccess::Read).is_err());
    assert!(ToolResource::new("file", "bad\npath", ToolResourceAccess::Read).is_err());
}

#[test]
fn resource_collection_limit_is_enforced_after_deduplication() {
    let resources = (0..129)
        .map(|index| {
            ToolResource::new(
                "file",
                format!("/workspace/{index}.txt"),
                ToolResourceAccess::Read,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(StaticResourceResolver::new(resources).is_err());
}

#[test]
fn registry_order_and_duplicate_conflicts_are_deterministic() {
    let mut registry = ToolRegistry::new();
    registry
        .register(spec("z_tool", "1.0.0"), resolver(), Arc::new(NeverExecutor))
        .unwrap();
    registry
        .register(spec("a_tool", "1.0.0"), resolver(), Arc::new(NeverExecutor))
        .unwrap();
    assert_eq!(
        registry.names().map(ToolName::as_str).collect::<Vec<_>>(),
        ["a_tool", "z_tool"]
    );
    assert_eq!(
        registry
            .register(spec("a_tool", "1.0.0"), resolver(), Arc::new(NeverExecutor))
            .unwrap_err(),
        ToolRegistryError::DuplicateTool
    );
    assert_eq!(
        registry
            .register(spec("a_tool", "2.0.0"), resolver(), Arc::new(NeverExecutor))
            .unwrap_err(),
        ToolRegistryError::VersionConflict
    );
}

#[test]
fn registry_projects_client_hosted_and_hybrid_routes_per_model() {
    let function_model = model(ModelCapabilities::text().with_tools(false));
    let hosted_model = model(
        ModelCapabilities::text()
            .with_tools(false)
            .with_hosted_tool(HostedToolKind::WebSearch),
    );

    let mut client = ToolRegistry::new();
    client
        .register(
            spec("web_search", "1.0.0"),
            resolver(),
            Arc::new(NeverExecutor),
        )
        .unwrap();
    assert!(
        client.model_definitions(&function_model).unwrap()[0]
            .as_function()
            .is_some()
    );

    let mut hosted = ToolRegistry::new();
    hosted
        .register_hosted(spec("web_search", "1.0.0"), web_search_options())
        .unwrap();
    let definitions = hosted.model_definitions(&hosted_model).unwrap();
    assert_eq!(
        definitions[0].hosted_kind(),
        Some(HostedToolKind::WebSearch)
    );
    assert!(matches!(
        hosted.model_definitions(&function_model).unwrap_err(),
        ToolRegistryError::NoSupportedToolRoute { tool, model }
            if tool.as_str() == "web_search" && model.as_str() == "test/model"
    ));

    let mut hybrid = ToolRegistry::new();
    hybrid
        .register_hybrid(
            spec("web_search", "1.0.0"),
            web_search_options(),
            ToolRoutePreference::PreferHosted,
            resolver(),
            Arc::new(NeverExecutor),
        )
        .unwrap();
    assert!(
        hybrid.model_definitions(&hosted_model).unwrap()[0]
            .as_hosted()
            .is_some()
    );
    assert!(
        hybrid.model_definitions(&function_model).unwrap()[0]
            .as_function()
            .is_some()
    );
}

#[test]
fn forced_client_hybrid_never_silently_selects_hosted() {
    let hosted_only_model =
        model(ModelCapabilities::text().with_hosted_tool(HostedToolKind::WebSearch));
    let both_model = model(
        ModelCapabilities::text()
            .with_tools(false)
            .with_hosted_tool(HostedToolKind::WebSearch),
    );
    let mut registry = ToolRegistry::new();
    registry
        .register_hybrid(
            spec("web_search", "1.0.0"),
            web_search_options(),
            ToolRoutePreference::ForceClient,
            resolver(),
            Arc::new(NeverExecutor),
        )
        .unwrap();

    assert!(
        registry.model_definitions(&both_model).unwrap()[0]
            .as_function()
            .is_some()
    );
    assert!(matches!(
        registry.model_definitions(&hosted_only_model).unwrap_err(),
        ToolRegistryError::NoSupportedToolRoute { tool, model }
            if tool.as_str() == "web_search" && model.as_str() == "test/model"
    ));
}

#[test]
fn hosted_registration_requires_the_canonical_kind_name() {
    let mut registry = ToolRegistry::new();
    assert_eq!(
        registry
            .register_hosted(spec("search", "1.0.0"), web_search_options())
            .unwrap_err(),
        ToolRegistryError::HostedToolNameMismatch
    );
}

#[test]
fn hosted_only_tools_cannot_enter_local_validation_or_execution() {
    let mut registry = ToolRegistry::new();
    registry
        .register_hosted(spec("web_search", "1.0.0"), web_search_options())
        .unwrap();
    let invocation = ToolInvocation::new(
        call_id(),
        ToolName::from_str("web_search").unwrap(),
        json!({"path":"query"}),
        ProtocolMetadata::default(),
    )
    .unwrap();

    assert_eq!(
        registry.validate(invocation).unwrap_err(),
        ToolRegistryError::HostedToolNotClientExecutable
    );
}

#[test]
fn invalid_arguments_never_reach_validated_invocation_or_executor() {
    let mut registry = ToolRegistry::new();
    registry
        .register(
            spec("read_file", "1.0.0"),
            resolver(),
            Arc::new(NeverExecutor),
        )
        .unwrap();
    let invocation = ToolInvocation::new(
        call_id(),
        ToolName::from_str("read_file").unwrap(),
        json!({"wrong":true}),
        ProtocolMetadata::default(),
    )
    .unwrap();
    assert!(matches!(
        registry.validate(invocation),
        Err(ToolRegistryError::InvalidArguments(_))
    ));
}

#[test]
fn valid_invocation_carries_resolved_resources_and_metadata() {
    let mut registry = ToolRegistry::new();
    registry
        .register(
            spec("read_file", "1.0.0"),
            resolver(),
            Arc::new(NeverExecutor),
        )
        .unwrap();
    let invocation = ToolInvocation::new(
        call_id(),
        ToolName::from_str("read_file").unwrap(),
        json!({"path":"/workspace/notes.txt"}),
        ProtocolMetadata::default(),
    )
    .unwrap();
    let validated = registry.validate(invocation).unwrap();
    assert_eq!(validated.name().as_str(), "read_file");
    assert_eq!(
        validated.arguments(),
        &json!({"path":"/workspace/notes.txt"})
    );
    assert_eq!(validated.resources().len(), 1);
    assert_eq!(
        validated.scheduler_class(),
        SchedulerClass::ParallelReadOnly
    );
}

#[test]
fn validated_invocation_freezes_registered_source() {
    let source = ToolSource::new(
        ToolSourceKind::Mcp,
        "workspace.files",
        ToolTrust::Workspace,
        DIGEST,
    )
    .unwrap();
    let mut registry = ToolRegistry::new();
    registry
        .register(
            spec("read_file", "1.0.0").with_source(source.clone()),
            resolver(),
            Arc::new(NeverExecutor),
        )
        .unwrap();
    let validated = registry
        .validate(
            ToolInvocation::new(
                call_id(),
                ToolName::from_str("read_file").unwrap(),
                json!({"path":"/workspace/notes.txt"}),
                ProtocolMetadata::default(),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(validated.source(), &source);
}

#[tokio::test(flavor = "current_thread")]
async fn unknown_tools_fail_before_execution() {
    let registry = ToolRegistry::new();
    let invocation = ToolInvocation::new(
        call_id(),
        ToolName::from_str("missing_tool").unwrap(),
        Value::Object(Map::default()),
        ProtocolMetadata::default(),
    )
    .unwrap();
    assert_eq!(
        registry.validate(invocation).unwrap_err(),
        ToolRegistryError::UnknownTool
    );
}

impl fmt::Display for NeverExecutor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("never")
    }
}
