use crate::common;

use std::fmt;

use serde_json::json;
use tea_policy::{
    ApprovalRequirement, PolicyDecision, PolicyEngine, PolicyExecutionTarget, PolicyLayer,
    PolicyRule, PolicyRuleDecision, PolicyRuleError,
};
use tea_tools::{ToolEffect, ToolResourceAccess};

#[derive(Debug)]
struct FixedRule {
    id: &'static str,
    layer: PolicyLayer,
    decision: PolicyRuleDecision,
}

impl PolicyRule for FixedRule {
    fn id(&self) -> &str {
        self.id
    }
    fn layer(&self) -> PolicyLayer {
        self.layer
    }
    fn evaluate(&self, _input: &tea_policy::PolicyInput) -> PolicyRuleDecision {
        self.decision.clone()
    }
}

fn input() -> tea_policy::PolicyInput {
    let invocation = common::validated_invocation(
        "write_file",
        vec![ToolEffect::FsWrite],
        vec![common::file_resource(
            "/workspace/a",
            ToolResourceAccess::Write,
        )],
        json!({"path":"/workspace/a"}),
    );
    common::input_with(
        &invocation,
        vec![],
        common::timestamp("2026-07-23T10:00:00.000Z"),
    )
}

#[test]
fn lower_layers_can_narrow_but_never_broaden() {
    let mut engine = PolicyEngine::new();
    engine
        .add_rule(FixedRule {
            id: "platform-allow",
            layer: PolicyLayer::Platform,
            decision: PolicyRuleDecision::allow(),
        })
        .unwrap();
    engine
        .add_rule(FixedRule {
            id: "product-ask",
            layer: PolicyLayer::Product,
            decision: PolicyRuleDecision::ask(
                ApprovalRequirement::new("mutation requires approval").unwrap(),
            ),
        })
        .unwrap();
    engine
        .add_rule(FixedRule {
            id: "workspace-allow",
            layer: PolicyLayer::Workspace,
            decision: PolicyRuleDecision::allow(),
        })
        .unwrap();
    let evaluation = engine.evaluate(&input());
    assert!(matches!(evaluation.decision(), PolicyDecision::Ask(_)));
    assert_eq!(evaluation.trace().len(), 3);
    assert_eq!(evaluation.trace()[0].layer(), PolicyLayer::Platform);
    assert_eq!(evaluation.trace()[1].layer(), PolicyLayer::Product);
    assert_eq!(evaluation.trace()[2].layer(), PolicyLayer::Workspace);
}

#[test]
fn hard_deny_is_terminal_and_later_rule_is_not_evaluated() {
    let mut engine = PolicyEngine::new();
    engine
        .add_rule(FixedRule {
            id: "platform-deny",
            layer: PolicyLayer::Platform,
            decision: PolicyRuleDecision::hard_deny("platform denied").unwrap(),
        })
        .unwrap();
    engine
        .add_rule(FixedRule {
            id: "workspace-allow",
            layer: PolicyLayer::Workspace,
            decision: PolicyRuleDecision::allow(),
        })
        .unwrap();
    let evaluation = engine.evaluate(&input());
    assert!(matches!(
        evaluation.decision(),
        PolicyDecision::HardDeny { .. }
    ));
    assert_eq!(evaluation.trace().len(), 1);
}

#[test]
fn redirect_is_preserved_against_later_allow_but_narrowed_by_ask() {
    let mut engine = PolicyEngine::new();
    engine
        .add_rule(FixedRule {
            id: "platform-redirect",
            layer: PolicyLayer::Platform,
            decision: PolicyRuleDecision::redirect(PolicyExecutionTarget::Sandbox),
        })
        .unwrap();
    engine
        .add_rule(FixedRule {
            id: "product-allow",
            layer: PolicyLayer::Product,
            decision: PolicyRuleDecision::allow(),
        })
        .unwrap();
    assert!(matches!(
        engine.evaluate(&input()).decision(),
        PolicyDecision::Redirect {
            target: PolicyExecutionTarget::Sandbox
        }
    ));

    engine
        .add_rule(FixedRule {
            id: "workspace-ask",
            layer: PolicyLayer::Workspace,
            decision: PolicyRuleDecision::ask(
                ApprovalRequirement::new("confirm sandbox mutation").unwrap(),
            ),
        })
        .unwrap();
    assert!(matches!(
        engine.evaluate(&input()).decision(),
        PolicyDecision::Ask(_)
    ));
}

#[test]
fn empty_or_abstaining_engine_fails_closed() {
    let engine = PolicyEngine::new();
    assert!(matches!(
        engine.evaluate(&input()).decision(),
        PolicyDecision::HardDeny { .. }
    ));

    let mut engine = PolicyEngine::new();
    engine
        .add_rule(FixedRule {
            id: "nothing",
            layer: PolicyLayer::Platform,
            decision: PolicyRuleDecision::abstain(),
        })
        .unwrap();
    assert!(matches!(
        engine.evaluate(&input()).decision(),
        PolicyDecision::HardDeny { .. }
    ));
}

#[test]
fn rule_ids_and_diagnostics_are_bounded() {
    let mut engine = PolicyEngine::new();
    assert_eq!(
        engine
            .add_rule(FixedRule {
                id: "Bad Rule",
                layer: PolicyLayer::Platform,
                decision: PolicyRuleDecision::allow()
            })
            .unwrap_err(),
        PolicyRuleError::InvalidRuleId
    );
    assert!(ApprovalRequirement::new("").is_err());
    assert!(PolicyRuleDecision::deny("x".repeat(4097)).is_err());
}

impl fmt::Display for FixedRule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.id)
    }
}
