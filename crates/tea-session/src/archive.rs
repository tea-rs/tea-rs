use std::fmt;

use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Number, Value};
use tea_protocol::{RecordDecodeError, RecordEnvelope, SessionId};
use thiserror::Error;

use crate::artifact::ArtifactState;
use crate::{
    AppendOutcome, AppendTransaction, ApprovalArtifactEntry, GrantJournalEntry, SessionReducer,
    SessionSnapshot, SessionStore, SessionStoreError, SessionStoreErrorCode, SessionStoreFuture,
};

/// Current independent JSON archive format version.
pub const CURRENT_ARCHIVE_FORMAT_VERSION: u32 = 1;
/// Maximum accepted encoded archive size.
pub const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
/// Maximum records or side-journal entries in one archive collection.
pub const MAX_ARCHIVE_ENTRIES: usize = 100_000;

/// Validated interchange/diagnostic representation of one complete session.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionArchive {
    format_version: u32,
    session_id: SessionId,
    records: Vec<RecordEnvelope>,
    approval_artifacts: Vec<ApprovalArtifactEntry>,
    grant_journal: Vec<GrantJournalEntry>,
}

impl SessionArchive {
    /// Builds and validates an archive from complete source facts.
    ///
    /// # Errors
    ///
    /// Returns an error when records, sequence, references, or typed journals
    /// do not form one valid session.
    pub fn new(
        session_id: SessionId,
        records: Vec<RecordEnvelope>,
        approval_artifacts: Vec<ApprovalArtifactEntry>,
        grant_journal: Vec<GrantJournalEntry>,
    ) -> Result<Self, SessionArchiveError> {
        if records.len() > MAX_ARCHIVE_ENTRIES
            || approval_artifacts.len() > MAX_ARCHIVE_ENTRIES
            || grant_journal.len() > MAX_ARCHIVE_ENTRIES
        {
            return Err(SessionArchiveError::OutOfBounds);
        }
        let archive = Self {
            format_version: CURRENT_ARCHIVE_FORMAT_VERSION,
            session_id,
            records,
            approval_artifacts,
            grant_journal,
        };
        archive.validate()?;
        Ok(archive)
    }

    /// Exports one immutable store snapshot after revalidating source facts.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot cannot be represented as a valid
    /// archive.
    pub fn from_snapshot(snapshot: &SessionSnapshot) -> Result<Self, SessionArchiveError> {
        Self::new(
            snapshot.state().session_id(),
            snapshot.records().to_vec(),
            snapshot.approval_artifacts().to_vec(),
            snapshot.grant_journal().to_vec(),
        )
    }

    /// Decodes duplicate-key-safe untrusted JSON and validates the full archive.
    ///
    /// # Errors
    ///
    /// Returns a stable archive error for oversized input, duplicate keys,
    /// unsupported format/protocol records, or invalid session semantics.
    pub fn decode_json(input: &str) -> Result<Self, SessionArchiveError> {
        if input.len() > MAX_ARCHIVE_BYTES {
            return Err(SessionArchiveError::OutOfBounds);
        }
        let mut deserializer = serde_json::Deserializer::from_str(input);
        let value = deserialize_unique_value(&mut deserializer)
            .map_err(|error| SessionArchiveError::Malformed(error.to_string()))?;
        deserializer
            .end()
            .map_err(|error| SessionArchiveError::Malformed(error.to_string()))?;
        Self::decode_value(value)
    }

