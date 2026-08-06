use std::str::FromStr;

use tea_policy::GrantId;
use tea_protocol::{ApprovalId, EventId, MessageId, RecordId, RunId, ToolCallId, TurnId};
use uuid::Uuid;

use crate::{KernelError, KernelErrorCode};

/// Replaceable source of typed `UUIDv7` runtime identities.
///
/// # Errors
///
/// Every method returns [`KernelErrorCode::IdExhausted`](crate::KernelErrorCode::IdExhausted)
/// when the configured source cannot provide the requested identity.
pub trait KernelIdSource: std::fmt::Debug + Send + Sync {
    /// Produces a run ID.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID source is exhausted.
    fn next_run_id(&self) -> Result<RunId, KernelError>;
    /// Produces a turn ID.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID source is exhausted.
    fn next_turn_id(&self) -> Result<TurnId, KernelError>;
    /// Produces a canonical message ID.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID source is exhausted.
    fn next_message_id(&self) -> Result<MessageId, KernelError>;
    /// Produces a canonical tool-call ID.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID source is exhausted.
    fn next_tool_call_id(&self) -> Result<ToolCallId, KernelError>;
    /// Produces an approval ID.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID source is exhausted.
    fn next_approval_id(&self) -> Result<ApprovalId, KernelError>;
    /// Produces a durable policy-grant ID.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID source is exhausted.
    fn next_grant_id(&self) -> Result<GrantId, KernelError>;
    /// Produces an observable event ID.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID source is exhausted.
    fn next_event_id(&self) -> Result<EventId, KernelError>;
    /// Produces a durable record ID.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID source is exhausted.
    fn next_record_id(&self) -> Result<RecordId, KernelError>;
}

/// Production `UUIDv7` identity source.
#[derive(Debug, Clone, Copy, Default)]
pub struct UuidV7KernelIdSource;

macro_rules! next_id {
    ($method:ident, $type:ty) => {
        fn $method(&self) -> Result<$type, KernelError> {
            <$type>::from_str(&Uuid::now_v7().hyphenated().to_string()).map_err(|_| {
                KernelError::new(
                    KernelErrorCode::IdExhausted,
                    "UUIDv7 source produced an invalid protocol ID",
                )
            })
        }
    };
}

impl KernelIdSource for UuidV7KernelIdSource {
    next_id!(next_run_id, RunId);
    next_id!(next_turn_id, TurnId);
    next_id!(next_message_id, MessageId);
    next_id!(next_tool_call_id, ToolCallId);
    next_id!(next_approval_id, ApprovalId);
    next_id!(next_grant_id, GrantId);
    next_id!(next_event_id, EventId);
    next_id!(next_record_id, RecordId);
}
