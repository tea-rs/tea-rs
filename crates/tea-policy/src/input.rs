use serde_json::Value;
use tea_protocol::{ProfileId, ProtocolTimestamp, RunId, SessionId, ToolCallId};
use tea_tools::{
    ToolEffect, ToolName, ToolResource, ToolSource, ToolVersion, ValidatedToolInvocation,
};
use thiserror::Error;

use crate::{ActorId, PolicyEnvironment, PolicyGrant, WorkspaceId};

/// Maximum grant candidates considered for one policy evaluation.
pub const MAX_POLICY_GRANTS: usize = 128;

/// Immutable, fully validated snapshot supplied to pure policy rules.
#[derive(Debug, Clone)]
pub struct PolicyInput {
    actor_id: ActorId,
    profile_id: ProfileId,
    session_id: SessionId,
    run_id: Option<RunId>,
    workspace_id: Option<WorkspaceId>,
    tool_call_id: ToolCallId,
    tool_name: ToolName,
    tool_version: ToolVersion,
    tool_source: ToolSource,
    arguments: Value,
    effects: Vec<ToolEffect>,
    resources: Vec<ToolResource>,
    environment: PolicyEnvironment,
    now: ProtocolTimestamp,
    grants: Vec<PolicyGrant>,
}

impl PolicyInput {
    /// Creates a policy snapshot from a registry-validated invocation.
    ///
    /// # Errors
    ///
    /// Returns an error when grant candidates exceed the deterministic bound.
    #[allow(clippy::too_many_arguments)]
    pub fn from_validated(
        actor_id: ActorId,
        profile_id: ProfileId,
        session_id: SessionId,
        run_id: Option<RunId>,
        workspace_id: Option<WorkspaceId>,
        invocation: &ValidatedToolInvocation,
        environment: PolicyEnvironment,
        now: ProtocolTimestamp,
        grants: impl IntoIterator<Item = PolicyGrant>,
    ) -> Result<Self, PolicyInputError> {
        let mut grants = grants.into_iter().collect::<Vec<_>>();
        grants.sort_by_key(PolicyGrant::id);
        grants.dedup_by_key(|grant| grant.id());
        if grants.len() > MAX_POLICY_GRANTS {
            return Err(PolicyInputError::TooManyGrants);
        }
        Ok(Self {
            actor_id,
            profile_id,
            session_id,
            run_id,
            workspace_id,
            tool_call_id: *invocation.tool_call_id(),
            tool_name: invocation.name().clone(),
            tool_version: invocation.spec().version().clone(),
            tool_source: invocation.source().clone(),
            arguments: invocation.arguments().clone(),
            effects: invocation.spec().effects().to_vec(),
            resources: invocation.resources().to_vec(),
            environment,
            now,
            grants,
        })
    }

    /// Returns actor identity.
    #[must_use]
    pub const fn actor_id(&self) -> &ActorId {
        &self.actor_id
    }
    /// Returns active profile.
    #[must_use]
    pub const fn profile_id(&self) -> &ProfileId {
        &self.profile_id
    }
    /// Returns active session.
    #[must_use]
    pub const fn session_id(&self) -> &SessionId {
        &self.session_id
    }
    /// Returns active run when present.
    #[must_use]
    pub const fn run_id(&self) -> Option<&RunId> {
        self.run_id.as_ref()
    }
    /// Returns workspace when present.
    #[must_use]
    pub const fn workspace_id(&self) -> Option<&WorkspaceId> {
        self.workspace_id.as_ref()
    }
    /// Returns canonical tool-call ID.
    #[must_use]
    pub const fn tool_call_id(&self) -> &ToolCallId {
        &self.tool_call_id
    }
    /// Returns registered tool name.
    #[must_use]
    pub const fn tool_name(&self) -> &ToolName {
        &self.tool_name
    }
    /// Returns registered tool version.
    #[must_use]
    pub const fn tool_version(&self) -> &ToolVersion {
        &self.tool_version
    }
    /// Returns frozen tool-source provenance.
    #[must_use]
    pub const fn tool_source(&self) -> &ToolSource {
        &self.tool_source
    }
    /// Returns schema-validated arguments.
    #[must_use]
    pub const fn arguments(&self) -> &Value {
        &self.arguments
    }
    /// Returns sorted effects.
    #[must_use]
    pub fn effects(&self) -> &[ToolEffect] {
        &self.effects
    }
    /// Returns sorted resolved resources.
    #[must_use]
    pub fn resources(&self) -> &[ToolResource] {
        &self.resources
    }
    /// Returns execution environment.
    #[must_use]
    pub const fn environment(&self) -> &PolicyEnvironment {
        &self.environment
    }
    /// Returns caller-supplied evaluation time.
    #[must_use]
    pub const fn now(&self) -> ProtocolTimestamp {
        self.now
    }
    /// Returns deduplicated grant candidates.
    #[must_use]
    pub fn grants(&self) -> &[PolicyGrant] {
        &self.grants
    }
}

/// Error constructing policy input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PolicyInputError {
    /// Too many grant candidates were supplied.
    #[error("policy input contains too many grants")]
    TooManyGrants,
}
