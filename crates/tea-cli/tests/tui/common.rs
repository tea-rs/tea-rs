use std::str::FromStr as _;

use serde_json::Value;
use tea_session::{InMemorySessionStore, SessionArchive, SessionSnapshot, SessionStore as _};

pub async fn archive_snapshot() -> SessionSnapshot {
    snapshot_from_value(serde_json::from_str(archive_json()).unwrap()).await
}

pub async fn reasoning_snapshot(effort: &str) -> SessionSnapshot {
    let mut value: Value = serde_json::from_str(archive_json()).unwrap();
    let records = value["records"].as_array_mut().unwrap();
    records.truncate(1);
    let mut configured = records[0].clone();
    configured["recordId"] = serde_json::json!("0195a0b1-5e51-79e1-8f4a-0aa7aa000023");
    configured["sequence"] = serde_json::json!("1");
    configured["type"] = serde_json::json!("configuration_changed");
    configured["payload"] = serde_json::json!({
        "model": {
            "providerId": "fake",
            "modelId": "fake/model"
        },
        "reasoningEffort": effort
    });
    records.push(configured);
    value["approvalArtifacts"] = serde_json::json!([]);
    value["grantJournal"] = serde_json::json!([]);
    snapshot_from_value(value).await
}

pub async fn pending_snapshot() -> SessionSnapshot {
    let mut value: Value = serde_json::from_str(archive_json()).unwrap();
    value["records"].as_array_mut().unwrap().truncate(6);
    value["approvalArtifacts"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    value["grantJournal"] = serde_json::json!([]);
    snapshot_from_value(value).await
}

pub async fn image_snapshot() -> SessionSnapshot {
    let mut value: Value = serde_json::from_str(archive_json()).unwrap();
    value["records"][1]["payload"]["message"]["content"] = serde_json::json!([
        {"type": "text", "text": "Inspect this image."},
        {
            "type": "image",
            "mimeType": "image/png",
            "source": {"type": "inline_base64", "data": "iVBORw0KGgo="}
        },
        {
            "type": "image",
            "mimeType": "image/jpeg",
            "source": {"type": "reference", "reference": "private:artifact-image"}
        }
    ]);
    snapshot_from_value(value).await
}

pub async fn hosted_search_snapshot() -> SessionSnapshot {
    let mut value: Value = serde_json::from_str(archive_json()).unwrap();
    let content = value["records"][2]["payload"]["message"]["content"]
        .as_array_mut()
        .unwrap();
    content.extend([
        serde_json::json!({
            "type": "hosted_tool",
            "activity": {
                "toolCallId": "0195a0b1-5e45-75be-8284-0aa7aa000099",
                "providerCallId": "srvtoolu_search_123",
                "toolName": "web_search",
                "arguments": {"query": "tea-rs hosted search architecture"},
                "outcome": {"status": "success"},
                "sources": [
                    {
                        "url": "https://Example.COM:443/a/../docs",
                        "title": "Hosted search architecture"
                    },
                    {
                        "url": "https://docs.example.test/provider-search",
                        "title": "Provider search reference"
                    }
                ],
                "continuation": {
                    "provider": "anthropic",
                    "format": "anthropic.messages.web_search.v1",
                    "payload": {"encryptedContent": "CONTINUATION_MUST_NOT_RENDER"}
                }
            }
        }),
        serde_json::json!({
            "type": "citation",
            "citation": {
                "toolCallId": "0195a0b1-5e45-75be-8284-0aa7aa000099",
                "source": {
                    "url": "https://example.com/docs",
                    "title": "Duplicate citation title"
                },
                "startIndex": 0,
                "endIndex": 8,
                "citedText": "tea-rs",
                "continuation": {
                    "provider": "anthropic",
                    "format": "anthropic.messages.web_search.citation.v1",
                    "payload": {"encryptedIndex": "CITATION_CONTINUATION_MUST_NOT_RENDER"}
                }
            }
        }),
        serde_json::json!({
            "type": "citation",
            "citation": {
                "toolCallId": "0195a0b1-5e45-75be-8284-0aa7aa000099",
                "source": {
                    "url": "https://docs.example.test/provider-search",
                    "title": "Duplicate provider reference"
                }
            }
        }),
    ]);
    snapshot_from_value(value).await
}

pub async fn diff_snapshot() -> SessionSnapshot {
    let mut value: Value = serde_json::from_str(archive_json()).unwrap();
    value["records"][8]["payload"]["presentation"] = serde_json::json!({
        "type":"code_change",
        "value":{
            "path":"src/lib.rs",
            "kind":"update",
            "hunks":[{
                "oldStart":1,
                "oldLines":3,
                "newStart":1,
                "newLines":3,
                "lines":[
                    {"kind":"context","oldLine":1,"newLine":1,"text":"pub fn answer() -> i32 {"},
                    {"kind":"deletion","oldLine":2,"text":"    1"},
                    {"kind":"addition","newLine":2,"text":"    2\u{1b}[31m"},
                    {"kind":"context","oldLine":3,"newLine":3,"text":"}"}
                ]
            }],
            "truncated":false,
            "patch":"--- src/lib.rs\n+++ src/lib.rs\n@@ -1,3 +1,3 @@\n pub fn answer() -> i32 {\n-    1\n+    2\u{1b}[31m\n }\n",
            "firstChangedLine":2
        }
    });
    snapshot_from_value(value).await
}

pub async fn web_fetch_snapshot() -> SessionSnapshot {
    let mut value: Value = serde_json::from_str(archive_json()).unwrap();
    value["records"][8]["payload"]["content"] = serde_json::json!([{
        "type":"text",
        "text":"MODEL_RESULT_MUST_NOT_RENDER"
    }]);
    value["records"][8]["payload"]["presentation"] = serde_json::json!({
        "type":"web_fetch",
        "value":{
            "requestedUrl":"https://example.com/start",
            "finalUrl":"https://example.com/final",
            "title":"A fetched page with a-very-long-unbroken-title-token-0123456789",
            "mimeType":"text/html; charset=utf-8",
            "body":"Normalized body with visible control [31m and a long-token-abcdefghijklmnopqrstuvwxyz-0123456789.",
            "truncation":"body_characters",
            "redirects":[{
                "from":"https://example.com/start",
                "to":"https://example.com/final",
                "status":302
            }]
        }
    });
    value["records"][9]["payload"]["message"]["content"] = serde_json::json!([{
        "type":"text",
        "text":"MODEL_RESULT_MUST_NOT_RENDER"
    }]);
    snapshot_from_value(value).await
}

pub fn startup() -> tea_cli::tui::StartupContext {
    tea_cli::tui::StartupContext::new("workspace/demo", 2, 3, 1, 0)
}

pub fn event(
    sequence: u64,
    event_id_suffix: u16,
    event: tea_protocol::AgentEvent,
) -> tea_protocol::EventEnvelope {
    let event_id = format!("0195a0b1-6e00-7000-8000-00000000{event_id_suffix:04x}")
        .parse()
        .unwrap();
    let run_id = matches!(
        event,
        tea_protocol::AgentEvent::RunStarted {} | tea_protocol::AgentEvent::RunFinished { .. }
    )
    .then(|| "0195a0b1-5e40-7136-8ae0-0aa7aa000006".parse().unwrap())
    .or_else(|| {
        matches!(
            event,
            tea_protocol::AgentEvent::MessageDelta { .. }
                | tea_protocol::AgentEvent::ToolCallRequested { .. }
                | tea_protocol::AgentEvent::ApprovalRequested { .. }
                | tea_protocol::AgentEvent::ToolExecutionProgress { .. }
                | tea_protocol::AgentEvent::ToolExecutionPreview { .. }
                | tea_protocol::AgentEvent::HostedToolStarted { .. }
                | tea_protocol::AgentEvent::HostedToolCompleted { .. }
                | tea_protocol::AgentEvent::ModelRetryScheduled { .. }
                | tea_protocol::AgentEvent::ModelRetryStarted { .. }
                | tea_protocol::AgentEvent::TurnCheckpointed {}
        )
        .then(|| "0195a0b1-5e40-7136-8ae0-0aa7aa000006".parse().unwrap())
    });
    let turn_id = matches!(
        event,
        tea_protocol::AgentEvent::MessageDelta { .. }
            | tea_protocol::AgentEvent::ToolCallRequested { .. }
            | tea_protocol::AgentEvent::ApprovalRequested { .. }
            | tea_protocol::AgentEvent::ToolExecutionProgress { .. }
            | tea_protocol::AgentEvent::ToolExecutionPreview { .. }
            | tea_protocol::AgentEvent::HostedToolStarted { .. }
            | tea_protocol::AgentEvent::HostedToolCompleted { .. }
            | tea_protocol::AgentEvent::ModelRetryScheduled { .. }
            | tea_protocol::AgentEvent::ModelRetryStarted { .. }
            | tea_protocol::AgentEvent::TurnCheckpointed {}
    )
    .then(|| "0195a0b1-5e42-7b38-af7c-0aa7aa000008".parse().unwrap());
    tea_protocol::EventEnvelope::new(
        event_id,
        tea_protocol::SessionId::from_str("0195a0b1-5e3a-7d72-a902-c4e85d828bf1").unwrap(),
        run_id,
        turn_id,
        tea_protocol::SessionSequence::new(sequence),
        "2026-07-24T10:00:00.000Z".parse().unwrap(),
        tea_protocol::ProtocolMetadata::default(),
        event,
    )
    .unwrap()
}

pub fn message_id() -> tea_protocol::MessageId {
    "0195a0b1-7e00-7000-8000-000000000001".parse().unwrap()
}

pub fn tool_call_id() -> tea_protocol::ToolCallId {
    "0195a0b1-7e00-7000-8000-000000000002".parse().unwrap()
}

async fn snapshot_from_value(value: Value) -> SessionSnapshot {
    let archive = SessionArchive::decode_json(&serde_json::to_string(&value).unwrap()).unwrap();
    let session_id = archive.session_id();
    let store = InMemorySessionStore::new();
    archive.import_into(&store).await.unwrap();
    store.load(session_id).await.unwrap()
}

fn archive_json() -> &'static str {
    include_str!("../../../tea-session/tests/fixtures/v1/session-archive.json")
}
