use crate::common;

use std::str::FromStr;

use serde_json::json;
use tea_policy::{
    ApprovalError, ApprovalPresentation, ApprovalRequest, ApprovalResolution, GrantScope,
    PolicyGrant, PolicyRedactor, ResourcePattern,
};
use tea_protocol::{ApprovalDecision, ApprovalId, ProfileId};
use tea_tools::{
    ToolEffect, ToolName, ToolResourceAccess, ToolSource, ToolSourceKind, ToolTrust, ToolVersion,
};

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn input() -> tea_policy::PolicyInput {
    let invocation = common::validated_invocation(
        "write_file",
        vec![ToolEffect::FsWrite],
        vec![common::file_resource(
            "/workspace/notes.txt",
            ToolResourceAccess::Write,
        )],
        json!({"path":"/workspace/notes.txt","apiKey":"secret"}),
    );
    common::input_with(
        &invocation,
        vec![],
        common::timestamp("2026-07-23T10:00:00.000Z"),
    )
}

fn request() -> ApprovalRequest {
    let input = input();
    let presentation = ApprovalPresentation::from_input(
        "workspace mutation requires approval",
        &input,
        PolicyRedactor,
    )
    .unwrap();
    ApprovalRequest::new(
        ApprovalId::from_str("0195a0b1-5e42-7a74-a5e2-0aa7aa000008").unwrap(),
        &input,
        common::timestamp("2026-07-23T10:00:00.000Z"),
        common::timestamp("2026-07-23T10:05:00.000Z"),
        presentation,
    )
    .unwrap()
}

fn matching_grant() -> PolicyGrant {
    PolicyGrant::new(
        common::grant_id(),
        "user:alice".parse().unwrap(),
        ProfileId::from_str("coding").unwrap(),
        ToolName::from_str("write_file").unwrap(),
        ToolVersion::from_str("1.0.0").unwrap(),
        [ToolEffect::FsWrite],
        [ResourcePattern::new("file", "/workspace/", Some(ToolResourceAccess::Write)).unwrap()],
        GrantScope::SessionResource {
            session_id: common::session_id(),
        },
        common::timestamp("2026-07-23T10:01:00.000Z"),
    )
    .unwrap()
    .with_source(ToolSource::native_product())
}

#[test]
fn request_contains_only_redacted_presentation_and_bounded_context() {
    let request = request();
    assert_eq!(request.tool_name().as_str(), "write_file");
    assert_eq!(request.effects(), &[ToolEffect::FsWrite]);
    assert_eq!(request.resources().len(), 1);
    assert_eq!(request.tool_source(), &ToolSource::native_product());
    assert_eq!(
        request.presentation().arguments().value()["apiKey"],
        "[REDACTED]"
    );
    let encoded = serde_json::to_string(&request).unwrap();
    assert!(!encoded.contains("secret"));
    assert_eq!(
        serde_json::from_str::<ApprovalRequest>(&encoded).unwrap(),
        request
    );
}

#[test]
fn legacy_native_request_without_source_remains_readable() {
    let request = request();
    let mut value = serde_json::to_value(&request).unwrap();
    value.as_object_mut().unwrap().remove("toolSource");
    let decoded = serde_json::from_value::<ApprovalRequest>(value).unwrap();
    assert_eq!(decoded.tool_source(), &ToolSource::native_product());
}

#[test]
fn request_and_resolution_temporal_boundaries_fail_closed() {
    let input = input();
    let presentation = ApprovalPresentation::from_input("reason", &input, PolicyRedactor).unwrap();
    assert_eq!(
        ApprovalRequest::new(
            ApprovalId::from_str("0195a0b1-5e42-7a74-a5e2-0aa7aa000008").unwrap(),
            &input,
            common::timestamp("2026-07-23T10:00:00.000Z"),
            common::timestamp("2026-07-23T10:00:00.000Z"),
            presentation,
        )
        .unwrap_err(),
        ApprovalError::InvalidExpiry
    );
    assert_eq!(
        ApprovalResolution::new(
            &request(),
            ApprovalDecision::AllowOnce,
            common::timestamp("2026-07-23T10:05:00.000Z"),
            None,
        )
        .unwrap_err(),
        ApprovalError::ResolutionOutsideLifetime
    );
}

#[test]
fn only_allow_session_can_issue_a_matching_grant() {
    let request = request();
    assert_eq!(
        ApprovalResolution::new(
            &request,
            ApprovalDecision::AllowOnce,
            common::timestamp("2026-07-23T10:02:00.000Z"),
            Some(matching_grant()),
        )
        .unwrap_err(),
        ApprovalError::GrantNotAllowed
    );
    let resolution = ApprovalResolution::new(
        &request,
        ApprovalDecision::AllowSession,
        common::timestamp("2026-07-23T10:02:00.000Z"),
        Some(matching_grant()),
    )
    .unwrap();
    assert!(resolution.issued_grant().is_some());
    let encoded = serde_json::to_string(&resolution).unwrap();
    assert_eq!(
        serde_json::from_str::<ApprovalResolution>(&encoded).unwrap(),
        resolution
    );
}

#[test]
fn approval_rejects_grant_source_drift() {
    let request = request();
    let drifted_source = ToolSource::new(
        ToolSourceKind::Mcp,
        "workspace.files",
        ToolTrust::Workspace,
        DIGEST,
    )
    .unwrap();
    let grant = matching_grant().with_source(drifted_source);
    assert_eq!(
        ApprovalResolution::new(
            &request,
            ApprovalDecision::AllowSession,
            common::timestamp("2026-07-23T10:02:00.000Z"),
            Some(grant),
        )
        .unwrap_err(),
        ApprovalError::GrantMismatch
    );
}

#[test]
fn direct_deserialization_revalidates_temporal_and_grant_invariants() {
    let request = request();
    let mut value = serde_json::to_value(&request).unwrap();
    value["expiresAt"] = value["createdAt"].clone();
    assert!(serde_json::from_value::<ApprovalRequest>(value).is_err());

    let resolution = ApprovalResolution::new(
        &request,
        ApprovalDecision::AllowSession,
        common::timestamp("2026-07-23T10:02:00.000Z"),
        Some(matching_grant()),
    )
    .unwrap();
    let mut value = serde_json::to_value(&resolution).unwrap();
    value["decision"] = json!({"type":"deny"});
    assert!(serde_json::from_value::<ApprovalResolution>(value).is_err());
}

#[test]
fn denial_produces_model_visible_machine_readable_failure() {
    let resolution = ApprovalResolution::new(
        &request(),
        ApprovalDecision::Deny,
        common::timestamp("2026-07-23T10:02:00.000Z"),
        None,
    )
    .unwrap();
    let failure = resolution.denial_tool_failure().unwrap();
    assert_eq!(failure.code(), "approval_denied");
    assert_eq!(failure.message(), "tool invocation was denied by approval");
}
