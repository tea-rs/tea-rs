use thiserror::Error;

/// Bounded failure returned by the `SQLite` session store.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SqliteSessionError {
    /// The database schema is unsupported or structurally incompatible.
    #[error("sqlite schema failure: {0}")]
    Schema(String),
    /// A `SQLite` operation failed.
    #[error("sqlite failure: {0}")]
    Sqlite(String),
    /// A canonical record or journal entry failed to serialize.
    #[error("serialization failure: {0}")]
    Serialization(String),
}

impl From<rusqlite::Error> for SqliteSessionError {
    fn from(error: rusqlite::Error) -> Self {
        match error {
            rusqlite::Error::SqliteFailure(error, Some(message))
                if error.extended_code == rusqlite::ffi::SQLITE_SCHEMA =>
            {
                Self::Schema(message)
            }
            error => Self::Sqlite(error.to_string()),
        }
    }
}

impl From<SqliteSessionError> for tea_session::SessionStoreError {
    fn from(error: SqliteSessionError) -> Self {
        use tea_session::SessionStoreErrorCode;
        let code = match &error {
            SqliteSessionError::Schema(_) => SessionStoreErrorCode::UnsupportedSchemaVersion,
            SqliteSessionError::Sqlite(message) if message.contains("constraint") => {
                SessionStoreErrorCode::SequenceConflict
            }
            SqliteSessionError::Sqlite(_) => SessionStoreErrorCode::StorageUnavailable,
            SqliteSessionError::Serialization(_) => SessionStoreErrorCode::InvalidRecord,
        };
        tea_session::SessionStoreError::new(code, error.to_string())
    }
}
