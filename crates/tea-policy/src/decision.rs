use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::PolicyExecutionTarget;

const MAX_POLICY_REASON_BYTES: usize = 4096;

/// Bounded reason for an interactive approval request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequirement {
    reason: String,
}

impl ApprovalRequirement {
    /// Creates a bounded technical approval reason.
    ///
    /// # Errors
    ///
    /// Returns an error for empty, oversized, or null-containing text.
    pub fn new(reason: impl Into<String>) -> Result<Self, PolicyDecisionError> {
        let reason = reason.into();
        validate_reason(&reason)?;
        Ok(Self { reason })
    }
    /// Returns technical reason.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// Final pure policy decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// Invocation is allowed.
    Allow,
    /// Invocation must use another execution adapter.
    Redirect {
        /// Required execution target.
        target: PolicyExecutionTarget,
    },
    /// Invocation requires interactive approval.
    Ask(ApprovalRequirement),
    /// Invocation is denied by a normal restrictive rule.
    Deny {
        /// Bounded English technical reason.
        reason: String,
    },
    /// Invocation is denied terminally by a high-authority rule.
    HardDeny {
        /// Bounded English technical reason.
        reason: String,
    },
}

impl PolicyDecision {
    pub(crate) const fn restriction(&self) -> u8 {
        match self {
            Self::Allow => 0,
            Self::Redirect { .. } => 1,
            Self::Ask(_) => 2,
            Self::Deny { .. } => 3,
            Self::HardDeny { .. } => 4,
        }
    }
}

/// Decision returned by one policy rule, including no-op abstention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRuleDecision(Option<PolicyDecision>);

impl PolicyRuleDecision {
    /// Creates abstention.
    #[must_use]
    pub const fn abstain() -> Self {
        Self(None)
    }
    /// Creates allow.
    #[must_use]
    pub const fn allow() -> Self {
        Self(Some(PolicyDecision::Allow))
    }
    /// Creates redirect.
    #[must_use]
    pub const fn redirect(target: PolicyExecutionTarget) -> Self {
        Self(Some(PolicyDecision::Redirect { target }))
    }
    /// Creates approval requirement.
    #[must_use]
    pub const fn ask(requirement: ApprovalRequirement) -> Self {
        Self(Some(PolicyDecision::Ask(requirement)))
    }
    /// Creates bounded deny.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid technical reason.
    pub fn deny(reason: impl Into<String>) -> Result<Self, PolicyDecisionError> {
        let reason = reason.into();
        validate_reason(&reason)?;
        Ok(Self(Some(PolicyDecision::Deny { reason })))
    }
    /// Creates bounded terminal hard deny.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid technical reason.
    pub fn hard_deny(reason: impl Into<String>) -> Result<Self, PolicyDecisionError> {
        let reason = reason.into();
        validate_reason(&reason)?;
        Ok(Self(Some(PolicyDecision::HardDeny { reason })))
    }
    /// Returns the rule's optional restriction.
    #[must_use]
    pub const fn decision(&self) -> Option<&PolicyDecision> {
        self.0.as_ref()
    }
    pub(crate) fn into_decision(self) -> Option<PolicyDecision> {
        self.0
    }
}

/// Error constructing policy decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PolicyDecisionError {
    /// Technical reason is empty, oversized, or contains null.
    #[error("policy decision reason is invalid")]
    InvalidReason,
}

fn validate_reason(value: &str) -> Result<(), PolicyDecisionError> {
    if value.is_empty() || value.len() > MAX_POLICY_REASON_BYTES || value.contains('\0') {
        Err(PolicyDecisionError::InvalidReason)
    } else {
        Ok(())
    }
}
