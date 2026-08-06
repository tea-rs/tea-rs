use std::str::FromStr;
use std::time::Duration;

use tea_policy::PolicyEnvironment;
use tea_tools::ToolName;

use crate::budget::ProfilePromptBudget;
use crate::identity::{ProfileRuleId, ProfileTrustLevel};
use crate::limits::ProfileRunLimits;
use crate::workspace::ProfileWorkspaceInstruction;
use crate::{
    AgentProfile, CURRENT_PROFILE_SCHEMA_VERSION, ProfileError, ProfileErrorCode,
    ProfileSchemaVersion,
};

/// Optional override fields applied on top of a base [`AgentProfile`].
///
/// Every field is `Option`; `None` inherits the base value wholesale. `Some`
/// replaces the entire field (limits, budget, environment, or instruction list)
/// without per-field partial merging. This keeps composition deterministic and
/// auditable.
#[derive(Debug, Clone, Default, PartialEq)]
#[must_use]
pub struct ProfileOverlay {
    schema_version: Option<ProfileSchemaVersion>,
    profile_id: Option<tea_protocol::ProfileId>,
    display_name: Option<crate::identity::ProfileDisplayName>,
    #[allow(clippy::option_option)]
    description: Option<Option<crate::identity::ProfileDescription>>,
    model: Option<tea_protocol::ModelRef>,
    active_tool_names: Option<Vec<ToolName>>,
    policy_rule_ids: Option<Vec<ProfileRuleId>>,
    prompt_budget: Option<ProfilePromptBudget>,
    run_limits: Option<ProfileRunLimits>,
    environment: Option<PolicyEnvironment>,
    approval_ttl: Option<Duration>,
    workspace_instructions: Option<Vec<ProfileWorkspaceInstruction>>,
}

impl ProfileOverlay {
    /// Creates an empty overlay.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the schema version. A mismatched version composes into an error.
    pub fn schema_version(mut self, version: ProfileSchemaVersion) -> Self {
        self.schema_version = Some(version);
        self
    }
    /// Replaces the profile selector. A selector different from the base is an error.
    pub fn profile_id(mut self, id: tea_protocol::ProfileId) -> Self {
        self.profile_id = Some(id);
        self
    }
    /// Replaces the display name.
    pub fn display_name(mut self, name: crate::identity::ProfileDisplayName) -> Self {
        self.display_name = Some(name);
        self
    }
    /// Replaces, clears, or inherits the description.
    pub fn description(mut self, value: Option<crate::identity::ProfileDescription>) -> Self {
        self.description = Some(value);
        self
    }
    /// Replaces the model selector.
    pub fn model(mut self, model: tea_protocol::ModelRef) -> Self {
        self.model = Some(model);
        self
    }
    /// Replaces the entire active tool list.
    pub fn active_tool_names(mut self, names: Vec<ToolName>) -> Self {
        self.active_tool_names = Some(names);
        self
    }
    /// Replaces the entire policy rule reference list.
    pub fn policy_rule_ids(mut self, ids: Vec<ProfileRuleId>) -> Self {
        self.policy_rule_ids = Some(ids);
        self
    }
    /// Replaces the prompt budget.
    pub fn prompt_budget(mut self, budget: ProfilePromptBudget) -> Self {
        self.prompt_budget = Some(budget);
        self
    }
    /// Replaces the run limits.
    pub fn run_limits(mut self, limits: ProfileRunLimits) -> Self {
        self.run_limits = Some(limits);
        self
    }
    /// Replaces the policy environment.
    pub fn environment(mut self, environment: PolicyEnvironment) -> Self {
        self.environment = Some(environment);
        self
    }
    /// Replaces the approval lifetime.
    pub fn approval_ttl(mut self, ttl: Duration) -> Self {
        self.approval_ttl = Some(ttl);
        self
    }
    /// Replaces the entire workspace instruction list.
    pub fn workspace_instructions(
        mut self,
        instructions: Vec<ProfileWorkspaceInstruction>,
    ) -> Self {
        self.workspace_instructions = Some(instructions);
        self
    }
}

