use std::str::FromStr;
use std::time::Duration;

use tea_policy::{ExecutionSurface, PolicyEnvironment, PolicyExecutionTarget};
use tea_profile::{AgentProfile, ProfileOverlay, ProfileRuleId, ProfileRunLimits};
use tea_protocol::{ModelRef, ProtocolMetadata};
use tea_tools::ToolName;

fn base() -> AgentProfile {
    AgentProfile::coding_agent().unwrap()
}

fn model_ref() -> ModelRef {
    ModelRef::new("fake".parse().unwrap(), "fake/model".parse().unwrap())
}

#[test]
fn overlay_none_inherits_base_wholesale() {
    let profile = base().compose(&ProfileOverlay::new()).unwrap();
    assert_eq!(profile, base());
}

#[test]
fn overlay_replaces_run_limits_wholesale() {
    let overlay = ProfileOverlay::new()
        .run_limits(ProfileRunLimits::new(1, Duration::from_secs(10), 1024, 10, 1).unwrap());
    let profile = base().compose(&overlay).unwrap();
    assert_eq!(profile.run_limits().max_tool_iterations(), 1);
    // Other limit fields come from the overlay, not merged from base.
    assert_eq!(profile.run_limits().max_assistant_output_bytes(), 1024);
}

#[test]
fn overlay_replaces_active_tools_wholesale() {
    let overlay = ProfileOverlay::new()
        .active_tool_names(vec![ToolName::from_str("clipboard_read").unwrap()]);
    let profile = base().compose(&overlay).unwrap();
    assert_eq!(
        profile
            .active_tool_names()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["clipboard_read"]
    );
}

#[test]
fn overlay_can_clear_description() {
    let profile = AgentProfile::builder(
        "coding-agent".parse().unwrap(),
        "Coding Agent".parse().unwrap(),
        model_ref(),
    )
    .description("Original".parse().unwrap())
    .active_tool(ToolName::from_str("read_file").unwrap())
    .policy_rule(ProfileRuleId::from_str("product.coding_workspace").unwrap())
    .environment(PolicyEnvironment::new(
        ExecutionSurface::Cli,
        PolicyExecutionTarget::Native,
        ProtocolMetadata::default(),
    ))
    .build()
    .unwrap();
    let cleared = profile
        .compose(&ProfileOverlay::new().description(None))
        .unwrap();
    assert!(cleared.description().is_none());
    let inherited = profile.compose(&ProfileOverlay::new()).unwrap();
    assert!(inherited.description().is_some());
}

#[test]
fn composition_rejects_schema_version_mismatch() {
    let mut overlay = ProfileOverlay::new();
    overlay = overlay.schema_version(tea_profile::ProfileSchemaVersion::new(2, 0, 0));
    let err = base().compose(&overlay).unwrap_err();
    assert_eq!(
        err.code(),
        tea_profile::ProfileErrorCode::CompositionConflict
    );
}

#[test]
fn composition_rejects_profile_id_mismatch() {
    let overlay = ProfileOverlay::new().profile_id("other-profile".parse().unwrap());
    let err = base().compose(&overlay).unwrap_err();
    assert_eq!(
        err.code(),
        tea_profile::ProfileErrorCode::CompositionConflict
    );
}

#[test]
fn composition_revalidates_overlay_values() {
    let overlay = ProfileOverlay::new().approval_ttl(Duration::ZERO);
    let err = base().compose(&overlay).unwrap_err();
    assert_eq!(err.code(), tea_profile::ProfileErrorCode::UnsupportedValue);
}

#[test]
fn example_profiles_are_distinct() {
    let minimal = AgentProfile::minimal_assistant().unwrap();
    let coding = AgentProfile::coding_agent().unwrap();
    let desktop = AgentProfile::desktop_assistant().unwrap();

    assert_eq!(minimal.profile_id().as_str(), "minimal-assistant");
    assert_eq!(coding.profile_id().as_str(), "coding-agent");
    assert_eq!(desktop.profile_id().as_str(), "desktop-assistant");

    // Distinct active tools.
    assert!(minimal.active_tool_names().is_empty());
    assert_ne!(coding.active_tool_names(), desktop.active_tool_names());

    // Distinct policy rule references.
    assert!(minimal.policy_rule_ids().is_empty());
    assert_ne!(coding.policy_rule_ids(), desktop.policy_rule_ids());

    // Distinct environment surfaces.
    assert_eq!(coding.environment().surface(), ExecutionSurface::Cli);
    assert_eq!(desktop.environment().surface(), ExecutionSurface::Desktop);

    // Distinct run limits.
    assert_ne!(
        coding.run_limits().max_tool_iterations(),
        desktop.run_limits().max_tool_iterations()
    );

    // All three share the same model selector.
    assert_eq!(minimal.model_id(), coding.model_id());
    assert_eq!(coding.model_id(), desktop.model_id());
}

#[test]
fn example_profiles_round_trip() {
    for profile in [
        AgentProfile::minimal_assistant().unwrap(),
        AgentProfile::coding_agent().unwrap(),
        AgentProfile::desktop_assistant().unwrap(),
    ] {
        let json = serde_json::to_string(&profile).unwrap();
        let reparsed: AgentProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(profile, reparsed);
    }
}
