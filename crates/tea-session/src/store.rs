use std::future::Future;
use std::pin::Pin;

use tea_policy::{ActorId, PolicyGrant};
use tea_protocol::{RecordEnvelope, SessionId, SessionSequence};

use crate::{
    ApprovalArtifactEntry, GrantJournalEntry, MaterializedSessionState, SessionStoreError,
};

/// Runtime-neutral boxed future returned by session storage ports.
pub type SessionStoreFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, SessionStoreError>> + Send + 'a>>;

/// Atomic append request guarded by the caller's expected session tail.
#[derive(Debug, Clone)]
pub struct AppendTransaction {
    session_id: SessionId,
    expected_sequence: Option<SessionSequence>,
    records: Vec<RecordEnvelope>,
    expected_journal_revision: Option<u64>,
    approval_artifacts: Vec<ApprovalArtifactEntry>,
    grant_entries: Vec<GrantJournalEntry>,
}

impl AppendTransaction {
    /// Creates a canonical-record append transaction.
    #[must_use]
    pub fn new(
        session_id: SessionId,
        expected_sequence: Option<SessionSequence>,
        records: Vec<RecordEnvelope>,
    ) -> Self {
        Self {
            session_id,
            expected_sequence,
            records,
            expected_journal_revision: None,
            approval_artifacts: Vec::new(),
            grant_entries: Vec::new(),
        }
    }

    /// Guards typed side-journal writes with the expected current revision.
    #[must_use]
    pub const fn with_expected_journal_revision(mut self, revision: u64) -> Self {
        self.expected_journal_revision = Some(revision);
        self
    }

    /// Attaches typed approval artifacts committed with canonical transitions.
    #[must_use]
    pub fn with_approval_artifacts(
        mut self,
        entries: impl IntoIterator<Item = ApprovalArtifactEntry>,
    ) -> Self {
        self.approval_artifacts = entries.into_iter().collect();
        self
    }

    /// Attaches append-only grant journal facts committed with this transaction.
    #[must_use]
    pub fn with_grant_entries(
        mut self,
        entries: impl IntoIterator<Item = GrantJournalEntry>,
    ) -> Self {
        self.grant_entries = entries.into_iter().collect();
        self
    }

    /// Returns the target session.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Returns expected existing tail, or `None` for creation.
    #[must_use]
    pub const fn expected_sequence(&self) -> Option<SessionSequence> {
        self.expected_sequence
    }

    /// Returns ordered canonical records.
    #[must_use]
    pub fn records(&self) -> &[RecordEnvelope] {
        &self.records
    }

    /// Returns expected policy journal revision, when side facts are appended.
    #[must_use]
    pub const fn expected_journal_revision(&self) -> Option<u64> {
        self.expected_journal_revision
    }

    /// Returns typed approval side-journal entries.
    #[must_use]
    pub fn approval_artifacts(&self) -> &[ApprovalArtifactEntry] {
        &self.approval_artifacts
    }

    /// Returns typed grant side-journal entries.
    #[must_use]
    pub fn grant_entries(&self) -> &[GrantJournalEntry] {
        &self.grant_entries
    }
}

/// Successful append details and rebuilt current projection.
#[derive(Debug, Clone, PartialEq)]
pub struct AppendOutcome {
    previous_sequence: Option<SessionSequence>,
    current_sequence: SessionSequence,
    state: MaterializedSessionState,
    journal_revision: u64,
}

impl AppendOutcome {
    pub(crate) const fn new(
        previous_sequence: Option<SessionSequence>,
        current_sequence: SessionSequence,
        state: MaterializedSessionState,
        journal_revision: u64,
    ) -> Self {
        Self {
            previous_sequence,
            current_sequence,
            state,
            journal_revision,
        }
    }

    /// Returns the durable tail before this append, or `None` on creation.
    #[must_use]
    pub const fn previous_sequence(&self) -> Option<SessionSequence> {
        self.previous_sequence
    }

    /// Returns the durable tail after this append.
    #[must_use]
    pub const fn current_sequence(&self) -> SessionSequence {
        self.current_sequence
    }

    /// Returns policy side-journal revision after this transaction.
    #[must_use]
    pub const fn journal_revision(&self) -> u64 {
        self.journal_revision
    }

    /// Returns the materialized state committed with this append.
    #[must_use]
    pub const fn state(&self) -> &MaterializedSessionState {
        &self.state
    }
}

/// Complete immutable read view of one stored session.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSnapshot {
    records: Vec<RecordEnvelope>,
    state: MaterializedSessionState,
    approval_artifacts: Vec<ApprovalArtifactEntry>,
    grant_journal: Vec<GrantJournalEntry>,
    active_grants: Vec<PolicyGrant>,
    journal_revision: u64,
}

impl SessionSnapshot {
    pub(crate) const fn new(
        records: Vec<RecordEnvelope>,
        state: MaterializedSessionState,
        approval_artifacts: Vec<ApprovalArtifactEntry>,
        grant_journal: Vec<GrantJournalEntry>,
        active_grants: Vec<PolicyGrant>,
        journal_revision: u64,
    ) -> Self {
        Self {
            records,
            state,
            approval_artifacts,
            grant_journal,
            active_grants,
            journal_revision,
        }
    }

    /// Returns canonical source records in authoritative sequence order.
    #[must_use]
    pub fn records(&self) -> &[RecordEnvelope] {
        &self.records
    }

    /// Returns current rebuildable projection.
    #[must_use]
    pub const fn state(&self) -> &MaterializedSessionState {
        &self.state
    }

    /// Returns append-only rich approval artifacts.
    #[must_use]
    pub fn approval_artifacts(&self) -> &[ApprovalArtifactEntry] {
        &self.approval_artifacts
    }

    /// Returns append-only grant history.
    #[must_use]
    pub fn grant_journal(&self) -> &[GrantJournalEntry] {
        &self.grant_journal
    }

    /// Returns current typed side-journal revision.
    #[must_use]
    pub const fn journal_revision(&self) -> u64 {
        self.journal_revision
    }

    /// Returns currently non-revoked grant candidates in stable grant-ID order.
    #[must_use]
    pub fn active_grants(&self) -> &[PolicyGrant] {
        &self.active_grants
    }
}

/// Replaceable append-only session repository contract.
pub trait SessionStore: std::fmt::Debug + Send + Sync {
    /// Loads one immutable session snapshot.
    fn load(&self, session_id: SessionId) -> SessionStoreFuture<'_, SessionSnapshot>;

    /// Atomically validates and appends one transaction.
    fn append(&self, transaction: AppendTransaction) -> SessionStoreFuture<'_, AppendOutcome>;

    /// Returns non-revoked grant candidates issued to one actor across sessions.
    fn active_grants_for_actor(
        &self,
        actor_id: ActorId,
    ) -> SessionStoreFuture<'_, Vec<PolicyGrant>>;
}
