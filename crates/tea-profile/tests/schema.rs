use std::str::FromStr;
use std::time::Duration;

use tea_policy::{ExecutionSurface, PolicyEnvironment, PolicyExecutionTarget};
use tea_profile::{
    AgentProfile, ProfileDisplayName, ProfileErrorCode, ProfilePromptBudget, ProfileRuleId,
    ProfileRunLimits, ProfileSegmentId, ProfileTrustLevel, ProfileWorkspaceInstruction,
};
use tea_protocol::{ModelRef, ProtocolMetadata};
use tea_tools::ToolName;

fn environment() -> PolicyEnvironment {
    PolicyEnvironment::new(
        ExecutionSurface::Cli,
        PolicyExecutionTarget::Native,
        ProtocolMetadata::default(),
    )
}

fn model_ref() -> ModelRef {
    ModelRef::new("fake".parse().unwrap(), "fake/model".parse().unwrap())
}

fn minimal_profile() -> AgentProfile {
    AgentProfile::builder(
        "minimal-assistant".parse().unwrap(),
        "Minimal Assistant".parse().unwrap(),
        model_ref(),
    )
    .environment(environment())
    .build()
    .unwrap()
}

fn coding_profile() -> AgentProfile {
    AgentProfile::builder(
        "coding-agent".parse().unwrap(),
        "Coding Agent".parse().unwrap(),
        model_ref(),
    )
    .active_tool(ToolName::from_str("read_file").unwrap())
    .active_tool(ToolName::from_str("write_file").unwrap())
    .policy_rule(ProfileRuleId::from_str("product.coding_workspace").unwrap())
    .prompt_budget(ProfilePromptBudget::new(16_384, 4_096).unwrap())
    .run_limits(ProfileRunLimits::new(8, Duration::from_mins(2), 1024, 1_000, 16).unwrap())
    .environment(environment())
    .approval_ttl(Duration::from_mins(5))
    .build()
    .unwrap()
}

#[test]
fn minimal_profile_has_no_tools_or_rules() {
    let profile = minimal_profile();
    assert_eq!(profile.profile_id().as_str(), "minimal-assistant");
    assert!(profile.active_tool_names().is_empty());
    assert!(profile.policy_rule_ids().is_empty());
    assert!(profile.workspace_instructions().is_empty());
    assert_eq!(profile.approval_ttl(), Duration::from_mins(10));
}

#[test]
fn coding_profile_round_trips_byte_identical() {
    let profile = coding_profile();
    let json = serde_json::to_string(&profile).unwrap();
    let reparsed: AgentProfile = serde_json::from_str(&json).unwrap();
    assert_eq!(profile, reparsed);
    assert_eq!(
        profile
            .active_tool_names()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["read_file", "write_file"]
    );
}

#[test]
fn rejects_empty_display_name() {
    assert!(ProfileDisplayName::new("").is_err());
    let oversized = "x".repeat(257);
    assert!(ProfileDisplayName::new(oversized).is_err());
}

#[test]
fn rejects_duplicate_active_tools() {
    let err = AgentProfile::builder(
        "coding-agent".parse().unwrap(),
        "Coding Agent".parse().unwrap(),
        model_ref(),
    )
    .active_tool(ToolName::from_str("read_file").unwrap())
    .active_tool(ToolName::from_str("read_file").unwrap())
    .environment(environment())
    .build()
    .unwrap_err();
    assert_eq!(err.code(), ProfileErrorCode::DuplicateEntry);
}

#[test]
fn rejects_duplicate_policy_rules() {
    let err = AgentProfile::builder(
        "coding-agent".parse().unwrap(),
        "Coding Agent".parse().unwrap(),
        model_ref(),
    )
    .policy_rule(ProfileRuleId::from_str("product.coding_workspace").unwrap())
    .policy_rule(ProfileRuleId::from_str("product.coding_workspace").unwrap())
    .environment(environment())
    .build()
    .unwrap_err();
    assert_eq!(err.code(), ProfileErrorCode::DuplicateEntry);
}

