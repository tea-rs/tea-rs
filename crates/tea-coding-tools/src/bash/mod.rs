mod command;
mod output;
mod process;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
use windows as platform;

use std::str::FromStr;

use serde_json::json;
use tea_control::CancellationScope;
use tea_protocol::ToolIdempotency;
use tea_tools::{
    BoxToolExecutionStream, ToolConcurrency, ToolEffect, ToolExecutionSemantics, ToolExecutor,
    ToolName, ToolResource, ToolResourceAccess, ToolResourceError, ToolRetrySafety, ToolSpec,
    ToolSpecError, ToolTimeout, ToolVersion, ValidatedToolInvocation,
};

pub use command::{BashConfig, BashOutputDirectory, BashShell};

use crate::{FileToolError, FileToolErrorCode, WorkspaceRoot};

/// Workspace-confined bounded shell executor.
#[derive(Debug, Clone)]
pub struct BashTool {
    workspace: WorkspaceRoot,
    config: BashConfig,
}

impl BashTool {
    /// Creates a shell executor from explicit host-owned configuration.
    #[must_use]
    pub const fn new(workspace: WorkspaceRoot, config: BashConfig) -> Self {
        Self { workspace, config }
    }

    /// Returns the workspace process resource declared by every invocation.
    ///
    /// # Errors
    ///
    /// Returns an error only if the static resource violates tool bounds.
    pub fn workspace_resource() -> Result<ToolResource, ToolResourceError> {
        ToolResource::new("file", "/workspace", ToolResourceAccess::Execute)
    }

    /// Builds the portable `bash` contract.
    ///
    /// # Errors
    ///
    /// Returns an error only if the static contract violates tool bounds.
    pub fn spec() -> Result<ToolSpec, ToolSpecError> {
        ToolSpec::new(
            ToolName::from_str("bash").map_err(|_| ToolSpecError::InvalidDescription)?,
            ToolVersion::from_str("1.0.0").map_err(|_| ToolSpecError::InvalidDescription)?,
            "Run a command through the configured shell in the workspace.",
            json!({"type":"object","properties":{"command":{"type":"string","minLength":1,"maxLength":65536}},"required":["command"],"additionalProperties":false}),
            json!({"type":"object","properties":{"stdout":{"type":"string"},"stderr":{"type":"string"},"exitCode":{"type":["integer","null"]},"success":{"type":"boolean"},"truncated":{"type":"boolean"},"overflowReference":{"type":["string","null"]}},"required":["stdout","stderr","exitCode","success","truncated","overflowReference"],"additionalProperties":false}),
            [ToolEffect::ProcessSpawn],
            ToolExecutionSemantics::new(
                ToolIdempotency::NonIdempotent,
                ToolRetrySafety::Never,
                ToolConcurrency::Serial,
                ToolTimeout::from_millis(120_000)?,
            )?,
        )?
        .with_prompt_hint("Commands run in the workspace through a host-configured shell; cancellation after spawn is uncertain.")
    }
}

impl ToolExecutor for BashTool {
    fn execute(
        &self,
        invocation: ValidatedToolInvocation,
        cancellation: CancellationScope,
    ) -> BoxToolExecutionStream {
        let command = invocation
            .arguments()
            .get("command")
            .and_then(serde_json::Value::as_str)
            .filter(|command| !command.is_empty() && command.len() <= 65_536)
            .map(str::to_owned);
        match command {
            Some(command) => process::execute_process(
                self.workspace.clone(),
                self.config.clone(),
                command,
                cancellation,
            ),
            None => Box::pin(futures_util::stream::iter([
                tea_tools::ToolExecutionEvent::Failed(
                    tea_tools::ToolExecutionFailure::execution(
                        FileToolError::new(FileToolErrorCode::InvalidArguments).message(),
                    )
                    .unwrap_or_else(|_| tea_tools::ToolExecutionFailure::internal_contract()),
                ),
            ])),
        }
    }
}
