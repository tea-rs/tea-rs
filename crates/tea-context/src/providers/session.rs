use std::str::FromStr;

use tea_protocol::RecordId;

use crate::{
    BudgetBehavior, CacheScope, ConflictClaim, ConflictKey, ConflictMode, ContextError,
    ContextErrorCode, ContextProvider, ContextProviderFuture, ContextProviderId, ContextRequest,
    PromptAuthority, PromptModule, PromptModuleId, PromptPriority, PromptProvenance, PromptSegment,
    PromptSegmentId, TrustLevel,
};

/// Explicit optional session-summary insertion point.
#[derive(Debug, Clone)]
pub struct SessionSummaryProvider {
    id: ContextProviderId,
    summary: Option<(String, RecordId, TrustLevel)>,
}

impl SessionSummaryProvider {
    /// Creates an empty optional summary provider.
    ///
    /// # Errors
    ///
    /// Returns an error only when a static built-in identity is invalid.
    pub fn empty() -> Result<Self, ContextError> {
        Ok(Self {
            id: ContextProviderId::from_str("builtin.session_summary").map_err(value_error)?,
            summary: None,
        })
    }

    /// Creates a provider from a caller-supplied durable summary snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when summary content violates prompt bounds.
    pub fn new(
        summary: impl Into<String>,
        record_id: RecordId,
        trust: TrustLevel,
    ) -> Result<Self, ContextError> {
        let summary = summary.into();
        let id = ContextProviderId::from_str("builtin.session_summary").map_err(value_error)?;
        validate(&id, &summary, record_id, trust)?;
        Ok(Self {
            id,
            summary: Some((summary, record_id, trust)),
        })
    }
}

impl ContextProvider for SessionSummaryProvider {
    fn id(&self) -> &ContextProviderId {
        &self.id
    }

    fn provide(&self, _request: ContextRequest) -> ContextProviderFuture<'_> {
        let id = self.id.clone();
        let summary = self.summary.clone();
        Box::pin(async move {
            let Some((summary, record_id, trust)) = summary else {
                return Ok(Vec::new());
            };
            let segment = PromptSegment::new(
                PromptSegmentId::from_str("session.summary").map_err(value_error)?,
                summary,
                PromptProvenance::new(id, "session_record", Some(record_id.to_string()))
                    .map_err(value_error)?,
                trust,
                CacheScope::Session,
                BudgetBehavior::Omit,
            )
            .map_err(value_error)?
            .with_conflict(ConflictClaim::new(
                ConflictKey::from_str("session.summary").map_err(value_error)?,
                ConflictMode::Replaceable,
            ));
            Ok(vec![
                PromptModule::new(
                    PromptModuleId::from_str("session.summary").map_err(value_error)?,
                    PromptAuthority::Session,
                    PromptPriority::new(0),
                    vec![segment],
                )
                .map_err(value_error)?,
            ])
        })
    }
}

fn validate(
    id: &ContextProviderId,
    summary: &str,
    record_id: RecordId,
    trust: TrustLevel,
) -> Result<(), ContextError> {
    PromptSegment::new(
        PromptSegmentId::from_str("session.summary").map_err(value_error)?,
        summary,
        PromptProvenance::new(id.clone(), "session_record", Some(record_id.to_string()))
            .map_err(value_error)?,
        trust,
        CacheScope::Session,
        BudgetBehavior::Omit,
    )
    .map(|_| ())
    .map_err(value_error)
}

fn value_error(error: impl std::fmt::Display) -> ContextError {
    ContextError::new(ContextErrorCode::InvalidValue, error.to_string())
}
