//! Shared append-validation and materialization engine for session stores.
//!
//! Both the in-memory reference store and the durable `SQLite` store call into
//! this engine so their observable behavior is identical: expected-sequence
//! conflicts, grant-id deduplication, journal-revision guards, and reducer
//! replay all happen through one code path.

use tea_policy::{ActorId, GrantId, PolicyGrant};
use tea_protocol::{RecordEnvelope, SessionSequence};

use crate::artifact::ArtifactState;
use crate::{
    AppendOutcome, AppendTransaction, ApprovalArtifactEntry, GrantJournalEntry,
    MaterializedSessionState, SessionReducer, SessionSnapshot, SessionStoreError,
    SessionStoreErrorCode,
};

/// Immutable view of one stored session's durable facts.
///
/// A store loads or reconstructs this view before calling [`apply_transaction`].
#[derive(Debug, Clone, Default)]
pub struct StoredSession {
    /// Canonical source records in authoritative sequence order.
    pub records: Vec<RecordEnvelope>,
    /// Rich approval artifacts linked to canonical approval transitions.
    pub approval_artifacts: Vec<ApprovalArtifactEntry>,
    /// Append-only grant journal facts.
    pub grant_journal: Vec<GrantJournalEntry>,
    /// Derived authorization state rebuilt from the journals.
    pub artifacts: ArtifactState,
    /// Current side-journal revision.
    pub journal_revision: u64,
    /// Incremental canonical reducer rebuilt once when durable facts are loaded.
    pub reducer: SessionReducer,
}

impl StoredSession {
    /// Rebuilds one validated stored session from durable facts.
    ///
    /// # Errors
    ///
    /// Returns a replay error when canonical records are corrupt.
    pub fn from_durable_facts(
        records: Vec<RecordEnvelope>,
        approval_artifacts: Vec<ApprovalArtifactEntry>,
        grant_journal: Vec<GrantJournalEntry>,
    ) -> Result<Self, SessionStoreError> {
        let reducer = SessionReducer::replay_reducer(records.iter().cloned())?;
        let artifacts = ArtifactState::rebuild_from_journals(&approval_artifacts, &grant_journal);
        let journal_revision = approval_artifacts
            .len()
            .checked_add(grant_journal.len())
            .and_then(|revision| u64::try_from(revision).ok())
            .ok_or_else(|| sequence_conflict("policy journal revision is out of range"))?;
        Ok(Self {
            records,
            approval_artifacts,
            grant_journal,
            artifacts,
            journal_revision,
            reducer,
        })
    }

    /// Builds a read snapshot from this stored view.
    #[must_use]
    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot::new(
            self.records.clone(),
            self.state(),
            self.approval_artifacts.clone(),
            self.grant_journal.clone(),
            self.artifacts.active_grants(),
            self.journal_revision,
        )
    }

    /// Replays the durable records to rebuild materialized state.
    ///
    /// # Panics
    ///
    /// Panics when the stored records are corrupt; durable stores validate
    /// every transaction before persistence.
    #[must_use]
    pub fn state(&self) -> MaterializedSessionState {
        self.reducer
            .state()
            .cloned()
            .expect("stored records include a validated creation record")
    }
}

/// Validates a transaction and applies it to an existing stored view.
///
/// `grant_id_in_use` returns whether a grant id is already issued across every
/// session (the in-memory store checks its map; the `SQLite` store queries its
/// grant-journal table). Returns the new stored view and the append outcome.
///
/// # Errors
///
/// Returns a typed store error for empty transactions, cross-session records,
/// stale expected sequences, stale journal revisions, duplicate grant ids, or
/// reducer/artifact validation failures.
#[allow(clippy::too_many_lines)]
pub fn apply_transaction(
    transaction: &AppendTransaction,
    existing: Option<&StoredSession>,
    grant_id_in_use: impl Fn(GrantId) -> bool,
) -> Result<(StoredSession, AppendOutcome), SessionStoreError> {
    let existed = existing.is_some();
    let mut stored = existing.cloned().unwrap_or_default();
    let outcome = apply_transaction_in_place(transaction, &mut stored, existed, grant_id_in_use)?;
    Ok((stored, outcome))
}

