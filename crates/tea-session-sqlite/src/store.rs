use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension as _};
use tea_policy::{ActorId, GrantId, PolicyGrant};
use tea_protocol::SessionId;
use tea_session::{
    AppendOutcome, AppendTransaction, ApprovalArtifactEntry, GrantJournalEntry,
    SessionCatalogEntry, SessionName, SessionSnapshot, SessionStore, SessionStoreError,
    SessionStoreErrorCode, SessionStoreFuture, StoredSession, apply_transaction_in_place,
};
use tokio::sync::oneshot;

use crate::error::SqliteSessionError;
use crate::schema::{CURRENT_SCHEMA_VERSION, ensure_schema};

/// Durable `SQLite` session store implementing `SessionStore`.
///
/// One dedicated blocking worker owns the synchronous connection. Async callers
/// exchange typed commands with that worker and never execute `rusqlite` on a
/// runtime worker thread.
#[derive(Debug)]
pub struct SqliteSessionStore {
    sender: mpsc::Sender<WorkerCommand>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl SqliteSessionStore {
    /// Opens or creates a store at the supplied `SQLite` path. Use `:memory:` for
    /// an ephemeral database.
    ///
    /// # Errors
    ///
    /// Returns an error when the database cannot be opened, initialized, or validated.
    pub fn open(path: &str) -> Result<Self, SqliteSessionError> {
        Self::from_connection(Connection::open(path)?)
    }

    /// Creates an in-memory store (single connection, process-local).
    ///
    /// # Errors
    ///
    /// Returns an error when the database cannot be created or validated.
    pub fn in_memory() -> Result<Self, SqliteSessionError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    /// Returns the installed schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        CURRENT_SCHEMA_VERSION
    }

    fn from_connection(mut connection: Connection) -> Result<Self, SqliteSessionError> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        ensure_schema(&mut connection)?;
        connection.execute_batch("PRAGMA journal_mode = WAL;")?;
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("tea-sqlite-session".to_owned())
            .spawn(move || Worker::new(connection).run(&receiver))
            .map_err(|error| SqliteSessionError::Sqlite(error.to_string()))?;
        Ok(Self {
            sender,
            worker: Mutex::new(Some(worker)),
        })
    }
}

impl Drop for SqliteSessionStore {
    fn drop(&mut self) {
        let _ = self.sender.send(WorkerCommand::Shutdown);
        if let Ok(worker) = self.worker.get_mut()
            && let Some(worker) = worker.take()
        {
            let _ = worker.join();
        }
    }
}

impl tea_session::SessionCatalog for SqliteSessionStore {
    fn list_sessions(&self) -> SessionStoreFuture<'_, Vec<SessionCatalogEntry>> {
        let sender = self.sender.clone();
        Box::pin(async move {
            let (reply, receiver) = oneshot::channel();
            send(&sender, WorkerCommand::ListSessions { reply })?;
            receive(receiver).await
        })
    }

    fn set_session_name(
        &self,
        session_id: SessionId,
        name: Option<SessionName>,
    ) -> SessionStoreFuture<'_, ()> {
        let sender = self.sender.clone();
        Box::pin(async move {
            let (reply, receiver) = oneshot::channel();
            send(
                &sender,
                WorkerCommand::SetSessionName {
                    session_id,
                    name,
                    reply,
                },
            )?;
            receive(receiver).await
        })
    }

    fn session_name(&self, session_id: SessionId) -> SessionStoreFuture<'_, Option<SessionName>> {
        let sender = self.sender.clone();
        Box::pin(async move {
            let (reply, receiver) = oneshot::channel();
            send(&sender, WorkerCommand::SessionName { session_id, reply })?;
            receive(receiver).await
        })
    }
}

