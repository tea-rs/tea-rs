use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use serde_json::{Value, json};
use tea_protocol::{
    AgentEvent, AgentEventType, CodeChange, CodeChangeKind, EventCompatibility, EventDecodeError,
    EventDelta, EventEnvelope, EventInspection, HostedToolOutcome, MAX_EVENT_DELTA_BYTES,
    MAX_HOSTED_TOOL_SOURCES, ProtocolMetadata, SessionSequence, ToolCallId, ToolPresentation,
};

const EVENT_ID: &str = "0195a0b1-5e41-7e75-bdc7-0aa7aa000007";
const SESSION_ID: &str = "0195a0b1-5e3a-7d72-a902-c4e85d828bf1";
const RUN_ID: &str = "0195a0b1-5e40-7136-8ae0-0aa7aa000006";
const TIMESTAMP: &str = "2026-07-23T09:30:12.300Z";

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/v1.0")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn event_fixtures_round_trip_with_explicit_shapes() {
    for name in [
        "event-run-started.json",
        "event-message-delta.json",
        "event-tool-call-requested.json",
        "event-approval-requested.json",
        "event-tool-execution-progress.json",
        "event-turn-checkpointed.json",
        "event-session-compacted.json",
        "event-session-forked.json",
        "event-run-finished.json",
    ] {
        let value = fixture(name);
        let envelope: EventEnvelope = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(envelope).unwrap(), value, "{name}");
    }
}

#[test]
fn initial_event_types_have_stable_classifications() {
    let expected = [
        (
            AgentEventType::RunStarted,
            "run_started",
            EventCompatibility::RequiredStateBearing,
        ),
        (
            AgentEventType::MessageDelta,
            "message_delta",
            EventCompatibility::SkippableObservation,
        ),
        (
            AgentEventType::ToolCallRequested,
            "tool_call_requested",
            EventCompatibility::RequiredStateBearing,
        ),
        (
            AgentEventType::ApprovalRequested,
            "approval_requested",
            EventCompatibility::RequiredStateBearing,
        ),
        (
            AgentEventType::ToolExecutionProgress,
            "tool_execution_progress",
            EventCompatibility::SkippableObservation,
        ),
        (
            AgentEventType::ToolExecutionPreview,
            "tool_execution_preview",
            EventCompatibility::SkippableObservation,
        ),
        (
            AgentEventType::HostedToolStarted,
            "hosted_tool_started",
            EventCompatibility::SkippableObservation,
        ),
        (
            AgentEventType::HostedToolCompleted,
            "hosted_tool_completed",
            EventCompatibility::SkippableObservation,
        ),
        (
            AgentEventType::ModelRetryScheduled,
            "model_retry_scheduled",
            EventCompatibility::SkippableObservation,
        ),
        (
            AgentEventType::ModelRetryStarted,
            "model_retry_started",
            EventCompatibility::SkippableObservation,
        ),
        (
            AgentEventType::TurnCheckpointed,
            "turn_checkpointed",
            EventCompatibility::RequiredStateBearing,
        ),
        (
            AgentEventType::SessionCompacted,
            "session_compacted",
            EventCompatibility::RequiredStateBearing,
        ),
        (
            AgentEventType::SessionForked,
            "session_forked",
            EventCompatibility::RequiredStateBearing,
        ),
        (
            AgentEventType::RunFinished,
            "run_finished",
            EventCompatibility::RequiredStateBearing,
        ),
    ];
    assert_eq!(AgentEventType::ALL, expected.map(|(kind, _, _)| kind));
    for (kind, wire, compatibility) in expected {
        assert_eq!(serde_json::to_value(kind).unwrap(), wire);
        assert_eq!(kind.compatibility(), compatibility);
    }
}

