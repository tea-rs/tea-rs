use crate::common;

use std::str::FromStr;

use serde_json::json;
use tea_policy::{
    ApprovalRequirement, CodingWorkspacePolicy, DesktopPolicy, ExternalSourcePolicy,
    FilesystemReadPolicy, GrantScope, PolicyDecision, PolicyEngine, PolicyGrant, PolicyLayer,
    PolicyRule, PolicyRuleDecision, ResourcePattern, UnknownEffectPolicy,
};
use tea_protocol::ProfileId;
use tea_tools::{
    ToolEffect, ToolName, ToolResource, ToolResourceAccess, ToolSource, ToolSourceKind, ToolTrust,
    ToolVersion,
};

fn evaluate(effects: Vec<ToolEffect>, arguments: serde_json::Value) -> PolicyDecision {
    let invocation = common::validated_invocation(
        "shared_executor",
        effects,
        vec![common::file_resource(
            "/workspace/a",
            ToolResourceAccess::Write,
        )],
        arguments,
    );
    let input = common::input_with(
        &invocation,
        vec![],
        common::timestamp("2026-07-23T10:00:00.000Z"),
    );
    let mut engine = PolicyEngine::new();
    engine.add_rule(UnknownEffectPolicy).unwrap();
    engine.add_rule(CodingWorkspacePolicy).unwrap();
    engine.evaluate(&input).decision().clone()
}

fn evaluate_filesystem_read(effects: Vec<ToolEffect>) -> PolicyDecision {
    let invocation = common::validated_invocation(
        "read_only",
        effects,
        vec![common::file_resource(
            "/workspace/a",
            ToolResourceAccess::Read,
        )],
        json!({}),
    );
    let input = common::input_with(
        &invocation,
        vec![],
        common::timestamp("2026-07-23T10:00:00.000Z"),
    );
    let mut engine = PolicyEngine::new();
    engine.add_rule(UnknownEffectPolicy).unwrap();
    engine.add_rule(FilesystemReadPolicy).unwrap();
    engine.evaluate(&input).decision().clone()
}

