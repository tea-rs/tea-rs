use crate::common;

use common::{
    APPROVAL_ID, TOOL_CALL_ID, approval_flow, assistant_tool_message, completed_tool_flow,
    creation, envelope, tool_requested,
};
use serde_json::json;
use std::str::FromStr;
use tea_protocol::{ApprovalId, ToolCallId};
use tea_session::{SessionReducer, SessionReplayError, ToolExecutionState};

#[test]
fn pending_approval_is_reconstructed() {
    let state = SessionReducer::replay(approval_flow()).unwrap();
    let approval_id = ApprovalId::from_str(APPROVAL_ID).unwrap();
    let tool_call_id = ToolCallId::from_str(TOOL_CALL_ID).unwrap();
    assert_eq!(state.pending_approvals().len(), 1);
    assert_eq!(
        state.pending_approvals()[&approval_id].tool_call_id(),
        tool_call_id
    );
    assert!(matches!(
        state.tool_calls()[&tool_call_id].execution(),
        ToolExecutionState::NotStarted
    ));
}

#[test]
fn completed_tool_and_result_message_replay() {
    let state = SessionReducer::replay(completed_tool_flow()).unwrap();
    let tool_call_id = ToolCallId::from_str(TOOL_CALL_ID).unwrap();
    assert!(state.pending_approvals().is_empty());
    assert!(matches!(
        state.tool_calls()[&tool_call_id].execution(),
        ToolExecutionState::Finished {
            is_error: false,
            ..
        }
    ));
    assert_eq!(state.messages().len(), 3);
}

#[test]
fn requested_tool_must_match_assistant_declaration() {
    let error = SessionReducer::replay([creation(), tool_requested(1)]).unwrap_err();
    assert!(matches!(
        error,
        SessionReplayError::InvalidReference {
            reference: "undeclared_tool_call"
        }
    ));

    let mismatch = envelope(
        2,
        "tool_call_requested",
        json!({
            "toolCallId":TOOL_CALL_ID,
            "toolName":"write_text_file",
            "arguments":{"path":"/different","content":"done"}
        }),
    );
    let error =
        SessionReducer::replay([creation(), assistant_tool_message(1), mismatch]).unwrap_err();
    assert!(matches!(
        error,
        SessionReplayError::InvalidReference {
            reference: "tool_call_declaration_mismatch"
        }
    ));
}

#[test]
fn approval_and_execution_order_fail_closed() {
    let mut records = vec![creation(), assistant_tool_message(1), tool_requested(2)];
    records.push(envelope(
        3,
        "approval_requested",
        json!({
            "approvalId":APPROVAL_ID,
            "toolCallId":TOOL_CALL_ID,
            "expiresAt":"2026-07-23T09:35:13.010Z"
        }),
    ));
    assert!(matches!(
        SessionReducer::replay(records),
        Err(SessionReplayError::InvalidTransition {
            transition: "approval_request"
        })
    ));

    let mut records = approval_flow();
    records.push(envelope(
        6,
        "tool_execution_started",
        json!({
            "toolCallId":TOOL_CALL_ID,
            "executionTarget":"native",
            "idempotency":"non_idempotent"
        }),
    ));
    assert!(matches!(
        SessionReducer::replay(records),
        Err(SessionReplayError::InvalidTransition {
            transition: "tool_execution_start"
        })
    ));
}

#[test]
fn denied_call_can_commit_failure_without_executing() {
    let failure = json!({
        "code":"policy_denied",
        "message":"tool invocation was denied by policy"
    });
    let records = [
        creation(),
        assistant_tool_message(1),
        tool_requested(2),
        envelope(
            3,
            "policy_decision_recorded",
            json!({"toolCallId":TOOL_CALL_ID,"decision":"deny"}),
        ),
        envelope(
            4,
            "tool_execution_finished",
            json!({
                "toolCallId":TOOL_CALL_ID,
                "isError":true,
                "content":[{"type":"text","text":"tool invocation was denied"}],
                "error":failure
            }),
        ),
        envelope(
            5,
            "message_committed",
            json!({
                "message":{
                    "id":common::TOOL_RESULT_ID,
                    "type":"tool_result",
                    "toolCallId":TOOL_CALL_ID,
                    "toolName":"write_text_file",
                    "content":[{"type":"text","text":"tool invocation was denied"}],
                    "isError":true,
                    "error":{
                        "code":"policy_denied",
                        "message":"tool invocation was denied by policy"
                    },
                    "timestamp":"2026-07-23T09:30:12.005Z"
                }
            }),
        ),
    ];
    let state = SessionReducer::replay(records).unwrap();
    let tool_call_id = ToolCallId::from_str(TOOL_CALL_ID).unwrap();
    assert!(matches!(
        state.tool_calls()[&tool_call_id].execution(),
        ToolExecutionState::Finished { is_error: true, .. }
    ));
    assert!(
        state.tool_calls()[&tool_call_id]
            .result_message_id()
            .is_some()
    );
}

#[test]
fn duplicate_messages_and_approval_resolution_fail_closed() {
    let mut duplicate_message = serde_json::to_value(assistant_tool_message(2)).unwrap();
    duplicate_message["sequence"] = json!("2");
    let duplicate_message = serde_json::from_value(duplicate_message).unwrap();
    let error = SessionReducer::replay([creation(), assistant_tool_message(1), duplicate_message])
        .unwrap_err();
    assert!(matches!(
        error,
        SessionReplayError::DuplicateEntity { entity: "message" }
    ));

    let resolution = envelope(
        1,
        "approval_resolved",
        json!({"approvalId":APPROVAL_ID,"decision":{"type":"deny"}}),
    );
    assert!(matches!(
        SessionReducer::replay([creation(), resolution]),
        Err(SessionReplayError::InvalidReference {
            reference: "pending_approval"
        })
    ));
}
