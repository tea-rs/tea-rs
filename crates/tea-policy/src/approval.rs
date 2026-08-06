use serde::{Deserialize, Deserializer, Serialize};
use tea_protocol::{
    ApprovalDecision, ApprovalId, ProfileId, ProtocolTimestamp, RunId, SessionId, ToolCallId,
    ToolFailure,
};
use tea_tools::{ToolEffect, ToolName, ToolResource, ToolSource, ToolVersion};
use thiserror::Error;

use crate::{
    ActorId, PolicyEnvironment, PolicyGrant, PolicyInput, PolicyRedactor, RedactedArguments,
    RedactionError, WorkspaceId,
};

/// Redacted bounded presentation attached to an approval request.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalPresentation {
    reason: String,
    arguments: RedactedArguments,
    resources: Vec<String>,
}

impl ApprovalPresentation {
    /// Creates presentation from immutable policy input and explicit redactor.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid reason or oversized redacted arguments.
    pub fn from_input(
        reason: impl Into<String>,
        input: &PolicyInput,
        redactor: PolicyRedactor,
    ) -> Result<Self, ApprovalError> {
        let reason = reason.into();
        if reason.is_empty() || reason.len() > 4096 || reason.contains('\0') {
            return Err(ApprovalError::InvalidPresentation);
        }
        let arguments = redactor.redact_arguments(input.arguments())?;
        let resources = input
            .resources()
            .iter()
            .map(|resource| redactor.redact_resource(resource.scheme(), resource.locator()))
            .collect();
        Self::from_parts(reason, arguments, resources)
    }
    fn from_parts(
        reason: String,
        arguments: RedactedArguments,
        resources: Vec<String>,
    ) -> Result<Self, ApprovalError> {
        if reason.is_empty()
            || reason.len() > 4096
            || reason.contains('\0')
            || resources.len() > 128
            || resources.iter().any(|resource| {
                resource.is_empty()
                    || resource.len() > 2048
                    || resource.chars().any(char::is_control)
            })
        {
            return Err(ApprovalError::InvalidPresentation);
        }
        Ok(Self {
            reason,
            arguments,
            resources,
        })
    }

    /// Returns technical approval reason.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
    /// Returns redacted arguments.
    #[must_use]
    pub const fn arguments(&self) -> &RedactedArguments {
        &self.arguments
    }
    /// Returns redacted resources.
    #[must_use]
    pub fn resources(&self) -> &[String] {
        &self.resources
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawApprovalPresentation {
    reason: String,
    arguments: RedactedArguments,
    resources: Vec<String>,
}

impl<'de> Deserialize<'de> for ApprovalPresentation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawApprovalPresentation::deserialize(deserializer)?;
        Self::from_parts(raw.reason, raw.arguments, raw.resources).map_err(serde::de::Error::custom)
    }
}

/// Serializable pending approval snapshot.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    approval_id: ApprovalId,
    tool_call_id: ToolCallId,
    actor_id: ActorId,
    profile_id: ProfileId,
    session_id: SessionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id: Option<RunId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_id: Option<WorkspaceId>,
    environment: PolicyEnvironment,
    tool_name: ToolName,
    tool_version: ToolVersion,
    tool_source: ToolSource,
    effects: Vec<ToolEffect>,
    resources: Vec<ToolResource>,
    created_at: ProtocolTimestamp,
    expires_at: ProtocolTimestamp,
    presentation: ApprovalPresentation,
}

