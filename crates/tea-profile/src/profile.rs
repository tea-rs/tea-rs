use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tea_policy::PolicyEnvironment;
use tea_protocol::{ModelId, ModelRef, ProfileId, ProviderId};
use tea_tools::ToolName;

use crate::budget::ProfilePromptBudget;
use crate::identity::{ProfileDescription, ProfileDisplayName, ProfileRuleId, ProfileSegmentId};
use crate::limits::ProfileRunLimits;
use crate::workspace::ProfileWorkspaceInstruction;
use crate::{CURRENT_PROFILE_SCHEMA_VERSION, ProfileError, ProfileErrorCode, ProfileSchemaVersion};

/// Maximum active tool names carried by one profile.
pub const MAX_PROFILE_ACTIVE_TOOLS: usize = 256;
/// Maximum policy rule references carried by one profile.
pub const MAX_PROFILE_POLICY_RULES: usize = 128;
/// Maximum approval lifetime carried by any profile (24 hours).
pub const MAX_PROFILE_APPROVAL_TTL: Duration = Duration::from_hours(24);

/// Versioned, serializable product profile schema.
///
/// A profile is a declarative description of how one product configures the
/// runtime. It never carries trait objects, live adapters, or closures. The
/// runtime resolves its tool and policy rule references against builder-owned
/// registrations; an unresolved reference fails runtime construction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentProfile {
    #[serde(deserialize_with = "deserialize_supported_version")]
    schema_version: ProfileSchemaVersion,
    profile_id: ProfileId,
    display_name: ProfileDisplayName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<ProfileDescription>,
    model: ModelRef,
    active_tool_names: Vec<ToolName>,
    policy_rule_ids: Vec<ProfileRuleId>,
    prompt_budget: ProfilePromptBudget,
    run_limits: ProfileRunLimits,
    environment: PolicyEnvironment,
    approval_ttl_millis: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    workspace_instructions: Vec<ProfileWorkspaceInstruction>,
}

impl AgentProfile {
    /// Creates a profile builder seeded with the current schema version.
    pub fn builder(
        profile_id: ProfileId,
        display_name: ProfileDisplayName,
        model: ModelRef,
    ) -> AgentProfileBuilder {
        AgentProfileBuilder::new(profile_id, display_name, model)
    }

    /// Returns the schema version.
    #[must_use]
    pub const fn schema_version(&self) -> ProfileSchemaVersion {
        self.schema_version
    }
    /// Returns the canonical profile selector.
    #[must_use]
    pub fn profile_id(&self) -> &ProfileId {
        &self.profile_id
    }
    /// Returns the bounded display name.
    #[must_use]
    pub fn display_name(&self) -> &ProfileDisplayName {
        &self.display_name
    }
    /// Returns the optional bounded description.
    #[must_use]
    pub fn description(&self) -> Option<&ProfileDescription> {
        self.description.as_ref()
    }
    /// Returns the canonical model selector.
    #[must_use]
    pub fn model_id(&self) -> &ModelId {
        self.model.model_id()
    }
    /// Returns the canonical provider selector.
    #[must_use]
    pub const fn provider_id(&self) -> &ProviderId {
        self.model.provider_id()
    }
    /// Returns the provider-qualified model selector.
    #[must_use]
    pub const fn model_ref(&self) -> &ModelRef {
        &self.model
    }
    /// Returns ordered unique active tool selectors.
    #[must_use]
    pub fn active_tool_names(&self) -> &[ToolName] {
        &self.active_tool_names
    }
    /// Returns ordered unique policy rule references.
    #[must_use]
    pub fn policy_rule_ids(&self) -> &[ProfileRuleId] {
        &self.policy_rule_ids
    }
    /// Returns the declared prompt budget.
    #[must_use]
    pub const fn prompt_budget(&self) -> ProfilePromptBudget {
        self.prompt_budget
    }
    /// Returns the declared run limits.
    #[must_use]
    pub const fn run_limits(&self) -> ProfileRunLimits {
        self.run_limits
    }
    /// Returns the declared policy environment.
    #[must_use]
    pub fn environment(&self) -> &PolicyEnvironment {
        &self.environment
    }
    /// Returns the approval lifetime assigned to new approval requests.
    #[must_use]
    pub fn approval_ttl(&self) -> Duration {
        Duration::from_millis(self.approval_ttl_millis)
    }
    /// Returns the declared workspace instructions.
    #[must_use]
    pub fn workspace_instructions(&self) -> &[ProfileWorkspaceInstruction] {
        &self.workspace_instructions
    }
}

/// Ergonomic validated builder for [`AgentProfile`].
#[derive(Debug, Clone)]
#[must_use]
pub struct AgentProfileBuilder {
    profile_id: ProfileId,
    display_name: ProfileDisplayName,
    description: Option<ProfileDescription>,
    model: ModelRef,
    active_tool_names: Vec<ToolName>,
    policy_rule_ids: Vec<ProfileRuleId>,
    prompt_budget: ProfilePromptBudget,
    run_limits: ProfileRunLimits,
    environment: Option<PolicyEnvironment>,
    approval_ttl: Duration,
    workspace_instructions: Vec<ProfileWorkspaceInstruction>,
}

impl AgentProfileBuilder {
    /// Creates a builder seeded with conservative defaults.
    pub fn new(profile_id: ProfileId, display_name: ProfileDisplayName, model: ModelRef) -> Self {
        Self {
            profile_id,
            display_name,
            description: None,
            model,
            active_tool_names: Vec::new(),
            policy_rule_ids: Vec::new(),
            prompt_budget: ProfilePromptBudget::default(),
            run_limits: ProfileRunLimits::default(),
            environment: None,
            approval_ttl: Duration::from_mins(10),
            workspace_instructions: Vec::new(),
        }
    }

