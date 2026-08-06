use std::str::FromStr;

use tea_policy::{ApprovalPresentation, ApprovalRequest, PolicyInput, PolicyRedactor};
use tea_protocol::ProtocolTimestamp;

use crate::{KernelError, KernelErrorCode};

pub(crate) fn request(
    approval_id: tea_protocol::ApprovalId,
    input: &PolicyInput,
    reason: &str,
    created_at: ProtocolTimestamp,
    ttl: std::time::Duration,
) -> Result<ApprovalRequest, KernelError> {
    let expires_at = add_duration(created_at, ttl)?;
    let presentation =
        ApprovalPresentation::from_input(reason, input, PolicyRedactor).map_err(policy_error)?;
    ApprovalRequest::new(approval_id, input, created_at, expires_at, presentation)
        .map_err(policy_error)
}

pub(crate) fn add_duration(
    timestamp: ProtocolTimestamp,
    duration: std::time::Duration,
) -> Result<ProtocolTimestamp, KernelError> {
    let duration = chrono::Duration::from_std(duration).map_err(|_| {
        KernelError::new(
            KernelErrorCode::ClockFailure,
            "approval duration cannot be represented",
        )
    })?;
    let value = timestamp
        .as_utc()
        .checked_add_signed(duration)
        .ok_or_else(|| {
            KernelError::new(
                KernelErrorCode::ClockFailure,
                "approval expiry cannot be represented",
            )
        })?
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    ProtocolTimestamp::from_str(&value).map_err(|_| {
        KernelError::new(
            KernelErrorCode::ClockFailure,
            "approval expiry is not canonical",
        )
    })
}

fn policy_error(error: impl std::fmt::Display) -> KernelError {
    KernelError::new(KernelErrorCode::PolicyFailure, error.to_string())
}