/// Validates and applies one transaction to an owned stored-session cache entry.
///
/// The success path mutates only the transaction delta. If a record or artifact
/// fails validation, the reducer is rebuilt from the unchanged durable vectors
/// before the error is returned.
///
/// # Errors
///
/// Returns the same stable validation errors as [`apply_transaction`].
#[allow(clippy::too_many_lines)]
pub fn apply_transaction_in_place(
    transaction: &AppendTransaction,
    stored: &mut StoredSession,
    existed: bool,
    grant_id_in_use: impl Fn(GrantId) -> bool,
) -> Result<AppendOutcome, SessionStoreError> {
    validate_transaction_shape(transaction)?;
    validate_expectation(existed.then_some(&*stored), transaction)?;
    validate_global_grant_ids(
        existed.then_some(&*stored),
        transaction.grant_entries(),
        &grant_id_in_use,
    )?;

    let previous_sequence = if existed {
        Some(
            stored
                .reducer
                .state()
                .ok_or_else(missing_materialized_state)?
                .tail_sequence(),
        )
    } else {
        None
    };
    let current_journal_revision = stored.journal_revision;
    let journal_entries = transaction
        .approval_artifacts()
        .len()
        .checked_add(transaction.grant_entries().len())
        .and_then(|count| u64::try_from(count).ok())
        .ok_or_else(|| sequence_conflict("policy journal entry count is out of range"))?;
    if journal_entries > 0
        && transaction.expected_journal_revision() != Some(current_journal_revision)
    {
        return Err(sequence_conflict(
            "expected policy journal revision is stale",
        ));
    }
    let journal_revision = current_journal_revision
        .checked_add(journal_entries)
        .ok_or_else(|| sequence_conflict("policy journal revision cannot advance"))?;

    for record in transaction.records() {
        if let Err(error) = stored.reducer.apply(record) {
            stored.reducer = rebuild_reducer(&stored.records)?;
            return Err(error.into());
        }
    }

    let original_record_len = stored.records.len();
    stored.records.extend_from_slice(transaction.records());
    let mut artifacts = stored.artifacts.clone();
    if let Err(error) = artifacts.apply(
        transaction.session_id(),
        &stored.records,
        transaction.records(),
        transaction.approval_artifacts(),
        transaction.grant_entries(),
    ) {
        stored.records.truncate(original_record_len);
        stored.reducer = rebuild_reducer(&stored.records)?;
        return Err(SessionStoreError::new(
            error.store_code(),
            error.to_string(),
        ));
    }

    stored
        .approval_artifacts
        .extend_from_slice(transaction.approval_artifacts());
    stored
        .grant_journal
        .extend_from_slice(transaction.grant_entries());
    stored.artifacts = artifacts;
    stored.journal_revision = journal_revision;
    let state = stored
        .reducer
        .state()
        .cloned()
        .ok_or_else(missing_materialized_state)?;
    let current_sequence = state.tail_sequence();
    Ok(AppendOutcome::new(
        previous_sequence,
        current_sequence,
        state,
        journal_revision,
    ))
}

fn rebuild_reducer(records: &[RecordEnvelope]) -> Result<SessionReducer, SessionStoreError> {
    if records.is_empty() {
        Ok(SessionReducer::new())
    } else {
        SessionReducer::replay_reducer(records.iter().cloned()).map_err(Into::into)
    }
}

fn missing_materialized_state() -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorCode::CorruptionDetected,
        "stored session is missing materialized state",
    )
}

