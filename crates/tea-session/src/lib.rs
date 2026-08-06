#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Append-only session semantics, deterministic replay, and replaceable storage
//! contracts for `tea-rs`.
//!
//! Canonical [`tea_protocol::RecordEnvelope`] values are the source
//! of truth. [`SessionReducer`] derives rebuildable materialized state without
//! reading clocks, executing tools, contacting providers, or depending on an
//! async runtime.
//!
//! # Replay
//!
//! ```
//! use tea_protocol::RecordEnvelope;
//! use tea_session::SessionReducer;
//!
//! # fn rebuild(records: Vec<RecordEnvelope>) -> Result<(), Box<dyn std::error::Error>> {
//! let state = SessionReducer::replay(records)?;
//! assert_eq!(state.tail_sequence().get() + 1, state.tail_sequence().get() + 1);
//! # Ok(())
//! # }
//! ```

mod archive;
mod artifact;
mod catalog;
mod error;
mod memory;
mod reducer;
mod state;
mod store;
mod store_engine;

pub use archive::{
    CURRENT_ARCHIVE_FORMAT_VERSION, MAX_ARCHIVE_BYTES, MAX_ARCHIVE_ENTRIES, SessionArchive,
    SessionArchiveError,
};
pub use artifact::{
    ApprovalArtifactEntry, ArtifactState, ArtifactValidationError, GrantJournalEntry,
};
pub use catalog::{
    MAX_SESSION_NAME_BYTES, SessionCatalog, SessionCatalogEntry, SessionName, SessionNameError,
};
pub use error::{SessionReplayError, SessionStoreError, SessionStoreErrorCode};
pub use memory::InMemorySessionStore;
pub use reducer::SessionReducer;
pub use state::{
    BranchSummary, MaterializedSessionState, PendingApproval, RunRecoveryState, SessionCompaction,
    SessionConfiguration, ToolCallState, ToolExecutionState, TurnCheckpoint,
};
pub use store::{
    AppendOutcome, AppendTransaction, SessionSnapshot, SessionStore, SessionStoreFuture,
};
pub use store_engine::{
    StoredSession, active_grants_for_actor, apply_transaction, apply_transaction_in_place,
};
