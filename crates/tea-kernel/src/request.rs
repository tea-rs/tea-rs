use tea_model::{ModelRequest, ModelSpec, ReasoningOptions};
use tea_session::MaterializedSessionState;
use tea_tools::{ToolName, ToolRegistry};

use crate::{KernelError, KernelErrorCode, KernelRunConfig};

/// Immutable model request plus the durable tail used to construct it.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnRequestSnapshot {
    request: ModelRequest,
    client_tool_names: Vec<ToolName>,
    hosted_tool_names: Vec<ToolName>,
    durable_tail: tea_protocol::SessionSequence,
}

impl TurnRequestSnapshot {
    /// Builds and validates one immutable request from committed session state.
    ///
    /// Registered tool specifications are projected in canonical name order and
    /// parallel calls remain disabled for the sequential milestone scheduler.
    ///
    /// # Errors
    ///
    /// Returns a typed error when no model is selected, the provider does not
    /// advertise it, or model/tool/request bounds are incompatible.
    pub fn build(
        state: &MaterializedSessionState,
        config: &KernelRunConfig,
        tools: &ToolRegistry,
        model: &ModelSpec,
    ) -> Result<Self, KernelError> {
        let model_ref = state.configuration().model_ref().ok_or_else(|| {
            KernelError::new(KernelErrorCode::InvalidModel, "session has no active model")
        })?;
        if model.model_ref() != model_ref {
            return Err(KernelError::new(
                KernelErrorCode::InvalidModel,
                "resolved model does not match the active session model",
            ));
        }
        let definitions = tools.model_definitions(model).map_err(|error| {
            KernelError::new(KernelErrorCode::InvalidRequest, error.to_string())
        })?;
        let mut client_tool_names = Vec::new();
        let mut hosted_tool_names = Vec::new();
        for (name, definition) in tools.names().zip(&definitions) {
            if definition.as_function().is_some() {
                client_tool_names.push(name.clone());
            } else if definition.as_hosted().is_some() {
                hosted_tool_names.push(name.clone());
            }
        }
        let mut request =
            ModelRequest::new(model_ref.model_id().clone(), state.messages().to_vec())?
                .with_tools(definitions, false)?
                .with_metadata(config.request_metadata().clone());
        if let (Some(requested), Some(profile)) = (
            state.configuration().reasoning_effort(),
            model.reasoning_profile(),
        ) {
            let effective = profile.resolve(requested).effective();
            request = request.with_reasoning(ReasoningOptions::new(effective));
        }
        if let Some(prompt) = config.system_prompt() {
            request = request.with_system_prompt(prompt)?;
        }
        request.validate_for(model)?;
        Ok(Self {
            request,
            client_tool_names,
            hosted_tool_names,
            durable_tail: state.tail_sequence(),
        })
    }

    /// Returns the immutable provider request.
    #[must_use]
    pub const fn request(&self) -> &ModelRequest {
        &self.request
    }

    /// Returns the exact client-executable function names frozen for this turn.
    ///
    /// Active hosted projections are deliberately absent even when their
    /// portable tool specifications also have a client route.
    #[must_use]
    pub fn client_tool_names(&self) -> &[ToolName] {
        &self.client_tool_names
    }

    /// Returns the exact provider-hosted tool names frozen for this turn.
    #[must_use]
    pub fn hosted_tool_names(&self) -> &[ToolName] {
        &self.hosted_tool_names
    }

    /// Returns whether a provider function call may enter local execution.
    #[must_use]
    pub fn allows_client_tool_call(&self, tool_name: &str) -> bool {
        self.client_tool_names
            .iter()
            .any(|name| name.as_str() == tool_name)
    }

    /// Returns whether this name was projected as a provider-hosted tool.
    #[must_use]
    pub fn is_hosted_tool_projection(&self, tool_name: &str) -> bool {
        self.hosted_tool_names
            .iter()
            .any(|name| name.as_str() == tool_name)
    }

    /// Consumes the snapshot into its provider request.
    #[must_use]
    pub fn into_request(self) -> ModelRequest {
        self.request
    }

    /// Returns the durable session tail captured with the request.
    #[must_use]
    pub const fn durable_tail(&self) -> tea_protocol::SessionSequence {
        self.durable_tail
    }
}

impl From<tea_model::ModelRequestError> for KernelError {
    fn from(error: tea_model::ModelRequestError) -> Self {
        Self::new(KernelErrorCode::InvalidRequest, error.to_string())
    }
}
