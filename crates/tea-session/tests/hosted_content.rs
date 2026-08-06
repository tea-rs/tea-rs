use crate::common;

use std::str::FromStr;

use common::{TOOL_CALL_ID, completed_tool_flow, creation, envelope};
use serde_json::{Value, json};
use tea_protocol::{
    CanonicalMessage, ContentBlock, ExternalSource, HostedToolActivity, HostedToolOutcome,
    MessageId, ProviderContinuation, RecordEnvelope, SourceCitation, StopReason, ToolCallId,
    ToolPresentation, WebFetchPresentation, WebFetchRedirect, WebFetchTruncation,
};
use tea_session::{SessionReducer, ToolExecutionState};

fn activity_continuation_payload() -> Value {
    json!({
        "encrypted_content":"activity-provider-state",
        "result_indexes":[0, 2],
        "resume":{"cursor":"next-page", "exhausted":false}
    })
}

fn citation_continuation_payload() -> Value {
    json!({
        "encrypted_index":"citation-provider-state",
        "source_position":{"item":3, "offset":17}
    })
}

fn hosted_assistant_message() -> CanonicalMessage {
    let tool_call_id = ToolCallId::from_str(TOOL_CALL_ID).unwrap();
    let source = ExternalSource::new("https://example.com/tea-rs-hosted-search")
        .unwrap()
        .with_title("tea-rs hosted search")
        .unwrap()
        .with_snippet("Provider-hosted search result")
        .unwrap();
    let activity_continuation = ProviderContinuation::new(
        "anthropic",
        "anthropic.messages.web_search.v1",
        activity_continuation_payload(),
    )
    .unwrap();
    let citation_continuation = ProviderContinuation::new(
        "anthropic",
        "anthropic.messages.web_search.citation.v1",
        citation_continuation_payload(),
    )
    .unwrap();
    let activity = HostedToolActivity::new(
        tool_call_id,
        "srvtoolu_persisted_123",
        "web_search",
        json!({"query":"tea-rs hosted search", "maxUses":5}),
        HostedToolOutcome::Success,
        vec![source.clone()],
        Some(activity_continuation),
    )
    .unwrap();
    let citation = SourceCitation::new(source)
        .with_tool_call_id(tool_call_id)
        .with_range(0, 20)
        .unwrap()
        .with_cited_text("tea-rs hosted search")
        .unwrap()
        .with_continuation(citation_continuation);

    CanonicalMessage::assistant(
        MessageId::from_str("0195a0b1-5e64-76d6-9a5a-0aa7aa000042").unwrap(),
        vec![
            ContentBlock::text("tea-rs hosted search").unwrap(),
            ContentBlock::hosted_tool(activity),
            ContentBlock::citation(citation),
        ],
        StopReason::Completed,
        "2026-07-23T09:30:13.000Z".parse().unwrap(),
    )
    .unwrap()
}

#[test]
fn hosted_content_round_trips_and_replays_without_local_tool_projection() {
    let message = hosted_assistant_message();
    let record = envelope(1, "message_committed", json!({"message":message.clone()}));
    let decoded = RecordEnvelope::decode_value(serde_json::to_value(&record).unwrap()).unwrap();
    assert_eq!(decoded, record);

    let state = SessionReducer::replay([creation(), decoded]).unwrap();
    assert_eq!(state.messages(), std::slice::from_ref(&message));
    assert!(state.tool_calls().is_empty());

    let CanonicalMessage::Assistant { content, .. } = &state.messages()[0] else {
        panic!("hosted content must remain in an assistant message");
    };
    let ContentBlock::HostedTool { activity } = &content[1] else {
        panic!("hosted activity must survive replay");
    };
    assert_eq!(
        activity.continuation().unwrap().payload(),
        &activity_continuation_payload()
    );
    let ContentBlock::Citation { citation } = &content[2] else {
        panic!("citation must survive replay");
    };
    assert_eq!(
        citation.continuation().unwrap().payload(),
        &citation_continuation_payload()
    );
}

fn web_fetch_presentation() -> ToolPresentation {
    ToolPresentation::WebFetch(Box::new(
        WebFetchPresentation::new(
            "https://example.com/start",
            "https://example.com/final",
            "text/html; charset=utf-8",
            "normalized replay body",
        )
        .unwrap()
        .with_title("Replay title")
        .unwrap()
        .with_truncation(WebFetchTruncation::BodyCharacters)
        .with_redirects(vec![
            WebFetchRedirect::new(
                "https://example.com/start",
                "https://example.com/final",
                302,
            )
            .unwrap(),
        ])
        .unwrap(),
    ))
}

#[test]
fn client_web_fetch_presentation_round_trips_and_replays_without_continuation() {
    let presentation = web_fetch_presentation();
    let mut records = completed_tool_flow();
    let mut terminal = serde_json::to_value(&records[8]).unwrap();
    terminal["payload"]["presentation"] = serde_json::to_value(&presentation).unwrap();
    records[8] = RecordEnvelope::decode_value(terminal).unwrap();

    let encoded = serde_json::to_string(&records[8]).unwrap();
    assert!(!encoded.contains("continuation"));
    assert!(!encoded.contains("providerCallId"));

    let state = SessionReducer::replay(records).unwrap();
    let tool_call_id = ToolCallId::from_str(TOOL_CALL_ID).unwrap();
    let ToolExecutionState::Finished {
        presentation: Some(replayed),
        ..
    } = state.tool_calls()[&tool_call_id].execution()
    else {
        panic!("web fetch presentation must survive in-memory replay");
    };
    assert_eq!(replayed, &presentation);
    let fetch = replayed.web_fetch().unwrap();
    assert_eq!(fetch.title(), Some("Replay title"));
    assert_eq!(fetch.body(), "normalized replay body");
    assert_eq!(fetch.redirects().len(), 1);
}
