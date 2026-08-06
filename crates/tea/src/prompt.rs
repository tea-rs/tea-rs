use std::sync::Arc;

use tea_context::{CompiledPrompt, ContextProvider, ContextRequest, PromptBudget, PromptCompiler};
use tea_protocol::{ProfileId, ProtocolMetadata, RunId, SessionId};

use crate::RuntimeError;

/// Compiles the system prompt for one run from the binding's context providers.
///
/// Providers are evaluated in stable binding order. The compiled prompt is
/// byte-identical for identical inputs and is attached to the kernel run config.
///
/// # Errors
///
/// Returns an error when a provider fails or compilation exceeds the budget.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn compile_prompt(
    compiler: &PromptCompiler,
    providers: &[Arc<dyn ContextProvider>],
    profile_id: ProfileId,
    session_id: SessionId,
    run_id: Option<RunId>,
    active_tools: &[tea_tools::ToolSpec],
    metadata: ProtocolMetadata,
    budget: PromptBudget,
) -> Result<CompiledPrompt, RuntimeError> {
    let request = ContextRequest::new(
        profile_id,
        session_id,
        run_id,
        active_tools.to_vec(),
        metadata,
    )?;
    let mut modules = Vec::new();
    for provider in providers {
        let provided = provider.provide(request.clone()).await?;
        modules.extend(provided);
    }
    compiler
        .compile(modules, budget)
        .map_err(RuntimeError::from)
}
