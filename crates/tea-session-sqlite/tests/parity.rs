//! Replay parity between the `SQLite` store and the in-memory reference.

use std::str::FromStr;

use tea_protocol::{
    CanonicalMessage, ContentBlock, MessageId, ProfileId, ProtocolMetadata, ProtocolTimestamp,
    RecordEnvelope, RecordId, SessionId, SessionRecord, SessionSequence, StopReason,
};
use tea_session::{AppendTransaction, InMemorySessionStore, SessionSnapshot, SessionStore};
use tea_session_sqlite::SqliteSessionStore;

const NOW: &str = "2026-07-23T09:30:12.125Z";

fn session_id() -> SessionId {
    SessionId::from_str("0195a0b1-5e3a-7d72-a902-c4e85d828bf1").unwrap()
}
fn timestamp() -> ProtocolTimestamp {
    ProtocolTimestamp::from_str(NOW).unwrap()
}
fn envelope(seq: u64, rid: &str, record: SessionRecord) -> RecordEnvelope {
    RecordEnvelope::new(
        RecordId::from_str(rid).unwrap(),
        session_id(),
        SessionSequence::new(seq),
        timestamp(),
        None,
        None,
        None,
        ProtocolMetadata::default(),
        record,
    )
    .unwrap()
}

fn build_log() -> Vec<Vec<RecordEnvelope>> {
    // A non-trivial transcript: create, config, two user messages, an assistant
    // message, and a turn checkpoint. Replaying through either store must
    // yield identical materialized state.
    let mid =
        |n: u8| MessageId::from_str(&format!("0195a0b1-5e52-74b2-8c25-0aa7aa00{n:04x}")).unwrap();
    vec![
        vec![envelope(
            0,
            "0195a0b1-5e50-79e1-8f4a-0aa7aa000001",
            SessionRecord::SessionCreated {
                profile_id: ProfileId::from_str("coding").unwrap(),
                metadata: ProtocolMetadata::default(),
            },
        )],
        vec![envelope(
            1,
            "0195a0b1-5e51-79e1-8f4a-0aa7aa000002",
            SessionRecord::ConfigurationChanged {
                model: Some(tea_protocol::ModelRef::new(
                    "fake".parse().unwrap(),
                    "fake/model".parse().unwrap(),
                )),
                profile_id: None,
                reasoning_effort: None,
            },
        )],
        vec![envelope(
            2,
            "0195a0b1-5e52-79e1-8f4a-0aa7aa000003",
            SessionRecord::MessageCommitted {
                message: CanonicalMessage::user(
                    mid(1),
                    vec![ContentBlock::text("hello").unwrap()],
                    timestamp(),
                )
                .unwrap(),
            },
        )],
        vec![envelope(
            3,
            "0195a0b1-5e53-79e1-8f4a-0aa7aa000004",
            SessionRecord::MessageCommitted {
                message: CanonicalMessage::assistant(
                    mid(2),
                    vec![ContentBlock::text("hi there").unwrap()],
                    StopReason::Completed,
                    timestamp(),
                )
                .unwrap(),
            },
        )],
        vec![envelope(
            4,
            "0195a0b1-5e54-79e1-8f4a-0aa7aa000005",
            SessionRecord::TurnCheckpointed {
                run_id: "0195a0b1-5e6a-7000-8000-0aa7aa000001".parse().unwrap(),
                turn_id: "0195a0b1-5e6b-7000-8000-0aa7aa000002".parse().unwrap(),
                next_action: tea_protocol::NextTurnAction::FinishRun,
            },
        )],
    ]
}

async fn replay_through(store: &dyn SessionStore) -> SessionSnapshot {
    let mut tail = None;
    for records in build_log() {
        let txn = AppendTransaction::new(session_id(), tail, records);
        tail = Some(store.append(txn).await.unwrap().current_sequence());
    }
    store.load(session_id()).await.unwrap()
}

#[tokio::test]
async fn sqlite_replay_matches_in_memory() {
    let sqlite = SqliteSessionStore::in_memory().unwrap();
    let memory = InMemorySessionStore::new();
    let sqlite_snapshot = replay_through(&sqlite).await;
    let memory_snapshot = replay_through(&memory).await;
    assert_eq!(sqlite_snapshot.state(), memory_snapshot.state());
    assert_eq!(sqlite_snapshot.records(), memory_snapshot.records());
    assert_eq!(sqlite_snapshot.state().messages().len(), 2);
}