#[test]
fn rejects_unset_environment_and_bad_approval_ttl() {
    let err = AgentProfile::builder(
        "minimal-assistant".parse().unwrap(),
        "Minimal Assistant".parse().unwrap(),
        model_ref(),
    )
    .build()
    .unwrap_err();
    assert_eq!(err.code(), ProfileErrorCode::InvalidSelector);

    let err = AgentProfile::builder(
        "minimal-assistant".parse().unwrap(),
        "Minimal Assistant".parse().unwrap(),
        model_ref(),
    )
    .environment(environment())
    .approval_ttl(Duration::ZERO)
    .build()
    .unwrap_err();
    assert_eq!(err.code(), ProfileErrorCode::UnsupportedValue);
}

#[test]
fn rejects_invalid_run_limits_and_budget() {
    let err = ProfileRunLimits::new(0, Duration::from_secs(1), 1, 1, 1).unwrap_err();
    assert_eq!(err.code(), ProfileErrorCode::UnsupportedValue);
    let err = ProfilePromptBudget::new(0, 1).unwrap_err();
    assert_eq!(err.code(), ProfileErrorCode::UnsupportedValue);
}

#[test]
fn workspace_instructions_round_trip_and_validate() {
    let segment = ProfileSegmentId::from_str("workspace.notes").unwrap();
    let instruction = ProfileWorkspaceInstruction::new(
        segment,
        "Keep answers brief.",
        "file:///workspace/AGENTS.md",
        ProfileTrustLevel::Trusted,
    )
    .unwrap();
    let profile = AgentProfile::builder(
        "coding-agent".parse().unwrap(),
        "Coding Agent".parse().unwrap(),
        model_ref(),
    )
    .active_tool(ToolName::from_str("read_file").unwrap())
    .policy_rule(ProfileRuleId::from_str("product.coding_workspace").unwrap())
    .environment(environment())
    .workspace_instruction(instruction)
    .build()
    .unwrap();
    assert_eq!(profile.workspace_instructions().len(), 1);
    assert_eq!(
        profile.workspace_instructions()[0].locator(),
        "file:///workspace/AGENTS.md"
    );
    let json = serde_json::to_string(&profile).unwrap();
    let reparsed: AgentProfile = serde_json::from_str(&json).unwrap();
    assert_eq!(profile, reparsed);
}

#[test]
fn rejects_duplicate_workspace_segment_ids() {
    let segment = ProfileSegmentId::from_str("workspace.notes").unwrap();
    let err = AgentProfile::builder(
        "coding-agent".parse().unwrap(),
        "Coding Agent".parse().unwrap(),
        model_ref(),
    )
    .environment(environment())
    .workspace_instruction(
        ProfileWorkspaceInstruction::new(
            segment.clone(),
            "one",
            "file:///a",
            ProfileTrustLevel::Trusted,
        )
        .unwrap(),
    )
    .workspace_instruction(
        ProfileWorkspaceInstruction::new(segment, "two", "file:///b", ProfileTrustLevel::Trusted)
            .unwrap(),
    )
    .build()
    .unwrap_err();
    assert_eq!(err.code(), ProfileErrorCode::DuplicateEntry);
}

#[test]
fn rejects_unknown_json_fields() {
    let json = minimal_json();
    let mut value = serde_json::from_str::<serde_json::Value>(&json).unwrap();
    value["extraField"] = serde_json::Value::String("unknown".into());
    let err = serde_json::from_value::<AgentProfile>(value).unwrap_err();
    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn rejects_unsupported_schema_version_on_decode() {
    let json = minimal_json();
    let mut value = serde_json::from_str::<serde_json::Value>(&json).unwrap();
    value["schemaVersion"] = serde_json::Value::String("2.0.0".into());
    let err = serde_json::from_value::<AgentProfile>(value).unwrap_err();
    assert!(
        err.to_string()
            .contains("unsupported profile schema version")
    );
}

fn minimal_json() -> String {
    let profile = minimal_profile();
    serde_json::to_string(&profile).unwrap()
}
