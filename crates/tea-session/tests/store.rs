use crate::common;

use common::{SESSION_ID, creation, envelope};
use serde_json::json;
use std::str::FromStr;
use tea_protocol::{ModelId, SessionId, SessionSequence};
use tea_session::{
    AppendTransaction, InMemorySessionStore, SessionReducer, SessionStore, SessionStoreErrorCode,
    StoredSession, apply_transaction_in_place,
};

#[tokio::test(flavor = "current_thread")]
async fn create_append_load_and_replay_are_equivalent() {
    let store = InMemorySessionStore::new();
    let session_id = SessionId::from_str(SESSION_ID).unwrap();
    let created = store
        .append(AppendTransaction::new(session_id, None, vec![creation()]))
        .await
        .unwrap();
    assert_eq!(created.previous_sequence(), None);
    assert_eq!(created.current_sequence(), SessionSequence::new(0));

    let changed = envelope(
        1,
        "configuration_changed",
        json!({"model":{"providerId":"openai","modelId":"openai/gpt-5"}}),
    );
    let outcome = store
        .append(AppendTransaction::new(
            session_id,
            Some(SessionSequence::new(0)),
            vec![changed],
        ))
        .await
        .unwrap();
    assert_eq!(outcome.current_sequence(), SessionSequence::new(1));

    let loaded = store.load(session_id).await.unwrap();
    assert_eq!(loaded.records().len(), 2);
    assert_eq!(
        loaded.state(),
        &SessionReducer::replay(loaded.records().to_vec()).unwrap()
    );
    assert_eq!(
        loaded.state().configuration().model_id(),
        Some(&ModelId::from_str("openai/gpt-5").unwrap())
    );
}

#[tokio::test(flavor = "current_thread")]
async fn expected_sequence_and_creation_conflicts_are_stable() {
    let store = InMemorySessionStore::new();
    let session_id = SessionId::from_str(SESSION_ID).unwrap();
    store
        .append(AppendTransaction::new(session_id, None, vec![creation()]))
        .await
        .unwrap();

    let duplicate = store
        .append(AppendTransaction::new(session_id, None, vec![creation()]))
        .await
        .unwrap_err();
    assert_eq!(
        duplicate.code(),
        SessionStoreErrorCode::SessionAlreadyExists
    );

    let stale = store
        .append(AppendTransaction::new(
            session_id,
            Some(SessionSequence::new(9)),
            vec![envelope(
                1,
                "configuration_changed",
                json!({"model":{"providerId":"openai","modelId":"openai/gpt-5"}}),
            )],
        ))
        .await
        .unwrap_err();
    assert_eq!(stale.code(), SessionStoreErrorCode::SequenceConflict);

    let missing_id = SessionId::from_str("0195a0b1-5e3a-7d72-a902-c4e85d828bf2").unwrap();
    let mut missing_creation = serde_json::to_value(creation()).unwrap();
    missing_creation["sessionId"] = json!(missing_id.to_string());
    let missing_creation = serde_json::from_value(missing_creation).unwrap();
    let missing = store
        .append(AppendTransaction::new(
            missing_id,
            Some(SessionSequence::new(0)),
            vec![missing_creation],
        ))
        .await
        .unwrap_err();
    assert_eq!(missing.code(), SessionStoreErrorCode::SessionNotFound);
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_batch_rolls_back_all_records() {
    let store = InMemorySessionStore::new();
    let session_id = SessionId::from_str(SESSION_ID).unwrap();
    store
        .append(AppendTransaction::new(session_id, None, vec![creation()]))
        .await
        .unwrap();
    let first = envelope(
        1,
        "configuration_changed",
        json!({"model":{"providerId":"openai","modelId":"openai/gpt-5"}}),
    );
    let gap = envelope(3, "configuration_changed", json!({"profileId":"coding"}));
    assert!(
        store
            .append(AppendTransaction::new(
                session_id,
                Some(SessionSequence::new(0)),
                vec![first, gap],
            ))
            .await
            .is_err()
    );
    let loaded = store.load(session_id).await.unwrap();
    assert_eq!(loaded.records().len(), 1);
    assert_eq!(loaded.state().tail_sequence(), SessionSequence::new(0));
}

#[test]
fn incremental_apply_fails_closed_for_an_inconsistent_cache() {
    let session_id = SessionId::from_str(SESSION_ID).unwrap();
    let mut stored = StoredSession::from_durable_facts(vec![creation()], vec![], vec![]).unwrap();
    stored.reducer = SessionReducer::new();
    let transaction = AppendTransaction::new(
        session_id,
        Some(SessionSequence::new(0)),
        vec![envelope(
            1,
            "configuration_changed",
            json!({"profileId":"coding"}),
        )],
    );

    let error = apply_transaction_in_place(&transaction, &mut stored, true, |_| false).unwrap_err();

    assert_eq!(error.code(), SessionStoreErrorCode::CorruptionDetected);
}

#[test]
fn store_is_object_safe_send_and_sync() {
    fn assert_store<T: SessionStore + Send + Sync>() {}
    assert_store::<InMemorySessionStore>();
    let _object: Box<dyn SessionStore> = Box::new(InMemorySessionStore::new());
}