impl SessionStore for SqliteSessionStore {
    fn load(&self, session_id: SessionId) -> SessionStoreFuture<'_, SessionSnapshot> {
        let sender = self.sender.clone();
        Box::pin(async move {
            let (reply, receiver) = oneshot::channel();
            send(&sender, WorkerCommand::Load { session_id, reply })?;
            receive(receiver).await
        })
    }

    fn append(&self, transaction: AppendTransaction) -> SessionStoreFuture<'_, AppendOutcome> {
        let sender = self.sender.clone();
        Box::pin(async move {
            let (reply, receiver) = oneshot::channel();
            send(&sender, WorkerCommand::Append { transaction, reply })?;
            receive(receiver).await
        })
    }

    fn active_grants_for_actor(
        &self,
        actor_id: ActorId,
    ) -> SessionStoreFuture<'_, Vec<PolicyGrant>> {
        let sender = self.sender.clone();
        Box::pin(async move {
            let (reply, receiver) = oneshot::channel();
            send(&sender, WorkerCommand::ActiveGrants { actor_id, reply })?;
            receive(receiver).await
        })
    }
}

fn send(
    sender: &mpsc::Sender<WorkerCommand>,
    command: WorkerCommand,
) -> Result<(), SessionStoreError> {
    sender.send(command).map_err(|_| worker_unavailable())
}

async fn receive<T>(
    receiver: oneshot::Receiver<Result<T, SessionStoreError>>,
) -> Result<T, SessionStoreError> {
    receiver.await.map_err(|_| worker_unavailable())?
}

fn worker_unavailable() -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorCode::StorageUnavailable,
        "sqlite session worker is unavailable",
    )
}

enum WorkerCommand {
    Load {
        session_id: SessionId,
        reply: oneshot::Sender<Result<SessionSnapshot, SessionStoreError>>,
    },
    Append {
        transaction: AppendTransaction,
        reply: oneshot::Sender<Result<AppendOutcome, SessionStoreError>>,
    },
    ActiveGrants {
        actor_id: ActorId,
        reply: oneshot::Sender<Result<Vec<PolicyGrant>, SessionStoreError>>,
    },
    ListSessions {
        reply: oneshot::Sender<Result<Vec<SessionCatalogEntry>, SessionStoreError>>,
    },
    SetSessionName {
        session_id: SessionId,
        name: Option<SessionName>,
        reply: oneshot::Sender<Result<(), SessionStoreError>>,
    },
    SessionName {
        session_id: SessionId,
        reply: oneshot::Sender<Result<Option<SessionName>, SessionStoreError>>,
    },
    Shutdown,
}

struct Worker {
    connection: Connection,
    sessions: BTreeMap<SessionId, StoredSession>,
}

impl Worker {
    fn new(connection: Connection) -> Self {
        Self {
            connection,
            sessions: BTreeMap::new(),
        }
    }

    fn run(mut self, receiver: &mpsc::Receiver<WorkerCommand>) {
        while let Ok(command) = receiver.recv() {
            match command {
                WorkerCommand::Load { session_id, reply } => {
                    let _ = reply.send(self.load(session_id));
                }
                WorkerCommand::Append { transaction, reply } => {
                    let _ = reply.send(self.append(&transaction));
                }
                WorkerCommand::ActiveGrants { actor_id, reply } => {
                    let _ = reply.send(self.active_grants(&actor_id));
                }
                WorkerCommand::ListSessions { reply } => {
                    let _ = reply.send(self.list_sessions());
                }
                WorkerCommand::SetSessionName {
                    session_id,
                    name,
                    reply,
                } => {
                    let _ = reply.send(self.set_session_name(session_id, name.as_ref()));
                }
                WorkerCommand::SessionName { session_id, reply } => {
                    let _ = reply.send(self.session_name(session_id));
                }
                WorkerCommand::Shutdown => break,
            }
        }
    }

    fn load(&mut self, session_id: SessionId) -> Result<SessionSnapshot, SessionStoreError> {
        self.ensure_loaded(session_id)?;
        self.sessions
            .get(&session_id)
            .map(StoredSession::snapshot)
            .ok_or_else(session_not_found)
    }

    fn ensure_loaded(&mut self, session_id: SessionId) -> Result<(), SessionStoreError> {
        if !self.sessions.contains_key(&session_id)
            && let Some(stored) = load_stored(&self.connection, session_id)?
        {
            self.sessions.insert(session_id, stored);
        }
        Ok(())
    }

