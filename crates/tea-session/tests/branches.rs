use crate::common;

use common::{FORK_BRANCH_ID, ROOT_BRANCH_ID, branched_creation, envelope_on_branch, user_message};
use serde_json::json;
use std::str::FromStr;
use tea_protocol::{BranchId, ModelId, ProfileId, ReasoningEffort};
use tea_session::{SessionReducer, SessionReplayError};

fn branch_record(
    sequence: usize,
    source_branch: &str,
    new_branch: &str,
    from_record: &str,
) -> tea_protocol::RecordEnvelope {
    envelope_on_branch(
        sequence,
        "branch_created",
        json!({
            "sourceBranchId":source_branch,
            "branchId":new_branch,
            "fromRecordId":from_record
        }),
        new_branch,
    )
}

#[test]
fn fork_clones_source_projection_without_rewriting_parent() {
    let creation = branched_creation();
    let first: tea_protocol::RecordEnvelope = serde_json::from_value({
        let mut value = serde_json::to_value(user_message(1)).unwrap();
        value["branchId"] = json!(ROOT_BRANCH_ID);
        value
    })
    .unwrap();
    let configured = envelope_on_branch(
        2,
        "configuration_changed",
        json!({"model":{"providerId":"openai","modelId":"openai/gpt-5"}}),
        ROOT_BRANCH_ID,
    );
    let fork = branch_record(
        3,
        ROOT_BRANCH_ID,
        FORK_BRANCH_ID,
        &first.record_id().to_string(),
    );
    let activate = envelope_on_branch(
        4,
        "active_branch_changed",
        json!({"branchId":FORK_BRANCH_ID}),
        FORK_BRANCH_ID,
    );
    let fork_config = envelope_on_branch(
        5,
        "configuration_changed",
        json!({"profileId":"fork-profile"}),
        FORK_BRANCH_ID,
    );
    let state = SessionReducer::replay([
        creation,
        first.clone(),
        configured.clone(),
        fork,
        activate,
        fork_config,
    ])
    .unwrap();

    let root = BranchId::from_str(ROOT_BRANCH_ID).unwrap();
    let fork = BranchId::from_str(FORK_BRANCH_ID).unwrap();
    assert_eq!(state.active_branch_id(), Some(fork));
    assert_eq!(state.messages().len(), 1);
    assert_eq!(state.configuration().model_id(), None);
    assert_eq!(
        state.configuration().profile_id(),
        &ProfileId::from_str("fork-profile").unwrap()
    );
    assert_eq!(
        state.branches()[&root].leaf_record_id(),
        configured.record_id()
    );
    assert_eq!(state.branches()[&fork].from_record_id(), first.record_id());
}

#[test]
fn branch_references_and_active_scope_fail_closed() {
    let creation = branched_creation();
    let unknown_source = branch_record(
        1,
        FORK_BRANCH_ID,
        "0195a0b1-5e60-7f10-b111-0aa7aa000040",
        &creation.record_id().to_string(),
    );
    assert!(matches!(
        SessionReducer::replay([creation.clone(), unknown_source]),
        Err(SessionReplayError::InvalidReference {
            reference: "source_branch"
        })
    ));

    let inactive_write = envelope_on_branch(
        1,
        "configuration_changed",
        json!({"model":{"providerId":"openai","modelId":"openai/gpt-5"}}),
        FORK_BRANCH_ID,
    );
    assert!(matches!(
        SessionReducer::replay([creation.clone(), inactive_write]),
        Err(SessionReplayError::InvalidReference {
            reference: "inactive_branch"
        })
    ));

    let unknown_active = envelope_on_branch(
        1,
        "active_branch_changed",
        json!({"branchId":FORK_BRANCH_ID}),
        FORK_BRANCH_ID,
    );
    assert!(matches!(
        SessionReducer::replay([creation, unknown_active]),
        Err(SessionReplayError::InvalidReference {
            reference: "active_branch_change"
        })
    ));
}

#[test]
fn fork_point_must_belong_to_source_history_and_branch_id_is_unique() {
    let creation = branched_creation();
    let config = envelope_on_branch(
        1,
        "configuration_changed",
        json!({"model":{"providerId":"openai","modelId":"openai/gpt-5"}}),
        ROOT_BRANCH_ID,
    );
    let outside = branch_record(
        2,
        ROOT_BRANCH_ID,
        FORK_BRANCH_ID,
        "0195a0b1-5e61-7a10-b111-0aa7aa000041",
    );
    assert!(matches!(
        SessionReducer::replay([creation.clone(), config.clone(), outside]),
        Err(SessionReplayError::InvalidReference {
            reference: "fork_point_outside_source_branch"
        })
    ));

    let fork = branch_record(
        2,
        ROOT_BRANCH_ID,
        FORK_BRANCH_ID,
        &config.record_id().to_string(),
    );
    let duplicate = branch_record(
        3,
        ROOT_BRANCH_ID,
        FORK_BRANCH_ID,
        &config.record_id().to_string(),
    );
    assert!(matches!(
        SessionReducer::replay([creation, config, fork, duplicate]),
        Err(SessionReplayError::DuplicateEntity { entity: "branch" })
    ));
}

