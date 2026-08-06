use std::fmt;
use std::str::FromStr;

use tea_protocol::{ModelId, ModelRef, ProfileId, ProtocolTimestamp, ProviderId, SessionId};
use thiserror::Error;

use crate::{SessionStoreError, SessionStoreFuture};

/// Maximum UTF-8 bytes accepted for a session display name.
pub const MAX_SESSION_NAME_BYTES: usize = 256;

/// Validated optional human-facing session name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionName(String);

impl SessionName {
    /// Creates a trimmed bounded display name.
    ///
    /// # Errors
    ///
    /// Returns an error for empty, oversized, or control-containing text.
    pub fn new(value: impl Into<String>) -> Result<Self, SessionNameError> {
        let value = value.into();
        let value = value.trim();
        if value.is_empty() {
            return Err(SessionNameError::Empty);
        }
        if value.len() > MAX_SESSION_NAME_BYTES {
            return Err(SessionNameError::TooLong);
        }
        if value.chars().any(char::is_control) {
            return Err(SessionNameError::ControlCharacter);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the validated display text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for SessionName {
    type Err = SessionNameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

/// Session-name validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SessionNameError {
    /// The trimmed name is empty.
    #[error("session name cannot be empty")]
    Empty,
    /// The name exceeds the stable UTF-8 byte bound.
    #[error("session name exceeds {MAX_SESSION_NAME_BYTES} bytes")]
    TooLong,
    /// The name contains a control character.
    #[error("session name cannot contain control characters")]
    ControlCharacter,
}

/// Immutable host-facing catalog entry derived from durable session state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCatalogEntry {
    session_id: SessionId,
    name: Option<SessionName>,
    updated_at: ProtocolTimestamp,
    profile_id: ProfileId,
    model: Option<ModelRef>,
    message_count: usize,
    pending_approval_count: usize,
}

impl SessionCatalogEntry {
    #[allow(clippy::too_many_arguments)]
    const fn new(
        session_id: SessionId,
        name: Option<SessionName>,
        updated_at: ProtocolTimestamp,
        profile_id: ProfileId,
        model: Option<ModelRef>,
        message_count: usize,
        pending_approval_count: usize,
    ) -> Self {
        Self {
            session_id,
            name,
            updated_at,
            profile_id,
            model,
            message_count,
            pending_approval_count,
        }
    }

    /// Builds a catalog projection from one immutable durable snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the snapshot has no durable records.
    pub fn from_snapshot(
        snapshot: &crate::SessionSnapshot,
        name: Option<SessionName>,
    ) -> Result<Self, SessionStoreError> {
        catalog_entry(snapshot, name)
    }

    /// Returns session identity.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Returns the optional display name.
    #[must_use]
    pub const fn name(&self) -> Option<&SessionName> {
        self.name.as_ref()
    }

    /// Returns the latest durable record timestamp.
    #[must_use]
    pub const fn updated_at(&self) -> ProtocolTimestamp {
        self.updated_at
    }

    /// Returns the active product profile.
    #[must_use]
    pub const fn profile_id(&self) -> &ProfileId {
        &self.profile_id
    }

    /// Returns the active model when configured.
    #[must_use]
    pub const fn model_id(&self) -> Option<&ModelId> {
        match &self.model {
            Some(model) => Some(model.model_id()),
            None => None,
        }
    }

    /// Returns the active provider when configured.
    #[must_use]
    pub const fn provider_id(&self) -> Option<&ProviderId> {
        match &self.model {
            Some(model) => Some(model.provider_id()),
            None => None,
        }
    }

    /// Returns the provider-qualified active model when configured.
    #[must_use]
    pub const fn model_ref(&self) -> Option<&ModelRef> {
        self.model.as_ref()
    }

    /// Returns the active transcript message count.
    #[must_use]
    pub const fn message_count(&self) -> usize {
        self.message_count
    }

    /// Returns the number of pending durable approvals.
    #[must_use]
    pub const fn pending_approval_count(&self) -> usize {
        self.pending_approval_count
    }
}

/// Read/list and display-metadata port kept separate from append semantics.
pub trait SessionCatalog: fmt::Debug + Send + Sync {
    /// Lists sessions ordered by latest durable timestamp descending, then ID.
    fn list_sessions(&self) -> SessionStoreFuture<'_, Vec<SessionCatalogEntry>>;

    /// Sets or clears one optional display name.
    fn set_session_name(
        &self,
        session_id: SessionId,
        name: Option<SessionName>,
    ) -> SessionStoreFuture<'_, ()>;

    /// Returns one optional display name.
    fn session_name(&self, session_id: SessionId) -> SessionStoreFuture<'_, Option<SessionName>>;
}

pub(crate) fn catalog_entry(
    snapshot: &crate::SessionSnapshot,
    name: Option<SessionName>,
) -> Result<SessionCatalogEntry, SessionStoreError> {
    let state = snapshot.state();
    let updated_at = snapshot
        .records()
        .last()
        .map(tea_protocol::RecordEnvelope::timestamp)
        .ok_or_else(|| {
            SessionStoreError::new(
                crate::SessionStoreErrorCode::InvalidRecord,
                "stored session has no durable records",
            )
        })?;
    Ok(SessionCatalogEntry::new(
        state.session_id(),
        name,
        updated_at,
        state.configuration().profile_id().clone(),
        state.configuration().model_ref().cloned(),
        state.messages().len(),
        state.pending_approvals().len(),
    ))
}

pub(crate) fn sort_catalog(entries: &mut [SessionCatalogEntry]) {
    entries.sort_by(|left, right| {
        right
            .updated_at()
            .cmp(&left.updated_at())
            .then_with(|| left.session_id().cmp(&right.session_id()))
    });
}
