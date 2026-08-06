use std::fmt;
use std::str::FromStr;

use tea_protocol::SessionId;

use crate::{RuntimeError, RuntimeErrorCode};

/// Replaceable source of canonical agent session identities.
///
/// The runtime generates a fresh `SessionId` when it creates a session. The
/// kernel's `KernelIdSource` does not produce session ids, so the runtime owns
/// this separate port to keep `tea-kernel` unmodified.
pub trait SessionIdSource: fmt::Debug + Send + Sync {
    /// Produces a stable session identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the source cannot produce a canonical id.
    fn next_session_id(&self) -> Result<SessionId, RuntimeError>;
}

/// Production `UUIDv7` session identity source.
#[derive(Debug, Clone, Copy, Default)]
pub struct UuidSessionIdSource;

impl SessionIdSource for UuidSessionIdSource {
    fn next_session_id(&self) -> Result<SessionId, RuntimeError> {
        SessionId::from_str(&uuid::Uuid::now_v7().hyphenated().to_string()).map_err(|_| {
            RuntimeError::new(
                RuntimeErrorCode::InvalidState,
                "session id source produced an invalid id",
            )
        })
    }
}
