use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use serde_json::{Value, json};
use tea_protocol::{
    AgentErrorCode, CausationId, CorrelationId, ReasoningEffort, RecordDecodeError, RecordEnvelope,
    SessionRecord, SessionRecordType,
};

const RECORD_ID: &str = "0195a0b1-5e4a-742a-b57f-0aa7aa000016";
const SESSION_ID: &str = "0195a0b1-5e3a-7d72-a902-c4e85d828bf1";
const CORRELATION_ID: &str = "0195a0b1-5e40-7136-8ae0-0aa7aa000006";
const TIMESTAMP: &str = "2026-07-23T09:30:12.124Z";

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/v1.0")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn existing_record_fixtures_round_trip() {
    for name in [
        "record-session-created.json",
        "record-message-committed.json",
        "record-assistant-tool-call-message-committed.json",
        "record-tool-result-message-committed.json",
        "record-final-assistant-message-committed.json",
        "record-configuration-changed.json",
        "record-tool-call-requested.json",
        "record-policy-decision-recorded.json",
        "record-approval-requested.json",
        "record-approval-resolved.json",
        "record-tool-execution-started.json",
        "record-tool-execution-finished.json",
        "record-tool-execution-interrupted.json",
        "record-run-interrupted.json",
        "record-run-cancelled.json",
        "record-branch-created.json",
        "record-active-branch-changed.json",
        "record-session-compacted.json",
        "record-turn-checkpointed.json",
    ] {
        let value = fixture(name);
        let record: RecordEnvelope = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(record).unwrap(), value, "{name}");
    }
}

#[test]
fn durable_record_types_are_explicit_and_required() {
    let expected = [
        (SessionRecordType::SessionCreated, "session_created"),
        (SessionRecordType::MessageCommitted, "message_committed"),
        (
            SessionRecordType::ConfigurationChanged,
            "configuration_changed",
        ),
        (SessionRecordType::ToolCallRequested, "tool_call_requested"),
        (
            SessionRecordType::PolicyDecisionRecorded,
            "policy_decision_recorded",
        ),
        (SessionRecordType::ApprovalRequested, "approval_requested"),
        (SessionRecordType::ApprovalResolved, "approval_resolved"),
        (
            SessionRecordType::ToolExecutionStarted,
            "tool_execution_started",
        ),
        (
            SessionRecordType::ToolExecutionFinished,
            "tool_execution_finished",
        ),
        (
            SessionRecordType::ToolExecutionInterrupted,
            "tool_execution_interrupted",
        ),
        (SessionRecordType::RunInterrupted, "run_interrupted"),
        (SessionRecordType::RunCancelled, "run_cancelled"),
        (SessionRecordType::BranchCreated, "branch_created"),
        (
            SessionRecordType::ActiveBranchChanged,
            "active_branch_changed",
        ),
        (SessionRecordType::SessionCompacted, "session_compacted"),
        (SessionRecordType::TurnCheckpointed, "turn_checkpointed"),
    ];
    assert_eq!(SessionRecordType::ALL, expected.map(|(kind, _)| kind));
    for (kind, wire) in expected {
        assert_eq!(serde_json::to_value(kind).unwrap(), wire);
        assert!(kind.is_required_for_replay());
    }
}

#[test]
fn unknown_records_stop_replay_with_stable_error() {
    let value = json!({
        "protocolVersion":"1.1",
        "type":"policy_revoked",
        "recordId":RECORD_ID,
        "sessionId":SESSION_ID,
        "sequence":"8",
        "timestamp":TIMESTAMP,
        "payload":{}
    });
    let error = RecordEnvelope::decode_value(value).unwrap_err();
    assert!(matches!(error, RecordDecodeError::UnsupportedType { .. }));
    let protocol_error =
        error.into_protocol_error(CorrelationId::from_str(CORRELATION_ID).unwrap());
    assert_eq!(protocol_error.code(), AgentErrorCode::UnsupportedRecord);
    assert_eq!(
        protocol_error.details()["dev.tea-rs.protocol"]["unsupportedType"],
        "policy_revoked"
    );
}

#[test]
fn incompatible_major_takes_precedence_over_unknown_record_type() {
    let value = json!({
        "protocolVersion":"2.0",
        "type":"future_record",
        "recordId":RECORD_ID,
        "sessionId":SESSION_ID,
        "sequence":"8",
        "timestamp":TIMESTAMP,
        "payload":{}
    });
    let error = RecordEnvelope::decode_value(value).unwrap_err();
    assert!(matches!(
        error,
        RecordDecodeError::UnsupportedVersion { .. }
    ));
    assert_eq!(
        error
            .into_protocol_error(CorrelationId::from_str(CORRELATION_ID).unwrap())
            .code(),
        AgentErrorCode::UnsupportedProtocolVersion
    );
}

#[test]
fn causation_and_correlation_are_distinct_strong_ids() {
    let causation = CausationId::from_str(RECORD_ID).unwrap();
    let correlation = CorrelationId::from_str(CORRELATION_ID).unwrap();
    assert_eq!(causation.to_string(), RECORD_ID);
    assert_eq!(correlation.to_string(), CORRELATION_ID);
}

#[test]
fn direct_invalid_record_construction_cannot_cross_the_wire() {
    let invalid = SessionRecord::ConfigurationChanged {
        model: None,
        profile_id: None,
        reasoning_effort: None,
    };
    assert!(serde_json::to_value(invalid).is_err());
}

#[test]
fn configuration_reasoning_is_additive_to_qualified_model_records() {
    let mut old = fixture("record-configuration-changed.json");
    let record: RecordEnvelope = serde_json::from_value(old.clone()).unwrap();
    let SessionRecord::ConfigurationChanged {
        reasoning_effort, ..
    } = record.record()
    else {
        panic!("expected configuration change");
    };
    assert_eq!(*reasoning_effort, None);

    old["payload"]["reasoningEffort"] = json!(ReasoningEffort::Maximum);
    let record: RecordEnvelope = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(record).unwrap(), old);
}

#[test]
fn branch_envelope_and_payload_references_must_agree() {
    let mut record = fixture("record-branch-created.json");
    record["branchId"] = json!("0195a0b1-5e4e-728c-bfe1-0aa7aa000020");
    assert!(serde_json::from_value::<RecordEnvelope>(record).is_err());
}

#[test]
fn tool_terminal_and_configuration_invariants_fail_closed() {
    let inconsistent_tool_result = json!({
        "protocolVersion":"1.0",
        "type":"tool_execution_finished",
        "recordId":RECORD_ID,
        "sessionId":SESSION_ID,
        "sequence":"6",
        "timestamp":TIMESTAMP,
        "payload":{
            "toolCallId":"0195a0b1-5e45-75be-8284-0aa7aa000011",
            "isError":true,
            "content":[{"type":"text","text":"failed"}]
        }
    });
    assert!(serde_json::from_value::<RecordEnvelope>(inconsistent_tool_result).is_err());

    let empty_configuration = json!({
        "protocolVersion":"1.0",
        "type":"configuration_changed",
        "recordId":RECORD_ID,
        "sessionId":SESSION_ID,
        "sequence":"2",
        "timestamp":TIMESTAMP,
        "payload":{}
    });
    assert!(serde_json::from_value::<RecordEnvelope>(empty_configuration).is_err());
}