#[test]
fn model_retry_lifecycle_is_a_bounded_skippable_observation() {
    let message_id = "0195a0b1-5e3d-73de-b461-0aa7aa000004".parse().unwrap();
    let turn_id = "0195a0b1-5e42-7b38-af7c-0aa7aa000008".parse().unwrap();
    let scheduled = EventEnvelope::new(
        EVENT_ID.parse().unwrap(),
        SESSION_ID.parse().unwrap(),
        Some(RUN_ID.parse().unwrap()),
        Some(turn_id),
        SessionSequence::new(7),
        TIMESTAMP.parse().unwrap(),
        ProtocolMetadata::default(),
        AgentEvent::ModelRetryScheduled {
            message_id,
            attempt: 1,
            max_retries: 3,
            delay_ms: 2_000,
        },
    )
    .unwrap();
    let value = serde_json::to_value(&scheduled).unwrap();
    assert_eq!(value["type"], "model_retry_scheduled");
    assert_eq!(value["compatibility"], "skippable_observation");
    assert_eq!(value["payload"]["messageId"], message_id.to_string());
    assert_eq!(value["payload"]["attempt"], 1);
    assert_eq!(value["payload"]["maxRetries"], 3);
    assert_eq!(value["payload"]["delayMs"], 2_000);
    assert_eq!(
        serde_json::from_value::<EventEnvelope>(value).unwrap(),
        scheduled
    );

    let started = EventEnvelope::new(
        "0195a0b1-5e41-7e75-bdc7-0aa7aa000008".parse().unwrap(),
        SESSION_ID.parse().unwrap(),
        Some(RUN_ID.parse().unwrap()),
        Some(turn_id),
        SessionSequence::new(8),
        TIMESTAMP.parse().unwrap(),
        ProtocolMetadata::default(),
        AgentEvent::ModelRetryStarted {
            message_id,
            attempt: 1,
            max_retries: 3,
        },
    )
    .unwrap();
    assert_eq!(started.event_type(), AgentEventType::ModelRetryStarted);
    assert_eq!(
        serde_json::to_value(started).unwrap()["compatibility"],
        "skippable_observation"
    );

    assert!(
        EventEnvelope::new(
            "0195a0b1-5e41-7e75-bdc7-0aa7aa000009".parse().unwrap(),
            SESSION_ID.parse().unwrap(),
            Some(RUN_ID.parse().unwrap()),
            Some(turn_id),
            SessionSequence::new(9),
            TIMESTAMP.parse().unwrap(),
            ProtocolMetadata::default(),
            AgentEvent::ModelRetryScheduled {
                message_id,
                attempt: 0,
                max_retries: 3,
                delay_ms: 2_000,
            },
        )
        .is_err()
    );
}

#[test]
fn tool_preview_is_a_skippable_typed_observation() {
    let event = EventEnvelope::new(
        EVENT_ID.parse().unwrap(),
        SESSION_ID.parse().unwrap(),
        Some(RUN_ID.parse().unwrap()),
        Some("0195a0b1-5e42-7b38-af7c-0aa7aa000008".parse().unwrap()),
        SessionSequence::new(7),
        TIMESTAMP.parse().unwrap(),
        ProtocolMetadata::default(),
        AgentEvent::ToolExecutionPreview {
            tool_call_id: ToolCallId::from_str("0195a0b1-5e45-75be-8284-0aa7aa000011").unwrap(),
            presentation: ToolPresentation::CodeChange(
                CodeChange::new(
                    "notes.txt",
                    CodeChangeKind::Update,
                    Vec::new(),
                    false,
                    None,
                    None,
                    None,
                )
                .unwrap(),
            ),
        },
    )
    .unwrap();
    let mut value = serde_json::to_value(event).unwrap();
    assert_eq!(value["compatibility"], "skippable_observation");
    assert!(matches!(
        EventEnvelope::inspect_value(value.clone()).unwrap(),
        EventInspection::Known(_)
    ));

    value["type"] = json!("tool_execution_preview_future");
    assert!(matches!(
        EventEnvelope::inspect_value(value).unwrap(),
        EventInspection::UnknownSkippable(_)
    ));
}

#[test]
fn hosted_tool_lifecycle_is_skippable_and_excludes_provider_continuation() {
    let event = EventEnvelope::new(
        EVENT_ID.parse().unwrap(),
        SESSION_ID.parse().unwrap(),
        Some(RUN_ID.parse().unwrap()),
        Some("0195a0b1-5e42-7b38-af7c-0aa7aa000008".parse().unwrap()),
        SessionSequence::new(7),
        TIMESTAMP.parse().unwrap(),
        ProtocolMetadata::default(),
        AgentEvent::HostedToolCompleted {
            tool_call_id: "0195a0b1-5e45-75be-8284-0aa7aa000011".parse().unwrap(),
            tool_name: "web_search".to_owned(),
            arguments: json!({"query":"current tea runtime"}),
            outcome: HostedToolOutcome::Success,
            source_count: 2,
        },
    )
    .unwrap();

    let value = serde_json::to_value(&event).unwrap();
    assert_eq!(value["type"], "hosted_tool_completed");
    assert_eq!(value["compatibility"], "skippable_observation");
    assert_eq!(value["payload"]["sourceCount"], 2);
    assert!(value["payload"].get("continuation").is_none());
    assert_eq!(
        serde_json::from_value::<EventEnvelope>(value).unwrap(),
        event
    );
}

