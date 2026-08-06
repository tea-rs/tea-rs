#![allow(dead_code)]

use serde_json::{Value, json};
use tea_protocol::RecordEnvelope;

pub const SESSION_ID: &str = "0195a0b1-5e3a-7d72-a902-c4e85d828bf1";
pub const PROFILE_ID: &str = "minimal-assistant";
pub const TOOL_CALL_ID: &str = "0195a0b1-5e45-75be-8284-0aa7aa000011";
pub const APPROVAL_ID: &str = "0195a0b1-5e46-7e2a-b230-0aa7aa000012";
pub const MESSAGE_ID: &str = "0195a0b1-5e3d-7bb4-863a-0aa7aa000003";
pub const ASSISTANT_ID: &str = "0195a0b1-5e64-76d6-9a5a-0aa7aa000042";
pub const TOOL_RESULT_ID: &str = "0195a0b1-5e66-7e4d-9af7-0aa7aa000044";
pub const ROOT_BRANCH_ID: &str = "0195a0b1-5e4e-728c-bfe1-0aa7aa000020";
pub const FORK_BRANCH_ID: &str = "0195a0b1-5e4f-7bd5-9760-0aa7aa000021";

const RECORD_IDS: [&str; 16] = [
    "0195a0b1-5e50-7af4-8972-0aa7aa000022",
    "0195a0b1-5e4a-742a-b57f-0aa7aa000016",
    "0195a0b1-5e63-7b8c-a5ad-0aa7aa000041",
    "0195a0b1-5e52-713b-9bfa-0aa7aa000024",
    "0195a0b1-5e53-7771-82ab-0aa7aa000025",
    "0195a0b1-5e4b-712a-9682-0aa7aa000017",
    "0195a0b1-5e54-7c92-b8ca-0aa7aa000026",
    "0195a0b1-5e4c-7c9f-82df-0aa7aa000018",
    "0195a0b1-5e4d-7f55-bc85-0aa7aa000019",
    "0195a0b1-5e65-7762-9938-0aa7aa000043",
    "0195a0b1-5e56-7da9-9342-0aa7aa000028",
    "0195a0b1-5e57-7e12-90c1-0aa7aa000029",
    "0195a0b1-5e58-778a-a74e-0aa7aa000030",
    "0195a0b1-5e59-7854-a36d-0aa7aa000031",
    "0195a0b1-5e5a-7a61-8d72-0aa7aa000032",
    "0195a0b1-5e5b-7ad1-83d9-0aa7aa000033",
];

pub fn envelope(sequence: usize, kind: &str, payload: Value) -> RecordEnvelope {
    envelope_for(SESSION_ID, sequence, kind, payload)
}

pub fn envelope_for(
    session_id: &str,
    sequence: usize,
    kind: &str,
    payload: Value,
) -> RecordEnvelope {
    raw_envelope(session_id, sequence, kind, payload, None)
}

pub fn envelope_on_branch(
    sequence: usize,
    kind: &str,
    payload: Value,
    branch_id: &str,
) -> RecordEnvelope {
    raw_envelope(SESSION_ID, sequence, kind, payload, Some(branch_id))
}

fn raw_envelope(
    session_id: &str,
    sequence: usize,
    kind: &str,
    payload: Value,
    branch_id: Option<&str>,
) -> RecordEnvelope {
    let mut value = json!({
        "protocolVersion": "1.0",
        "type": kind,
        "recordId": RECORD_IDS[sequence],
        "sessionId": session_id,
        "sequence": sequence.to_string(),
        "timestamp": format!("2026-07-23T09:30:{:02}.{:03}Z", 12 + sequence / 1000, sequence % 1000),
        "payload": null
    });
    value["payload"] = payload;
    if let Some(branch_id) = branch_id {
        value["branchId"] = json!(branch_id);
    }
    serde_json::from_value(value).unwrap()
}

pub fn creation() -> RecordEnvelope {
    envelope(0, "session_created", json!({"profileId": PROFILE_ID}))
}

pub fn branched_creation() -> RecordEnvelope {
    envelope_on_branch(
        0,
        "session_created",
        json!({"profileId": PROFILE_ID}),
        ROOT_BRANCH_ID,
    )
}

pub fn user_message(sequence: usize) -> RecordEnvelope {
    envelope(
        sequence,
        "message_committed",
        json!({
            "message": {
                "id": MESSAGE_ID,
                "type": "user",
                "content": [{"type":"text", "text":"Inspect the workspace."}],
                "timestamp": "2026-07-23T09:30:12.124Z"
            }
        }),
    )
}

