use tea_policy::{PolicyDecision, PolicyExecutionTarget};
use tea_protocol::ExecutionTarget;
use tea_tools::ToolSourceKind;

pub(crate) fn durable_decision(decision: &PolicyDecision) -> tea_protocol::PolicyDecision {
    match decision {
        PolicyDecision::Allow | PolicyDecision::Redirect { .. } => {
            tea_protocol::PolicyDecision::Allow
        }
        PolicyDecision::Ask(_) => tea_protocol::PolicyDecision::RequireApproval,
        PolicyDecision::Deny { .. } | PolicyDecision::HardDeny { .. } => {
            tea_protocol::PolicyDecision::Deny
        }
    }
}

pub(crate) const fn execution_target(target: PolicyExecutionTarget) -> ExecutionTarget {
    match target {
        PolicyExecutionTarget::Mcp => ExecutionTarget::Mcp,
        PolicyExecutionTarget::Remote => ExecutionTarget::Remote,
        PolicyExecutionTarget::Native
        | PolicyExecutionTarget::Subprocess
        | PolicyExecutionTarget::Sandbox
        | PolicyExecutionTarget::Wasm => ExecutionTarget::Native,
    }
}

pub(crate) const fn selected_target(
    decision: &PolicyDecision,
    configured: PolicyExecutionTarget,
    source: ToolSourceKind,
) -> ExecutionTarget {
    match source {
        ToolSourceKind::Mcp => return ExecutionTarget::Mcp,
        ToolSourceKind::Remote => return ExecutionTarget::Remote,
        ToolSourceKind::Native => {}
    }
    match decision {
        PolicyDecision::Redirect { target } => execution_target(*target),
        PolicyDecision::Allow
        | PolicyDecision::Ask(_)
        | PolicyDecision::Deny { .. }
        | PolicyDecision::HardDeny { .. } => execution_target(configured),
    }
}

pub(crate) fn denial_reason(decision: &PolicyDecision) -> Option<&str> {
    match decision {
        PolicyDecision::Deny { reason } | PolicyDecision::HardDeny { reason } => Some(reason),
        PolicyDecision::Allow | PolicyDecision::Redirect { .. } | PolicyDecision::Ask(_) => None,
    }
}
