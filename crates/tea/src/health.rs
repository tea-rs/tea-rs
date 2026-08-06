use tea_profile::ProfileRuleId;
use tea_protocol::{ModelId, ModelRef, ProfileId, ProviderId};

/// Immutable runtime health and configuration summary.
///
/// Health inspection performs no I/O and never enumerates the session store.
/// It reports only what the runtime owns and tracks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeHealth {
    provider_ids: Vec<ProviderId>,
    model_refs: Vec<ModelRef>,
    model_ids: Vec<ModelId>,
    profile_ids: Vec<ProfileId>,
    policy_rule_ids: Vec<ProfileRuleId>,
    tool_count: usize,
    session_count: usize,
}

impl RuntimeHealth {
    /// Creates an empty health snapshot for an unconfigured runtime.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Creates a health snapshot from owned configuration values.
    #[must_use]
    pub fn new(
        provider_ids: Vec<ProviderId>,
        model_refs: Vec<ModelRef>,
        profile_ids: Vec<ProfileId>,
        policy_rule_ids: Vec<ProfileRuleId>,
        tool_count: usize,
        session_count: usize,
    ) -> Self {
        let model_ids = model_refs
            .iter()
            .map(|model| model.model_id().clone())
            .collect();
        Self {
            provider_ids,
            model_refs,
            model_ids,
            profile_ids,
            policy_rule_ids,
            tool_count,
            session_count,
        }
    }

    /// Returns the provider id only when exactly one provider is registered.
    #[must_use]
    pub fn provider_id(&self) -> Option<&str> {
        (self.provider_ids.len() == 1).then(|| self.provider_ids[0].as_str())
    }
    /// Returns registered provider identities in canonical order.
    #[must_use]
    pub fn provider_ids(&self) -> &[ProviderId] {
        &self.provider_ids
    }
    /// Returns complete model identities in stable order.
    #[must_use]
    pub fn model_refs(&self) -> &[ModelRef] {
        &self.model_refs
    }
    /// Returns advertised model selectors in stable order.
    #[must_use]
    pub fn model_ids(&self) -> &[ModelId] {
        &self.model_ids
    }
    /// Returns the count of advertised models.
    #[must_use]
    pub fn model_count(&self) -> usize {
        self.model_refs.len()
    }
    /// Returns registered profile selectors in stable order.
    #[must_use]
    pub fn profile_ids(&self) -> &[ProfileId] {
        &self.profile_ids
    }
    /// Returns registered policy rule ids in stable order.
    #[must_use]
    pub fn policy_rule_ids(&self) -> &[ProfileRuleId] {
        &self.policy_rule_ids
    }
    /// Returns the count of registered master tools.
    #[must_use]
    pub const fn tool_count(&self) -> usize {
        self.tool_count
    }
    /// Returns the count of sessions created through this runtime.
    #[must_use]
    pub const fn session_count(&self) -> usize {
        self.session_count
    }
}
