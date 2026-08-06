use std::collections::BTreeMap;
use std::sync::Arc;

use futures_util::{StreamExt, stream};
use tea_control::CancellationScope;
#[cfg(feature = "model-projection")]
use tea_model::{HostedToolOptions, ModelRequestError, ModelSpec, ModelToolDefinition};
#[cfg(feature = "model-projection")]
use tea_protocol::ModelId;
use thiserror::Error;

use crate::{
    BoxToolExecutionStream, CompiledToolSchema, SchemaCompilationError, SchemaValidationFailure,
    ToolExecutionEvent, ToolExecutionFailure, ToolExecutor, ToolInvocation, ToolName,
    ToolResourceError, ToolResourceResolver, ToolSpec, ValidatedToolInvocation,
};
use tea_protocol::ToolPresentation;

#[derive(Debug)]
struct RegisteredTool {
    spec: Arc<ToolSpec>,
    input: CompiledToolSchema,
    output: CompiledToolSchema,
    binding: ToolBinding,
}

type ClientBindingRefs<'a> = (&'a Arc<dyn ToolResourceResolver>, &'a Arc<dyn ToolExecutor>);

/// Preferred route for a hybrid tool with hosted and client implementations.
#[cfg(feature = "model-projection")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRoutePreference {
    /// Use provider-hosted execution when supported, otherwise use the client route.
    PreferHosted,
    /// Require the client route even when hosted execution is available.
    ForceClient,
}

/// Complete executable binding associated with one portable [`ToolSpec`].
#[derive(Debug, Clone)]
pub enum ToolBinding {
    /// A function tool resolved and executed by the client runtime.
    Client {
        /// Resource resolver used before policy evaluation.
        resolver: Arc<dyn ToolResourceResolver>,
        /// Local executor invoked after policy approval.
        executor: Arc<dyn ToolExecutor>,
    },
    /// A tool executed entirely inside a capable model provider.
    #[cfg(feature = "model-projection")]
    Hosted {
        /// Provider-neutral hosted tool kind and common policy.
        options: HostedToolOptions,
    },
    /// A tool with both provider-hosted and real client execution routes.
    #[cfg(feature = "model-projection")]
    Hybrid {
        /// Provider-neutral hosted tool kind and common policy.
        options: HostedToolOptions,
        /// Resource resolver for the client fallback.
        resolver: Arc<dyn ToolResourceResolver>,
        /// Executor for the client fallback.
        executor: Arc<dyn ToolExecutor>,
        /// Route preference applied when freezing a model request.
        preference: ToolRoutePreference,
    },
}

impl ToolBinding {
    /// Creates a client-only binding.
    #[must_use]
    pub fn client(
        resolver: Arc<dyn ToolResourceResolver>,
        executor: Arc<dyn ToolExecutor>,
    ) -> Self {
        Self::Client { resolver, executor }
    }

    /// Creates a provider-hosted-only binding.
    #[cfg(feature = "model-projection")]
    #[must_use]
    pub const fn hosted(options: HostedToolOptions) -> Self {
        Self::Hosted { options }
    }

    /// Creates a binding with hosted and client routes.
    #[cfg(feature = "model-projection")]
    #[must_use]
    pub fn hybrid(
        options: HostedToolOptions,
        preference: ToolRoutePreference,
        resolver: Arc<dyn ToolResourceResolver>,
        executor: Arc<dyn ToolExecutor>,
    ) -> Self {
        Self::Hybrid {
            options,
            resolver,
            executor,
            preference,
        }
    }

    /// Returns whether this binding has a real client execution route.
    #[must_use]
    pub const fn has_client_execution(&self) -> bool {
        match self {
            Self::Client { .. } => true,
            #[cfg(feature = "model-projection")]
            Self::Hosted { .. } => false,
            #[cfg(feature = "model-projection")]
            Self::Hybrid { .. } => true,
        }
    }

