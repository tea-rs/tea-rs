use crate::common;

use std::sync::Arc;

use tea::{AgentRuntimeBuilder, RuntimeCommandOutcome, RuntimeErrorCode};
use tea_protocol::CanonicalMessage;
use tea_session::{InMemorySessionStore, SessionCatalog, SessionName};
use tea_testkit::ScriptedModelResponse;

use common::{TestIds, TestSessionIds, user_message};

fn builder(store: Arc<InMemorySessionStore>) -> AgentRuntimeBuilder {
    AgentRuntimeBuilder::new()
        .provider(common::provider_with([ScriptedModelResponse::text([
            "done",
        ])]))
        .clock(Arc::new(common::FixedClock))
        .ids(Arc::new(TestIds::default()))
        .session_id_source(Arc::new(TestSessionIds::default()))
        .session_store(store.clone())
        .session_catalog(store)
        .actor("user:alice".parse().unwrap())
        .tool(
            common::spec("read_file", tea_tools::ToolEffect::FsRead),
            Arc::new(
                tea_tools::ArgumentResourceResolver::new(
                    "path",
                    "file",
                    tea_tools::ToolResourceAccess::Read,
                )
                .unwrap(),
            ),
            Arc::new(tea_testkit::FakeReadTool::new([(
                "/a".to_owned(),
                "x".to_owned(),
            )])),
        )
        .unwrap()
        .tool(
            common::spec("write_file", tea_tools::ToolEffect::FsWrite),
            Arc::new(
                tea_tools::ArgumentResourceResolver::new(
                    "path",
                    "file",
                    tea_tools::ToolResourceAccess::Write,
                )
                .unwrap(),
            ),
            Arc::new(tea_testkit::FakeWriteTool::new()),
        )
        .unwrap()
        .policy_rule(
            "product.coding_workspace".parse().unwrap(),
            Arc::new(tea_policy::CodingWorkspacePolicy),
        )
        .unwrap()
        .profile(tea_profile::AgentProfile::coding_agent().unwrap())
}

#[tokio::test]
async fn fresh_runtime_attaches_existing_session_and_reports_state() {
    let store = Arc::new(InMemorySessionStore::new());
    let first = builder(store.clone()).build().unwrap();
    let session_id = match first
        .send(common::envelope_create("coding-agent"))
        .await
        .unwrap()
    {
        RuntimeCommandOutcome::Created { session_id } => session_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let prompt = user_message("hello");
    let _ = first
        .send(common::envelope_prompt(prompt, session_id))
        .await
        .unwrap();
    store
        .set_session_name(
            session_id,
            Some(SessionName::new("Attached session").unwrap()),
        )
        .await
        .unwrap();
    drop(first);

    let second = builder(store).build().unwrap();
    let attached = second.attach_session(session_id).await.unwrap();
    assert_eq!(attached.session_id(), session_id);
    assert_eq!(second.health().session_count(), 1);
    assert_eq!(attached.message_count(), 2);
    assert!(!attached.is_running());
    assert!(attached.pending_approval_id().is_none());
    assert_eq!(attached.name().unwrap().as_str(), "Attached session");

    let stats = second.session_stats(session_id).await.unwrap();
    assert_eq!(stats.message_count(), 2);
    assert_eq!(stats.user_messages(), 1);
    assert_eq!(stats.assistant_messages(), 1);
    assert_eq!(stats.tool_calls(), 0);
    assert!(matches!(
        second
            .snapshot(session_id)
            .await
            .unwrap()
            .state()
            .messages()[0],
        CanonicalMessage::User { .. }
    ));
}

#[tokio::test]
async fn attach_rejects_missing_session() {
    let store = Arc::new(InMemorySessionStore::new());
    let runtime = builder(store).build().unwrap();
    let unknown: tea_protocol::SessionId = "0195a0b1-5e3a-7000-8000-000000000099".parse().unwrap();
    let error = runtime.attach_session(unknown).await.unwrap_err();
    assert_eq!(error.code(), RuntimeErrorCode::SessionFailure);
}