impl ApprovalRequest {
    /// Creates a pending request from policy input.
    ///
    /// # Errors
    ///
    /// Returns an error when expiry is not strictly after creation.
    pub fn new(
        approval_id: ApprovalId,
        input: &PolicyInput,
        created_at: ProtocolTimestamp,
        expires_at: ProtocolTimestamp,
        presentation: ApprovalPresentation,
    ) -> Result<Self, ApprovalError> {
        Self::from_parts(
            approval_id,
            *input.tool_call_id(),
            input.actor_id().clone(),
            input.profile_id().clone(),
            *input.session_id(),
            input.run_id().copied(),
            input.workspace_id().cloned(),
            input.environment().clone(),
            input.tool_name().clone(),
            input.tool_version().clone(),
            input.tool_source().clone(),
            input.effects().to_vec(),
            input.resources().to_vec(),
            created_at,
            expires_at,
            presentation,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        approval_id: ApprovalId,
        tool_call_id: ToolCallId,
        actor_id: ActorId,
        profile_id: ProfileId,
        session_id: SessionId,
        run_id: Option<RunId>,
        workspace_id: Option<WorkspaceId>,
        environment: PolicyEnvironment,
        tool_name: ToolName,
        tool_version: ToolVersion,
        tool_source: ToolSource,
        effects: Vec<ToolEffect>,
        resources: Vec<ToolResource>,
        created_at: ProtocolTimestamp,
        expires_at: ProtocolTimestamp,
        presentation: ApprovalPresentation,
    ) -> Result<Self, ApprovalError> {
        if expires_at <= created_at
            || effects.is_empty()
            || effects.len() > 64
            || resources.len() > 128
        {
            return Err(if expires_at <= created_at {
                ApprovalError::InvalidExpiry
            } else {
                ApprovalError::InvalidContext
            });
        }
        Ok(Self {
            approval_id,
            tool_call_id,
            actor_id,
            profile_id,
            session_id,
            run_id,
            workspace_id,
            environment,
            tool_name,
            tool_version,
            tool_source,
            effects,
            resources,
            created_at,
            expires_at,
            presentation,
        })
    }
    /// Returns approval ID.
    #[must_use]
    pub const fn approval_id(&self) -> &ApprovalId {
        &self.approval_id
    }
    /// Returns tool-call ID.
    #[must_use]
    pub const fn tool_call_id(&self) -> &ToolCallId {
        &self.tool_call_id
    }
    /// Returns owning session.
    #[must_use]
    pub const fn session_id(&self) -> &SessionId {
        &self.session_id
    }
    /// Returns active run when present.
    #[must_use]
    pub const fn run_id(&self) -> Option<&RunId> {
        self.run_id.as_ref()
    }
    /// Returns active workspace when present.
    #[must_use]
    pub const fn workspace_id(&self) -> Option<&WorkspaceId> {
        self.workspace_id.as_ref()
    }
    /// Returns the approved execution environment snapshot.
    #[must_use]
    pub const fn environment(&self) -> &PolicyEnvironment {
        &self.environment
    }
    /// Returns actor.
    #[must_use]
    pub const fn actor_id(&self) -> &ActorId {
        &self.actor_id
    }
    /// Returns profile.
    #[must_use]
    pub const fn profile_id(&self) -> &ProfileId {
        &self.profile_id
    }
    /// Returns tool name.
    #[must_use]
    pub const fn tool_name(&self) -> &ToolName {
        &self.tool_name
    }
    /// Returns tool version.
    #[must_use]
    pub const fn tool_version(&self) -> &ToolVersion {
        &self.tool_version
    }
    /// Returns frozen tool-source provenance.
    #[must_use]
    pub const fn tool_source(&self) -> &ToolSource {
        &self.tool_source
    }
    /// Returns effects.
    #[must_use]
    pub fn effects(&self) -> &[ToolEffect] {
        &self.effects
    }
    /// Returns resources.
    #[must_use]
    pub fn resources(&self) -> &[ToolResource] {
        &self.resources
    }
    /// Returns creation time.
    #[must_use]
    pub const fn created_at(&self) -> ProtocolTimestamp {
        self.created_at
    }
    /// Returns expiry time.
    #[must_use]
    pub const fn expires_at(&self) -> ProtocolTimestamp {
        self.expires_at
    }
    /// Returns redacted presentation.
    #[must_use]
    pub const fn presentation(&self) -> &ApprovalPresentation {
        &self.presentation
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawApprovalRequest {
    approval_id: ApprovalId,
    tool_call_id: ToolCallId,
    actor_id: ActorId,
    profile_id: ProfileId,
    session_id: SessionId,
    #[serde(default)]
    run_id: Option<RunId>,
    #[serde(default)]
    workspace_id: Option<WorkspaceId>,
    environment: PolicyEnvironment,
    tool_name: ToolName,
    tool_version: ToolVersion,
    #[serde(default)]
    tool_source: Option<ToolSource>,
    effects: Vec<ToolEffect>,
    resources: Vec<ToolResource>,
    created_at: ProtocolTimestamp,
    expires_at: ProtocolTimestamp,
    presentation: ApprovalPresentation,
}

impl<'de> Deserialize<'de> for ApprovalRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawApprovalRequest::deserialize(deserializer)?;
        Self::from_parts(
            raw.approval_id,
            raw.tool_call_id,
            raw.actor_id,
            raw.profile_id,
            raw.session_id,
            raw.run_id,
            raw.workspace_id,
            raw.environment,
            raw.tool_name,
            raw.tool_version,
            raw.tool_source.unwrap_or_else(ToolSource::native_product),
            raw.effects,
            raw.resources,
            raw.created_at,
            raw.expires_at,
            raw.presentation,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Serializable terminal approval resolution with optional issued grant.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalResolution {
    request: ApprovalRequest,
    decision: ApprovalDecision,
    decided_at: ProtocolTimestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    issued_grant: Option<PolicyGrant>,
}

impl ApprovalResolution {
    /// Resolves one request and optionally issues a context-matching grant.
    ///
    /// # Errors
    ///
    /// Returns an error for wrong timing, invalid grant decision, or grant mismatch.
    pub fn new(
        request: &ApprovalRequest,
        decision: ApprovalDecision,
        decided_at: ProtocolTimestamp,
        issued_grant: Option<PolicyGrant>,
    ) -> Result<Self, ApprovalError> {
        Self::from_parts(request.clone(), decision, decided_at, issued_grant)
    }

    fn from_parts(
        request: ApprovalRequest,
        decision: ApprovalDecision,
        decided_at: ProtocolTimestamp,
        issued_grant: Option<PolicyGrant>,
    ) -> Result<Self, ApprovalError> {
        if decided_at < request.created_at || decided_at >= request.expires_at {
            return Err(ApprovalError::ResolutionOutsideLifetime);
        }
        if issued_grant.is_some() && !matches!(decision, ApprovalDecision::AllowSession) {
            return Err(ApprovalError::GrantNotAllowed);
        }
        if let Some(grant) = &issued_grant
            && (grant.issued_at() > decided_at
                || grant.actor_id() != request.actor_id()
                || grant.profile_id() != request.profile_id()
                || grant.tool_name() != request.tool_name()
                || grant.tool_version() != request.tool_version()
                || !grant_source_matches(grant.tool_source(), request.tool_source())
                || !request
                    .effects()
                    .iter()
                    .all(|effect| grant.effects().contains(effect))
                || !request.resources().iter().all(|resource| {
                    grant
                        .resources()
                        .iter()
                        .any(|pattern| pattern.matches(resource))
                }))
        {
            return Err(ApprovalError::GrantMismatch);
        }
        Ok(Self {
            request,
            decision,
            decided_at,
            issued_grant,
        })
    }

    /// Returns the self-contained validated request snapshot.
    #[must_use]
    pub const fn request(&self) -> &ApprovalRequest {
        &self.request
    }
    /// Returns canonical approval decision.
    #[must_use]
    pub const fn decision(&self) -> ApprovalDecision {
        self.decision
    }
    /// Returns when the approval reached its terminal decision.
    #[must_use]
    pub const fn decided_at(&self) -> ProtocolTimestamp {
        self.decided_at
    }
    /// Returns issued grant.
    #[must_use]
    pub const fn issued_grant(&self) -> Option<&PolicyGrant> {
        self.issued_grant.as_ref()
    }
    /// Converts denial into a model-visible machine-readable tool failure.
    #[must_use]
    pub fn denial_tool_failure(&self) -> Option<ToolFailure> {
        matches!(self.decision, ApprovalDecision::Deny).then(ToolFailure::approval_denied)
    }
}

fn grant_source_matches(grant: Option<&ToolSource>, request: &ToolSource) -> bool {
    grant.map_or_else(|| request.is_native_product(), |grant| grant == request)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawApprovalResolution {
    request: ApprovalRequest,
    decision: ApprovalDecision,
    decided_at: ProtocolTimestamp,
    #[serde(default)]
    issued_grant: Option<PolicyGrant>,
}

impl<'de> Deserialize<'de> for ApprovalResolution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawApprovalResolution::deserialize(deserializer)?;
        Self::from_parts(raw.request, raw.decision, raw.decided_at, raw.issued_grant)
            .map_err(serde::de::Error::custom)
    }
}

/// Approval construction/resolution error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ApprovalError {
    /// Presentation reason or redaction is invalid.
    #[error("approval presentation is invalid")]
    InvalidPresentation,
    /// Request expiry is invalid.
    #[error("approval expiry must be after creation")]
    InvalidExpiry,
    /// Effects or resources exceed approval context bounds.
    #[error("approval context is invalid")]
    InvalidContext,
    /// Resolution is before creation or at/after expiry.
    #[error("approval resolution is outside request lifetime")]
    ResolutionOutsideLifetime,
    /// Grant can only accompany allow-session decision.
    #[error("approval decision cannot issue a grant")]
    GrantNotAllowed,
    /// Issued grant does not contain request context.
    #[error("approval grant does not match request")]
    GrantMismatch,
    /// Redaction failed.
    #[error("approval redaction failed: {0}")]
    Redaction(#[from] RedactionError),
}
