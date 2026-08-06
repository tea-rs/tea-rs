use std::fmt::Debug;

use thiserror::Error;

use crate::{GrantId, PolicyDecision, PolicyInput, PolicyRuleDecision};

const MAX_POLICY_RULES: usize = 128;

/// Fixed policy composition layer from highest to lowest authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PolicyLayer {
    /// Platform safety policy.
    Platform,
    /// Organization policy.
    Organization,
    /// Product/profile policy.
    Product,
    /// Workspace policy.
    Workspace,
}

/// Pure synchronous policy rule.
pub trait PolicyRule: Debug + Send + Sync {
    /// Returns canonical stable rule identifier.
    fn id(&self) -> &str;
    /// Returns fixed composition layer.
    fn layer(&self) -> PolicyLayer;
    /// Evaluates immutable input without side effects.
    fn evaluate(&self, input: &PolicyInput) -> PolicyRuleDecision;
}

/// One bounded safe trace entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyTraceEntry {
    rule_id: String,
    layer: PolicyLayer,
    decision: PolicyRuleDecision,
}

impl PolicyTraceEntry {
    /// Returns rule ID.
    #[must_use]
    pub fn rule_id(&self) -> &str {
        &self.rule_id
    }
    /// Returns rule layer.
    #[must_use]
    pub const fn layer(&self) -> PolicyLayer {
        self.layer
    }
    /// Returns bounded rule decision.
    #[must_use]
    pub const fn decision(&self) -> &PolicyRuleDecision {
        &self.decision
    }
}

/// Final decision, trace, and optional matching grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyEvaluation {
    decision: PolicyDecision,
    trace: Vec<PolicyTraceEntry>,
    matched_grant_id: Option<GrantId>,
}

impl PolicyEvaluation {
    /// Returns final decision.
    #[must_use]
    pub const fn decision(&self) -> &PolicyDecision {
        &self.decision
    }
    /// Returns ordered safe trace.
    #[must_use]
    pub fn trace(&self) -> &[PolicyTraceEntry] {
        &self.trace
    }
    /// Returns grant that satisfied approval, when any.
    #[must_use]
    pub const fn matched_grant_id(&self) -> Option<GrantId> {
        self.matched_grant_id
    }
}

/// Deterministic ordered policy engine.
#[derive(Debug, Default)]
pub struct PolicyEngine {
    rules: Vec<Box<dyn PolicyRule>>,
}

impl PolicyEngine {
    /// Creates an empty fail-closed engine.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a rule and sorts by layer while preserving insertion order within a layer.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid or duplicate rule IDs.
    pub fn add_rule<R>(&mut self, rule: R) -> Result<(), PolicyRuleError>
    where
        R: PolicyRule + 'static,
    {
        validate_rule_id(rule.id())?;
        if self.rules.len() >= MAX_POLICY_RULES {
            return Err(PolicyRuleError::TooManyRules);
        }
        if self.rules.iter().any(|existing| existing.id() == rule.id()) {
            return Err(PolicyRuleError::DuplicateRuleId);
        }
        self.rules.push(Box::new(rule));
        self.rules.sort_by_key(|rule| rule.layer());
        Ok(())
    }

    /// Evaluates all rules in authority order and applies matching grants to Ask only.
    #[must_use]
    pub fn evaluate(&self, input: &PolicyInput) -> PolicyEvaluation {
        let mut current: Option<PolicyDecision> = None;
        let mut trace = Vec::new();
        for rule in &self.rules {
            let rule_decision = rule.evaluate(input);
            let decision = rule_decision.clone().into_decision();
            trace.push(PolicyTraceEntry {
                rule_id: rule.id().to_owned(),
                layer: rule.layer(),
                decision: rule_decision,
            });
            if let Some(decision) = decision {
                if matches!(decision, PolicyDecision::HardDeny { .. }) {
                    return PolicyEvaluation {
                        decision,
                        trace,
                        matched_grant_id: None,
                    };
                }
                if current
                    .as_ref()
                    .is_none_or(|value| decision.restriction() > value.restriction())
                {
                    current = Some(decision);
                }
            }
        }
        let mut decision = current.unwrap_or_else(|| PolicyDecision::HardDeny {
            reason: "no policy rule authorized the invocation".to_owned(),
        });
        let mut matched_grant_id = None;
        if matches!(decision, PolicyDecision::Ask(_))
            && let Some(grant) = input.grants().iter().find(|grant| grant.matches(input))
        {
            decision = PolicyDecision::Allow;
            matched_grant_id = Some(grant.id());
        }
        PolicyEvaluation {
            decision,
            trace,
            matched_grant_id,
        }
    }
}

/// Rule registration error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PolicyRuleError {
    /// Rule ID is not canonical.
    #[error("policy rule ID is invalid")]
    InvalidRuleId,
    /// Rule ID already exists.
    #[error("policy rule ID is duplicated")]
    DuplicateRuleId,
    /// Engine already contains the maximum rule count.
    #[error("policy engine contains too many rules")]
    TooManyRules,
}

fn validate_rule_id(value: &str) -> Result<(), PolicyRuleError> {
    let mut bytes = value.bytes();
    if value.len() > 128
        || !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        || !bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
    {
        Err(PolicyRuleError::InvalidRuleId)
    } else {
        Ok(())
    }
}