#[test]
fn fork_rejects_pending_or_incomplete_tool_state() {
    let creation = branched_creation();
    let assistant: tea_protocol::RecordEnvelope = serde_json::from_value({
        let mut value = serde_json::to_value(common::assistant_tool_message(1)).unwrap();
        value["branchId"] = json!(ROOT_BRANCH_ID);
        value
    })
    .unwrap();
    let fork = branch_record(
        2,
        ROOT_BRANCH_ID,
        FORK_BRANCH_ID,
        &assistant.record_id().to_string(),
    );
    assert!(matches!(
        SessionReducer::replay([creation, assistant, fork]),
        Err(SessionReplayError::InvalidTransition {
            transition: "unsafe_fork_point"
        })
    ));
}

#[test]
fn root_branch_configuration_is_materialized() {
    let config = envelope_on_branch(
        1,
        "configuration_changed",
        json!({"model":{"providerId":"openai","modelId":"openai/gpt-5"}}),
        ROOT_BRANCH_ID,
    );
    let state = SessionReducer::replay([branched_creation(), config]).unwrap();
    assert_eq!(
        state.configuration().model_id(),
        Some(&ModelId::from_str("openai/gpt-5").unwrap())
    );
}

#[test]
fn fork_retains_reasoning_selected_before_the_fork_point() {
    let configured = envelope_on_branch(
        1,
        "configuration_changed",
        json!({
            "model":{"providerId":"openai","modelId":"openai/gpt-5"},
            "reasoningEffort":"high"
        }),
        ROOT_BRANCH_ID,
    );
    let fork = branch_record(
        2,
        ROOT_BRANCH_ID,
        FORK_BRANCH_ID,
        &configured.record_id().to_string(),
    );
    let activate = envelope_on_branch(
        3,
        "active_branch_changed",
        json!({"branchId":FORK_BRANCH_ID}),
        FORK_BRANCH_ID,
    );

    let state = SessionReducer::replay([branched_creation(), configured, fork, activate]).unwrap();
    assert_eq!(
        state.active_branch_id(),
        Some(FORK_BRANCH_ID.parse().unwrap())
    );
    assert_eq!(
        state.configuration().reasoning_effort(),
        Some(ReasoningEffort::High)
    );
}

#[test]
fn nested_fork_replays_branch_creation_as_a_historical_fork_point() {
    const SECOND_FORK_BRANCH_ID: &str = "0195a0b1-5e60-7f10-b111-0aa7aa000040";

    let first: tea_protocol::RecordEnvelope = serde_json::from_value({
        let mut value = serde_json::to_value(user_message(1)).unwrap();
        value["branchId"] = json!(ROOT_BRANCH_ID);
        value
    })
    .unwrap();
    let configured = envelope_on_branch(
        2,
        "configuration_changed",
        json!({"model":{"providerId":"openai","modelId":"openai/gpt-5"}}),
        ROOT_BRANCH_ID,
    );
    let first_fork = branch_record(
        3,
        ROOT_BRANCH_ID,
        FORK_BRANCH_ID,
        &first.record_id().to_string(),
    );
    let activate_first = envelope_on_branch(
        4,
        "active_branch_changed",
        json!({"branchId":FORK_BRANCH_ID}),
        FORK_BRANCH_ID,
    );
    let configure_first = envelope_on_branch(
        5,
        "configuration_changed",
        json!({"profileId":"first-fork-profile"}),
        FORK_BRANCH_ID,
    );
    let second_fork = branch_record(
        6,
        FORK_BRANCH_ID,
        SECOND_FORK_BRANCH_ID,
        &first_fork.record_id().to_string(),
    );
    let activate_second = envelope_on_branch(
        7,
        "active_branch_changed",
        json!({"branchId":SECOND_FORK_BRANCH_ID}),
        SECOND_FORK_BRANCH_ID,
    );

    let state = SessionReducer::replay([
        branched_creation(),
        first,
        configured,
        first_fork,
        activate_first,
        configure_first,
        second_fork,
        activate_second,
    ])
    .unwrap();

    assert_eq!(
        state.active_branch_id(),
        Some(SECOND_FORK_BRANCH_ID.parse().unwrap())
    );
    assert_eq!(state.messages().len(), 1);
    assert_eq!(state.configuration().model_id(), None);
    assert_eq!(
        state.configuration().profile_id(),
        &ProfileId::from_str("minimal-assistant").unwrap()
    );
}