pub fn assistant_tool_message(sequence: usize) -> RecordEnvelope {
    envelope(
        sequence,
        "message_committed",
        json!({
            "message": {
                "id": ASSISTANT_ID,
                "type": "assistant",
                "content": [
                    {"type":"text", "text":"I will write the notes."},
                    {
                        "type":"tool_call",
                        "toolCallId": TOOL_CALL_ID,
                        "toolName":"write_text_file",
                        "arguments":{"path":"/workspace/notes.txt","content":"done"}
                    }
                ],
                "stopReason":"tool_use",
                "timestamp":"2026-07-23T09:30:13.000Z"
            }
        }),
    )
}

pub fn tool_requested(sequence: usize) -> RecordEnvelope {
    envelope(
        sequence,
        "tool_call_requested",
        json!({
            "toolCallId":TOOL_CALL_ID,
            "toolName":"write_text_file",
            "arguments":{"path":"/workspace/notes.txt","content":"done"}
        }),
    )
}

pub fn approval_flow() -> Vec<RecordEnvelope> {
    vec![
        creation(),
        user_message(1),
        assistant_tool_message(2),
        tool_requested(3),
        envelope(
            4,
            "policy_decision_recorded",
            json!({"toolCallId":TOOL_CALL_ID,"decision":"require_approval"}),
        ),
        envelope(
            5,
            "approval_requested",
            json!({
                "approvalId":APPROVAL_ID,
                "toolCallId":TOOL_CALL_ID,
                "expiresAt":"2026-07-23T09:35:13.010Z"
            }),
        ),
    ]
}

pub fn completed_tool_flow() -> Vec<RecordEnvelope> {
    let mut records = approval_flow();
    records.extend([
        envelope(
            6,
            "approval_resolved",
            json!({"approvalId":APPROVAL_ID,"decision":{"type":"allow_once"}}),
        ),
        envelope(
            7,
            "tool_execution_started",
            json!({
                "toolCallId":TOOL_CALL_ID,
                "executionTarget":"native",
                "idempotency":"non_idempotent"
            }),
        ),
        envelope(
            8,
            "tool_execution_finished",
            json!({
                "toolCallId":TOOL_CALL_ID,
                "isError":false,
                "content":[{"type":"text","text":"wrote notes"}]
            }),
        ),
        envelope(
            9,
            "message_committed",
            json!({
                "message": {
                    "id": TOOL_RESULT_ID,
                    "type":"tool_result",
                    "toolCallId":TOOL_CALL_ID,
                    "toolName":"write_text_file",
                    "content":[{"type":"text","text":"wrote notes"}],
                    "isError":false,
                    "timestamp":"2026-07-23T09:30:14.121Z"
                }
            }),
        ),
    ]);
    records
}

pub fn branched_archive_flow() -> Vec<RecordEnvelope> {
    let mut records = completed_tool_flow()
        .into_iter()
        .map(|record| {
            let mut value = serde_json::to_value(record).unwrap();
            value["branchId"] = json!(ROOT_BRANCH_ID);
            serde_json::from_value::<RecordEnvelope>(value).unwrap()
        })
        .collect::<Vec<_>>();
    let source_record_id = records.last().unwrap().record_id().to_string();
    records.extend([
        envelope_on_branch(
            10,
            "branch_created",
            json!({
                "sourceBranchId":ROOT_BRANCH_ID,
                "branchId":FORK_BRANCH_ID,
                "fromRecordId":source_record_id
            }),
            FORK_BRANCH_ID,
        ),
        envelope_on_branch(
            11,
            "active_branch_changed",
            json!({"branchId":FORK_BRANCH_ID}),
            FORK_BRANCH_ID,
        ),
        envelope_on_branch(
            12,
            "turn_checkpointed",
            json!({
                "runId":"0195a0b1-5e40-7136-8ae0-0aa7aa000006",
                "turnId":"0195a0b1-5e42-7b38-af7c-0aa7aa000008",
                "nextAction":"model_request"
            }),
            FORK_BRANCH_ID,
        ),
        envelope_on_branch(
            13,
            "run_interrupted",
            json!({
                "runId":"0195a0b1-5e40-7136-8ae0-0aa7aa000006",
                "turnId":"0195a0b1-5e42-7b38-af7c-0aa7aa000008",
                "reason":"provider stream ended before terminal output"
            }),
            FORK_BRANCH_ID,
        ),
    ]);
    records
}
