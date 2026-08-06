//! Process-restart recovery: a durable store reopens and reconstructs state.

use std::str::FromStr;

use serde_json::{Value, json};
use tea_protocol::{
    CanonicalMessage, ContentBlock, ExecutionTarget, ExternalSource, HostedToolActivity,
    HostedToolOutcome, MessageId, PolicyDecision, ProfileId, ProtocolMetadata, ProtocolTimestamp,
    ProviderContinuation, ReasoningEffort, RecordEnvelope, RecordId, SessionId, SessionRecord,
    SessionSequence, SourceCitation, StopReason, ToolCallId, ToolIdempotency, ToolPresentation,
    WebFetchPresentation, WebFetchRedirect, WebFetchTruncation,
};
use tea_session::{AppendTransaction, SessionStore, SessionStoreErrorCode, ToolExecutionState};
use tea_session_sqlite::SqliteSessionStore;

const NOW: &str = "2026-07-23T09:30:12.125Z";

fn session_id() -> SessionId {
    SessionId::from_str("0195a0b1-5e3a-7d72-a902-c4e85d828bf1").unwrap()
}
fn timestamp() -> ProtocolTimestamp {
    ProtocolTimestamp::from_str(NOW).unwrap()
}
fn envelope(seq: u64, rid: &str, record: SessionRecord) -> RecordEnvelope {
    RecordEnvelope::new(
        RecordId::from_str(rid).unwrap(),
        session_id(),
        SessionSequence::new(seq),
        timestamp(),
        None,
        None,
        None,
        ProtocolMetadata::default(),
        record,
    )
    .unwrap()
}

fn initial_transaction() -> AppendTransaction {
    AppendTransaction::new(
        session_id(),
        None,
        vec![
            envelope(
                0,
                "0195a0b1-5e50-79e1-8f4a-0aa7aa000022",
                SessionRecord::SessionCreated {
                    profile_id: ProfileId::from_str("coding").unwrap(),
                    metadata: ProtocolMetadata::default(),
                },
            ),
            envelope(
                1,
                "0195a0b1-5e51-79e1-8f4a-0aa7aa000023",
                SessionRecord::ConfigurationChanged {
                    model: Some(tea_protocol::ModelRef::new(
                        "fake".parse().unwrap(),
                        "fake/model".parse().unwrap(),
                    )),
                    profile_id: None,
                    reasoning_effort: Some(ReasoningEffort::High),
                },
            ),
            envelope(
                2,
                "0195a0b1-5e52-79e1-8f4a-0aa7aa000024",
                SessionRecord::MessageCommitted {
                    message: CanonicalMessage::user(
                        MessageId::from_str("0195a0b1-5e53-74b2-8c25-0aa7aa000025").unwrap(),
                        vec![ContentBlock::text("before restart").unwrap()],
                        timestamp(),
                    )
                    .unwrap(),
                },
            ),
        ],
    )
}

fn activity_continuation_payload() -> Value {
    json!({
        "encrypted_content":"durable-activity-state",
        "result_indexes":[1, 4],
        "resume":{"cursor":"reopen-cursor", "exhausted":false}
    })
}

fn citation_continuation_payload() -> Value {
    json!({
        "encrypted_index":"durable-citation-state",
        "source_position":{"item":2, "offset":9}
    })
}

fn hosted_assistant_message() -> CanonicalMessage {
    let tool_call_id = ToolCallId::from_str("0195a0b1-5e45-75be-8284-0aa7aa000011").unwrap();
    let source = ExternalSource::new("https://example.com/durable-hosted-result")
        .unwrap()
        .with_title("Durable hosted result")
        .unwrap()
        .with_snippet("This source must survive a SQLite reopen")
        .unwrap();
    let activity = HostedToolActivity::new(
        tool_call_id,
        "srvtoolu_sqlite_reopen_123",
        "web_search",
        json!({"query":"durable hosted search", "maxUses":7}),
        HostedToolOutcome::Success,
        vec![source.clone()],
        Some(
            ProviderContinuation::new(
                "anthropic",
                "anthropic.messages.web_search.v1",
                activity_continuation_payload(),
            )
            .unwrap(),
        ),
    )
    .unwrap();
    let citation = SourceCitation::new(source)
        .with_tool_call_id(tool_call_id)
        .with_range(0, 21)
        .unwrap()
        .with_cited_text("durable hosted result")
        .unwrap()
        .with_continuation(
            ProviderContinuation::new(
                "anthropic",
                "anthropic.messages.web_search.citation.v1",
                citation_continuation_payload(),
            )
            .unwrap(),
        );

    CanonicalMessage::assistant(
        MessageId::from_str("0195a0b1-5e64-76d6-9a5a-0aa7aa000042").unwrap(),
        vec![
            ContentBlock::text("durable hosted result").unwrap(),
            ContentBlock::hosted_tool(activity),
            ContentBlock::citation(citation),
        ],
        StopReason::Completed,
        timestamp(),
    )
    .unwrap()
}

