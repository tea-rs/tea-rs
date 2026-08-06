use tea::{
    AgentSession,
    model::{
        ModelCompletion, ModelEvent, ModelProvider, ModelResponseInfo, ModelStreamIndex,
        ProviderToolCallId, ToolCallCompleted, ToolCallStarted,
    },
    protocol::StopReason,
};
use tea_coding_tools::{WorkspaceRoot, read_only_workspace_tools};
use tea_testkit::ScriptedModelResponse;

use crate::common::provider_with;

fn tool_script(name: &str, arguments: serde_json::Value, opaque_id: &str) -> ScriptedModelResponse {
    let index = ModelStreamIndex::new(0).unwrap();
    let provider_id: ProviderToolCallId = opaque_id.parse().unwrap();
    ScriptedModelResponse::events([
        ModelEvent::Started(ModelResponseInfo::new()),
        ModelEvent::ToolCallStarted(
            ToolCallStarted::new(index, provider_id.clone(), name).unwrap(),
        ),
        ModelEvent::ToolCallCompleted(
            ToolCallCompleted::new(index, provider_id, name, arguments).unwrap(),
        ),
        ModelEvent::Completed(ModelCompletion::new(StopReason::ToolUse).unwrap()),
    ])
}

#[tokio::test]
async fn prompt_returns_aggregated_assistant_text() {
    let provider = provider_with([ScriptedModelResponse::text(["hello", " from tea"])]);
    let model = provider.models()[0].model_ref().clone();
    let session = AgentSession::builder(provider, model)
        .build()
        .await
        .unwrap();

    let response = session.prompt("Say hello.").await.unwrap();

    assert_eq!(response.text(), "hello from tea");
}

#[tokio::test]
async fn default_policy_executes_a_read_only_workspace_tool() {
    let provider = provider_with([
        tool_script("read", serde_json::json!({"path":"Cargo.toml"}), "read-1"),
        ScriptedModelResponse::text(["read completed"]),
    ]);
    let model = provider.models()[0].model_ref().clone();
    let workspace = WorkspaceRoot::new(std::env::current_dir().unwrap()).unwrap();
    let tools = read_only_workspace_tools(&workspace).unwrap();

    let session = AgentSession::builder(provider, model)
        .tools(tools)
        .build()
        .await
        .unwrap();

    let response = session.prompt("Read Cargo.toml.").await.unwrap();

    assert_eq!(response.text(), "read completed");
}
