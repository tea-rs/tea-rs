# tea-session-sqlite

Durable SQLite session store for `tea-rs`.

The package is `tea-session-sqlite`; Rust code imports it as `tea_session_sqlite`. It implements the `tea_session::SessionStore` contract over a versioned, append-only SQLite event log and reuses the shared `apply_transaction` engine so its observable behavior is identical to the in-memory reference store.

## Schema

One row per canonical record (JSON-encoded envelope), plus side tables for approval artifacts and grant-journal facts. A unique index on `(session_id, sequence)` enforces expected-sequence conflicts; a unique index on grant ids prevents cross-session duplicate issuance. The current schema is version 1 and is validated as a complete layout; incompatible existing schemas are rejected rather than migrated.

## Concurrency

The store uses a single connection guarded by a `Mutex`; appends run in `IMMEDIATE` transactions. A stale expected sequence receives a `SequenceConflict` exactly like the in-memory store. After a process restart, the durable log is the source of truth: replay reconstructs materialized state, pending approvals, and uncertain tool execution.
