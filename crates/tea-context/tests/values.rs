use std::str::FromStr;

use tea_context::{
    CacheScope, ConflictKey, ContextProviderId, PromptAuthority, PromptModuleId, PromptProvenance,
    PromptSegmentId, SkillId, TrustLevel,
};

#[test]
fn canonical_context_ids_round_trip_and_reject_invalid_values() {
    for value in ["kernel.identity", "workspace/main", "tool-read_hint"] {
        let id = PromptModuleId::from_str(value).unwrap();
        assert_eq!(id.as_str(), value);
        assert_eq!(
            serde_json::from_value::<PromptModuleId>(serde_json::to_value(&id).unwrap()).unwrap(),
            id
        );
    }
    for value in ["", "Upper", "../escape", "double..dot", "has space"] {
        assert!(PromptSegmentId::from_str(value).is_err());
        assert!(ContextProviderId::from_str(value).is_err());
        assert!(ConflictKey::from_str(value).is_err());
        assert!(SkillId::from_str(value).is_err());
    }
}

#[test]
fn authority_order_is_fixed_high_to_low() {
    assert!(PromptAuthority::Kernel < PromptAuthority::Organization);
    assert!(PromptAuthority::Organization < PromptAuthority::Product);
    assert!(PromptAuthority::Product < PromptAuthority::Workspace);
    assert!(PromptAuthority::Workspace < PromptAuthority::Tool);
    assert!(PromptAuthority::Tool < PromptAuthority::Skill);
    assert!(PromptAuthority::Skill < PromptAuthority::Session);
    assert!(PromptAuthority::Session < PromptAuthority::UserAddition);
}

#[test]
fn provenance_trust_and_cache_round_trip() {
    let provenance = PromptProvenance::new(
        ContextProviderId::from_str("workspace.instructions").unwrap(),
        "workspace_file",
        Some("/workspace/AGENTS.md".to_owned()),
    )
    .unwrap();
    assert_eq!(provenance.source_kind(), "workspace_file");
    assert_eq!(provenance.locator(), Some("/workspace/AGENTS.md"));
    assert_eq!(
        serde_json::from_value::<PromptProvenance>(serde_json::to_value(&provenance).unwrap())
            .unwrap(),
        provenance
    );
    for trust in [
        TrustLevel::Trusted,
        TrustLevel::Delegated,
        TrustLevel::Untrusted,
    ] {
        serde_json::to_value(trust).unwrap();
    }
    for scope in [
        CacheScope::None,
        CacheScope::Run,
        CacheScope::Session,
        CacheScope::Profile,
        CacheScope::Global,
    ] {
        serde_json::to_value(scope).unwrap();
    }
}

#[test]
fn provenance_bounds_fail_closed() {
    let provider = ContextProviderId::from_str("test.provider").unwrap();
    assert!(PromptProvenance::new(provider.clone(), "Invalid", None).is_err());
    assert!(PromptProvenance::new(provider, "source", Some("x".repeat(2049))).is_err());
}