/// Returns the active grants for one actor across a collection of stored views.
pub fn active_grants_for_actor<'a>(
    sessions: impl Iterator<Item = &'a StoredSession>,
    actor_id: &ActorId,
) -> Vec<PolicyGrant> {
    let mut grants = sessions
        .flat_map(|stored| stored.artifacts.active_grants())
        .filter(|grant| grant.actor_id() == actor_id)
        .collect::<Vec<_>>();
    grants.sort_by_key(PolicyGrant::id);
    grants.dedup_by_key(|grant| grant.id());
    grants
}

pub(crate) fn validate_transaction_shape(
    transaction: &AppendTransaction,
) -> Result<(), SessionStoreError> {
    if transaction.records().is_empty()
        && transaction.approval_artifacts().is_empty()
        && transaction.grant_entries().is_empty()
    {
        return Err(SessionStoreError::new(
            SessionStoreErrorCode::InvalidRecord,
            "append transaction must contain a canonical or typed journal fact",
        ));
    }
    if transaction
        .records()
        .iter()
        .any(|record| record.session_id() != transaction.session_id())
    {
        return Err(SessionStoreError::new(
            SessionStoreErrorCode::InvalidRecord,
            "append transaction contains another session",
        ));
    }
    Ok(())
}

pub(crate) fn validate_expectation(
    existing: Option<&StoredSession>,
    transaction: &AppendTransaction,
) -> Result<(), SessionStoreError> {
    let existing_records =
        existing.map_or(&[] as &[RecordEnvelope], |stored| stored.records.as_slice());
    let tail = existing_records.len().checked_sub(1);
    match (existing_records.is_empty(), transaction.expected_sequence()) {
        (true, None) => {
            let first = transaction.records().first().ok_or_else(|| {
                SessionStoreError::new(
                    SessionStoreErrorCode::InvalidRecord,
                    "session creation requires a canonical creation record",
                )
            })?;
            if first.sequence() != SessionSequence::new(0) {
                return Err(sequence_conflict("new session must begin at sequence zero"));
            }
        }
        (true, Some(_)) => {
            return Err(SessionStoreError::new(
                SessionStoreErrorCode::SessionNotFound,
                "cannot append to a missing session",
            ));
        }
        (false, None) => {
            return Err(SessionStoreError::new(
                SessionStoreErrorCode::SessionAlreadyExists,
                "session already exists",
            ));
        }
        (false, Some(expected)) => {
            let stored_tail = tail
                .and_then(|index| existing_records.get(index))
                .map_or(SessionSequence::new(0), RecordEnvelope::sequence);
            if stored_tail != expected {
                return Err(sequence_conflict("expected session sequence is stale"));
            }
            let next = expected
                .checked_next()
                .ok_or_else(|| sequence_conflict("session sequence cannot advance"))?;
            if transaction
                .records()
                .first()
                .is_some_and(|record| record.sequence() != next)
            {
                return Err(sequence_conflict(
                    "first appended record does not follow expected sequence",
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_global_grant_ids(
    existing: Option<&StoredSession>,
    entries: &[GrantJournalEntry],
    grant_id_in_use: &impl Fn(GrantId) -> bool,
) -> Result<(), SessionStoreError> {
    let mut seen_in_batch = std::collections::HashSet::new();
    for entry in entries {
        if matches!(entry, GrantJournalEntry::Issued { .. }) {
            let id = entry.grant_id();
            if !seen_in_batch.insert(id) {
                return Err(SessionStoreError::new(
                    SessionStoreErrorCode::InvalidRecord,
                    "grant identity is already issued",
                ));
            }
            if grant_id_in_use(id)
                || existing.is_some_and(|stored| {
                    stored
                        .grant_journal
                        .iter()
                        .any(|existing| existing.grant_id() == id)
                })
            {
                return Err(SessionStoreError::new(
                    SessionStoreErrorCode::InvalidRecord,
                    "grant identity is already issued",
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn sequence_conflict(message: &str) -> SessionStoreError {
    SessionStoreError::new(SessionStoreErrorCode::SequenceConflict, message)
}
