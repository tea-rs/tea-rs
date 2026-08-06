use std::str::FromStr;

use tea_protocol::{
    CommandEnvelope, EventEnvelope, ProtocolErrorEnvelope, ProtocolMetadata, ProtocolTimestamp,
    ProtocolVersion, RecordEnvelope, SessionId, SessionSequence,
};

#[test]
fn invalid_protocol_versions_are_rejected() {
    for value in ["", "1", "1.0.0", "01.0", "1.00", "+1.0", " 1.0"] {
        assert!(
            ProtocolVersion::from_str(value).is_err(),
            "accepted {value:?}"
        );
    }
}

#[test]
fn non_canonical_or_non_v7_ids_are_rejected() {
    for value in [
        "",
        "550e8400-e29b-41d4-a716-446655440000",
        "0195A0B1-5E3A-7D72-A902-C4E85D828BF1",
        "0195a0b15e3a7d72a902c4e85d828bf1",
    ] {
        assert!(SessionId::from_str(value).is_err(), "accepted {value:?}");
    }
}

#[test]
fn invalid_sequences_are_rejected() {
    for value in ["", "00", "01", "+1", "-1", " 1", "1 ", "1.0", "1e3"] {
        assert!(
            SessionSequence::from_str(value).is_err(),
            "accepted {value:?}"
        );
    }
    assert!(SessionSequence::new(u64::MAX).checked_next().is_none());
    assert!(serde_json::from_str::<SessionSequence>("42").is_err());
}

#[test]
fn duplicate_fields_are_rejected_recursively_at_envelope_boundaries() {
    let duplicate_command_id = r#"{
        "protocolVersion":"1.0",
        "type":"abort",
        "commandId":"0195a0b1-5e5e-741f-b474-0aa7aa000036",
        "commandId":"0195a0b1-5e5b-739d-bf5c-0aa7aa000033",
        "sessionId":"0195a0b1-5e3a-7d72-a902-c4e85d828bf1",
        "timestamp":"2026-07-23T09:30:15.200Z",
        "payload":{}
    }"#;
    assert!(serde_json::from_str::<CommandEnvelope>(duplicate_command_id).is_err());

    let duplicate_payload = r#"{
        "protocolVersion":"1.0",
        "type":"run_finished",
        "eventId":"0195a0b1-5e49-7ec5-8d81-0aa7aa000015",
        "sessionId":"0195a0b1-5e3a-7d72-a902-c4e85d828bf1",
        "runId":"0195a0b1-5e40-7136-8ae0-0aa7aa000006",
        "sequence":"6",
        "timestamp":"2026-07-23T09:30:15.000Z",
        "payload":{"status":"completed","status":"failed"}
    }"#;
    assert!(serde_json::from_str::<EventEnvelope>(duplicate_payload).is_err());

    let duplicate_record_type = r#"{
        "protocolVersion":"1.0",
        "type":"run_cancelled",
        "type":"run_interrupted",
        "recordId":"0195a0b1-5e57-78ff-80e1-0aa7aa000029",
        "sessionId":"0195a0b1-5e3a-7d72-a902-c4e85d828bf1",
        "sequence":"7",
        "timestamp":"2026-07-23T09:30:14.130Z",
        "payload":{"runId":"0195a0b1-5e40-7136-8ae0-0aa7aa000006"}
    }"#;
    assert!(serde_json::from_str::<RecordEnvelope>(duplicate_record_type).is_err());

    let duplicate_error_code = r#"{
        "protocolVersion":"1.0",
        "type":"protocol_error",
        "error":{"code":"internal","code":"invalid_input","message":"bad","retry":"never"}
    }"#;
    assert!(serde_json::from_str::<ProtocolErrorEnvelope>(duplicate_error_code).is_err());
}

#[test]
fn duplicate_metadata_namespaces_are_rejected_directly() {
    let duplicate = r#"{"com.example":{"value":1},"com.example":{"value":2}}"#;
    assert!(serde_json::from_str::<ProtocolMetadata>(duplicate).is_err());
}

#[test]
fn incompatible_protocol_majors_are_rejected_by_all_envelopes() {
    let cases = [
        r#"{"protocolVersion":"2.0","type":"abort","commandId":"0195a0b1-5e5e-741f-b474-0aa7aa000036","sessionId":"0195a0b1-5e3a-7d72-a902-c4e85d828bf1","timestamp":"2026-07-23T09:30:15.200Z","payload":{}}"#,
        r#"{"protocolVersion":"2.0","type":"run_started","eventId":"0195a0b1-5e3f-742a-9891-0aa7aa000005","sessionId":"0195a0b1-5e3a-7d72-a902-c4e85d828bf1","runId":"0195a0b1-5e40-7136-8ae0-0aa7aa000006","sequence":"1","timestamp":"2026-07-23T09:30:12.125Z","payload":{}}"#,
        r#"{"protocolVersion":"2.0","type":"run_cancelled","recordId":"0195a0b1-5e57-78ff-80e1-0aa7aa000029","sessionId":"0195a0b1-5e3a-7d72-a902-c4e85d828bf1","sequence":"7","timestamp":"2026-07-23T09:30:14.130Z","payload":{"runId":"0195a0b1-5e40-7136-8ae0-0aa7aa000006"}}"#,
    ];
    assert!(serde_json::from_str::<CommandEnvelope>(cases[0]).is_err());
    assert!(serde_json::from_str::<EventEnvelope>(cases[1]).is_err());
    assert!(serde_json::from_str::<RecordEnvelope>(cases[2]).is_err());
}

#[test]
fn invalid_or_lossy_timestamps_are_rejected() {
    for value in [
        "",
        "2026-07-23 09:30:12.123Z",
        "2026-07-23T09:30:12.123",
        "2026-07-23T09:30:12.123456Z",
    ] {
        assert!(
            ProtocolTimestamp::from_str(value).is_err(),
            "accepted {value:?}"
        );
    }
}
