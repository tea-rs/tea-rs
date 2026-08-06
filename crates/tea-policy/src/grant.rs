use serde::{Deserialize, Deserializer, Serialize};
use tea_protocol::{ProfileId, ProtocolTimestamp, RunId, SessionId, ToolCallId};
use tea_tools::{ToolEffect, ToolName, ToolResource, ToolResourceAccess, ToolSource, ToolVersion};
use thiserror::Error;

use crate::{ActorId, GrantId, PolicyInput};

const MAX_GRANT_EFFECTS: usize = 64;
const MAX_GRANT_PATTERNS: usize = 128;

/// Resource constraint attached to a policy grant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePattern {
    scheme: String,
    locator_prefix: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    access: Option<ToolResourceAccess>,
}

impl ResourcePattern {
    /// Creates a bounded resource-prefix matcher.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid schemes or empty/control/oversized prefixes.
    pub fn new(
        scheme: impl Into<String>,
        locator_prefix: impl Into<String>,
        access: Option<ToolResourceAccess>,
    ) -> Result<Self, PolicyGrantError> {
        let scheme = scheme.into();
        let locator_prefix = locator_prefix.into();
        let mut bytes = scheme.bytes();
        if scheme.len() > 64
            || !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            || !bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || locator_prefix.is_empty()
            || locator_prefix.len() > 2048
            || locator_prefix.chars().any(char::is_control)
        {
            return Err(PolicyGrantError::InvalidResourcePattern);
        }
        Ok(Self {
            scheme,
            locator_prefix,
            access,
        })
    }

    /// Returns whether this pattern contains one resolved resource.
    #[must_use]
    pub fn matches(&self, resource: &ToolResource) -> bool {
        self.scheme == resource.scheme()
            && resource.locator().starts_with(&self.locator_prefix)
            && self.access.is_none_or(|access| access == resource.access())
    }

    /// Returns resource scheme.
    #[must_use]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }
    /// Returns locator prefix.
    #[must_use]
    pub fn locator_prefix(&self) -> &str {
        &self.locator_prefix
    }
    /// Returns optional exact access constraint.
    #[must_use]
    pub const fn access(&self) -> Option<ToolResourceAccess> {
        self.access
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawResourcePattern {
    scheme: String,
    locator_prefix: String,
    #[serde(default)]
    access: Option<ToolResourceAccess>,
}

impl<'de> Deserialize<'de> for ResourcePattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawResourcePattern::deserialize(deserializer)?;
        Self::new(raw.scheme, raw.locator_prefix, raw.access).map_err(serde::de::Error::custom)
    }
}

/// Lifetime/context scope of one policy grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GrantScope {
    /// Grant matches exactly one tool call.
    Once {
        /// Authorized tool call.
        tool_call_id: ToolCallId,
    },
    /// Grant matches invocations in one run.
    Run {
        /// Authorized run.
        run_id: RunId,
    },
    /// Resource grant lasts for one session.
    SessionResource {
        /// Authorized session.
        session_id: SessionId,
    },
    /// Resource grant is reusable until an explicit expiry.
    PersistentResource {
        /// Mandatory expiration boundary.
        expires_at: ProtocolTimestamp,
    },
}

/// Serializable, bounded policy grant candidate.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyGrant {
    id: GrantId,
    actor_id: ActorId,
    profile_id: ProfileId,
    tool_name: ToolName,
    tool_version: ToolVersion,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_source: Option<ToolSource>,
    effects: Vec<ToolEffect>,
    resources: Vec<ResourcePattern>,
    scope: GrantScope,
    issued_at: ProtocolTimestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    revoked_at: Option<ProtocolTimestamp>,
}

