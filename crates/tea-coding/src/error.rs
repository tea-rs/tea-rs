use thiserror::Error;

/// Stable coding-product failure classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingErrorCode {
    /// Configuration or resource input was malformed or exceeded bounds.
    InvalidInput,
    /// A required path or resource was absent.
    NotFound,
    /// Project-local input was not trusted for the requested mode.
    ProjectNotTrusted,
    /// Persistent state could not be read or committed.
    Persistence,
    /// Credential resolution failed without exposing the credential.
    Credential,
    /// Provider construction or model execution failed.
    Provider,
    /// Policy denied or could not authorize an operation.
    PolicyDenied,
    /// An owned run was cancelled.
    Cancelled,
    /// Runtime assembly or execution failed.
    Runtime,
}

/// Bounded path- and secret-independent coding-product error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{code:?}: {message}")]
pub struct CodingError {
    code: CodingErrorCode,
    message: String,
}

impl CodingError {
    pub(crate) fn new(code: CodingErrorCode, message: impl Into<String>) -> Self {
        let mut message = message.into();
        if message.is_empty() {
            "coding operation failed".clone_into(&mut message);
        }
        message.retain(|character| character != '\0');
        if message.len() > 512 {
            let boundary = message
                .char_indices()
                .map(|(index, _)| index)
                .take_while(|index| *index <= 512)
                .last()
                .unwrap_or(0);
            message.truncate(boundary);
        }
        Self { code, message }
    }

    /// Returns the stable machine-readable classification.
    #[must_use]
    pub const fn code(&self) -> CodingErrorCode {
        self.code
    }

    /// Returns the bounded safe diagnostic.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<tea::RuntimeError> for CodingError {
    fn from(error: tea::RuntimeError) -> Self {
        let code = match error.code() {
            tea::RuntimeErrorCode::ProviderFailure
            | tea::RuntimeErrorCode::UnknownProvider
            | tea::RuntimeErrorCode::UnknownModel => CodingErrorCode::Provider,
            tea::RuntimeErrorCode::PolicyFailure => CodingErrorCode::PolicyDenied,
            tea::RuntimeErrorCode::Cancelled => CodingErrorCode::Cancelled,
            tea::RuntimeErrorCode::InvalidRequest
            | tea::RuntimeErrorCode::UnknownProfile
            | tea::RuntimeErrorCode::UnknownTool
            | tea::RuntimeErrorCode::UnknownPolicyRule => CodingErrorCode::InvalidInput,
            tea::RuntimeErrorCode::SessionFailure => CodingErrorCode::Persistence,
            _ => CodingErrorCode::Runtime,
        };
        let message = match code {
            CodingErrorCode::Provider if error.is_safe_diagnostic() => error.message(),
            CodingErrorCode::Provider => "model provider operation failed",
            CodingErrorCode::Persistence => "session persistence operation failed",
            CodingErrorCode::Runtime => "coding runtime operation failed",
            CodingErrorCode::Cancelled => "coding operation was cancelled",
            CodingErrorCode::PolicyDenied
            | CodingErrorCode::Credential
            | CodingErrorCode::InvalidInput
            | CodingErrorCode::NotFound
            | CodingErrorCode::ProjectNotTrusted => error.message(),
        };
        Self::new(code, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_provider_diagnostic_survives_product_error_conversion() {
        let kernel = tea_kernel::KernelError::provider_failure(
            tea_kernel::KernelErrorCode::ModelFailure,
            "HTTP 403: blocked by gateway WAF",
        );
        let runtime = tea::RuntimeError::from(kernel);
        let coding = CodingError::from(runtime);
        assert_eq!(coding.code(), CodingErrorCode::Provider);
        assert_eq!(coding.message(), "HTTP 403: blocked by gateway WAF");
    }

    #[test]
    fn untrusted_provider_runtime_messages_stay_generic() {
        let runtime = tea::RuntimeError::new(
            tea::RuntimeErrorCode::ProviderFailure,
            "sk-seeded-cli-credential-must-never-persist",
        );
        let coding = CodingError::from(runtime);
        assert_eq!(coding.message(), "model provider operation failed");
    }
}
