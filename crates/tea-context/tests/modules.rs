use std::str::FromStr;

use tea_context::{
    BudgetBehavior, CacheScope, ConflictClaim, ConflictKey, ConflictMode, ContextProviderId,
    PromptAuthority, PromptModule, PromptModuleId, PromptPriority, PromptProvenance, PromptSegment,
    PromptSegmentId, TrustLevel,
};

fn segment(id: &str, content: &str) -> PromptSegment {
    PromptSegment::new(
        PromptSegmentId::from_str(id).unwrap(),
        content,
        PromptProvenance::new(
            ContextProviderId::from_str("test.provider").unwrap(),
            "test",
            None,
        )
        .unwrap(),
        TrustLevel::Trusted,
        CacheScope::Run,
        BudgetBehavior::Required,
    )
    .unwrap()
}

#[test]
fn segment_preserves_all_compiler_dimensions() {
    let segment =
        segment("product.identity", "You are concise.").with_conflict(ConflictClaim::new(
            ConflictKey::from_str("assistant.identity").unwrap(),
            ConflictMode::Protected,
        ));
    assert_eq!(segment.id().as_str(), "product.identity");
    assert_eq!(segment.content(), "You are concise.");
    assert_eq!(segment.trust(), TrustLevel::Trusted);
    assert_eq!(segment.cache_scope(), CacheScope::Run);
    assert_eq!(segment.budget_behavior(), BudgetBehavior::Required);
    assert_eq!(segment.conflict().unwrap().mode(), ConflictMode::Protected);
    assert_eq!(
        serde_json::from_value::<PromptSegment>(serde_json::to_value(&segment).unwrap()).unwrap(),
        segment
    );
}

#[test]
fn segment_content_bounds_are_revalidated() {
    assert!(
        PromptSegment::new(
            PromptSegmentId::from_str("empty").unwrap(),
            "",
            PromptProvenance::new(ContextProviderId::from_str("test").unwrap(), "test", None)
                .unwrap(),
            TrustLevel::Untrusted,
            CacheScope::None,
            BudgetBehavior::Omit,
        )
        .is_err()
    );
    let mut value = serde_json::to_value(segment("valid", "content")).unwrap();
    value["content"] = serde_json::json!("bad\0content");
    assert!(serde_json::from_value::<PromptSegment>(value).is_err());
}

#[test]
fn module_preserves_source_order_and_rejects_duplicate_ids() {
    let first = segment("first", "one");
    let second = segment("second", "two");
    let module = PromptModule::new(
        PromptModuleId::from_str("product.core").unwrap(),
        PromptAuthority::Product,
        PromptPriority::new(42),
        vec![first.clone(), second.clone()],
    )
    .unwrap();
    assert_eq!(module.segments(), [first.clone(), second]);
    assert_eq!(module.priority().get(), 42);
    assert!(
        PromptModule::new(
            PromptModuleId::from_str("invalid.empty").unwrap(),
            PromptAuthority::Product,
            PromptPriority::new(0),
            vec![],
        )
        .is_err()
    );
    assert!(
        PromptModule::new(
            PromptModuleId::from_str("invalid.duplicate").unwrap(),
            PromptAuthority::Product,
            PromptPriority::new(0),
            vec![first.clone(), first],
        )
        .is_err()
    );
}

#[test]
fn module_wire_boundary_revalidates_duplicate_segment_ids() {
    let module = PromptModule::new(
        PromptModuleId::from_str("module").unwrap(),
        PromptAuthority::Workspace,
        PromptPriority::new(-2),
        vec![segment("first", "one"), segment("second", "two")],
    )
    .unwrap();
    let mut value = serde_json::to_value(module).unwrap();
    value["segments"][1]["id"] = serde_json::json!("first");
    assert!(serde_json::from_value::<PromptModule>(value).is_err());
}