    /// Adds an optional bounded description.
    pub fn description(mut self, description: ProfileDescription) -> Self {
        self.description = Some(description);
        self
    }

    /// Adds one active tool selector. Duplicates are rejected at build time.
    pub fn active_tool(mut self, name: ToolName) -> Self {
        self.active_tool_names.push(name);
        self
    }

    /// Adds one policy rule reference. Duplicates are rejected at build time.
    pub fn policy_rule(mut self, id: ProfileRuleId) -> Self {
        self.policy_rule_ids.push(id);
        self
    }

    /// Replaces the default prompt budget.
    pub fn prompt_budget(mut self, budget: ProfilePromptBudget) -> Self {
        self.prompt_budget = budget;
        self
    }

    /// Replaces the default run limits.
    pub fn run_limits(mut self, limits: ProfileRunLimits) -> Self {
        self.run_limits = limits;
        self
    }

    /// Sets the policy environment.
    pub fn environment(mut self, environment: PolicyEnvironment) -> Self {
        self.environment = Some(environment);
        self
    }

    /// Replaces the default approval lifetime.
    pub fn approval_ttl(mut self, ttl: Duration) -> Self {
        self.approval_ttl = ttl;
        self
    }

    /// Adds one workspace instruction. Duplicate segment ids are rejected.
    pub fn workspace_instruction(mut self, instruction: ProfileWorkspaceInstruction) -> Self {
        self.workspace_instructions.push(instruction);
        self
    }

    /// Builds and validates the profile.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported schema versions, duplicate tools or
    /// rules, duplicate workspace instruction segment ids, an unset
    /// environment, or an unsupported approval lifetime.
    pub fn build(self) -> Result<AgentProfile, ProfileError> {
        let environment = self.environment.ok_or_else(|| {
            ProfileError::new(
                ProfileErrorCode::InvalidSelector,
                "profile environment is required",
            )
        })?;
        if self.approval_ttl.is_zero() || self.approval_ttl > MAX_PROFILE_APPROVAL_TTL {
            return Err(ProfileError::new(
                ProfileErrorCode::UnsupportedValue,
                "profile approval lifetime is invalid",
            ));
        }
        let active_tool_names = canonical_unique(self.active_tool_names, || {
            ProfileError::new(
                ProfileErrorCode::DuplicateEntry,
                "active tool name is duplicated",
            )
        })?;
        if active_tool_names.len() > MAX_PROFILE_ACTIVE_TOOLS {
            return Err(ProfileError::new(
                ProfileErrorCode::BoundsExceeded,
                "profile declares too many active tools",
            ));
        }
        let policy_rule_ids = canonical_unique(self.policy_rule_ids, || {
            ProfileError::new(
                ProfileErrorCode::DuplicateEntry,
                "policy rule reference is duplicated",
            )
        })?;
        if policy_rule_ids.len() > MAX_PROFILE_POLICY_RULES {
            return Err(ProfileError::new(
                ProfileErrorCode::BoundsExceeded,
                "profile declares too many policy rules",
            ));
        }
        if self.workspace_instructions.len() > crate::workspace::MAX_PROFILE_WORKSPACE_INSTRUCTIONS
        {
            return Err(ProfileError::new(
                ProfileErrorCode::BoundsExceeded,
                "profile declares too many workspace instructions",
            ));
        }
        let mut workspace_instructions = self.workspace_instructions;
        workspace_instructions.sort_by(|left, right| left.segment_id().cmp(right.segment_id()));
        if workspace_instructions
            .windows(2)
            .any(|items| items[0].segment_id() == items[1].segment_id())
        {
            return Err(ProfileError::new(
                ProfileErrorCode::DuplicateEntry,
                "workspace instruction segment id is duplicated",
            ));
        }
        let approval_ttl_millis = self.approval_ttl.as_millis().try_into().map_err(|_| {
            ProfileError::new(
                ProfileErrorCode::UnsupportedValue,
                "profile approval lifetime is out of range",
            )
        })?;
        Ok(AgentProfile {
            schema_version: CURRENT_PROFILE_SCHEMA_VERSION,
            profile_id: self.profile_id,
            display_name: self.display_name,
            description: self.description,
            model: self.model,
            active_tool_names,
            policy_rule_ids,
            prompt_budget: self.prompt_budget,
            run_limits: self.run_limits,
            environment,
            approval_ttl_millis,
            workspace_instructions,
        })
    }
}

fn canonical_unique<T: Ord + Clone>(
    mut values: Vec<T>,
    duplicate_error: impl Fn() -> ProfileError,
) -> Result<Vec<T>, ProfileError> {
    values.sort();
    if values.windows(2).any(|window| window[0] == window[1]) {
        return Err(duplicate_error());
    }
    Ok(values)
}

/// Parses a canonical [`ProfileSegmentId`] from a string for callers building
/// workspace instructions.
///
/// # Errors
///
/// Returns an error for non-canonical selectors.
pub fn parse_segment_id(
    value: &str,
) -> Result<ProfileSegmentId, crate::identity::ProfileSelectorError> {
    ProfileSegmentId::from_str(value)
}

/// Parses a canonical [`ProfileRuleId`] from `layer.id` text.
///
/// # Errors
///
/// Returns an error for non-canonical rule references.
pub fn parse_rule_id(value: &str) -> Result<ProfileRuleId, crate::identity::ProfileSelectorError> {
    ProfileRuleId::from_str(value)
}

fn deserialize_supported_version<'de, D>(deserializer: D) -> Result<ProfileSchemaVersion, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let version = ProfileSchemaVersion::deserialize(deserializer)?;
    if !version.is_supported() {
        return Err(serde::de::Error::custom(format!(
            "unsupported profile schema version {version}"
        )));
    }
    Ok(version)
}
