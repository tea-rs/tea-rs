use crate::common;

use serde_json::json;
use tea_policy::{MAX_POLICY_GRANTS, PolicyInput};
use tea_tools::{ToolEffect, ToolResourceAccess};

#[test]
fn policy_input_snapshots_all_validated_dimensions() {
    let invocation = common::validated_invocation(
        "write_file",
        vec![ToolEffect::FsWrite],
        vec![common::file_resource(
            "/workspace/notes.txt",
            ToolResourceAccess::Write,
        )],
        json!({"path":"/workspace/notes.txt"}),
    );
    let now = common::timestamp("2026-07-23T10:00:00.000Z");
    let input = common::input_with(&invocation, vec![], now);

    assert_eq!(input.actor_id().as_str(), "user:alice");
    assert_eq!(input.profile_id().as_str(), "coding");
    assert_eq!(input.session_id(), &common::session_id());
    assert_eq!(input.run_id(), Some(&common::run_id()));
    assert_eq!(input.workspace_id().unwrap().as_str(), "workspace/main");
    assert_eq!(input.tool_call_id(), &common::tool_call_id());
    assert_eq!(input.tool_name().as_str(), "write_file");
    assert_eq!(input.tool_version().to_string(), "1.0.0");
    assert_eq!(input.arguments(), &json!({"path":"/workspace/notes.txt"}));
    assert_eq!(input.effects(), &[ToolEffect::FsWrite]);
    assert_eq!(input.resources().len(), 1);
    assert_eq!(input.now(), now);
    assert!(input.grants().is_empty());
}

#[test]
fn policy_input_is_send_sync_and_bounds_grant_candidates() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PolicyInput>();
    assert_eq!(MAX_POLICY_GRANTS, 128);
}