impl AgentProfile {
    /// Composes this profile with an overlay, revalidating the result.
    ///
    /// `None` overlay fields inherit the base. A schema-version mismatch or a
    /// profile-id mismatch returns a [`ProfileErrorCode::CompositionConflict`].
    ///
    /// # Errors
    ///
    /// Returns an error for composition conflicts or for any invalid
    /// combination of inherited and overridden values.
    pub fn compose(&self, overlay: &ProfileOverlay) -> Result<AgentProfile, ProfileError> {
        let schema_version = overlay.schema_version.unwrap_or(self.schema_version());
        if schema_version != CURRENT_PROFILE_SCHEMA_VERSION
            || self.schema_version() != CURRENT_PROFILE_SCHEMA_VERSION
        {
            return Err(ProfileError::new(
                ProfileErrorCode::CompositionConflict,
                "composed schema version does not match the base",
            ));
        }
        let profile_id = overlay
            .profile_id
            .clone()
            .unwrap_or_else(|| self.profile_id().clone());
        if profile_id != *self.profile_id() {
            return Err(ProfileError::new(
                ProfileErrorCode::CompositionConflict,
                "composed profile id does not match the base",
            ));
        }
        let description = match &overlay.description {
            Some(value) => value.clone(),
            None => self.description().cloned(),
        };
        let active_tool_names = overlay
            .active_tool_names
            .clone()
            .unwrap_or_else(|| self.active_tool_names().to_vec());
        let policy_rule_ids = overlay
            .policy_rule_ids
            .clone()
            .unwrap_or_else(|| self.policy_rule_ids().to_vec());
        let workspace_instructions = overlay
            .workspace_instructions
            .clone()
            .unwrap_or_else(|| self.workspace_instructions().to_vec());
        let mut builder = AgentProfile::builder(
            profile_id,
            overlay
                .display_name
                .clone()
                .unwrap_or_else(|| self.display_name().clone()),
            overlay
                .model
                .clone()
                .unwrap_or_else(|| self.model_ref().clone()),
        );
        if let Some(description) = description {
            builder = builder.description(description);
        }
        for name in active_tool_names {
            builder = builder.active_tool(name);
        }
        for id in policy_rule_ids {
            builder = builder.policy_rule(id);
        }
        for instruction in workspace_instructions {
            builder = builder.workspace_instruction(instruction);
        }
        builder = builder
            .prompt_budget(overlay.prompt_budget.unwrap_or(self.prompt_budget()))
            .run_limits(overlay.run_limits.unwrap_or(self.run_limits()))
            .environment(
                overlay
                    .environment
                    .clone()
                    .unwrap_or_else(|| self.environment().clone()),
            )
            .approval_ttl(overlay.approval_ttl.unwrap_or(self.approval_ttl()));
        builder.build()
    }

    /// Minimal read-only assistant with no tools and conservative limits.
    ///
    /// # Errors
    ///
    /// Returns an error only when a built-in selector is invalid.
    pub fn minimal_assistant() -> Result<AgentProfile, ProfileError> {
        Self::builder(
            tea_protocol::ProfileId::from_str("minimal-assistant").map_err(invalid)?,
            crate::identity::ProfileDisplayName::new("Minimal Assistant")?,
            fake_model()?,
        )
        .prompt_budget(ProfilePromptBudget::new(16_384, 4_096)?)
        .run_limits(ProfileRunLimits::new(
            4,
            Duration::from_mins(1),
            512 * 1024,
            10_000,
            16,
        )?)
        .environment(cli_environment())
        .approval_ttl(Duration::from_mins(5))
        .build()
    }

