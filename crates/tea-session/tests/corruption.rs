use crate::common;

use common::{SESSION_ID, creation, envelope, envelope_for};
use serde_json::json;
use tea_session::{SessionReducer, SessionReplayError, SessionStoreErrorCode};

#[test]
fn stable_error_codes_round_trip() {
    for code in [
        SessionStoreErrorCode::SessionNotFound,
        SessionStoreErrorCode::SessionAlreadyExists,
        SessionStoreErrorCode::SequenceConflict,
        SessionStoreErrorCode::InvalidRecord,
        SessionStoreErrorCode::InvalidReference,
        SessionStoreErrorCode::UnsupportedSchemaVersion,
        SessionStoreErrorCode::CorruptionDetected,
        SessionStoreErrorCode::TransactionFailed,
        SessionStoreErrorCode::StorageUnavailable,
    ] {
        let value = serde_json::to_value(code).unwrap();
        assert_eq!(
            serde_json::from_value::<SessionStoreErrorCode>(value).unwrap(),
            code
        );
    }
}

#[test]
fn empty_and_non_creation_logs_fail_closed() {
    assert_eq!(
        SessionReducer::replay(Vec::new()).unwrap_err(),
        SessionReplayError::EmptyLog
    );
    let error = SessionReducer::replay([envelope(
        0,
        "configuration_changed",
        json!({"model":{"providerId":"openai","modelId":"openai/gpt-5"}}),
    )])
    .unwrap_err();
    assert_eq!(error, SessionReplayError::InvalidCreation);
}

#[test]
fn sequence_and_session_corruption_are_detected() {
    let wrong_sequence = envelope(
        1,
        "session_created",
        json!({"profileId":"minimal-assistant"}),
    );
    assert!(matches!(
        SessionReducer::replay([wrong_sequence]),
        Err(SessionReplayError::SequenceMismatch { .. })
    ));

    let other_session = "0195a0b1-5e3a-7d72-a902-c4e85d828bf2";
    let changed = envelope_for(
        other_session,
        1,
        "configuration_changed",
        json!({"model":{"providerId":"openai","modelId":"openai/gpt-5"}}),
    );
    let error = SessionReducer::replay([creation(), changed]).unwrap_err();
    assert!(matches!(error, SessionReplayError::SessionMismatch { .. }));
    assert_eq!(
        error.store_code(),
        SessionStoreErrorCode::CorruptionDetected
    );
    assert_ne!(other_session, SESSION_ID);
}

#[test]
fn duplicate_creation_and_record_identity_are_corruption() {
    let second_creation = envelope(
        1,
        "session_created",
        json!({"profileId":"minimal-assistant"}),
    );
    assert_eq!(
        SessionReducer::replay([creation(), second_creation]).unwrap_err(),
        SessionReplayError::InvalidCreation
    );

    let mut duplicate_record = serde_json::to_value(envelope(
        1,
        "configuration_changed",
        json!({"model":{"providerId":"openai","modelId":"openai/gpt-5"}}),
    ))
    .unwrap();
    duplicate_record["recordId"] = json!(creation().record_id().to_string());
    let duplicate_record = serde_json::from_value(duplicate_record).unwrap();
    assert!(matches!(
        SessionReducer::replay([creation(), duplicate_record]),
        Err(SessionReplayError::DuplicateRecord { .. })
    ));
}
