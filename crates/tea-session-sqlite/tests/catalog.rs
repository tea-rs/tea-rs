use std::str::FromStr;

use tea_protocol::{
    ProfileId, ProtocolMetadata, ProtocolTimestamp, RecordEnvelope, RecordId, SessionId,
    SessionRecord, SessionSequence,
};
use tea_session::{AppendTransaction, SessionCatalog, SessionName, SessionStore};
use tea_session_sqlite::SqliteSessionStore;

const NOW: &str = "2026-07-24T09:30:12.125Z";

fn session_id() -> SessionId {
    SessionId::from_str("0195a0b1-5e3a-7000-8000-000000000011").unwrap()
}

fn creation() -> RecordEnvelope {
    RecordEnvelope::new(
        RecordId::from_str("0195a0b1-5e4a-7000-8000-000000000011").unwrap(),
        session_id(),
        SessionSequence::new(0),
        ProtocolTimestamp::from_str(NOW).unwrap(),
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
async fn sqlite_catalog_name_and_listing_survive_reopen() {
    let path = format!("/tmp/tea-catalog-{}.sqlite", std::process::id());
    let _ = std::fs::remove_file(&path);
    {
        let store = SqliteSessionStore::open(&path).unwrap();
        store
            .append(AppendTransaction::new(session_id(), None, vec![creation()]))
            .await
            .unwrap();
        store
            .set_session_name(
                session_id(),
                Some(SessionName::new("Persistent CLI").unwrap()),
            )
            .await
            .unwrap();
    }

    let reopened = SqliteSessionStore::open(&path).unwrap();
    let sessions = reopened.list_sessions().await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id(), session_id());
    assert_eq!(sessions[0].name().unwrap().as_str(), "Persistent CLI");
    assert_eq!(reopened.schema_version(), 1);
    let _ = std::fs::remove_file(path);
}
