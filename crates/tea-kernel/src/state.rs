use thiserror::Error;

/// Explicit lifecycle state of one kernel run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    /// No operation has started.
    Idle,
    /// Durable session context is being loaded and frozen.
    PreparingContext,
    /// One immutable model request is being streamed.
    StreamingModel,
    /// Complete model tool calls are being committed and validated.
    PlanningToolCalls,
    /// A validated invocation is being evaluated by policy.
    EvaluatingPolicy,
    /// The run is durably paused for a caller decision.
    WaitingApproval,
    /// One authorized tool is executing.
    ExecutingTool,
    /// Durable turn output and checkpoint are being committed.
    CommittingTurn,
    /// The run completed normally.
    Completed,
    /// The run was cooperatively cancelled.
    Cancelled,
    /// The run was interrupted at a recoverable boundary.
    Interrupted,
    /// The run failed before a safe continuation boundary.
    Failed,
}

impl RunState {
    /// Validates and applies one explicit transition.
    ///
    /// # Errors
    ///
    /// Returns an error when the target is not legal from this state.
    pub fn transition(self, next: Self) -> Result<Self, StateTransitionError> {
        let valid = matches!(
            (self, next),
            (Self::Idle, Self::PreparingContext)
                | (
                    Self::PreparingContext | Self::CommittingTurn,
                    Self::StreamingModel
                )
                | (
                    Self::PreparingContext | Self::EvaluatingPolicy,
                    Self::WaitingApproval
                )
                | (
                    Self::StreamingModel,
                    Self::PlanningToolCalls | Self::CommittingTurn
                )
                | (
                    Self::PlanningToolCalls | Self::ExecutingTool,
                    Self::EvaluatingPolicy
                )
                | (
                    Self::PlanningToolCalls
                        | Self::EvaluatingPolicy
                        | Self::WaitingApproval
                        | Self::ExecutingTool,
                    Self::CommittingTurn
                )
                | (
                    Self::EvaluatingPolicy | Self::WaitingApproval,
                    Self::ExecutingTool
                )
                | (Self::CommittingTurn, Self::Completed)
                | (_, Self::Cancelled | Self::Interrupted | Self::Failed)
        );
        valid.then_some(next).ok_or(StateTransitionError)
    }

    /// Returns whether this state is terminal for the current invocation.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::WaitingApproval
                | Self::Completed
                | Self::Cancelled
                | Self::Interrupted
                | Self::Failed
        )
    }
}

/// Explicit lifecycle state of one model/tool turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnState {
    /// The immutable request has not been built.
    Preparing,
    /// The provider stream is active.
    Streaming,
    /// Complete tool declarations are being processed.
    ProcessingTools,
    /// A durable checkpoint has been committed.
    Checkpointed,
    /// The turn ended before a safe checkpoint.
    Interrupted,
}

/// Invalid runtime-state transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("kernel state transition is invalid")]
pub struct StateTransitionError;