#[test]
fn observers_may_skip_only_explicit_unknown_observations() {
    let skippable = json!({
        "protocolVersion":"1.1",
        "type":"provider_heartbeat",
        "compatibility":"skippable_observation",
        "eventId":EVENT_ID,
        "sessionId":SESSION_ID,
        "runId":RUN_ID,
        "sequence":"7",
        "timestamp":TIMESTAMP,
        "payload":{"latencyMs":12}
    });
    let EventInspection::UnknownSkippable(unknown) =
        EventEnvelope::inspect_value(skippable).unwrap()
    else {
        panic!("expected skippable unknown event");
    };
    assert_eq!(unknown.event_type(), "provider_heartbeat");
    assert_eq!(unknown.sequence().get(), 7);

    let required = json!({
        "protocolVersion":"1.1",
        "type":"policy_changed",
        "eventId":EVENT_ID,
        "sessionId":SESSION_ID,
        "sequence":"8",
        "timestamp":TIMESTAMP,
        "payload":{}
    });
    assert!(matches!(
        EventEnvelope::inspect_value(required),
        Err(EventDecodeError::UnsupportedStateBearing { .. })
    ));
}

#[test]
fn malformed_unknown_events_are_never_skipped() {
    let malformed = json!({
        "protocolVersion":"2.0",
        "type":"provider_heartbeat",
        "compatibility":"skippable_observation",
        "eventId":EVENT_ID,
        "sessionId":SESSION_ID,
        "sequence":"7",
        "timestamp":TIMESTAMP,
        "payload":{}
    });
    assert!(matches!(
        EventEnvelope::inspect_value(malformed),
        Err(EventDecodeError::Invalid(_))
    ));

    let invalid_type = json!({
        "protocolVersion":"1.1",
        "type":"Provider Heartbeat",
        "compatibility":"skippable_observation",
        "eventId":EVENT_ID,
        "sessionId":SESSION_ID,
        "sequence":"7",
        "timestamp":TIMESTAMP,
        "payload":{}
    });
    assert!(matches!(
        EventEnvelope::inspect_value(invalid_type),
        Err(EventDecodeError::Invalid(_))
    ));
}

#[test]
fn direct_invalid_event_construction_cannot_cross_the_wire() {
    let invalid_delta = EventDelta::TextDelta {
        text: String::new(),
    };
    assert!(serde_json::to_value(invalid_delta).is_err());

    let invalid_progress = AgentEvent::ToolExecutionProgress {
        tool_call_id: "0195a0b1-5e45-75be-8284-0aa7aa000011".parse().unwrap(),
        message: "writing".to_owned(),
        completed_units: 2,
        total_units: Some(1),
    };
    assert!(serde_json::to_value(invalid_progress).is_err());

    let invalid_hosted = AgentEvent::HostedToolCompleted {
        tool_call_id: "0195a0b1-5e45-75be-8284-0aa7aa000011".parse().unwrap(),
        tool_name: "web_search".to_owned(),
        arguments: json!({"query":"tea"}),
        outcome: HostedToolOutcome::Success,
        source_count: u32::try_from(MAX_HOSTED_TOOL_SOURCES + 1).unwrap(),
    };
    assert!(serde_json::to_value(invalid_hosted).is_err());
}

#[test]
fn deltas_progress_and_collections_are_bounded() {
    let mut delta = fixture("event-message-delta.json");
    delta["payload"]["delta"]["text"] = Value::String("x".repeat(MAX_EVENT_DELTA_BYTES + 1));
    assert!(serde_json::from_value::<EventEnvelope>(delta).is_err());

    let mut progress = fixture("event-tool-execution-progress.json");
    progress["payload"]["completedUnits"] = json!(2);
    progress["payload"]["totalUnits"] = json!(1);
    assert!(serde_json::from_value::<EventEnvelope>(progress).is_err());

    let mut approval = fixture("event-approval-requested.json");
    approval["payload"]["capabilities"] = json!((0..65).map(|_| "fs.write").collect::<Vec<_>>());
    assert!(serde_json::from_value::<EventEnvelope>(approval).is_err());
}
