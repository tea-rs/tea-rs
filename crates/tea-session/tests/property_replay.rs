use crate::common;

use common::{SESSION_ID, branched_archive_flow};
use serde_json::{Value, json};
use tea_protocol::{RecordEnvelope, SessionId};
use tea_session::{SessionReducer, SessionReplayError};

const RECORD_IDS: [&str; 16] = [
    "0195a0b1-6000-7000-8000-0aa7aa000000",
    "0195a0b1-6001-7000-8000-0aa7aa000001",
    "0195a0b1-6002-7000-8000-0aa7aa000002",
    "0195a0b1-6003-7000-8000-0aa7aa000003",
    "0195a0b1-6004-7000-8000-0aa7aa000004",
    "0195a0b1-6005-7000-8000-0aa7aa000005",
    "0195a0b1-6006-7000-8000-0aa7aa000006",
    "0195a0b1-6007-7000-8000-0aa7aa000007",
    "0195a0b1-6008-7000-8000-0aa7aa000008",
    "0195a0b1-6009-7000-8000-0aa7aa000009",
    "0195a0b1-600a-7000-8000-0aa7aa00000a",
    "0195a0b1-600b-7000-8000-0aa7aa00000b",
    "0195a0b1-600c-7000-8000-0aa7aa00000c",
    "0195a0b1-600d-7000-8000-0aa7aa00000d",
    "0195a0b1-600e-7000-8000-0aa7aa00000e",
    "0195a0b1-600f-7000-8000-0aa7aa00000f",
];

fn generated_record(sequence: usize, kind: &str, payload: Value) -> RecordEnvelope {
    let mut value = json!({
        "protocolVersion":"1.0",
        "type":kind,
        "recordId":RECORD_IDS[sequence],
        "sessionId":SESSION_ID,
        "sequence":sequence.to_string(),
        "timestamp":format!("2026-07-23T11:00:00.{sequence:03}Z"),
        "payload":null
    });
    value["payload"] = payload;
    serde_json::from_value(value).unwrap()
}

fn generated_trace(seed: usize) -> Vec<RecordEnvelope> {
    let mut records = vec![generated_record(
        0,
        "session_created",
        json!({"profileId":"minimal-assistant"}),
    )];
    let changes = 2 + seed % 7;
    for sequence in 1..=changes {
        let payload = match (seed + sequence) % 3 {
            0 => json!({"model":{
                "providerId":"test",
                "modelId":format!("test/model-{seed}-{sequence}")
            }}),
            1 => json!({"profileId":format!("profile-{seed}-{sequence}")}),
            _ => json!({
                "model":{
                    "providerId":"test",
                    "modelId":format!("test/model-{seed}-{sequence}")
                },
                "profileId":format!("profile-{seed}-{sequence}")
            }),
        };
        records.push(generated_record(sequence, "configuration_changed", payload));
    }
    let checkpoint_sequence = records.len();
    records.push(generated_record(
        checkpoint_sequence,
        "turn_checkpointed",
        json!({
            "runId":"0195a0b1-6100-7000-8000-0aa7aa000010",
            "turnId":"0195a0b1-6101-7000-8000-0aa7aa000011",
            "nextAction":if seed.is_multiple_of(2) {"model_request"} else {"finish_run"}
        }),
    ));
    let terminal_sequence = records.len();
    records.push(if seed.is_multiple_of(2) {
        generated_record(
            terminal_sequence,
            "run_interrupted",
            json!({
                "runId":"0195a0b1-6100-7000-8000-0aa7aa000010",
                "turnId":"0195a0b1-6101-7000-8000-0aa7aa000011",
                "reason":"deterministic generated interruption"
            }),
        )
    } else {
        generated_record(
            terminal_sequence,
            "run_cancelled",
            json!({"runId":"0195a0b1-6100-7000-8000-0aa7aa000010"}),
        )
    });
    records
}

#[test]
fn generated_incremental_states_equal_full_replay() {
    for seed in 0..128 {
        let records = generated_trace(seed);
        let expected = SessionReducer::replay(records.clone()).unwrap();
        let mut reducer = SessionReducer::new();
        for (index, record) in records.iter().enumerate() {
            reducer.apply(record).unwrap();
            let prefix = SessionReducer::replay(records[..=index].to_vec()).unwrap();
            assert_eq!(reducer.state(), Some(&prefix), "seed {seed}, step {index}");
        }
        assert_eq!(reducer.state(), Some(&expected), "seed {seed}");
    }
}

#[test]
fn complex_branch_approval_tool_trace_is_prefix_replayable() {
    let records = branched_archive_flow();
    let mut reducer = SessionReducer::new();
    for (index, record) in records.iter().enumerate() {
        reducer.apply(record).unwrap();
        let replayed = SessionReducer::replay(records[..=index].to_vec()).unwrap();
        assert_eq!(reducer.state(), Some(&replayed), "prefix {index}");
    }
}

#[test]
fn single_invariant_mutations_fail_with_stable_categories() {
    let base = generated_trace(7);

    let mut sequence = serde_json::to_value(&base[1]).unwrap();
    sequence["sequence"] = json!("9");
    let sequence: RecordEnvelope = serde_json::from_value(sequence).unwrap();
    assert!(matches!(
        SessionReducer::replay([base[0].clone(), sequence]),
        Err(SessionReplayError::SequenceMismatch { .. })
    ));

    let mut session = serde_json::to_value(&base[1]).unwrap();
    session["sessionId"] = json!("0195a0b1-6200-7000-8000-0aa7aa000020");
    let session: RecordEnvelope = serde_json::from_value(session).unwrap();
    assert!(matches!(
        SessionReducer::replay([base[0].clone(), session]),
        Err(SessionReplayError::SessionMismatch { .. })
    ));

    let mut record_id = serde_json::to_value(&base[1]).unwrap();
    record_id["recordId"] = json!(base[0].record_id().to_string());
    let record_id: RecordEnvelope = serde_json::from_value(record_id).unwrap();
    assert!(matches!(
        SessionReducer::replay([base[0].clone(), record_id]),
        Err(SessionReplayError::DuplicateRecord { .. })
    ));

    let session_id: SessionId = SESSION_ID.parse().unwrap();
    assert_eq!(base[0].session_id(), session_id);
}