    fn append(
        &mut self,
        transaction: &AppendTransaction,
    ) -> Result<AppendOutcome, SessionStoreError> {
        let session_id = transaction.session_id();
        self.ensure_loaded(session_id)?;
        let existed = self.sessions.contains_key(&session_id);
        let mut stored = self.sessions.remove(&session_id).unwrap_or_default();
        let previous_grant_count = stored.grant_journal.len();
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(SqliteSessionError::from)
            .map_err(SessionStoreError::from)?;
        if let Err(error) = validate_persisted_expectation(&tx, transaction) {
            self.sessions.insert(session_id, stored);
            return Err(error);
        }
        let known_grant_ids = persisted_grant_ids(&tx, transaction.grant_entries())?;
        let outcome =
            match apply_transaction_in_place(transaction, &mut stored, existed, |grant_id| {
                known_grant_ids.contains(&grant_id)
            }) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.sessions.insert(session_id, stored);
                    return Err(error);
                }
            };
        persist_delta(&tx, transaction, previous_grant_count)
            .and_then(|()| persist_active_grants(&tx, session_id, transaction.grant_entries()))?;
        tx.commit()
            .map_err(SqliteSessionError::from)
            .map_err(SessionStoreError::from)?;
        self.sessions.insert(session_id, stored);
        Ok(outcome)
    }

    fn active_grants(&self, actor_id: &ActorId) -> Result<Vec<PolicyGrant>, SessionStoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT grant_json FROM active_grants
                 WHERE actor_id = ? AND revoked = 0 ORDER BY grant_id",
            )
            .map_err(SqliteSessionError::from)?;
        let rows = statement
            .query_map(rusqlite::params![actor_id.to_string()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(SqliteSessionError::from)?;
        let mut grants = Vec::new();
        for row in rows {
            grants.push(decode_json(&row.map_err(SqliteSessionError::from)?)?);
        }
        Ok(grants)
    }

    fn list_sessions(&mut self) -> Result<Vec<SessionCatalogEntry>, SessionStoreError> {
        let values = self
            .connection
            .prepare("SELECT DISTINCT session_id FROM records ORDER BY session_id")
            .map_err(SqliteSessionError::from)?
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(SqliteSessionError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(SqliteSessionError::from)?;
        let mut entries = Vec::with_capacity(values.len());
        for value in values {
            let session_id = parse_session_id(&value)?;
            self.ensure_loaded(session_id)?;
            let stored = self.sessions.get(&session_id).ok_or_else(|| {
                invalid_record("catalog session records disappeared during listing")
            })?;
            entries.push(SessionCatalogEntry::from_snapshot(
                &stored.snapshot(),
                query_session_name(&self.connection, session_id)?,
            )?);
        }
        entries.sort_by(|left, right| {
            right
                .updated_at()
                .cmp(&left.updated_at())
                .then_with(|| left.session_id().cmp(&right.session_id()))
        });
        Ok(entries)
    }

    fn set_session_name(
        &self,
        session_id: SessionId,
        name: Option<&SessionName>,
    ) -> Result<(), SessionStoreError> {
        require_session(&self.connection, session_id)?;
        match name {
            Some(name) => self.connection.execute(
                "INSERT INTO session_catalog (session_id, display_name) VALUES (?, ?)
                 ON CONFLICT(session_id) DO UPDATE SET display_name = excluded.display_name",
                rusqlite::params![session_id.to_string(), name.as_str()],
            ),
            None => self.connection.execute(
                "DELETE FROM session_catalog WHERE session_id = ?",
                rusqlite::params![session_id.to_string()],
            ),
        }
        .map_err(SqliteSessionError::from)?;
        Ok(())
    }

    fn session_name(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionName>, SessionStoreError> {
        require_session(&self.connection, session_id)?;
        query_session_name(&self.connection, session_id)
    }
}

fn load_stored(
    connection: &Connection,
    session_id: SessionId,
) -> Result<Option<StoredSession>, SessionStoreError> {
    let record_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM records WHERE session_id = ?",
            rusqlite::params![session_id.to_string()],
            |row| row.get(0),
        )
        .map_err(SqliteSessionError::from)?;
    if record_count == 0 {
        return Ok(None);
    }
    StoredSession::from_durable_facts(
        load_records(connection, session_id)?,
        load_approval_artifacts(connection, session_id)?,
        load_grant_journal(connection, session_id)?,
    )
    .map(Some)
}

fn load_records(
    connection: &Connection,
    session_id: SessionId,
) -> Result<Vec<tea_protocol::RecordEnvelope>, SessionStoreError> {
    let mut statement = connection
        .prepare("SELECT envelope FROM records WHERE session_id = ? ORDER BY sequence")
        .map_err(SqliteSessionError::from)?;
    let rows = statement
        .query_map(rusqlite::params![session_id.to_string()], |row| {
            row.get::<_, String>(0)
        })
        .map_err(SqliteSessionError::from)?;
    let mut records = Vec::new();
    for row in rows {
        let value: serde_json::Value = decode_json(&row.map_err(SqliteSessionError::from)?)?;
        records.push(
            tea_protocol::RecordEnvelope::decode_value(value)
                .map_err(|error| invalid_record(&error.to_string()))?,
        );
    }
    Ok(records)
}

fn load_approval_artifacts(
    connection: &Connection,
    session_id: SessionId,
) -> Result<Vec<ApprovalArtifactEntry>, SessionStoreError> {
    load_json_rows(
        connection,
        "SELECT envelope FROM approval_artifacts WHERE session_id = ? ORDER BY record_id",
        session_id,
    )
}

fn load_grant_journal(
    connection: &Connection,
    session_id: SessionId,
) -> Result<Vec<GrantJournalEntry>, SessionStoreError> {
    load_json_rows(
        connection,
        "SELECT envelope FROM grant_journal WHERE session_id = ? ORDER BY seq",
        session_id,
    )
}

fn load_json_rows<T: serde::de::DeserializeOwned>(
    connection: &Connection,
    sql: &str,
    session_id: SessionId,
) -> Result<Vec<T>, SessionStoreError> {
    let mut statement = connection.prepare(sql).map_err(SqliteSessionError::from)?;
    let rows = statement
        .query_map(rusqlite::params![session_id.to_string()], |row| {
            row.get::<_, String>(0)
        })
        .map_err(SqliteSessionError::from)?;
    let mut values = Vec::new();
    for row in rows {
        values.push(decode_json(&row.map_err(SqliteSessionError::from)?)?);
    }
    Ok(values)
}

fn validate_persisted_expectation(
    tx: &rusqlite::Transaction<'_>,
    transaction: &AppendTransaction,
) -> Result<(), SessionStoreError> {
    let tail: Option<i64> = tx
        .query_row(
            "SELECT MAX(sequence) FROM records WHERE session_id = ?",
            rusqlite::params![transaction.session_id().to_string()],
            |row| row.get(0),
        )
        .map_err(SqliteSessionError::from)?;
    match (tail, transaction.expected_sequence()) {
        (None, None) => {}
        (None, Some(_)) => return Err(session_not_found()),
        (Some(_), None) => {
            return Err(SessionStoreError::new(
                SessionStoreErrorCode::SessionAlreadyExists,
                "session already exists",
            ));
        }
        (Some(tail), Some(expected)) if u64::try_from(tail).ok() == Some(expected.get()) => {}
        (Some(_), Some(_)) => return Err(sequence_conflict("expected session sequence is stale")),
    }
    let side_entries = transaction
        .approval_artifacts()
        .len()
        .saturating_add(transaction.grant_entries().len());
    if side_entries > 0 {
        let revision: i64 = tx
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM approval_artifacts WHERE session_id = ?1) +
                    (SELECT COUNT(*) FROM grant_journal WHERE session_id = ?1)",
                rusqlite::params![transaction.session_id().to_string()],
                |row| row.get(0),
            )
            .map_err(SqliteSessionError::from)?;
        if u64::try_from(revision).ok() != transaction.expected_journal_revision() {
            return Err(sequence_conflict(
                "expected policy journal revision is stale",
            ));
        }
    }
    Ok(())
}