#[tokio::test]
async fn reopened_store_reconstructs_durable_state() {
    let dir = tempfile_directory();
    let path = format!("{dir}/tea-recovery.sqlite");
    {
        let store = SqliteSessionStore::open(&path).unwrap();
        store.append(initial_transaction()).await.unwrap();
        let snapshot = store.load(session_id()).await.unwrap();
        assert_eq!(snapshot.state().messages().len(), 1);
        assert_eq!(
            snapshot
                .state()
                .configuration()
                .model_id()
                .map(tea_protocol::ModelId::as_str),
            Some("fake/model"),
        );
    } // store dropped, simulating process termination

    let reopened = SqliteSessionStore::open(&path).unwrap();
    let snapshot = reopened.load(session_id()).await.unwrap();
    assert_eq!(snapshot.state().messages().len(), 1);
    assert_eq!(
        snapshot.state().configuration().profile_id().as_str(),
        "coding",
    );
    assert_eq!(
        snapshot.state().configuration().reasoning_effort(),
        Some(ReasoningEffort::High)
    );
    // Stale expected sequence still conflicts after reopen.
    let stale = AppendTransaction::new(
        session_id(),
        Some(SessionSequence::new(99)),
        vec![envelope(
            3,
            "0195a0b1-5e54-79e1-8f4a-0aa7aa000026",
            SessionRecord::MessageCommitted {
                message: CanonicalMessage::user(
                    MessageId::from_str("0195a0b1-5e55-74b2-8c25-0aa7aa000027").unwrap(),
                    vec![ContentBlock::text("after restart").unwrap()],
                    timestamp(),
                )
                .unwrap(),
            },
        )],
    );
    let error = reopened.append(stale).await.unwrap_err();
    assert_eq!(error.code(), SessionStoreErrorCode::SequenceConflict);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn reopened_store_replays_once_then_serves_the_validated_cache() {
    let dir = tempfile_directory();
    let path = format!("{dir}/tea-cache-replay.sqlite");
    {
        let writer = SqliteSessionStore::open(&path).unwrap();
        writer.append(initial_transaction()).await.unwrap();
    }

    let cached = SqliteSessionStore::open(&path).unwrap();
    let first = cached.load(session_id()).await.unwrap();
    assert_eq!(first.records().len(), 3);
    let mutator = rusqlite::Connection::open(&path).unwrap();
    mutator
        .execute(
            "UPDATE records SET envelope = '{}' WHERE session_id = ? AND sequence = 0",
            [session_id().to_string()],
        )
        .unwrap();
    drop(mutator);

    let second = cached.load(session_id()).await.unwrap();
    assert_eq!(
        second, first,
        "cache hits must not replay durable rows again"
    );
    drop(cached);

    let reopened = SqliteSessionStore::open(&path).unwrap();
    let error = reopened.load(session_id()).await.unwrap_err();
    assert_eq!(error.code(), SessionStoreErrorCode::InvalidRecord);
    drop(reopened);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn reopened_store_preserves_hosted_content_and_provider_continuations() {
    let root = std::env::temp_dir().join(format!(
        "tea-session-sqlite-hosted-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("hosted-content.sqlite");
    let path = path.to_str().unwrap();

    let message = hosted_assistant_message();
    let records = vec![
        envelope(
            0,
            "0195a0b1-5e70-79e1-8f4a-0aa7aa000050",
            SessionRecord::SessionCreated {
                profile_id: ProfileId::from_str("coding").unwrap(),
                metadata: ProtocolMetadata::default(),
            },
        ),
        envelope(
            1,
            "0195a0b1-5e71-79e1-8f4a-0aa7aa000051",
            SessionRecord::MessageCommitted {
                message: message.clone(),
            },
        ),
    ];

    {
        let store = SqliteSessionStore::open(path).unwrap();
        store
            .append(AppendTransaction::new(session_id(), None, records.clone()))
            .await
            .unwrap();
    }

    {
        let reopened = SqliteSessionStore::open(path).unwrap();
        let snapshot = reopened.load(session_id()).await.unwrap();
        assert_eq!(snapshot.records(), records.as_slice());
        assert_eq!(snapshot.state().messages(), std::slice::from_ref(&message));
        assert!(snapshot.state().tool_calls().is_empty());

        let CanonicalMessage::Assistant { content, .. } = &snapshot.state().messages()[0] else {
            panic!("hosted content must remain in an assistant message");
        };
        let ContentBlock::HostedTool { activity } = &content[1] else {
            panic!("hosted activity must survive SQLite reopen");
        };
        assert_eq!(
            activity.continuation().unwrap().payload(),
            &activity_continuation_payload()
        );
        let ContentBlock::Citation { citation } = &content[2] else {
            panic!("citation must survive SQLite reopen");
        };
        assert_eq!(
            citation.continuation().unwrap().payload(),
            &citation_continuation_payload()
        );
    }

    std::fs::remove_dir_all(root).unwrap();
}

fn web_fetch_presentation() -> ToolPresentation {
    ToolPresentation::WebFetch(Box::new(
        WebFetchPresentation::new(
            "https://example.com/start",
            "https://example.com/final",
            "text/plain; charset=utf-8",
            "SQLite normalized fetch body",
        )
        .unwrap()
        .with_title("SQLite fetch title")
        .unwrap()
        .with_truncation(WebFetchTruncation::DecodedBytes)
        .with_redirects(vec![
            WebFetchRedirect::new(
                "https://example.com/start",
                "https://example.com/final",
                301,
            )
            .unwrap(),
        ])
        .unwrap(),
    ))
}

fn web_fetch_records(
    tool_call_id: ToolCallId,
    presentation: &ToolPresentation,
) -> Vec<RecordEnvelope> {
    let arguments = json!({"url":"https://example.com/start"});
    let content = vec![ContentBlock::text("normalized model-visible fetch result").unwrap()];
    let assistant = CanonicalMessage::assistant(
        MessageId::from_str("0195a0b1-5e64-76d6-9a5a-0aa7aa000062").unwrap(),
        vec![ContentBlock::tool_call(tool_call_id, "web_fetch", arguments.clone()).unwrap()],
        StopReason::ToolUse,
        timestamp(),
    )
    .unwrap();
    let result_message = CanonicalMessage::tool_result_success(
        MessageId::from_str("0195a0b1-5e66-7e4d-9af7-0aa7aa000063").unwrap(),
        tool_call_id,
        "web_fetch",
        content.clone(),
        timestamp(),
    )
    .unwrap();
    let records = [
        SessionRecord::SessionCreated {
            profile_id: ProfileId::from_str("coding").unwrap(),
            metadata: ProtocolMetadata::default(),
        },
        SessionRecord::MessageCommitted { message: assistant },
        SessionRecord::ToolCallRequested {
            tool_call_id,
            tool_name: "web_fetch".to_owned(),
            arguments,
        },
        SessionRecord::PolicyDecisionRecorded {
            tool_call_id,
            decision: PolicyDecision::Allow,
        },
        SessionRecord::ToolExecutionStarted {
            tool_call_id,
            execution_target: ExecutionTarget::Native,
            idempotency: ToolIdempotency::Idempotent,
        },
        SessionRecord::ToolExecutionFinished {
            tool_call_id,
            is_error: false,
            content,
            error: None,
            presentation: Some(presentation.clone()),
        },
        SessionRecord::MessageCommitted {
            message: result_message,
        },
    ];
    let record_ids = [
        "0195a0b1-5e70-79e1-8f4a-0aa7aa000060",
        "0195a0b1-5e71-79e1-8f4a-0aa7aa000061",
        "0195a0b1-5e72-79e1-8f4a-0aa7aa000062",
        "0195a0b1-5e73-79e1-8f4a-0aa7aa000063",
        "0195a0b1-5e74-79e1-8f4a-0aa7aa000064",
        "0195a0b1-5e75-79e1-8f4a-0aa7aa000065",
        "0195a0b1-5e76-79e1-8f4a-0aa7aa000066",
    ];
    records
        .into_iter()
        .zip(record_ids)
        .enumerate()
        .map(|(sequence, (record, record_id))| {
            envelope(u64::try_from(sequence).unwrap(), record_id, record)
        })
        .collect()
}

#[tokio::test]
async fn reopened_store_preserves_bounded_client_web_fetch_presentation() {
    let root = std::env::temp_dir().join(format!(
        "tea-session-sqlite-web-fetch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("web-fetch-content.sqlite");
    let path = path.to_str().unwrap();
    let tool_call_id = ToolCallId::from_str("0195a0b1-5e45-75be-8284-0aa7aa000061").unwrap();
    let presentation = web_fetch_presentation();
    let records = web_fetch_records(tool_call_id, &presentation);

    {
        let store = SqliteSessionStore::open(path).unwrap();
        store
            .append(AppendTransaction::new(session_id(), None, records.clone()))
            .await
            .unwrap();
    }

    let reopened = SqliteSessionStore::open(path).unwrap();
    let snapshot = reopened.load(session_id()).await.unwrap();
    assert_eq!(snapshot.records(), records.as_slice());
    let ToolExecutionState::Finished {
        presentation: Some(replayed),
        ..
    } = snapshot.state().tool_calls()[&tool_call_id].execution()
    else {
        panic!("web fetch presentation must survive SQLite reopen");
    };
    assert_eq!(replayed, &presentation);
    assert_eq!(
        replayed.web_fetch().unwrap().body(),
        "SQLite normalized fetch body"
    );
    let stored = serde_json::to_string(snapshot.records()).unwrap();
    assert!(!stored.contains("continuation"));
    assert!(!stored.contains("providerCallId"));

    drop(reopened);
    std::fs::remove_dir_all(root).unwrap();
}

fn tempfile_directory() -> String {
    let path = format!("/tmp/tea-test-{}", std::process::id());
    let _ = std::fs::create_dir_all(&path);
    path
}