impl PolicyGrant {
    /// Creates a grant bounded across actor, profile, tool, effects, resources, and scope.
    ///
    /// # Errors
    ///
    /// Returns an error for empty/oversized constraints or invalid persistent expiry.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: GrantId,
        actor_id: ActorId,
        profile_id: ProfileId,
        tool_name: ToolName,
        tool_version: ToolVersion,
        effects: impl IntoIterator<Item = ToolEffect>,
        resources: impl IntoIterator<Item = ResourcePattern>,
        scope: GrantScope,
        issued_at: ProtocolTimestamp,
    ) -> Result<Self, PolicyGrantError> {
        let mut effects = effects.into_iter().collect::<Vec<_>>();
        effects.sort();
        effects.dedup();
        if effects.is_empty() || effects.len() > MAX_GRANT_EFFECTS {
            return Err(PolicyGrantError::InvalidEffects);
        }
        let mut resources = resources.into_iter().collect::<Vec<_>>();
        resources.sort();
        resources.dedup();
        if resources.is_empty() || resources.len() > MAX_GRANT_PATTERNS {
            return Err(PolicyGrantError::InvalidResources);
        }
        if matches!(&scope, GrantScope::PersistentResource { expires_at } if *expires_at <= issued_at)
        {
            return Err(PolicyGrantError::InvalidExpiry);
        }
        Ok(Self {
            id,
            actor_id,
            profile_id,
            tool_name,
            tool_version,
            tool_source: None,
            effects,
            resources,
            scope,
            issued_at,
            revoked_at: None,
        })
    }

    /// Constrains this grant to one complete frozen tool source.
    #[must_use]
    pub fn with_source(mut self, source: ToolSource) -> Self {
        self.tool_source = Some(source);
        self
    }

    /// Returns an immutable revoked form of this grant.
    ///
    /// # Errors
    ///
    /// Returns an error when revocation precedes issuance.
    pub fn revoke(mut self, revoked_at: ProtocolTimestamp) -> Result<Self, PolicyGrantError> {
        if revoked_at < self.issued_at {
            return Err(PolicyGrantError::InvalidRevocation);
        }
        self.revoked_at = Some(revoked_at);
        Ok(self)
    }

    /// Returns whether this grant fully authorizes one policy input at its supplied time.
    #[must_use]
    pub fn matches(&self, input: &PolicyInput) -> bool {
        if input.now() < self.issued_at
            || self
                .revoked_at
                .is_some_and(|revoked_at| input.now() >= revoked_at)
            || self.actor_id != *input.actor_id()
            || self.profile_id != *input.profile_id()
            || self.tool_name != *input.tool_name()
            || self.tool_version != *input.tool_version()
            || !self.tool_source.as_ref().map_or_else(
                || input.tool_source().is_native_product(),
                |source| source == input.tool_source(),
            )
            || !input
                .effects()
                .iter()
                .all(|effect| self.effects.contains(effect))
            || !input.resources().iter().all(|resource| {
                self.resources
                    .iter()
                    .any(|pattern| pattern.matches(resource))
            })
        {
            return false;
        }
        match self.scope {
            GrantScope::Once { tool_call_id } => tool_call_id == *input.tool_call_id(),
            GrantScope::Run { run_id } => input.run_id() == Some(&run_id),
            GrantScope::SessionResource { session_id } => session_id == *input.session_id(),
            GrantScope::PersistentResource { expires_at } => input.now() < expires_at,
        }
    }

    /// Returns stable grant ID.
    #[must_use]
    pub const fn id(&self) -> GrantId {
        self.id
    }
    /// Returns constrained actor.
    #[must_use]
    pub const fn actor_id(&self) -> &ActorId {
        &self.actor_id
    }
    /// Returns constrained profile.
    #[must_use]
    pub const fn profile_id(&self) -> &ProfileId {
        &self.profile_id
    }
    /// Returns constrained tool name.
    #[must_use]
    pub const fn tool_name(&self) -> &ToolName {
        &self.tool_name
    }
    /// Returns constrained tool version.
    #[must_use]
    pub const fn tool_version(&self) -> &ToolVersion {
        &self.tool_version
    }
    /// Returns the optional complete tool-source constraint.
    #[must_use]
    pub const fn tool_source(&self) -> Option<&ToolSource> {
        self.tool_source.as_ref()
    }
    /// Returns constrained effects.
    #[must_use]
    pub fn effects(&self) -> &[ToolEffect] {
        &self.effects
    }
    /// Returns constrained resource patterns.
    #[must_use]
    pub fn resources(&self) -> &[ResourcePattern] {
        &self.resources
    }
    /// Returns grant scope.
    #[must_use]
    pub const fn scope(&self) -> &GrantScope {
        &self.scope
    }
    /// Returns issuance timestamp.
    #[must_use]
    pub const fn issued_at(&self) -> ProtocolTimestamp {
        self.issued_at
    }
    /// Returns revocation timestamp.
    #[must_use]
    pub const fn revoked_at(&self) -> Option<ProtocolTimestamp> {
        self.revoked_at
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPolicyGrant {
    id: GrantId,
    actor_id: ActorId,
    profile_id: ProfileId,
    tool_name: ToolName,
    tool_version: ToolVersion,
    #[serde(default)]
    tool_source: Option<ToolSource>,
    effects: Vec<ToolEffect>,
    resources: Vec<ResourcePattern>,
    scope: GrantScope,
    issued_at: ProtocolTimestamp,
    #[serde(default)]
    revoked_at: Option<ProtocolTimestamp>,
}

impl<'de> Deserialize<'de> for PolicyGrant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawPolicyGrant::deserialize(deserializer)?;
        let mut grant = Self::new(
            raw.id,
            raw.actor_id,
            raw.profile_id,
            raw.tool_name,
            raw.tool_version,
            raw.effects,
            raw.resources,
            raw.scope,
            raw.issued_at,
        )
        .map_err(serde::de::Error::custom)?;
        if let Some(source) = raw.tool_source {
            grant = grant.with_source(source);
        }
        if let Some(revoked_at) = raw.revoked_at {
            grant = grant.revoke(revoked_at).map_err(serde::de::Error::custom)?;
        }
        Ok(grant)
    }
}

/// Error constructing or revoking grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PolicyGrantError {
    /// Resource pattern is invalid.
    #[error("grant resource pattern is invalid")]
    InvalidResourcePattern,
    /// Effect constraint is empty or oversized.
    #[error("grant effects are invalid")]
    InvalidEffects,
    /// Resource constraints are empty or oversized.
    #[error("grant resources are invalid")]
    InvalidResources,
    /// Persistent expiry is not after issuance.
    #[error("grant expiry is invalid")]
    InvalidExpiry,
    /// Revocation precedes issuance.
    #[error("grant revocation is invalid")]
    InvalidRevocation,
}
