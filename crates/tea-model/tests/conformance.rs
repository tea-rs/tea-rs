use std::str::FromStr;

use serde_json::json;
use tea_model::{
    HostedToolCompleted, HostedToolStarted, ModelCompletion, ModelEvent, ModelFailure,
    ModelFailureCode, ModelResponseInfo, ModelSourceCitation, ModelStreamIndex,
    ModelStreamValidator, ModelStreamViolation, ProviderToolCallId, ToolArgumentsDelta,
    ToolCallCompleted, ToolCallStarted, Utf8Delta,
};
use tea_protocol::{ExternalSource, HostedToolOutcome, RetryClass, SourceCitation, StopReason};

fn started() -> ModelEvent {
    ModelEvent::Started(ModelResponseInfo::new())
}

fn completed() -> ModelEvent {
    ModelEvent::Completed(ModelCompletion::new(StopReason::Completed).unwrap())
}

fn failed() -> ModelEvent {
    ModelEvent::Failed(
        ModelFailure::new(
            ModelFailureCode::Transport,
            "transport failed",
            RetryClass::Immediate,
        )
        .unwrap(),
    )
}

fn call_id(value: &str) -> ProviderToolCallId {
    ProviderToolCallId::from_str(value).unwrap()
}

#[test]
fn valid_stream_grammar_finishes_once() {
    let index = ModelStreamIndex::new(0).unwrap();
    let id = call_id("call_1");
    let events = [
        started(),
        ModelEvent::TextDelta(Utf8Delta::new("hello").unwrap()),
        ModelEvent::ToolCallStarted(ToolCallStarted::new(index, id.clone(), "read_file").unwrap()),
        ModelEvent::ToolArgumentsDelta(
            ToolArgumentsDelta::new(index, id.clone(), r#"{"path":"notes"#).unwrap(),
        ),
        ModelEvent::ToolCallCompleted(
            ToolCallCompleted::new(index, id, "read_file", json!({"path":"notes.txt"})).unwrap(),
        ),
        ModelEvent::Completed(ModelCompletion::new(StopReason::ToolUse).unwrap()),
    ];

    let mut validator = ModelStreamValidator::new();
    for event in &events {
        validator.observe(event).unwrap();
    }
    let summary = validator.finish().unwrap();
    assert_eq!(summary.event_count(), 6);
    assert_eq!(summary.completed_tool_calls(), 1);
    assert!(summary.succeeded());
}

#[test]
fn stream_must_start_and_terminate_exactly_once() {
    let mut validator = ModelStreamValidator::new();
    assert_eq!(
        validator.observe(&completed()).unwrap_err(),
        ModelStreamViolation::EventBeforeStart
    );
    assert_eq!(
        validator.finish().unwrap_err(),
        ModelStreamViolation::MissingStart
    );

    let mut validator = ModelStreamValidator::new();
    validator.observe(&started()).unwrap();
    assert_eq!(
        validator.observe(&started()).unwrap_err(),
        ModelStreamViolation::DuplicateStart
    );
    assert_eq!(
        validator.finish().unwrap_err(),
        ModelStreamViolation::MissingTerminal
    );

    let mut validator = ModelStreamValidator::new();
    validator.observe(&started()).unwrap();
    validator.observe(&completed()).unwrap();
    assert_eq!(
        validator.observe(&failed()).unwrap_err(),
        ModelStreamViolation::EventAfterTerminal
    );
}

#[test]
fn tool_fragments_must_match_a_unique_started_call() {
    let index = ModelStreamIndex::new(0).unwrap();
    let id = call_id("call_1");
    let other = call_id("call_2");
    let mut validator = ModelStreamValidator::new();
    validator.observe(&started()).unwrap();

    assert_eq!(
        validator
            .observe(&ModelEvent::ToolArgumentsDelta(
                ToolArgumentsDelta::new(index, id.clone(), "{").unwrap()
            ))
            .unwrap_err(),
        ModelStreamViolation::UnknownToolIndex
    );

    validator
        .observe(&ModelEvent::ToolCallStarted(
            ToolCallStarted::new(index, id.clone(), "read_file").unwrap(),
        ))
        .unwrap();
    assert_eq!(
        validator
            .observe(&ModelEvent::ToolCallStarted(
                ToolCallStarted::new(index, other.clone(), "write_file").unwrap()
            ))
            .unwrap_err(),
        ModelStreamViolation::DuplicateToolIndex
    );
    assert_eq!(
        validator
            .observe(&ModelEvent::ToolArgumentsDelta(
                ToolArgumentsDelta::new(index, other, "{}").unwrap()
            ))
            .unwrap_err(),
        ModelStreamViolation::ToolIdentityMismatch
    );
    assert_eq!(
        validator
            .observe(&ModelEvent::ToolCallCompleted(
                ToolCallCompleted::new(index, id, "write_file", json!({})).unwrap()
            ))
            .unwrap_err(),
        ModelStreamViolation::ToolIdentityMismatch
    );
}

#[test]
fn completed_tool_index_cannot_be_reused_in_one_response() {
    let index = ModelStreamIndex::new(0).unwrap();
    let first = call_id("call_1");
    let mut validator = ModelStreamValidator::new();
    validator.observe(&started()).unwrap();
    validator
        .observe(&ModelEvent::ToolCallStarted(
            ToolCallStarted::new(index, first.clone(), "read_file").unwrap(),
        ))
        .unwrap();
    validator
        .observe(&ModelEvent::ToolCallCompleted(
            ToolCallCompleted::new(index, first, "read_file", json!({})).unwrap(),
        ))
        .unwrap();

    assert_eq!(
        validator
            .observe(&ModelEvent::ToolCallStarted(
                ToolCallStarted::new(index, call_id("call_2"), "write_file").unwrap()
            ))
            .unwrap_err(),
        ModelStreamViolation::DuplicateToolIndex
    );
}

#[test]
fn successful_terminal_rejects_incomplete_tool_calls() {
    let index = ModelStreamIndex::new(0).unwrap();
    let mut validator = ModelStreamValidator::new();
    validator.observe(&started()).unwrap();
    validator
        .observe(&ModelEvent::ToolCallStarted(
            ToolCallStarted::new(index, call_id("call_1"), "read_file").unwrap(),
        ))
        .unwrap();
    assert_eq!(
        validator.observe(&completed()).unwrap_err(),
        ModelStreamViolation::IncompleteToolCalls
    );

    let mut failed_validator = ModelStreamValidator::new();
    failed_validator.observe(&started()).unwrap();
    failed_validator
        .observe(&ModelEvent::ToolCallStarted(
            ToolCallStarted::new(index, call_id("call_1"), "read_file").unwrap(),
        ))
        .unwrap();
    failed_validator.observe(&failed()).unwrap();
    let summary = failed_validator.finish().unwrap();
    assert!(!summary.succeeded());
}

#[test]
fn hosted_tool_grammar_is_distinct_and_citations_reference_completed_activity() {
    let index = ModelStreamIndex::new(1).unwrap();
    let id = call_id("ws_1");
    let source = ExternalSource::new("https://example.com/result").unwrap();
    let mut validator = ModelStreamValidator::new();
    validator.observe(&started()).unwrap();
    validator
        .observe(&ModelEvent::HostedToolStarted(
            HostedToolStarted::new(index, id.clone(), "web_search").unwrap(),
        ))
        .unwrap();
    assert_eq!(
        validator
            .observe(&ModelEvent::ToolCallStarted(
                ToolCallStarted::new(index, call_id("call_2"), "web_search").unwrap()
            ))
            .unwrap_err(),
        ModelStreamViolation::DuplicateToolIndex
    );
    validator
        .observe(&ModelEvent::HostedToolCompleted(
            HostedToolCompleted::new(
                index,
                id.clone(),
                "web_search",
                json!({"query":"example"}),
                HostedToolOutcome::Success,
                vec![source.clone()],
                None,
            )
            .unwrap(),
        ))
        .unwrap();
    validator
        .observe(&ModelEvent::SourceCitation(
            ModelSourceCitation::new(Some(id), SourceCitation::new(source)).unwrap(),
        ))
        .unwrap();
    validator.observe(&completed()).unwrap();
    let summary = validator.finish().unwrap();
    assert_eq!(summary.completed_tool_calls(), 0);
    assert_eq!(summary.completed_hosted_tools(), 1);
}

#[test]
fn hosted_completion_and_citation_identity_fail_closed() {
    let index = ModelStreamIndex::new(1).unwrap();
    let id = call_id("ws_1");
    let source = ExternalSource::new("https://example.com/result").unwrap();
    let mut validator = ModelStreamValidator::new();
    validator.observe(&started()).unwrap();
    assert_eq!(
        validator
            .observe(&ModelEvent::SourceCitation(
                ModelSourceCitation::new(Some(id.clone()), SourceCitation::new(source.clone()))
                    .unwrap()
            ))
            .unwrap_err(),
        ModelStreamViolation::UnknownHostedCitation
    );
    validator
        .observe(&ModelEvent::HostedToolStarted(
            HostedToolStarted::new(index, id, "web_search").unwrap(),
        ))
        .unwrap();
    assert_eq!(
        validator.observe(&completed()).unwrap_err(),
        ModelStreamViolation::IncompleteToolCalls
    );
}
