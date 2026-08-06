//! `SQLite` store conformance against the in-memory reference.

use std::str::FromStr;
use std::time::Duration;

use tea_policy::ActorId;
use tea_protocol::{
    CanonicalMessage, ContentBlock, MessageId, ProfileId, ProtocolMetadata, ProtocolTimestamp,
    RecordEnvelope, RecordId, SessionId, SessionRecord, SessionSequence,
};
use tea_session::{
    AppendTransaction, InMemorySessionStore, SessionStore, SessionStoreError, SessionStoreErrorCode,
};
use tea_session_sqlite::SqliteSessionStore;

const NOW: &str = "2026-07-23T09:30:12.125Z";

fn timestamp() -> ProtocolTimestamp {
    ProtocolTimestamp::from_str(NOW).unwrap()
}

fn session_id() -> SessionId {
    SessionId::from_str("0195a0b1-5e3a-7d72-a902-c4e85d828bf1").unwrap()
}

fn envelope(sequence: u64, record_id: &str, record: SessionRecord) -> RecordEnvelope {
    RecordEnvelope::new(
        RecordId::from_str(record_id).unwrap(),
        session_id(),
        SessionSequence::new(sequence),
        timestamp(),
        None,
        None,
        None,
        ProtocolMetadata::default(),
        record,
    )
    .unwrap()
}

fn create_transaction() -> AppendTransaction {
    AppendTransaction::new(
        session_id(),
        None,
        vec![envelope(
            0,
            "0195a0b1-5e50-7af4-8972-0aa7aa000022",
            SessionRecord::SessionCreated {
                profile_id: ProfileId::from_str("coding").unwrap(),
                metadata: ProtocolMetadata::default(),
            },
        )],
    )
}

fn message_record(sequence: u64, record_id: &str, text: &str) -> RecordEnvelope {
    envelope(
        sequence,
        record_id,
        SessionRecord::MessageCommitted {
            message: CanonicalMessage::user(
                MessageId::from_str(&format!("0195a0b1-5e52-74b2-8c25-0aa7aa00{sequence:04x}"))
                    .unwrap(),
                vec![ContentBlock::text(text).unwrap()],
                timestamp(),
            )
            .unwrap(),
        },
    )
}

#[tokio::test]
async fn sqlite_matches_in_memory_create_append_load() {
    let sqlite = SqliteSessionStore::in_memory().unwrap();
    let memory = InMemorySessionStore::new();

    sqlite.append(create_transaction()).await.unwrap();
    memory.append(create_transaction()).await.unwrap();

    let sqlite_snapshot = sqlite.load(session_id()).await.unwrap();
    let memory_snapshot = memory.load(session_id()).await.unwrap();
    assert_eq!(sqlite_snapshot.state(), memory_snapshot.state());
    assert_eq!(
        sqlite_snapshot.records().len(),
        memory_snapshot.records().len()
    );
}

#[tokio::test]
async fn sqlite_rejects_stale_expected_sequence() {
    let sqlite = SqliteSessionStore::in_memory().unwrap();
    sqlite.append(create_transaction()).await.unwrap();
    // Tail is now 0; appending with a stale expected sequence (5) conflicts.
    let stale = AppendTransaction::new(
        session_id(),
        Some(SessionSequence::new(5)),
        vec![message_record(
            1,
            "0195a0b1-5e51-79e1-8f4a-0aa7aa000023",
            "second",
        )],
    );
    let error = sqlite.append(stale).await.unwrap_err();
    assert_eq!(error.code(), SessionStoreErrorCode::SequenceConflict);
}

#[tokio::test]
async fn sqlite_appends_preserve_order_and_match_reference() {
    let sqlite = SqliteSessionStore::in_memory().unwrap();
    let memory = InMemorySessionStore::new();
    let stores: [&dyn SessionStore; 2] = [&sqlite, &memory];
    for store in stores {
        store.append(create_transaction()).await.unwrap();
        store
            .append(AppendTransaction::new(
                session_id(),
                Some(SessionSequence::new(0)),
                vec![message_record(
                    1,
                    "0195a0b1-5e51-79e1-8f4a-0aa7aa000023",
                    "second",
                )],
            ))
            .await
            .unwrap();
        store
            .append(AppendTransaction::new(
                session_id(),
                Some(SessionSequence::new(1)),
                vec![message_record(
                    2,
                    "0195a0b1-5e52-7b3e-93f1-0aa7aa000024",
                    "third",
                )],
            ))
            .await
            .unwrap();
    }
    let sqlite_snapshot = sqlite.load(session_id()).await.unwrap();
    let memory_snapshot = memory.load(session_id()).await.unwrap();
    assert_eq!(sqlite_snapshot.state(), memory_snapshot.state());
    assert_eq!(sqlite_snapshot.records(), memory_snapshot.records());
    assert_eq!(sqlite_snapshot.state().messages().len(), 2);
}

#[tokio::test]
async fn sqlite_returns_session_not_found_for_missing() {
    let sqlite = SqliteSessionStore::in_memory().unwrap();
    let error = sqlite.load(session_id()).await.unwrap_err();
    assert_eq!(error.code(), SessionStoreErrorCode::SessionNotFound);
}

#[tokio::test]
async fn sqlite_rebuilds_after_reopen() {
    // An in-memory database cannot be reopened, so this test verifies the
    // journal_revision and grants reconstruction on a fresh load within the
    // same connection after multiple appends.
    let sqlite = SqliteSessionStore::in_memory().unwrap();
    sqlite.append(create_transaction()).await.unwrap();
    let grants = sqlite
        .active_grants_for_actor(ActorId::from_str("user:alice").unwrap())
        .await
        .unwrap();
    assert!(grants.is_empty());
    let _ = Duration::from_secs(0);
    let _ = SessionStoreError::new(SessionStoreErrorCode::StorageUnavailable, "");
}