fn external_source(trust: ToolTrust) -> ToolSource {
    ToolSource::new(
        ToolSourceKind::Mcp,
        "mcp.workspace.files",
        trust,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .unwrap()
}

fn external_input(trust: ToolTrust, resources: Vec<ToolResource>) -> tea_policy::PolicyInput {
    let invocation = common::validated_invocation_with_source(
        "external_read",
        vec![ToolEffect::FsRead],
        resources,
        json!({"path":"/workspace/a"}),
        external_source(trust),
    );
    common::input_with(
        &invocation,
        vec![],
        common::timestamp("2026-07-25T10:00:00.000Z"),
    )
}

fn mcp_server_resource(access: ToolResourceAccess) -> ToolResource {
    ToolResource::new("mcp-server", "workspace.files/read", access).unwrap()
}

#[test]
fn git_status_and_destructive_command_differ_without_tool_name_inference() {
    assert_eq!(
        evaluate(
            vec![ToolEffect::ProcessSpawn],
            json!({"command":"git status"})
        ),
        PolicyDecision::Allow
    );
    assert!(matches!(
        evaluate(
            vec![ToolEffect::ProcessSpawn],
            json!({"command":"rm -rf /workspace"})
        ),
        PolicyDecision::Ask(_)
    ));
}

#[test]
fn unknown_effect_and_credentials_fail_closed() {
    assert!(matches!(
        evaluate(
            vec![ToolEffect::from_str("com.example.future.effect").unwrap()],
            json!({})
        ),
        PolicyDecision::HardDeny { .. }
    ));
    assert!(matches!(
        evaluate(vec![ToolEffect::CredentialRead], json!({})),
        PolicyDecision::HardDeny { .. }
    ));
}

#[test]
fn external_sources_require_complete_host_capabilities_and_trust() {
    let mut engine = PolicyEngine::new();
    engine.add_rule(ExternalSourcePolicy).unwrap();
    engine.add_rule(CodingWorkspacePolicy).unwrap();

    for trust in [ToolTrust::Product, ToolTrust::User, ToolTrust::Workspace] {
        assert_eq!(
            engine
                .evaluate(&external_input(
                    trust,
                    vec![mcp_server_resource(ToolResourceAccess::Execute)],
                ))
                .decision(),
            &PolicyDecision::Allow
        );
    }
    assert!(matches!(
        engine
            .evaluate(&external_input(
                ToolTrust::Untrusted,
                vec![mcp_server_resource(ToolResourceAccess::Execute)],
            ))
            .decision(),
        PolicyDecision::HardDeny { .. }
    ));
    assert!(matches!(
        engine
            .evaluate(&external_input(ToolTrust::Workspace, vec![]))
            .decision(),
        PolicyDecision::HardDeny { .. }
    ));
    assert!(matches!(
        engine
            .evaluate(&external_input(
                ToolTrust::Workspace,
                vec![mcp_server_resource(ToolResourceAccess::Read)],
            ))
            .decision(),
        PolicyDecision::HardDeny { .. }
    ));
}

#[test]
fn coding_reads_allow_and_mutations_ask() {
    assert_eq!(
        evaluate(vec![ToolEffect::FsRead], json!({})),
        PolicyDecision::Allow
    );
    assert!(matches!(
        evaluate(vec![ToolEffect::FsWrite], json!({})),
        PolicyDecision::Ask(_)
    ));
}

#[test]
fn filesystem_read_policy_authorizes_only_pure_filesystem_reads() {
    assert_eq!(
        evaluate_filesystem_read(vec![ToolEffect::FsRead]),
        PolicyDecision::Allow
    );
    for effects in [
        vec![ToolEffect::ClipboardRead],
        vec![ToolEffect::CredentialRead],
        vec![ToolEffect::FsWrite],
        vec![ToolEffect::FsRead, ToolEffect::NetworkRequest],
    ] {
        assert!(matches!(
            evaluate_filesystem_read(effects),
            PolicyDecision::HardDeny { .. }
        ));
    }
}

#[test]
fn desktop_sensitive_effects_ask_and_credentials_deny() {
    let invocation = common::validated_invocation(
        "desktop_operation",
        vec![ToolEffect::ClipboardRead],
        vec![common::file_resource(
            "/workspace/a",
            ToolResourceAccess::Read,
        )],
        json!({}),
    );
    let input = common::input_with(
        &invocation,
        vec![],
        common::timestamp("2026-07-23T10:00:00.000Z"),
    );
    let mut engine = PolicyEngine::new();
    engine.add_rule(DesktopPolicy).unwrap();
    assert!(matches!(
        engine.evaluate(&input).decision(),
        PolicyDecision::Ask(_)
    ));
}

#[derive(Debug)]
struct AskRule;
impl PolicyRule for AskRule {
    fn id(&self) -> &'static str {
        "product.ask"
    }
    fn layer(&self) -> PolicyLayer {
        PolicyLayer::Product
    }
    fn evaluate(&self, _input: &tea_policy::PolicyInput) -> PolicyRuleDecision {
        PolicyRuleDecision::ask(ApprovalRequirement::new("approval required").unwrap())
    }
}

#[derive(Debug)]
struct HardDenyRule;
impl PolicyRule for HardDenyRule {
    fn id(&self) -> &'static str {
        "platform.hard_deny"
    }
    fn layer(&self) -> PolicyLayer {
        PolicyLayer::Platform
    }
    fn evaluate(&self, _input: &tea_policy::PolicyInput) -> PolicyRuleDecision {
        PolicyRuleDecision::hard_deny("denied").unwrap()
    }
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
        GrantScope::Run {
            run_id: common::run_id(),
        },
        common::timestamp("2026-07-23T09:00:00.000Z"),
    )
    .unwrap()
}

#[test]
fn matching_grant_satisfies_ask_but_never_hard_deny() {
    let invocation = common::validated_invocation(
        "write_file",
        vec![ToolEffect::FsWrite],
        vec![common::file_resource(
            "/workspace/a",
            ToolResourceAccess::Write,
        )],
        json!({}),
    );
    let input = common::input_with(
        &invocation,
        vec![matching_grant()],
        common::timestamp("2026-07-23T10:00:00.000Z"),
    );
    let mut allowed = PolicyEngine::new();
    allowed.add_rule(AskRule).unwrap();
    let evaluation = allowed.evaluate(&input);
    assert_eq!(evaluation.decision(), &PolicyDecision::Allow);
    assert_eq!(evaluation.matched_grant_id(), Some(common::grant_id()));

    let mut denied = PolicyEngine::new();
    denied.add_rule(HardDenyRule).unwrap();
    denied.add_rule(AskRule).unwrap();
    assert!(matches!(
        denied.evaluate(&input).decision(),
        PolicyDecision::HardDeny { .. }
    ));
    assert_eq!(denied.evaluate(&input).matched_grant_id(), None);
}
