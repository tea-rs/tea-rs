use std::future::Future;
use std::pin::Pin;

use tea_protocol::{ProfileId, ProtocolMetadata, RunId, SessionId};
use tea_tools::ToolSpec;

use crate::{ContextError, ContextProviderId, PromptModule};

/// Maximum active tools visible to a context snapshot.
pub const MAX_CONTEXT_TOOLS: usize = 256;

/// Immutable inputs visible to context providers for one turn snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextRequest {
    profile_id: ProfileId,
    session_id: SessionId,
    run_id: Option<RunId>,
    active_tools: Vec<ToolSpec>,
    metadata: ProtocolMetadata,
}

impl ContextRequest {
    /// Creates a bounded request and canonicalizes active tool order.
    ///
    /// # Errors
    ///
    /// Returns an error for too many tools or duplicate active tool names.
    pub fn new(
        profile_id: ProfileId,
        session_id: SessionId,
        run_id: Option<RunId>,
        mut active_tools: Vec<ToolSpec>,
        metadata: ProtocolMetadata,
    ) -> Result<Self, ContextError> {
        if active_tools.len() > MAX_CONTEXT_TOOLS {
            return Err(ContextError::new(
                crate::ContextErrorCode::BoundsExceeded,
                "context request contains too many active tools",
            ));
        }
        active_tools.sort_by(|left, right| left.name().cmp(right.name()));
        if active_tools
            .windows(2)
            .any(|tools| tools[0].name() == tools[1].name())
        {
            return Err(ContextError::new(
                crate::ContextErrorCode::DuplicateIdentity,
                "context request contains duplicate active tool names",
            ));
        }
        Ok(Self {
            profile_id,
            session_id,
            run_id,
            active_tools,
            metadata,
        })
    }

    /// Returns active profile.
    #[must_use]
    pub const fn profile_id(&self) -> &ProfileId {
        &self.profile_id
    }
    /// Returns active session.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }
    /// Returns active run when present.
    #[must_use]
    pub const fn run_id(&self) -> Option<RunId> {
        self.run_id
    }
    /// Returns canonical active tools.
    #[must_use]
    pub fn active_tools(&self) -> &[ToolSpec] {
        &self.active_tools
    }
    /// Returns bounded request metadata.
    #[must_use]
    pub const fn metadata(&self) -> &ProtocolMetadata {
        &self.metadata
    }
}

/// Runtime-neutral boxed context-provider future.
pub type ContextProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<PromptModule>, ContextError>> + Send + 'a>>;

/// Object-safe context source evaluated before prompt compilation.
pub trait ContextProvider: std::fmt::Debug + Send + Sync {
    /// Returns stable provider identity.
    fn id(&self) -> &ContextProviderId;
    /// Produces bounded modules from one immutable request.
    fn provide(&self, request: ContextRequest) -> ContextProviderFuture<'_>;
}

/// Deterministic provider returning an immutable module snapshot.
#[derive(Debug, Clone)]
pub struct StaticContextProvider {
    id: ContextProviderId,
    modules: Vec<PromptModule>,
}

impl StaticContextProvider {
    /// Creates one static provider.
    #[must_use]
    pub const fn new(id: ContextProviderId, modules: Vec<PromptModule>) -> Self {
        Self { id, modules }
    }
}

impl ContextProvider for StaticContextProvider {
    fn id(&self) -> &ContextProviderId {
        &self.id
    }
    fn provide(&self, _request: ContextRequest) -> ContextProviderFuture<'_> {
        let modules = self.modules.clone();
        Box::pin(async move { Ok(modules) })
    }
}
