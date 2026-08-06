use crate::common;

use std::str::FromStr;

use serde_json::json;
use tea_policy::{GrantScope, PolicyGrant, PolicyGrantError, ResourcePattern};
use tea_protocol::ProfileId;
use tea_tools::{
    ToolEffect, ToolName, ToolResourceAccess, ToolSource, ToolSourceKind, ToolTrust, ToolVersion,
};

const DIGEST_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn mcp_source(digest: &str) -> ToolSource {
    ToolSource::new(
        ToolSourceKind::Mcp,
        "workspace.files",
        ToolTrust::Workspace,
        digest,
    )
    .unwrap()
}

fn grant(scope: GrantScope) -> PolicyGrant {
    PolicyGrant::new(
        common::grant_id(),
        "user:alice".parse().unwrap(),
        ProfileId::from_str("coding").unwrap(),
        ToolName::from_str("write_file").unwrap(),
        ToolVersion::from_str("1.0.0").unwrap(),
        [ToolEffect::FsWrite],
        [ResourcePattern::new("file", "/workspace/", Some(ToolResourceAccess::Write)).unwrap()],
        scope,
        common::timestamp("2026-07-23T09:00:00.000Z"),
    )
    .unwrap()
}

fn input_with(grant: PolicyGrant, now: &str) -> tea_policy::PolicyInput {
    let invocation = common::validated_invocation(
        "write_file",
        vec![ToolEffect::FsWrite],
        vec![common::file_resource(
            "/workspace/notes.txt",
            ToolResourceAccess::Write,
        )],
        json!({"path":"/workspace/notes.txt"}),
    );
    common::input_with(&invocation, vec![grant], common::timestamp(now))
}

#[test]
fn once_run_session_and_persistent_scopes_match_exact_context() {
    let once = grant(GrantScope::Once {
        tool_call_id: common::tool_call_id(),
    });
    assert!(once.matches(&input_with(once.clone(), "2026-07-23T10:00:00.000Z")));

    let run = grant(GrantScope::Run {
        run_id: common::run_id(),
    });
    assert!(run.matches(&input_with(run.clone(), "2026-07-23T10:00:00.000Z")));

    let session = grant(GrantScope::SessionResource {
        session_id: common::session_id(),
    });
    assert!(session.matches(&input_with(session.clone(), "2026-07-23T10:00:00.000Z")));

    let persistent = grant(GrantScope::PersistentResource {
        expires_at: common::timestamp("2026-07-23T11:00:00.000Z"),
    });
    assert!(persistent.matches(&input_with(persistent.clone(), "2026-07-23T10:59:59.999Z")));
    assert!(!persistent.matches(&input_with(persistent.clone(), "2026-07-23T11:00:00.000Z")));
}

#[test]
fn resource_effect_actor_profile_tool_and_version_mismatches_fail_closed() {
    let grant = grant(GrantScope::Run {
        run_id: common::run_id(),
    });
    let wrong_resource = common::validated_invocation(
        "write_file",
        vec![ToolEffect::FsWrite],
        vec![common::file_resource(
            "/outside/notes.txt",
            ToolResourceAccess::Write,
        )],
        json!({}),
    );
    let input = common::input_with(
        &wrong_resource,
        vec![grant.clone()],
        common::timestamp("2026-07-23T10:00:00.000Z"),
    );
    assert!(!grant.matches(&input));

    let wrong_effect = common::validated_invocation(
        "write_file",
        vec![ToolEffect::FsDelete],
        vec![common::file_resource(
            "/workspace/notes.txt",
            ToolResourceAccess::Write,
        )],
        json!({}),
    );
    let input = common::input_with(
        &wrong_effect,
        vec![grant.clone()],
        common::timestamp("2026-07-23T10:00:00.000Z"),
    );
    assert!(!grant.matches(&input));
}

