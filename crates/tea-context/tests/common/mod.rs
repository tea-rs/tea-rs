#![allow(dead_code)]

use std::str::FromStr;

use tea_context::{
    BudgetBehavior, CacheScope, ConflictClaim, ConflictKey, ConflictMode, ContextProviderId,
    PromptAuthority, PromptModule, PromptModuleId, PromptPriority, PromptProvenance, PromptSegment,
    PromptSegmentId, TrustLevel,
};

pub fn segment(id: &str, content: &str, behavior: BudgetBehavior) -> PromptSegment {
    PromptSegment::new(
        PromptSegmentId::from_str(id).unwrap(),
        content,
        PromptProvenance::new(
            ContextProviderId::from_str("test.provider").unwrap(),
            "test",
            Some(format!("source:{id}")),
        )
        .unwrap(),
        TrustLevel::Delegated,
        CacheScope::Run,
        behavior,
    )
    .unwrap()
}

pub fn conflicted(id: &str, content: &str, key: &str, mode: ConflictMode) -> PromptSegment {
    segment(id, content, BudgetBehavior::Required).with_conflict(ConflictClaim::new(
        ConflictKey::from_str(key).unwrap(),
        mode,
    ))
}

pub fn module(
    id: &str,
    authority: PromptAuthority,
    priority: i16,
    segments: Vec<PromptSegment>,
) -> PromptModule {
    PromptModule::new(
        PromptModuleId::from_str(id).unwrap(),
        authority,
        PromptPriority::new(priority),
        segments,
    )
    .unwrap()
}
