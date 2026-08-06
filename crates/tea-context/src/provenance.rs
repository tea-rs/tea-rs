use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ContextProviderId;

/// Fixed high-to-low prompt authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptAuthority {
    /// Runtime safety and protocol invariants.
    Kernel,
    /// Organization-wide behavior and policy guidance.
    Organization,
    /// Product/profile identity and behavior.
    Product,
    /// Caller-supplied workspace instructions.
    Workspace,
    /// Active tool-specific guidance.
    Tool,
    /// Explicitly active skill metadata or instructions.
    Skill,
    /// Session summary or retrieved session context.
    Session,
    /// Explicit user-supplied system addition.
    UserAddition,
}

/// Declared origin trust for inspection and downstream policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// Content is owned by the runtime/product trust boundary.
    Trusted,
    /// Content is supplied by a configured delegated source.
    Delegated,
    /// Content is caller-marked untrusted and receives no safety claim.
    Untrusted,
}

/// Intended reuse scope for prompt caching adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheScope {
    /// Never reuse beyond this compiled prompt.
    None,
    /// Reuse within one active run.
    Run,
    /// Reuse within one session.
    Session,
    /// Reuse for one product profile.
    Profile,
    /// Reuse globally when the embedder can prove identity.
    Global,
}

/// Bounded source attribution retained through compilation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptProvenance {
    provider_id: ContextProviderId,
    source_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    locator: Option<String>,
}

impl PromptProvenance {
    /// Creates bounded explicit provenance.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid source kind or locator text.
    pub fn new(
        provider_id: ContextProviderId,
        source_kind: impl Into<String>,
        locator: Option<String>,
    ) -> Result<Self, ProvenanceError> {
        let source_kind = source_kind.into();
        if !valid_source_kind(&source_kind)
            || locator.as_ref().is_some_and(|value| {
                value.is_empty() || value.len() > 2048 || value.chars().any(char::is_control)
            })
        {
            return Err(ProvenanceError);
        }
        Ok(Self {
            provider_id,
            source_kind,
            locator,
        })
    }

    /// Returns the producing provider.
    #[must_use]
    pub const fn provider_id(&self) -> &ContextProviderId {
        &self.provider_id
    }
    /// Returns canonical source category.
    #[must_use]
    pub fn source_kind(&self) -> &str {
        &self.source_kind
    }
    /// Returns optional bounded source locator.
    #[must_use]
    pub fn locator(&self) -> Option<&str> {
        self.locator.as_deref()
    }
}

/// Invalid prompt provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("prompt provenance is invalid")]
pub struct ProvenanceError;

fn valid_source_kind(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= 128
        && bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}