fn persisted_grant_ids(
    tx: &rusqlite::Transaction<'_>,
    entries: &[GrantJournalEntry],
) -> Result<HashSet<GrantId>, SessionStoreError> {
    let mut ids = HashSet::new();
    for entry in entries {
        if !matches!(entry, GrantJournalEntry::Issued { .. }) {
            continue;
        }
        let grant_id = entry.grant_id();
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM active_grants WHERE grant_id = ?)",
                rusqlite::params![grant_id.to_string()],
                |row| row.get(0),
            )
            .map_err(SqliteSessionError::from)?;
        if exists {
            ids.insert(grant_id);
        }
    }
    Ok(ids)
}

fn persist_delta(
    tx: &rusqlite::Transaction<'_>,
    transaction: &AppendTransaction,
    previous_grant_count: usize,
) -> Result<(), SessionStoreError> {
    let session_id = transaction.session_id().to_string();
    for record in transaction.records() {
        tx.execute(
            "INSERT INTO records (session_id, sequence, record_id, envelope) VALUES (?, ?, ?, ?)",
            rusqlite::params![
                session_id,
                i64::try_from(record.sequence().get()).unwrap_or(i64::MAX),
                record.record_id().to_string(),
                encode_json(record)?,
            ],
        )
        .map_err(SqliteSessionError::from)?;
    }
    for artifact in transaction.approval_artifacts() {
        tx.execute(
            "INSERT INTO approval_artifacts (session_id, record_id, envelope) VALUES (?, ?, ?)",
            rusqlite::params![
                session_id,
                artifact.record_id().to_string(),
                encode_json(artifact)?
            ],
        )
        .map_err(SqliteSessionError::from)?;
    }
    for (offset, entry) in transaction.grant_entries().iter().enumerate() {
        let sequence = previous_grant_count
            .checked_add(offset)
            .and_then(|value| i64::try_from(value).ok())
            .ok_or_else(|| sequence_conflict("grant journal sequence is out of range"))?;
        tx.execute(
            "INSERT INTO grant_journal (session_id, seq, grant_id, envelope) VALUES (?, ?, ?, ?)",
            rusqlite::params![
                session_id,
                sequence,
                entry.grant_id().to_string(),
                encode_json(entry)?
            ],
        )
        .map_err(SqliteSessionError::from)?;
    }
    Ok(())
}

