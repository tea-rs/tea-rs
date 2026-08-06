use std::str::FromStr;

use futures_util::FutureExt;
use tea_context::{
    ContextProvider, ContextRequest, PromptBudget, PromptCompiler, PromptSegmentId,
    SessionSummaryProvider, TrustLevel, WorkspaceInstruction, WorkspaceInstructionProvider,
};
use tea_protocol::{ProfileId, ProtocolMetadata, RecordId, SessionId};

fn request() -> ContextRequest {
    ContextRequest::new(
        ProfileId::from_str("coding").unwrap(),
        SessionId::from_str("0195a0b1-5e3a-7d72-a902-c4e85d828bf1").unwrap(),
        None,
        vec![],
        ProtocolMetadata::default(),
    )
    .unwrap()
}

#[test]
fn workspace_documents_are_canonical_and_preserve_trust_provenance() {
    let provider = WorkspaceInstructionProvider::new(vec![
        WorkspaceInstruction::new(
            PromptSegmentId::from_str("workspace.zed").unwrap(),
            "second",
            "/workspace/sub/AGENTS.md",
            TrustLevel::Untrusted,
        )
        .unwrap(),
        WorkspaceInstruction::new(
            PromptSegmentId::from_str("workspace.root").unwrap(),
            "first",
            "/workspace/AGENTS.md",
            TrustLevel::Delegated,
        )
        .unwrap(),
    ])
    .unwrap();
    let modules = provider.provide(request()).now_or_never().unwrap().unwrap();
    let prompt = PromptCompiler
        .compile(modules, PromptBudget::new(100, 100).unwrap())
        .unwrap();
    assert_eq!(prompt.text(), "first\n\nsecond");
    assert_eq!(prompt.inspection()[0].trust(), TrustLevel::Delegated);
    assert_eq!(
        prompt.inspection()[1].provenance().locator(),
        Some("/workspace/sub/AGENTS.md")
    );
}

#[test]
fn duplicate_workspace_ids_and_invalid_locator_fail_closed() {
    let item = WorkspaceInstruction::new(
        PromptSegmentId::from_str("workspace.root").unwrap(),
        "content",
        "/workspace/AGENTS.md",
        TrustLevel::Delegated,
    )
    .unwrap();
    assert!(WorkspaceInstructionProvider::new(vec![item.clone(), item]).is_err());
    assert!(
        WorkspaceInstruction::new(
            PromptSegmentId::from_str("workspace.invalid").unwrap(),
            "content",
            "bad\npath",
            TrustLevel::Untrusted,
        )
        .is_err()
    );
}

#[test]
fn session_summary_is_optional_and_carries_record_provenance() {
    let empty = SessionSummaryProvider::empty().unwrap();
    assert!(
        empty
            .provide(request())
            .now_or_never()
            .unwrap()
            .unwrap()
            .is_empty()
    );

    let record_id = RecordId::from_str("0195a0b1-5e58-778a-a74e-0aa7aa000030").unwrap();
    let provider =
        SessionSummaryProvider::new("summary", record_id, TrustLevel::Delegated).unwrap();
    let modules = provider.provide(request()).now_or_never().unwrap().unwrap();
    let prompt = PromptCompiler
        .compile(modules, PromptBudget::new(100, 100).unwrap())
        .unwrap();
    assert_eq!(prompt.text(), "summary");
    assert_eq!(
        prompt.inspection()[0].provenance().locator(),
        Some(record_id.to_string().as_str())
    );
}
