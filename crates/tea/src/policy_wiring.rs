use std::sync::Arc;

use tea_policy::{PolicyEngine, PolicyLayer, PolicyRule, PolicyRuleError};
use tea_profile::ProfileRuleId;

/// Shared policy rule backing one or more per-profile engines by delegation.
///
/// The runtime stores concrete rules as `Arc<dyn PolicyRule>` and registers a
/// `SharedPolicyRule` clone into each per-profile `PolicyEngine`. This avoids
/// modifying `tea-policy`: the wrapper is `PolicyRule + 'static` and
/// delegates `id`, `layer`, and `evaluate`.
#[derive(Debug, Clone)]
pub(crate) struct SharedPolicyRule(Arc<dyn PolicyRule>);

impl SharedPolicyRule {
    /// Wraps one shared rule instance.
    pub(crate) fn new(rule: Arc<dyn PolicyRule>) -> Self {
        Self(rule)
    }
}

impl PolicyRule for SharedPolicyRule {
    fn id(&self) -> &str {
        self.0.id()
    }
    fn layer(&self) -> PolicyLayer {
        self.0.layer()
    }
    fn evaluate(&self, input: &tea_policy::PolicyInput) -> tea_policy::PolicyRuleDecision {
        self.0.evaluate(input)
    }
}

/// Registered policy rule keyed by its canonical `ProfileRuleId`.
#[derive(Debug)]
pub(crate) struct RegisteredPolicyRule {
    pub(crate) id: ProfileRuleId,
    pub(crate) rule: Arc<dyn PolicyRule>,
}

/// Builds a `PolicyEngine` from resolved rules, ensuring the platform
/// fail-closed `UnknownEffectPolicy` is present exactly once.
pub(crate) fn build_engine(rules: &[SharedPolicyRule]) -> Result<PolicyEngine, PolicyRuleError> {
    let mut engine = PolicyEngine::new();
    let mut has_platform = false;
    for rule in rules {
        if rule.id() == tea_policy::UnknownEffectPolicy.id() {
            has_platform = true;
        }
        engine.add_rule(rule.clone())?;
    }
    if !has_platform {
        engine.add_rule(tea_policy::UnknownEffectPolicy)?;
    }
    Ok(engine)
}
