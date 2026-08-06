use std::str::FromStr;

use futures_util::FutureExt;
use serde_json::json;
use tea_context::{
    ContextProvider, ContextRequest, PromptBudget, PromptCompiler, ToolHintProvider,
};
use tea_protocol::{ProfileId, ProtocolMetadata, SessionId, ToolIdempotency};
use tea_tools::{
    ToolConcurrency, ToolEffect, ToolExecutionSemantics, ToolName, ToolRetrySafety, ToolSpec,
    ToolTimeout, ToolVersion,
};

fn tool(name: &str, snippet: Option<&str>, guidelines: &[&str]) -> ToolSpec {
    let spec = ToolSpec::new(
        ToolName::from_str(name).unwrap(),
        ToolVersion::from_str("1.0.0").unwrap(),
        "Test tool.",
        json!({"type":"object"}),
        json!({"type":"object"}),
        [ToolEffect::FsRead],
        ToolExecutionSemantics::new(
            ToolIdempotency::Idempotent,
            ToolRetrySafety::Automatic,
            ToolConcurrency::Parallel,
            ToolTimeout::from_millis(1_000).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let spec = snippet
        .map_or(Ok(spec.clone()), |value| spec.with_prompt_snippet(value))
        .unwrap();
    spec.with_prompt_guidelines(guidelines.iter().copied())
        .unwrap()
}

fn request(tools: Vec<ToolSpec>) -> ContextRequest {
    ContextRequest::new(
        ProfileId::from_str("coding").unwrap(),
        SessionId::from_str("0195a0b1-5e3a-7d72-a902-c4e85d828bf1").unwrap(),
        None,
        tools,
        ProtocolMetadata::default(),
    )
    .unwrap()
}

#[test]
fn active_tool_hints_are_canonical_and_track_removal() {
    let provider = ToolHintProvider::new().unwrap();
    let all = provider
        .provide(request(vec![
            tool("z_tool", Some("Use Z."), &["Keep Z bounded."]),
            tool("no_hint", None, &[]),
            tool(
                "a_tool",
                Some("Use A."),
                &["Keep A bounded.", "Check A first."],
            ),
        ]))
        .now_or_never()
        .unwrap()
        .unwrap();
    let prompt = PromptCompiler
        .compile(all, PromptBudget::new(4096, 4096).unwrap())
        .unwrap();
    assert_eq!(
        prompt.text(),
        "Tool `a_tool`: Use A.\n\nTool `z_tool`: Use Z.\n\nTool `a_tool` guidelines:\n- Keep A bounded.\n- Check A first.\n\nTool `z_tool` guidelines:\n- Keep Z bounded."
    );
    assert!(!prompt.text().contains("no_hint"));

    let reduced = provider
        .provide(request(vec![tool("z_tool", Some("Use Z."), &[])]))
        .now_or_never()
        .unwrap()
        .unwrap();
    let next = PromptCompiler
        .compile(reduced, PromptBudget::new(4096, 4096).unwrap())
        .unwrap();
    assert_eq!(next.text(), "Tool `z_tool`: Use Z.");
    assert_eq!(next.inspection().len(), 1);
}
