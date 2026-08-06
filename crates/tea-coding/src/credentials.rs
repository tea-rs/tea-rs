use std::fmt;
use std::sync::Arc;

use tea_provider_openai::{CredentialResolver, OpenAiConfig};

use crate::{CodingError, CodingErrorCode};

/// Product credential port that owns only a resolver, never a resolved secret.
#[derive(Clone)]
pub struct CodingCredentialResolver {
    resolver: Arc<dyn CredentialResolver>,
}

impl CodingCredentialResolver {
    /// Wraps an existing provider credential resolver.
    #[must_use]
    pub fn new(resolver: Arc<dyn CredentialResolver>) -> Self {
        Self { resolver }
    }

    /// Resolves credentials only at the provider construction/request boundary.
    ///
    /// # Errors
    ///
    /// Returns a secret-independent coding error when resolution fails.
    pub fn resolve(&self) -> Result<OpenAiConfig, CodingError> {
        self.resolver.resolve().map_err(|_| {
            CodingError::new(
                CodingErrorCode::Credential,
                "provider credentials could not be resolved",
            )
        })
    }
}

impl fmt::Debug for CodingCredentialResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodingCredentialResolver")
            .field("resolver", &"**REDACTED**")
            .finish()
    }
}
