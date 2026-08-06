use tea_policy::PolicyRedactor;
use tea_protocol::{
    BranchId, ProtocolMetadata, ProtocolTimestamp, RecordEnvelope, SessionId, SessionRecord,
    SessionSequence,
};
use tea_tools::{ToolAuditMetadata, ToolAuditResource, ValidatedToolInvocation};

use crate::{KernelClock, KernelError, KernelErrorCode, KernelIdSource};

pub(crate) fn envelopes(
    ids: &dyn KernelIdSource,
    clock: &dyn KernelClock,
    session_id: SessionId,
    previous: SessionSequence,
    branch_id: Option<BranchId>,
    records: impl IntoIterator<Item = SessionRecord>,
) -> Result<Vec<RecordEnvelope>, KernelError> {
    envelopes_at(ids, session_id, previous, branch_id, clock.now()?, records)
}

pub(crate) fn envelopes_at(
    ids: &dyn KernelIdSource,
    session_id: SessionId,
    previous: SessionSequence,
    branch_id: Option<BranchId>,
    timestamp: ProtocolTimestamp,
    records: impl IntoIterator<Item = SessionRecord>,
) -> Result<Vec<RecordEnvelope>, KernelError> {
    envelopes_at_with_metadata(
        ids,
        session_id,
        previous,
        branch_id,
        timestamp,
        records
            .into_iter()
            .map(|record| (ProtocolMetadata::default(), record)),
    )
}

pub(crate) fn envelopes_with_metadata(
    ids: &dyn KernelIdSource,
    clock: &dyn KernelClock,
    session_id: SessionId,
    previous: SessionSequence,
    branch_id: Option<BranchId>,
    records: impl IntoIterator<Item = (ProtocolMetadata, SessionRecord)>,
) -> Result<Vec<RecordEnvelope>, KernelError> {
    envelopes_at_with_metadata(ids, session_id, previous, branch_id, clock.now()?, records)
}

fn envelopes_at_with_metadata(
    ids: &dyn KernelIdSource,
    session_id: SessionId,
    previous: SessionSequence,
    branch_id: Option<BranchId>,
    timestamp: ProtocolTimestamp,
    records: impl IntoIterator<Item = (ProtocolMetadata, SessionRecord)>,
) -> Result<Vec<RecordEnvelope>, KernelError> {
    let mut sequence = previous;
    records
        .into_iter()
        .map(|(metadata, record)| {
            sequence = sequence.checked_next().ok_or_else(|| {
                KernelError::new(
                    KernelErrorCode::SessionFailure,
                    "durable session sequence cannot advance",
                )
            })?;
            RecordEnvelope::new(
                ids.next_record_id()?,
                session_id,
                sequence,
                timestamp,
                None,
                None,
                branch_id,
                metadata,
                record,
            )
            .map_err(|error| KernelError::new(KernelErrorCode::SessionFailure, error.to_string()))
        })
        .collect()
}

pub(crate) fn tool_audit_metadata(
    invocation: &ValidatedToolInvocation,
) -> Result<ProtocolMetadata, KernelError> {
    let redactor = PolicyRedactor;
    let resources = invocation
        .resources()
        .iter()
        .map(|resource| {
            ToolAuditResource::new(
                resource.scheme(),
                redactor.redact_resource(resource.scheme(), resource.locator()),
                resource.access(),
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| audit_error(&error))?;
    ToolAuditMetadata::new(
        invocation.spec().version().clone(),
        invocation.source().clone(),
        invocation.spec().effects().iter().cloned(),
        resources,
    )
    .and_then(|metadata| metadata.to_protocol_metadata())
    .map_err(|error| audit_error(&error))
}

fn audit_error(error: &tea_tools::ToolAuditMetadataError) -> KernelError {
    KernelError::new(
        KernelErrorCode::InvalidState,
        format!("validated tool audit metadata is invalid: {error}"),
    )
}