fn persist_active_grants(
    tx: &rusqlite::Transaction<'_>,
    session_id: SessionId,
    entries: &[GrantJournalEntry],
) -> Result<(), SessionStoreError> {
    for entry in entries {
        match entry {
            GrantJournalEntry::Issued { grant, .. } => {
                tx.execute(
                    "INSERT INTO active_grants
                     (grant_id, session_id, actor_id, grant_json, revoked)
                     VALUES (?, ?, ?, ?, 0)",
                    rusqlite::params![
                        grant.id().to_string(),
                        session_id.to_string(),
                        grant.actor_id().to_string(),
                        encode_json(grant)?,
                    ],
                )
                .map_err(SqliteSessionError::from)?;
            }
            GrantJournalEntry::Revoked { grant } => {
                let updated = tx
                    .execute(
                        "UPDATE active_grants SET grant_json = ?, revoked = 1
                         WHERE grant_id = ? AND session_id = ?",
                        rusqlite::params![
                            encode_json(grant)?,
                            grant.id().to_string(),
                            session_id.to_string(),
                        ],
                    )
                    .map_err(SqliteSessionError::from)?;
                if updated != 1 {
                    return Err(invalid_record(
                        "revoked grant is missing from materialized index",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn require_session(
    connection: &Connection,
    session_id: SessionId,
) -> Result<(), SessionStoreError> {
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM records WHERE session_id = ?)",
            rusqlite::params![session_id.to_string()],
            |row| row.get(0),
        )
        .map_err(SqliteSessionError::from)?;
    exists.then_some(()).ok_or_else(session_not_found)
}

fn query_session_name(
    connection: &Connection,
    session_id: SessionId,
) -> Result<Option<SessionName>, SessionStoreError> {
    let value = connection
        .query_row(
            "SELECT display_name FROM session_catalog WHERE session_id = ?",
            rusqlite::params![session_id.to_string()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(SqliteSessionError::from)?;
    value
        .map(|name| name.parse())
        .transpose()
        .map_err(|_| invalid_record("stored session name is invalid"))
}

fn parse_session_id(value: &str) -> Result<SessionId, SessionStoreError> {
    value
        .parse()
        .map_err(|_| invalid_record("stored session id is not canonical"))
}

fn encode_json(value: &impl serde::Serialize) -> Result<String, SessionStoreError> {
    serde_json::to_string(value)
        .map_err(|error| SqliteSessionError::Serialization(error.to_string()).into())
}

fn decode_json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, SessionStoreError> {
    serde_json::from_str(value)
        .map_err(|error| SqliteSessionError::Serialization(error.to_string()).into())
}

fn session_not_found() -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorCode::SessionNotFound,
        "session does not exist",
    )
}

fn sequence_conflict(message: &str) -> SessionStoreError {
    SessionStoreError::new(SessionStoreErrorCode::SequenceConflict, message)
}

fn invalid_record(message: &str) -> SessionStoreError {
    SessionStoreError::new(SessionStoreErrorCode::InvalidRecord, message)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use serde_json::json;
    use tea_protocol::RecordId;

    use super::*;

    fn grant() -> PolicyGrant {
        serde_json::from_value(json!({
            "id": "0195a0b1-5e69-70ac-807e-0aa7aa000047",
            "actorId": "user:alice",
            "profileId": "minimal-assistant",
            "toolName": "write_text_file",
            "toolVersion": "1.0.0",
            "effects": ["fs.write"],
            "resources": [{
                "scheme": "file",
                "locatorPrefix": "/workspace/",
                "access": "write"
            }],
            "scope": {
                "type": "session_resource",
                "session_id": "0195a0b1-5e3a-7d72-a902-c4e85d828bf1"
            },
            "issuedAt": "2026-07-23T09:30:12.006Z"
        }))
        .unwrap()
    }

    #[test]
    fn active_grants_are_written_through_and_revoked_in_place() {
        let mut connection = Connection::open_in_memory().unwrap();
        ensure_schema(&mut connection).unwrap();
        let session_id = SessionId::from_str("0195a0b1-5e3a-7d72-a902-c4e85d828bf1").unwrap();
        let issued_grant = grant();
        let issued = GrantJournalEntry::Issued {
            approval_record_id: RecordId::from_str("0195a0b1-5e54-7c92-b8ca-0aa7aa000026").unwrap(),
            grant: issued_grant.clone(),
        };
        let transaction = connection.transaction().unwrap();
        persist_active_grants(&transaction, session_id, &[issued]).unwrap();
        transaction.commit().unwrap();

        let (stored, revoked): (String, bool) = connection
            .query_row(
                "SELECT grant_json, revoked FROM active_grants WHERE grant_id = ?",
                [issued_grant.id().to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<PolicyGrant>(&stored).unwrap(),
            issued_grant
        );
        assert!(!revoked);

        let revoked_grant = issued_grant
            .revoke("2026-07-23T10:00:00.000Z".parse().unwrap())
            .unwrap();
        let transaction = connection.transaction().unwrap();
        persist_active_grants(
            &transaction,
            session_id,
            &[GrantJournalEntry::Revoked {
                grant: revoked_grant.clone(),
            }],
        )
        .unwrap();
        transaction.commit().unwrap();

        let (stored, revoked): (String, bool) = connection
            .query_row(
                "SELECT grant_json, revoked FROM active_grants WHERE grant_id = ?",
                [revoked_grant.id().to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<PolicyGrant>(&stored).unwrap(),
            revoked_grant
        );
        assert!(revoked);
    }
}
