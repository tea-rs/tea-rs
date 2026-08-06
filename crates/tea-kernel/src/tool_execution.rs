use futures_util::StreamExt;
use tea_control::CancellationScope;
use tea_protocol::{
    AgentEvent, ContentBlock, RunId, ToolCallId, ToolFailure, ToolPresentation, TurnId,
};
use tea_tools::{
    ToolExecutionEvent, ToolExecutionFailure, ToolExecutionFailureCode, ToolRegistry,
    ToolStreamValidator, ValidatedToolInvocation,
};

use crate::observe::EventEmitter;
use crate::{KernelError, KernelErrorCode};

#[derive(Debug)]
pub(crate) struct ToolTerminal {
    pub(crate) content: Vec<ContentBlock>,
    pub(crate) failure: Option<ToolFailure>,
    pub(crate) presentation: Option<ToolPresentation>,
}

pub(crate) struct ToolExecutionContext<'a, 'b> {
    pub(crate) emitter: &'a mut EventEmitter<'b>,
    pub(crate) run_id: RunId,
    pub(crate) turn_id: TurnId,
    pub(crate) clock: &'a dyn crate::KernelClock,
    pub(crate) deadline: tea_protocol::ProtocolTimestamp,
}

pub(crate) async fn execute(
    tools: &ToolRegistry,
    invocation: ValidatedToolInvocation,
    cancellation: CancellationScope,
    context: &mut ToolExecutionContext<'_, '_>,
) -> Result<ToolTerminal, KernelError> {
    let tool_call_id = *invocation.tool_call_id();
    let mut stream = tools
        .execute_validated(invocation, cancellation.clone())
        .map_err(|error| KernelError::new(KernelErrorCode::ToolFailure, error.to_string()))?;
    let mut validator = ToolStreamValidator::new();
    let mut terminal = None;
    loop {
        let event = tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                return Err(KernelError::new(
                    KernelErrorCode::Cancelled,
                    "tool execution was cancelled after durable start",
                ));
            }
            () = context.clock.sleep_until(context.deadline) => {
                return Err(KernelError::new(
                    KernelErrorCode::LimitExceeded,
                    "run deadline was reached during tool execution",
                ));
            }
            event = stream.next() => event,
        };
        let Some(event) = event else { break };
        validator
            .observe(&event)
            .map_err(|error| KernelError::new(KernelErrorCode::ToolFailure, error.to_string()))?;
        match event {
            ToolExecutionEvent::Progress(progress) => {
                context
                    .emitter
                    .emit(
                        Some(context.run_id),
                        Some(context.turn_id),
                        AgentEvent::ToolExecutionProgress {
                            tool_call_id,
                            message: progress.message().to_owned(),
                            completed_units: progress.completed_units(),
                            total_units: progress.total_units(),
                        },
                    )
                    .await?;
            }
            ToolExecutionEvent::Finished(result) => {
                terminal = Some(ToolTerminal {
                    content: result.content().to_vec(),
                    failure: None,
                    presentation: result.presentation().cloned(),
                });
            }
            ToolExecutionEvent::Failed(failure) => {
                terminal = Some(ToolTerminal {
                    content: vec![ContentBlock::text(failure.message())?],
                    failure: Some(tool_failure(&failure)?),
                    presentation: None,
                });
            }
        }
    }
    validator
        .finish()
        .map_err(|error| KernelError::new(KernelErrorCode::ToolFailure, error.to_string()))?;
    terminal.ok_or_else(|| {
        KernelError::new(
            KernelErrorCode::ToolFailure,
            "tool stream has no terminal result",
        )
    })
}

pub(crate) fn failed_terminal(code: &str, message: &str) -> Result<ToolTerminal, KernelError> {
    Ok(ToolTerminal {
        content: vec![ContentBlock::text(message)?],
        failure: Some(ToolFailure::new(code, message)?),
        presentation: None,
    })
}

fn tool_failure(failure: &ToolExecutionFailure) -> Result<ToolFailure, KernelError> {
    let code = match failure.code() {
        ToolExecutionFailureCode::ExecutionFailed => "tool_execution_failed",
        ToolExecutionFailureCode::Cancelled => "tool_cancelled",
        ToolExecutionFailureCode::InvalidOutput => "invalid_tool_output",
        ToolExecutionFailureCode::Internal => "tool_internal_failure",
    };
    ToolFailure::new(code, failure.message())
        .map_err(|error| KernelError::new(KernelErrorCode::ToolFailure, error.to_string()))
}

/// One buffered progress observation collected during parallel execution.
#[derive(Debug, Clone)]
pub(crate) struct CollectedProgress {
    pub(crate) tool_call_id: ToolCallId,
    pub(crate) message: String,
    pub(crate) completed_units: u64,
    pub(crate) total_units: Option<u64>,
}

/// Result of polling one tool stream into a local collector without emitting.
#[derive(Debug)]
pub(crate) struct CollectedExecution {
    pub(crate) progress: Vec<CollectedProgress>,
    pub(crate) terminal: ToolTerminal,
}

/// Polls one validated invocation to a terminal without emitting observations.
///
/// Used for parallel-lane execution so progress can be emitted in canonical
/// source order after the lane completes, keeping the durable and observation
/// order independent of completion order.
pub(crate) async fn collect_execution(
    tools: &ToolRegistry,
    invocation: ValidatedToolInvocation,
    cancellation: CancellationScope,
    clock: &dyn crate::KernelClock,
    deadline: tea_protocol::ProtocolTimestamp,
) -> Result<CollectedExecution, KernelError> {
    let tool_call_id = *invocation.tool_call_id();
    let mut stream = tools
        .execute_validated(invocation, cancellation.clone())
        .map_err(|error| KernelError::new(KernelErrorCode::ToolFailure, error.to_string()))?;
    let mut validator = ToolStreamValidator::new();
    let mut progress = Vec::new();
    let mut terminal = None;
    loop {
        let event = tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                return Err(KernelError::new(
                    KernelErrorCode::Cancelled,
                    "tool execution was cancelled after durable start",
                ));
            }
            () = clock.sleep_until(deadline) => {
                return Err(KernelError::new(
                    KernelErrorCode::LimitExceeded,
                    "run deadline was reached during tool execution",
                ));
            }
            event = stream.next() => event,
        };
        let Some(event) = event else { break };
        validator
            .observe(&event)
            .map_err(|error| KernelError::new(KernelErrorCode::ToolFailure, error.to_string()))?;
        match event {
            ToolExecutionEvent::Progress(item) => {
                progress.push(CollectedProgress {
                    tool_call_id,
                    message: item.message().to_owned(),
                    completed_units: item.completed_units(),
                    total_units: item.total_units(),
                });
            }
            ToolExecutionEvent::Finished(result) => {
                terminal = Some(ToolTerminal {
                    content: result.content().to_vec(),
                    failure: None,
                    presentation: result.presentation().cloned(),
                });
            }
            ToolExecutionEvent::Failed(failure) => {
                terminal = Some(ToolTerminal {
                    content: vec![ContentBlock::text(failure.message())?],
                    failure: Some(tool_failure(&failure)?),
                    presentation: None,
                });
            }
        }
    }
    validator
        .finish()
        .map_err(|error| KernelError::new(KernelErrorCode::ToolFailure, error.to_string()))?;
    let terminal = terminal.ok_or_else(|| {
        KernelError::new(
            KernelErrorCode::ToolFailure,
            "tool stream has no terminal result",
        )
    })?;
    Ok(CollectedExecution { progress, terminal })
}