#[test]
fn actor_profile_tool_version_and_scope_mismatches_fail_closed() {
    let base_input = input_with(
        grant(GrantScope::Run {
            run_id: common::run_id(),
        }),
        "2026-07-23T10:00:00.000Z",
    );
    let make = |actor: &str, profile: &str, tool: &str, version: &str, scope: GrantScope| {
        PolicyGrant::new(
            common::grant_id(),
            actor.parse().unwrap(),
            ProfileId::from_str(profile).unwrap(),
            ToolName::from_str(tool).unwrap(),
            ToolVersion::from_str(version).unwrap(),
            [ToolEffect::FsWrite],
            [
                ResourcePattern::new("file", "/workspace/", Some(ToolResourceAccess::Write))
                    .unwrap(),
            ],
            scope,
            common::timestamp("2026-07-23T09:00:00.000Z"),
        )
        .unwrap()
    };
    assert!(
        !make(
            "user:bob",
            "coding",
            "write_file",
            "1.0.0",
            GrantScope::Run {
                run_id: common::run_id()
            }
        )
        .matches(&base_input)
    );
    assert!(
        !make(
            "user:alice",
            "desktop",
            "write_file",
            "1.0.0",
            GrantScope::Run {
                run_id: common::run_id()
            }
        )
        .matches(&base_input)
    );
    assert!(
        !make(
            "user:alice",
            "coding",
            "other_tool",
            "1.0.0",
            GrantScope::Run {
                run_id: common::run_id()
            }
        )
        .matches(&base_input)
    );
    assert!(
        !make(
            "user:alice",
            "coding",
            "write_file",
            "2.0.0",
            GrantScope::Run {
                run_id: common::run_id()
            }
        )
        .matches(&base_input)
    );
    let other_run = tea_protocol::RunId::from_str("0195a0b1-5e70-7c8d-9e2f-0aa7aa000048").unwrap();
    assert!(
        !make(
            "user:alice",
            "coding",
            "write_file",
            "1.0.0",
            GrantScope::Run { run_id: other_run }
        )
        .matches(&base_input)
    );
}

#[test]
fn revocation_and_pre_issuance_time_fail_closed() {
    let original = grant(GrantScope::Run {
        run_id: common::run_id(),
    });
    assert!(!original.matches(&input_with(original.clone(), "2026-07-23T08:59:59.999Z")));
    let revoked = original
        .revoke(common::timestamp("2026-07-23T10:00:00.000Z"))
        .unwrap();
    assert!(!revoked.matches(&input_with(revoked.clone(), "2026-07-23T10:00:00.000Z")));
}

#[test]
fn persistent_expiry_and_revocation_boundaries_are_validated() {
    assert_eq!(
        PolicyGrant::new(
            common::grant_id(),
            "user:alice".parse().unwrap(),
            ProfileId::from_str("coding").unwrap(),
            ToolName::from_str("write_file").unwrap(),
            ToolVersion::from_str("1.0.0").unwrap(),
            [ToolEffect::FsWrite],
            [ResourcePattern::new("file", "/workspace/", None).unwrap()],
            GrantScope::PersistentResource {
                expires_at: common::timestamp("2026-07-23T09:00:00.000Z")
            },
            common::timestamp("2026-07-23T09:00:00.000Z"),
        )
        .unwrap_err(),
        PolicyGrantError::InvalidExpiry
    );
}

#[test]
fn grant_round_trip_revalidates_all_invariants() {
    let grant = grant(GrantScope::PersistentResource {
        expires_at: common::timestamp("2026-07-24T09:00:00.000Z"),
    });
    let encoded = serde_json::to_string(&grant).unwrap();
    assert_eq!(
        serde_json::from_str::<PolicyGrant>(&encoded).unwrap(),
        grant
    );
}

#[test]
fn grants_match_complete_source_and_legacy_grants_never_authorize_mcp() {
    let scope = GrantScope::Run {
        run_id: common::run_id(),
    };
    let source = mcp_source(DIGEST_A);
    let exact = grant(scope.clone()).with_source(source.clone());
    let invocation = common::validated_invocation_with_source(
        "write_file",
        vec![ToolEffect::FsWrite],
        vec![common::file_resource(
            "/workspace/notes.txt",
            ToolResourceAccess::Write,
        )],
        json!({"path":"/workspace/notes.txt"}),
        source.clone(),
    );
    let input = common::input_with(
        &invocation,
        vec![exact.clone()],
        common::timestamp("2026-07-23T10:00:00.000Z"),
    );
    assert_eq!(input.tool_source(), &source);
    assert_eq!(exact.tool_source(), Some(&source));
    assert!(exact.matches(&input));

    let drifted = grant(scope.clone()).with_source(mcp_source(DIGEST_B));
    assert!(!drifted.matches(&input));
    let untrusted = ToolSource::new(
        ToolSourceKind::Mcp,
        "workspace.files",
        ToolTrust::Untrusted,
        DIGEST_A,
    )
    .unwrap();
    assert!(!grant(scope.clone()).with_source(untrusted).matches(&input));
    assert!(!grant(scope.clone()).matches(&input));

    let native_untrusted = ToolSource::new(
        ToolSourceKind::Native,
        "workspace.native",
        ToolTrust::Untrusted,
        DIGEST_A,
    )
    .unwrap();
    let invocation = common::validated_invocation_with_source(
        "write_file",
        vec![ToolEffect::FsWrite],
        vec![common::file_resource(
            "/workspace/notes.txt",
            ToolResourceAccess::Write,
        )],
        json!({"path":"/workspace/notes.txt"}),
        native_untrusted,
    );
    let input = common::input_with(
        &invocation,
        vec![],
        common::timestamp("2026-07-23T10:00:00.000Z"),
    );
    assert!(!grant(scope).matches(&input));
}
