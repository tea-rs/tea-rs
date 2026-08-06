#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Durable `SQLite` session store for `tea-rs`.
//!
//! Implements `tea_session::SessionStore` over a versioned,
//! append-only `SQLite` event log. Reuses the shared `apply_transaction` engine
//! so its observable behavior matches the in-memory reference store exactly.
//!
//! # Example
//!
//! ```no_run
//! use tea_session_sqlite::SqliteSessionStore;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let store = SqliteSessionStore::open("agents.sqlite")?;
//! # Ok(())
//! # }
//! ```

mod error;
mod schema;
mod store;

pub use error::SqliteSessionError;
pub use store::SqliteSessionStore;
