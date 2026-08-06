use crate::common;

use common::{creation, envelope, user_message};
use serde_json::json;
use std::str::FromStr;
use tea_protocol::{ModelId, ProfileId, ReasoningEffort, SessionSequence};
use tea_session::SessionReducer;

#[test]
fn replay_reconstructs_creation_messages_and_configuration() {
    let records = vec![
        creation(),
        user_message(1),
        envelope(
            2,
            "configuration_changed",
            json!({
                "model":{"providerId":"openai","modelId":"openai/gpt-5"},
                "profileId":"coding-agent",
                "reasoningEffort":"high"
            }),
        ),
    ];
    let state = SessionReducer::replay(records).unwrap();
    assert_eq!(state.tail_sequence(), SessionSequence::new(2));
    assert_eq!(state.messages().len(), 1);
    assert_eq!(
        state.configuration().model_id(),
        Some(&ModelId::from_str("openai/gpt-5").unwrap())
    );
    assert_eq!(
        state.configuration().profile_id(),
        &ProfileId::from_str("coding-agent").unwrap()
    );
    assert_eq!(
        state.configuration().reasoning_effort(),
        Some(ReasoningEffort::High)
    );
}

#[test]
fn incremental_reduction_equals_full_replay() {
    let records = vec![
        creation(),
        user_message(1),
        envelope(
            2,
            "configuration_changed",
            json!({"model":{"providerId":"openai","modelId":"openai/gpt-5"}}),
        ),
    ];
    let expected = SessionReducer::replay(records.clone()).unwrap();
    let mut reducer = SessionReducer::new();
    for record in &records {
        reducer.apply(record).unwrap();
    }
    assert_eq!(reducer.state(), Some(&expected));
}

#[test]
fn failed_incremental_apply_does_not_mutate_state() {
    let mut reducer = SessionReducer::new();
    reducer.apply(&creation()).unwrap();
    let before = reducer.state().cloned();
    let gap = envelope(
        2,
        "configuration_changed",
        json!({"model":{"providerId":"openai","modelId":"openai/gpt-5"}}),
    );
    assert!(reducer.apply(&gap).is_err());
    assert_eq!(reducer.state(), before.as_ref());
}
