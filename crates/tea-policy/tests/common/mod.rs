#![allow(dead_code)]

use std::str::FromStr;
use std::sync::Arc;

use futures_util::stream;
use serde_json::{Value, json};
use tea_control::CancellationScope;
use tea_policy::{
    ActorId, ExecutionSurface, PolicyEnvironment, PolicyExecutionTarget, PolicyGrant, PolicyInput,
    WorkspaceId,
};
use tea_protocol::{
    ProfileId, ProtocolMetadata, ProtocolTimestamp, RunId, SessionId, ToolCallId, ToolIdempotency,
};
use tea_tools::{
    BoxToolExecutionStream, StaticResourceResolver, ToolConcurrency, ToolEffect,
    ToolExecutionEvent, ToolExecutionFailure, ToolExecutionSemantics, ToolExecutor, ToolInvocation,
    ToolName, ToolRegistry, ToolResource, ToolResourceAccess, ToolRetrySafety, ToolSource,
    ToolSpec, ToolTimeout, ToolVersion, ValidatedToolInvocation,
};

#[derive(Debug)]
struct NoopExecutor;
impl ToolExecutor for NoopExecutor {
    fn execute(
        &self,
        _invocation: ValidatedToolInvocation,
        _cancellation: CancellationScope,
    ) -> BoxToolExecutionStream {
        Box::pin(stream::iter([ToolExecutionEvent::Failed(
            ToolExecutionFailure::execution("not executed").unwrap(),
        )]))
    }
}

pub fn timestamp(value: &str) -> ProtocolTimestamp {
    ProtocolTimestamp::from_str(value).unwrap()
}
pub fn grant_id() -> tea_policy::GrantId {
    tea_policy::GrantId::from_str("0195a0b1-5e69-70ac-807e-0aa7aa000047").unwrap()
}
pub fn tool_call_id() -> ToolCallId {
    ToolCallId::from_str("0195a0b1-5e45-75be-8284-0aa7aa000011").unwrap()
}
pub fn run_id() -> RunId {
    RunId::from_str("0195a0b1-5e3b-7ef0-8ec1-0aa7aa000001").unwrap()
}
pub fn session_id() -> SessionId {
    SessionId::from_str("0195a0b1-5e3a-7d72-a902-c4e85d828bf1").unwrap()
}

pub fn validated_invocation(
    name: &str,
    effects: Vec<ToolEffect>,
    resources: Vec<ToolResource>,
    arguments: Value,
) -> ValidatedToolInvocation {
    validated_invocation_with_source(
        name,
        effects,
        resources,
        arguments,
        ToolSource::native_product(),
    )
}

pub fn validated_invocation_with_source(
    name: &str,
    effects: Vec<ToolEffect>,
    resources: Vec<ToolResource>,
    arguments: Value,
    source: ToolSource,
) -> ValidatedToolInvocation {
    let mut registry = ToolRegistry::new();
    let schema = json!({"type":"object"});
    let spec = ToolSpec::new(
        ToolName::from_str(name).unwrap(),
        ToolVersion::from_str("1.0.0").unwrap(),
        "Policy test tool.",
        schema.clone(),
        schema,
        effects,
        ToolExecutionSemantics::new(
            ToolIdempotency::Idempotent,
            ToolRetrySafety::ExplicitOnly,
            ToolConcurrency::Serial,
            ToolTimeout::from_millis(1_000).unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
    .with_source(source);
    registry
        .register(
            spec,
            Arc::new(StaticResourceResolver::new(resources).unwrap()),
            Arc::new(NoopExecutor),
        )
        .unwrap();
    registry
        .validate(
            ToolInvocation::new(
                tool_call_id(),
                ToolName::from_str(name).unwrap(),
                arguments,
                ProtocolMetadata::default(),
            )
            .unwrap(),
        )
        .unwrap()
}

pub fn input_with(
    invocation: &ValidatedToolInvocation,
    grants: Vec<PolicyGrant>,
    now: ProtocolTimestamp,
) -> PolicyInput {
    PolicyInput::from_validated(
        ActorId::from_str("user:alice").unwrap(),
        ProfileId::from_str("coding").unwrap(),
        session_id(),
        Some(run_id()),
        Some(WorkspaceId::from_str("workspace/main").unwrap()),
        invocation,
        PolicyEnvironment::new(
            ExecutionSurface::Desktop,
            PolicyExecutionTarget::Native,
            ProtocolMetadata::default(),
        ),
        now,
        grants,
    )
    .unwrap()
}

pub fn file_resource(path: &str, access: ToolResourceAccess) -> ToolResource {
    ToolResource::new("file", path, access).unwrap()
}
