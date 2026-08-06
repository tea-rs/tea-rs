use std::str::FromStr;

use tea_protocol::{
    ProfileId, ProtocolMetadata, ProtocolTimestamp, RecordEnvelope, RecordId, SessionId,
    SessionRecord, SessionSequence,
};
use tea_session::{
    AppendTransaction, InMemorySessionStore, SessionCatalog, SessionName, SessionStore,
};

const NOW: &str = "2026-07-24T09:30:12.125Z";
const LATER: &str = "2026-07-24T09:31:12.125Z";

fn session(value: u16) -> SessionId {
    format!("0195a0b1-5e3a-7000-8000-{value:012}")
        .parse()
        .unwrap()
}

fn record(session_id: SessionId, timestamp: &str, value: u16) -> RecordEnvelope {
    RecordEnvelope::new(
        RecordId::from_str(&format!("0195a0b1-5e4a-7000-8000-{value:012}")).unwrap(),
        session_id,
        SessionSequence::new(0),
        ProtocolTimestamp::from_str(timestamp).unwrap(),
        None,
        None,
        None,
        ProtocolMetadata::default(),
        SessionRecord::SessionCreated {
            profile_id: ProfileId::from_str("coding").unwrap(),
            metadata: ProtocolMetadata::default(),
        },
    )
    .unwrap()
}

#[tokio::test]
async fn in_memory_catalog_lists_recent_sessions_and_names_them() {
    let store = InMemorySessionStore::new();
    for (value, timestamp) in [(1, NOW), (2, LATER)] {
        let id = session(value);
        store
            .append(AppendTransaction::new(
                id,
                None,
                vec![record(id, timestamp, value)],
            ))
            .await
            .unwrap();
    }

    let sessions = store.list_sessions().await.unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].session_id(), session(2));
    assert_eq!(sessions[1].session_id(), session(1));
    assert_eq!(sessions[0].message_count(), 0);

    let name = SessionName::new("Refactor runtime").unwrap();
    store
        .set_session_name(session(2), Some(name.clone()))
        .await
        .unwrap();
    assert_eq!(store.list_sessions().await.unwrap()[0].name(), Some(&name));
    store.set_session_name(session(2), None).await.unwrap();
    assert!(store.list_sessions().await.unwrap()[0].name().is_none());
}

#[test]
fn session_name_is_bounded_and_control_free() {
    assert!(SessionName::new("").is_err());
    assert!(SessionName::new("bad\nname").is_err());
    assert!(SessionName::new("x".repeat(257)).is_err());
    assert_eq!(
        SessionName::new(" Build CLI ").unwrap().as_str(),
        "Build CLI"
    );
}