    /// Imports this complete archive into a store as one create transaction.
    ///
    /// Destination stores reject an existing session and validate all source
    /// facts before making any data visible.
    pub fn import_into(self, store: &dyn SessionStore) -> SessionStoreFuture<'_, AppendOutcome> {
        Box::pin(async move {
            self.validate().map_err(SessionStoreError::from)?;
            let transaction = AppendTransaction::new(self.session_id, None, self.records)
                .with_expected_journal_revision(0)
                .with_approval_artifacts(self.approval_artifacts)
                .with_grant_entries(self.grant_journal);
            store.append(transaction).await
        })
    }

    /// Returns archive format version.
    #[must_use]
    pub const fn format_version(&self) -> u32 {
        self.format_version
    }

    /// Returns archived session identity.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Returns canonical records in authoritative order.
    #[must_use]
    pub fn records(&self) -> &[RecordEnvelope] {
        &self.records
    }

    /// Returns rich approval journal.
    #[must_use]
    pub fn approval_artifacts(&self) -> &[ApprovalArtifactEntry] {
        &self.approval_artifacts
    }

    /// Returns append-only grant journal.
    #[must_use]
    pub fn grant_journal(&self) -> &[GrantJournalEntry] {
        &self.grant_journal
    }

    fn decode_value(value: Value) -> Result<Self, SessionArchiveError> {
        let raw: RawSessionArchive = serde_json::from_value(value)
            .map_err(|error| SessionArchiveError::Malformed(error.to_string()))?;
        if raw.format_version != CURRENT_ARCHIVE_FORMAT_VERSION {
            return Err(SessionArchiveError::UnsupportedFormatVersion(
                raw.format_version,
            ));
        }
        if raw.records.len() > MAX_ARCHIVE_ENTRIES
            || raw.approval_artifacts.len() > MAX_ARCHIVE_ENTRIES
            || raw.grant_journal.len() > MAX_ARCHIVE_ENTRIES
        {
            return Err(SessionArchiveError::OutOfBounds);
        }
        let records = raw
            .records
            .into_iter()
            .map(RecordEnvelope::decode_value)
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(
            raw.session_id,
            records,
            raw.approval_artifacts,
            raw.grant_journal,
        )
    }

    fn validate(&self) -> Result<(), SessionArchiveError> {
        if self.format_version != CURRENT_ARCHIVE_FORMAT_VERSION {
            return Err(SessionArchiveError::UnsupportedFormatVersion(
                self.format_version,
            ));
        }
        if self
            .records
            .iter()
            .any(|record| record.session_id() != self.session_id)
        {
            return Err(SessionArchiveError::SessionMismatch);
        }
        SessionReducer::replay(self.records.clone())?;
        ArtifactState::default().apply(
            self.session_id,
            &self.records,
            &self.records,
            &self.approval_artifacts,
            &self.grant_journal,
        )?;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSessionArchive {
    format_version: u32,
    session_id: SessionId,
    records: Vec<Value>,
    #[serde(default)]
    approval_artifacts: Vec<ApprovalArtifactEntry>,
    #[serde(default)]
    grant_journal: Vec<GrantJournalEntry>,
}

/// Error decoding or validating a session interchange archive.
#[derive(Debug, Error)]
pub enum SessionArchiveError {
    /// Independent archive format is unsupported.
    #[error("unsupported session archive format version: {0}")]
    UnsupportedFormatVersion(u32),
    /// Encoded bytes or collection counts exceed archive bounds.
    #[error("session archive exceeds supported bounds")]
    OutOfBounds,
    /// JSON structure is malformed or contains duplicate keys.
    #[error("malformed session archive: {0}")]
    Malformed(String),
    /// One canonical record belongs to another session.
    #[error("session archive contains another session")]
    SessionMismatch,
    /// Canonical record decode failed.
    #[error("session archive record is invalid: {0}")]
    Record(#[from] RecordDecodeError),
    /// Canonical replay failed.
    #[error("session archive replay failed: {0}")]
    Replay(#[from] crate::SessionReplayError),
    /// Typed approval/grant journal validation failed.
    #[error("session archive policy journal failed: {0}")]
    Artifact(#[from] crate::ArtifactValidationError),
}

impl SessionArchiveError {
    /// Returns stable storage-facing classification.
    #[must_use]
    pub const fn store_code(&self) -> SessionStoreErrorCode {
        match self {
            Self::UnsupportedFormatVersion(_)
            | Self::Record(
                RecordDecodeError::UnsupportedVersion { .. }
                | RecordDecodeError::UnsupportedType { .. },
            ) => SessionStoreErrorCode::UnsupportedSchemaVersion,
            Self::Replay(error) => error.store_code(),
            Self::Artifact(error) => error.store_code(),
            Self::OutOfBounds | Self::Malformed(_) | Self::SessionMismatch | Self::Record(_) => {
                SessionStoreErrorCode::InvalidRecord
            }
        }
    }
}

impl From<SessionArchiveError> for SessionStoreError {
    fn from(error: SessionArchiveError) -> Self {
        Self::new(error.store_code(), error.to_string())
    }
}

fn deserialize_unique_value<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(UniqueValueVisitor)
}

struct UniqueValueVisitor;

impl<'de> Visitor<'de> for UniqueValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_unique_value(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
        while let Some(value) = sequence.next_element_seed(UniqueValueSeed)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(serde::de::Error::custom(format_args!(
                    "duplicate JSON object key: {key}"
                )));
            }
            let value = object.next_value_seed(UniqueValueSeed)?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

struct UniqueValueSeed;

impl<'de> serde::de::DeserializeSeed<'de> for UniqueValueSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_unique_value(deserializer)
    }
}