    /// Coding agent with read/write tools and the coding-workspace policy.
    ///
    /// # Errors
    ///
    /// Returns an error only when a built-in selector is invalid.
    pub fn coding_agent() -> Result<AgentProfile, ProfileError> {
        Self::builder(
            tea_protocol::ProfileId::from_str("coding-agent").map_err(invalid)?,
            crate::identity::ProfileDisplayName::new("Coding Agent")?,
            fake_model()?,
        )
        .active_tool(ToolName::from_str("read_file").map_err(invalid)?)
        .active_tool(ToolName::from_str("write_file").map_err(invalid)?)
        .policy_rule(ProfileRuleId::from_str("product.coding_workspace").map_err(invalid)?)
        .policy_rule(ProfileRuleId::from_str("platform.unknown_effect").map_err(invalid)?)
        .prompt_budget(ProfilePromptBudget::new(32_768, 8_192)?)
        .run_limits(ProfileRunLimits::new(
            16,
            Duration::from_mins(5),
            4 * 1024 * 1024,
            100_000,
            64,
        )?)
        .environment(cli_environment())
        .approval_ttl(Duration::from_mins(10))
        .build()
    }

    /// Desktop assistant with clipboard and write tools and the desktop policy.
    ///
    /// # Errors
    ///
    /// Returns an error only when a built-in selector is invalid.
    pub fn desktop_assistant() -> Result<AgentProfile, ProfileError> {
        Self::builder(
            tea_protocol::ProfileId::from_str("desktop-assistant").map_err(invalid)?,
            crate::identity::ProfileDisplayName::new("Desktop Assistant")?,
            fake_model()?,
        )
        .active_tool(ToolName::from_str("clipboard_read").map_err(invalid)?)
        .active_tool(ToolName::from_str("write_file").map_err(invalid)?)
        .policy_rule(ProfileRuleId::from_str("product.desktop").map_err(invalid)?)
        .policy_rule(ProfileRuleId::from_str("platform.unknown_effect").map_err(invalid)?)
        .prompt_budget(ProfilePromptBudget::new(24_576, 6_144)?)
        .run_limits(ProfileRunLimits::new(
            12,
            Duration::from_mins(3),
            2 * 1024 * 1024,
            50_000,
            32,
        )?)
        .environment(desktop_environment())
        .approval_ttl(Duration::from_mins(10))
        .build()
    }
}

fn fake_model() -> Result<tea_protocol::ModelRef, ProfileError> {
    Ok(tea_protocol::ModelRef::new(
        tea_protocol::ProviderId::from_str("fake").map_err(invalid)?,
        tea_protocol::ModelId::from_str("fake/model").map_err(invalid)?,
    ))
}

fn cli_environment() -> PolicyEnvironment {
    PolicyEnvironment::new(
        tea_policy::ExecutionSurface::Cli,
        tea_policy::PolicyExecutionTarget::Native,
        tea_protocol::ProtocolMetadata::default(),
    )
}

fn desktop_environment() -> PolicyEnvironment {
    PolicyEnvironment::new(
        tea_policy::ExecutionSurface::Desktop,
        tea_policy::PolicyExecutionTarget::Native,
        tea_protocol::ProtocolMetadata::default(),
    )
}

fn invalid<E: std::fmt::Display>(error: E) -> ProfileError {
    ProfileError::new(ProfileErrorCode::InvalidSelector, error.to_string())
}

/// Builds a reusable [`ProfileWorkspaceInstruction`] for example profiles.
///
/// # Errors
///
/// Returns an error for invalid content or locator values.
pub fn example_workspace_instruction(
    segment_id: &str,
    content: &str,
    locator: &str,
) -> Result<ProfileWorkspaceInstruction, ProfileError> {
    ProfileWorkspaceInstruction::new(
        crate::identity::ProfileSegmentId::from_str(segment_id).map_err(invalid)?,
        content,
        locator,
        ProfileTrustLevel::Trusted,
    )
}
