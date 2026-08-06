# tea-session

Append-only session records, deterministic replay, materialized state, branches, approval/grant journals, and storage contracts for `tea-rs`.

The package is `tea-session`; Rust code imports it as `tea_session`. Its normal
dependency graph is Tokio-free and contains no model provider, tool executor,
JSON Schema compiler, filesystem, process, network, database, or wall-clock
implementation.

## Source of truth and replay

Canonical Protocol 1.0 `RecordEnvelope` values are authoritative. `SessionReducer` requires:

- one `SessionCreated` record at sequence zero;
- contiguous session-local `SessionSequence` values;
- one session ID and globally unique record/message/tool-call/approval IDs;
- matching assistant tool declarations, policy/approval ordering, tool terminals, and tool-result messages;
- valid checkpoint, interruption, cancellation, compaction, and branch references.

`MaterializedSessionState` is a rebuildable projection. It exposes the active transcript and configuration, pending approvals, tool recovery state, run terminals, checkpoints, compaction provenance, branch summaries, and active branch. Reducer failures never partially mutate incremental state.

## Branches and recovery

A branch-aware session places its root `branchId` on `SessionCreated`. `BranchCreated` clones the source projection at a durable record in source ancestry; `ActiveBranchChanged` selects it. Forks never rewrite source records or the parent leaf. Fork points with pending approvals or incomplete tool calls are rejected to avoid duplicating uncertain external effects.

Existing Protocol 1.0 logs without a root branch remain replayable as legacy unbranched sessions.

Started tools interrupted before a terminal result remain explicitly uncertain with execution target and idempotency. Non-idempotent work is never replayed by this crate. Provider streams become interrupted run state rather than invented completed messages.

## Store and policy journals

`SessionStore` is object-safe and returns project-owned boxed futures without exposing Tokio. `InMemorySessionStore` is the semantic reference implementation used to verify storage adapters such as `tea-session-sqlite`.

An `AppendTransaction` is all-or-nothing:

- canonical appends use expected session sequence;
- typed approval/grant journals use an independent fact-count revision;
- rich approval snapshots validate canonical approval/tool/profile/time context;
- a grant issued by `AllowSession` is committed with the matching rich resolution;
- revocation appends an immutable revoked grant rather than updating authorization in place.

Active non-revoked grants can be read from one session snapshot or queried by actor across sessions. Grants remain policy candidates only; this crate cannot make them override a deny.

## JSON archive

`SessionArchive` is a versioned interchange/diagnostic format, not a concurrent store. Decoding rejects duplicate keys and oversized collections, preserves protocol compatibility errors, and preflights complete canonical replay and typed journals before one create transaction. Import never merges or renumbers records and cannot leave partial state.

SQLite persistence and schema management belong to `tea-session-sqlite`; this
crate supplies the storage contract, replay semantics, catalog projection, and
recovery state. Retention/deletion policy and automatic replay of uncertain
external work are host responsibilities and are not performed by this crate.
