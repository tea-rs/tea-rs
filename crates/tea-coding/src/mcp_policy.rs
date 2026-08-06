use tea_policy::{ApprovalRequirement, PolicyInput, PolicyLayer, PolicyRule, PolicyRuleDecision};
use tea_tools::{ToolSourceKind, ToolTrust};

/// Coding product policy for explicitly configured MCP sources.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CodingMcpPolicy;

impl PolicyRule for CodingMcpPolicy {
    fn id(&self) -> &'static str {
        "product.coding_mcp"
    }

    fn layer(&self) -> PolicyLayer {
        PolicyLayer::Product
    }

    fn evaluate(&self, input: &PolicyInput) -> PolicyRuleDecision {
        if input.tool_source().kind() != ToolSourceKind::Mcp {
            return PolicyRuleDecision::abstain();
        }
        decision_for_mcp_trust(input.tool_source().trust())
    }
}

fn decision_for_mcp_trust(trust: ToolTrust) -> PolicyRuleDecision {
    match trust {
        ToolTrust::Untrusted => hard_deny("untrusted MCP sources are denied by default"),
        ToolTrust::User | ToolTrust::Workspace => {
            ask("configured MCP source requires explicit approval")
        }
        ToolTrust::Product => PolicyRuleDecision::abstain(),
    }
}

fn ask(reason: &str) -> PolicyRuleDecision {
    ApprovalRequirement::new(reason)
        .map_or_else(|_| PolicyRuleDecision::abstain(), PolicyRuleDecision::ask)
}

fn hard_deny(reason: &str) -> PolicyRuleDecision {
    PolicyRuleDecision::hard_deny(reason).unwrap_or_else(|_| PolicyRuleDecision::abstain())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::Arc;

    use serde_json::json;
    use tea_policy::{
        ActorId, CodingWorkspacePolicy, ExecutionSurface, ExternalSourcePolicy, PolicyDecision,
        PolicyEngine, PolicyEnvironment, PolicyExecutionTarget, PolicyInput, UnknownEffectPolicy,
        WorkspaceId,
    };
    use tea_protocol::{
        ProfileId, ProtocolMetadata, ProtocolTimestamp, RunId, SessionId, ToolCallId,
        ToolIdempotency,
    };
    use tea_testkit::FakeReadTool;
    use tea_tools::{
        StaticResourceResolver, ToolConcurrency, ToolEffect, ToolExecutionSemantics,
        ToolInvocation, ToolName, ToolRegistry, ToolResource, ToolResourceAccess, ToolRetrySafety,
        ToolSource, ToolSourceKind, ToolSpec, ToolTimeout, ToolTrust, ToolVersion,
    };

    use super::{CodingMcpPolicy, decision_for_mcp_trust};

    fn input(trust: ToolTrust, effects: Vec<ToolEffect>) -> PolicyInput {
        let source = ToolSource::new(
            ToolSourceKind::Mcp,
            "mcp.workspace.files",
            trust,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();
        let mut tools = ToolRegistry::new();
        tools
            .register(
                ToolSpec::new(
                    ToolName::from_str("mcp_read").unwrap(),
                    ToolVersion::from_str("1.0.0").unwrap(),
                    "Read a file through MCP.",
                    json!({"type":"object"}),
                    json!({"type":"object"}),
                    effects,
                    ToolExecutionSemantics::new(
                        ToolIdempotency::Idempotent,
                        ToolRetrySafety::ExplicitOnly,
                        ToolConcurrency::Serial,
                        ToolTimeout::from_millis(1_000).unwrap(),
                    )
                    .unwrap(),
                )
                .unwrap()
                .with_source(source),
                Arc::new(
                    StaticResourceResolver::new([ToolResource::new(
                        "mcp-server",
                        "workspace.files/read",
                        ToolResourceAccess::Execute,
                    )
                    .unwrap()])
                    .unwrap(),
                ),
                Arc::new(FakeReadTool::new([(
                    "/unused".to_owned(),
                    "unused".to_owned(),
                )])),
            )
            .unwrap();
        let invocation = tools
            .validate(
                ToolInvocation::new(
                    ToolCallId::from_str("0195a0b1-5e45-75be-8284-0aa7aa000011").unwrap(),
                    ToolName::from_str("mcp_read").unwrap(),
                    json!({}),
                    ProtocolMetadata::default(),
                )
                .unwrap(),
            )
            .unwrap();
        PolicyInput::from_validated(
            ActorId::from_str("user:alice").unwrap(),
            ProfileId::from_str("coding-agent").unwrap(),
            SessionId::from_str("0195a0b1-5e3a-7d72-a902-c4e85d828bf1").unwrap(),
            Some(RunId::from_str("0195a0b1-5e3b-7ef0-8ec1-0aa7aa000001").unwrap()),
            Some(WorkspaceId::from_str("workspace/main").unwrap()),
            &invocation,
            PolicyEnvironment::new(
                ExecutionSurface::Test,
                PolicyExecutionTarget::Mcp,
                ProtocolMetadata::default(),
            ),
            ProtocolTimestamp::from_str("2026-07-25T10:00:00.000Z").unwrap(),
            [],
        )
        .unwrap()
    }

    fn evaluate(trust: ToolTrust, effects: Vec<ToolEffect>) -> PolicyDecision {
        let mut engine = PolicyEngine::new();
        engine.add_rule(UnknownEffectPolicy).unwrap();
        engine.add_rule(ExternalSourcePolicy).unwrap();
        engine.add_rule(CodingWorkspacePolicy).unwrap();
        engine.add_rule(CodingMcpPolicy).unwrap();
        engine.evaluate(&input(trust, effects)).decision().clone()
    }

    #[test]
    fn user_and_workspace_mcp_sources_require_approval() {
        for trust in [ToolTrust::User, ToolTrust::Workspace] {
            assert!(matches!(
                decision_for_mcp_trust(trust).decision(),
                Some(PolicyDecision::Ask(_))
            ));
        }
        assert!(
            decision_for_mcp_trust(ToolTrust::Product)
                .decision()
                .is_none()
        );
        assert!(matches!(
            decision_for_mcp_trust(ToolTrust::Untrusted).decision(),
            Some(PolicyDecision::HardDeny { .. })
        ));
    }

    #[test]
    fn coding_mcp_rule_requests_approval_without_weakening_hard_denies() {
        for trust in [ToolTrust::User, ToolTrust::Workspace] {
            assert!(matches!(
                evaluate(trust, vec![ToolEffect::FsRead]),
                PolicyDecision::Ask(_)
            ));
        }
        assert_eq!(
            evaluate(ToolTrust::Product, vec![ToolEffect::FsRead]),
            PolicyDecision::Allow
        );
        assert!(matches!(
            evaluate(ToolTrust::Workspace, vec![ToolEffect::CredentialRead]),
            PolicyDecision::HardDeny { .. }
        ));
        assert!(matches!(
            evaluate(
                ToolTrust::Workspace,
                vec![ToolEffect::from_str("com.example.future.effect").unwrap()],
            ),
            PolicyDecision::HardDeny { .. }
        ));
    }
}