    fn client_parts(&self) -> Option<ClientBindingRefs<'_>> {
        match self {
            Self::Client { resolver, executor } => Some((resolver, executor)),
            #[cfg(feature = "model-projection")]
            Self::Hosted { .. } => None,
            #[cfg(feature = "model-projection")]
            Self::Hybrid {
                resolver, executor, ..
            } => Some((resolver, executor)),
        }
    }

    #[cfg(feature = "model-projection")]
    const fn hosted_options(&self) -> Option<&HostedToolOptions> {
        match self {
            Self::Client { .. } => None,
            Self::Hosted { options } | Self::Hybrid { options, .. } => Some(options),
        }
    }
}

/// Deterministic active tool registry.
#[derive(Debug, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<ToolName, RegisteredTool>,
}

impl ToolRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one complete tool contract atomically.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate/conflicting names or invalid schemas.
    pub fn register(
        &mut self,
        spec: ToolSpec,
        resolver: Arc<dyn ToolResourceResolver>,
        executor: Arc<dyn ToolExecutor>,
    ) -> Result<(), ToolRegistryError> {
        self.register_binding(spec, ToolBinding::client(resolver, executor))
    }

    /// Registers one complete tool specification and execution binding atomically.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate/conflicting names, invalid schemas, or a
    /// hosted binding whose stable name differs from the specification.
    pub fn register_binding(
        &mut self,
        spec: ToolSpec,
        binding: ToolBinding,
    ) -> Result<(), ToolRegistryError> {
        if let Some(existing) = self.tools.get(spec.name()) {
            return if existing.spec.version() == spec.version() {
                Err(ToolRegistryError::DuplicateTool)
            } else {
                Err(ToolRegistryError::VersionConflict)
            };
        }
        #[cfg(feature = "model-projection")]
        if binding
            .hosted_options()
            .is_some_and(|options| spec.name().as_str() != options.kind().name())
        {
            return Err(ToolRegistryError::HostedToolNameMismatch);
        }
        let input = CompiledToolSchema::compile(spec.input_schema().clone())?;
        let output = CompiledToolSchema::compile(spec.output_schema().clone())?;
        let name = spec.name().clone();
        self.tools.insert(
            name,
            RegisteredTool {
                spec: Arc::new(spec),
                input,
                output,
                binding,
            },
        );
        Ok(())
    }

    /// Registers a provider-hosted-only tool.
    ///
    /// # Errors
    ///
    /// Returns an error under the same conditions as [`Self::register_binding`].
    #[cfg(feature = "model-projection")]
    pub fn register_hosted(
        &mut self,
        spec: ToolSpec,
        options: HostedToolOptions,
    ) -> Result<(), ToolRegistryError> {
        self.register_binding(spec, ToolBinding::hosted(options))
    }

    /// Registers a tool with hosted and client execution routes.
    ///
    /// # Errors
    ///
    /// Returns an error under the same conditions as [`Self::register_binding`].
    #[cfg(feature = "model-projection")]
    pub fn register_hybrid(
        &mut self,
        spec: ToolSpec,
        options: HostedToolOptions,
        preference: ToolRoutePreference,
        resolver: Arc<dyn ToolResourceResolver>,
        executor: Arc<dyn ToolExecutor>,
    ) -> Result<(), ToolRegistryError> {
        self.register_binding(
            spec,
            ToolBinding::hybrid(options, preference, resolver, executor),
        )
    }

    /// Returns registered names in canonical sorted order.
    pub fn names(&self) -> impl Iterator<Item = &ToolName> {
        self.tools.keys()
    }

    /// Returns registered specifications in canonical tool-name order.
    pub fn specs(&self) -> impl Iterator<Item = &ToolSpec> {
        self.tools.values().map(|tool| tool.spec.as_ref())
    }

    /// Projects active tools for one selected model in canonical name order.
    ///
    /// # Errors
    ///
    /// Returns an error when an active tool has no route supported by the model
    /// or a model-layer definition violates stricter bounds.
    #[cfg(feature = "model-projection")]
    pub fn model_definitions(
        &self,
        model: &ModelSpec,
    ) -> Result<Vec<ModelToolDefinition>, ToolRegistryError> {
        self.tools
            .values()
            .map(|tool| project_model_definition(tool, model))
            .collect()
    }

    /// Validates arguments and resolves resources without executing side effects.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown tools, invalid arguments, or resource failures.
    pub fn validate(
        &self,
        invocation: ToolInvocation,
    ) -> Result<ValidatedToolInvocation, ToolRegistryError> {
        let registered = self
            .tools
            .get(invocation.name())
            .ok_or(ToolRegistryError::UnknownTool)?;
        let (resolver, _) = registered
            .binding
            .client_parts()
            .ok_or(ToolRegistryError::HostedToolNotClientExecutable)?;
        registered
            .input
            .validate(invocation.arguments())
            .map_err(ToolRegistryError::InvalidArguments)?;
        let mut resources = resolver.resolve(invocation.name(), invocation.arguments())?;
        resources.sort();
        resources.dedup();
        if resources.len() > crate::MAX_TOOL_RESOURCES {
            return Err(ToolRegistryError::Resources(
                ToolResourceError::TooManyResources,
            ));
        }
        Ok(ValidatedToolInvocation::new(
            invocation,
            Arc::clone(&registered.spec),
            resources,
        ))
    }

    /// Validates, resolves, executes, and enforces terminal output schema.
    ///
    /// # Errors
    ///
    /// Returns an error before execution for unknown tools, invalid arguments,
    /// or resource failures. In-stream executor failures remain terminal events.
    pub fn execute(
        &self,
        invocation: ToolInvocation,
        cancellation: CancellationScope,
    ) -> Result<BoxToolExecutionStream, ToolRegistryError> {
        let validated = self.validate(invocation)?;
        self.execute_validated(validated, cancellation)
    }

    /// Executes an invocation already validated by this registry.
    ///
    /// This is the policy-safe boundary: callers can evaluate one immutable
    /// [`ValidatedToolInvocation`] and execute that exact value without running
    /// schema validation or resource resolution a second time.
    ///
    /// # Errors
    ///
    /// Returns an error when the validated tool is no longer registered with
    /// the same specification.
    pub fn execute_validated(
        &self,
        invocation: ValidatedToolInvocation,
        cancellation: CancellationScope,
    ) -> Result<BoxToolExecutionStream, ToolRegistryError> {
        let registered = self
            .tools
            .get(invocation.name())
            .filter(|tool| tool.spec.as_ref() == invocation.spec())
            .ok_or(ToolRegistryError::UnknownTool)?;
        let (_, executor) = registered
            .binding
            .client_parts()
            .ok_or(ToolRegistryError::HostedToolNotClientExecutable)?;
        let upstream = executor.execute(invocation, cancellation);
        let output = registered.output.clone();
        let state = ExecutionValidationState {
            upstream,
            output,
            done: false,
        };
        Ok(Box::pin(stream::unfold(state, |mut state| async move {
            if state.done {
                return None;
            }
            let Some(event) = state.upstream.next().await else {
                state.done = true;
                return Some((
                    ToolExecutionEvent::Failed(ToolExecutionFailure::internal_contract()),
                    state,
                ));
            };
            let event = match event {
                ToolExecutionEvent::Finished(result) => {
                    state.done = true;
                    if state.output.validate(result.output()).is_ok() {
                        ToolExecutionEvent::Finished(result)
                    } else {
                        ToolExecutionEvent::Failed(ToolExecutionFailure::invalid_output())
                    }
                }
                ToolExecutionEvent::Failed(failure) => {
                    state.done = true;
                    ToolExecutionEvent::Failed(failure)
                }
                ToolExecutionEvent::Progress(progress) => ToolExecutionEvent::Progress(progress),
            };
            Some((event, state))
        })))
    }

    /// Produces a non-durable preview for an invocation already validated by
    /// this registry.
    ///
    /// The registry identity check keeps preview generation on the same frozen
    /// tool contract that policy evaluated. Preview failures are represented by
    /// `None` at the executor boundary and never alter execution semantics.
    #[must_use]
    pub fn preview_validated(
        &self,
        invocation: &ValidatedToolInvocation,
    ) -> Option<ToolPresentation> {
        self.tools
            .get(invocation.name())
            .filter(|tool| tool.spec.as_ref() == invocation.spec())
            .and_then(|tool| tool.binding.client_parts())
            .and_then(|(_, executor)| executor.preview(invocation))
    }
}

