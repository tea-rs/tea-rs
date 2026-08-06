use crate::common;

use common::{TOOL_CALL_ID, approval_flow, creation, envelope};
use serde_json::json;
use std::str::FromStr;
use tea_protocol::{RunId, ToolCallId};
use tea_session::{RunRecoveryState, SessionReducer, ToolExecutionState};

const RUN_ID: &str = "0195a0b1-5e40-7136-8ae0-0aa7aa000006";
const TURN_ID: &str = "0195a0b1-5e42-7b38-af7c-0aa7aa000008";

#[test]
fn checkpoint_and_run_interruption_are_reconstructed() {
    let state = SessionReducer::replay([
        creation(),
        envelope(
            1,
            "turn_checkpointed",
            json!({"runId":RUN_ID,"turnId":TURN_ID,"nextAction":"model_request"}),
        ),
        envelope(
            2,
            "run_interrupted",
            json!({"runId":RUN_ID,"turnId":TURN_ID,"reason":"provider stream ended"}),
        ),
    ])
    .unwrap();
    assert_eq!(state.latest_checkpoint().unwrap().sequence().get(), 1);
    assert!(matches!(
        state.run_recovery()[&RunId::from_str(RUN_ID).unwrap()],
        RunRecoveryState::Interrupted { .. }
    ));
}

#[test]
fn started_then_interrupted_tool_has_uncertain_outcome() {
    let mut records = approval_flow();
    records.extend([
        envelope(
            6,
            "approval_resolved",
            json!({"approvalId":common::APPROVAL_ID,"decision":{"type":"allow_once"}}),
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
            "tool_execution_interrupted",
            json!({"toolCallId":TOOL_CALL_ID,"reason":"process terminated"}),
        ),
    ]);
    let state = SessionReducer::replay(records).unwrap();
    assert!(matches!(
        state.tool_calls()[&ToolCallId::from_str(TOOL_CALL_ID).unwrap()].execution(),
        ToolExecutionState::Interrupted { .. }
    ));
}

#[test]
fn duplicate_run_terminal_fails_closed() {
    let error = SessionReducer::replay([
        creation(),
        envelope(1, "run_cancelled", json!({"runId":RUN_ID})),
        envelope(
            2,
            "run_interrupted",
            json!({"runId":RUN_ID,"turnId":TURN_ID,"reason":"late interruption"}),
        ),
    ])
    .unwrap_err();
    assert!(error.to_string().contains("duplicate_run_terminal"));
}