#[cfg(feature = "model-projection")]
fn project_model_definition(
    tool: &RegisteredTool,
    model: &ModelSpec,
) -> Result<ModelToolDefinition, ToolRegistryError> {
    let capabilities = model.capabilities();
    match &tool.binding {
        ToolBinding::Client { .. } => {
            if capabilities.supports_tools() {
                function_definition(tool)
            } else {
                Err(no_supported_tool_route(tool, model))
            }
        }
        ToolBinding::Hosted { options } => {
            if capabilities.supports_hosted_tool(options.kind()) {
                hosted_definition(tool, options)
            } else {
                Err(no_supported_tool_route(tool, model))
            }
        }
        ToolBinding::Hybrid {
            options,
            preference,
            ..
        } => {
            let select_hosted = matches!(preference, ToolRoutePreference::PreferHosted)
                && capabilities.supports_hosted_tool(options.kind());
            if select_hosted {
                hosted_definition(tool, options)
            } else if capabilities.supports_tools() {
                function_definition(tool)
            } else {
                Err(no_supported_tool_route(tool, model))
            }
        }
    }
}

#[cfg(feature = "model-projection")]
fn no_supported_tool_route(tool: &RegisteredTool, model: &ModelSpec) -> ToolRegistryError {
    ToolRegistryError::NoSupportedToolRoute {
        tool: tool.spec.name().clone(),
        model: model.model_id().clone(),
    }
}

#[cfg(feature = "model-projection")]
fn function_definition(tool: &RegisteredTool) -> Result<ModelToolDefinition, ToolRegistryError> {
    ModelToolDefinition::new(
        tool.spec.name().as_str(),
        tool.spec.description(),
        tool.spec.input_schema().clone(),
    )
    .map_err(ToolRegistryError::ModelProjection)
}

#[cfg(feature = "model-projection")]
fn hosted_definition(
    tool: &RegisteredTool,
    options: &HostedToolOptions,
) -> Result<ModelToolDefinition, ToolRegistryError> {
    ModelToolDefinition::hosted(
        tool.spec.description(),
        tool.spec.input_schema().clone(),
        options.clone(),
    )
    .map_err(ToolRegistryError::ModelProjection)
}

struct ExecutionValidationState {
    upstream: BoxToolExecutionStream,
    output: CompiledToolSchema,
    done: bool,
}

/// Registry validation, conflict, or resource failure.
#[derive(Debug, PartialEq, Eq, Error)]
pub enum ToolRegistryError {
    /// No active tool has this name.
    #[error("tool is not registered")]
    UnknownTool,
    /// Same name/version is already registered.
    #[error("tool is already registered")]
    DuplicateTool,
    /// Same name has another active version.
    #[error("tool name has a version conflict")]
    VersionConflict,
    /// A hosted binding does not use its kind's stable tool name.
    #[cfg(feature = "model-projection")]
    #[error("hosted tool name does not match its capability kind")]
    HostedToolNameMismatch,
    /// The selected model supports neither the hosted nor client route.
    #[cfg(feature = "model-projection")]
    #[error(
        "active tool {tool} has no execution route supported by selected model {model}; declare the model capability or configure a supported client route"
    )]
    NoSupportedToolRoute {
        /// Active tool that could not be projected.
        tool: ToolName,
        /// Selected model that lacks a usable route.
        model: ModelId,
    },
    /// A provider-hosted-only tool was sent to the client execution boundary.
    #[error("hosted tool has no client execution route")]
    HostedToolNotClientExecutable,
    /// A projected model definition violates model request bounds.
    #[cfg(feature = "model-projection")]
    #[error("tool cannot be projected into the model request: {0}")]
    ModelProjection(ModelRequestError),
    /// Input/output schema cannot compile.
    #[error("tool schema cannot compile: {0}")]
    Schema(#[from] SchemaCompilationError),
    /// Arguments violate the registered input schema.
    #[error("tool arguments are invalid: {0}")]
    InvalidArguments(SchemaValidationFailure),
    /// Resource resolution failed.
    #[error("tool resource resolution failed: {0}")]
    Resources(#[from] ToolResourceError),
}
