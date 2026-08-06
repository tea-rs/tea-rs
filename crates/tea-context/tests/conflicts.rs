use crate::common;

use common::{conflicted, module, segment};
use tea_context::{
    ConflictMode, ContextErrorCode, PromptAuthority, PromptBudget, PromptCompiler,
    PromptDiagnosticCode, SegmentDisposition,
};

fn budget() -> PromptBudget {
    PromptBudget::new(4096, 4096).unwrap()
}

#[test]
fn compiler_order_is_stable_across_input_permutations() {
    let modules = vec![
        module(
            "workspace",
            PromptAuthority::Workspace,
            0,
            vec![segment(
                "workspace.one",
                "workspace",
                tea_context::BudgetBehavior::Required,
            )],
        ),
        module(
            "kernel",
            PromptAuthority::Kernel,
            0,
            vec![segment(
                "kernel.one",
                "kernel",
                tea_context::BudgetBehavior::Required,
            )],
        ),
        module(
            "product.high",
            PromptAuthority::Product,
            10,
            vec![segment(
                "product.high",
                "high",
                tea_context::BudgetBehavior::Required,
            )],
        ),
        module(
            "product.low",
            PromptAuthority::Product,
            -1,
            vec![segment(
                "product.low",
                "low",
                tea_context::BudgetBehavior::Required,
            )],
        ),
    ];
    let expected = PromptCompiler.compile(modules.clone(), budget()).unwrap();
    let mut reversed = modules;
    reversed.reverse();
    let actual = PromptCompiler.compile(reversed, budget()).unwrap();
    assert_eq!(expected, actual);
    assert_eq!(expected.text(), "kernel\n\nhigh\n\nlow\n\nworkspace");
}

#[test]
fn lower_authority_never_overrides_protected_conflict() {
    let prompt = PromptCompiler
        .compile(
            [
                module(
                    "user",
                    PromptAuthority::UserAddition,
                    100,
                    vec![conflicted(
                        "user.identity",
                        "user override",
                        "assistant.identity",
                        ConflictMode::Replaceable,
                    )],
                ),
                module(
                    "kernel",
                    PromptAuthority::Kernel,
                    -100,
                    vec![conflicted(
                        "kernel.identity",
                        "kernel identity",
                        "assistant.identity",
                        ConflictMode::Protected,
                    )],
                ),
            ],
            budget(),
        )
        .unwrap();
    assert_eq!(prompt.text(), "kernel identity");
    assert_eq!(
        prompt.diagnostics()[0].code(),
        PromptDiagnosticCode::ProtectedConflict
    );
    assert!(prompt.inspection().iter().any(|entry| {
        entry.segment_id().as_str() == "user.identity"
            && entry.disposition() == SegmentDisposition::ConflictShadowed
    }));
}

#[test]
fn equal_precedence_conflict_fails_closed() {
    let error = PromptCompiler
        .compile(
            [module(
                "product",
                PromptAuthority::Product,
                0,
                vec![
                    conflicted("one", "first", "identity", ConflictMode::Protected),
                    conflicted("two", "second", "identity", ConflictMode::Replaceable),
                ],
            )],
            budget(),
        )
        .unwrap_err();
    assert_eq!(error.code(), ContextErrorCode::AmbiguousConflict);
}

#[test]
fn exact_duplicates_deduplicate_but_divergent_ids_fail() {
    let duplicate = segment("same", "content", tea_context::BudgetBehavior::Required);
    let prompt = PromptCompiler
        .compile(
            [
                module(
                    "first.module",
                    PromptAuthority::Product,
                    0,
                    vec![duplicate.clone()],
                ),
                module(
                    "second.module",
                    PromptAuthority::Product,
                    0,
                    vec![duplicate],
                ),
            ],
            budget(),
        )
        .unwrap();
    assert_eq!(prompt.text(), "content");
    assert_eq!(
        prompt.diagnostics()[0].code(),
        PromptDiagnosticCode::ExactDuplicate
    );

    let error = PromptCompiler
        .compile(
            [
                module(
                    "divergent",
                    PromptAuthority::Product,
                    0,
                    vec![
                        segment("reuse", "one", tea_context::BudgetBehavior::Required),
                        segment("other", "safe", tea_context::BudgetBehavior::Required),
                    ],
                ),
                module(
                    "divergent",
                    PromptAuthority::Product,
                    0,
                    vec![segment(
                        "reuse",
                        "two",
                        tea_context::BudgetBehavior::Required,
                    )],
                ),
            ],
            budget(),
        )
        .unwrap_err();
    assert_eq!(error.code(), ContextErrorCode::DuplicateIdentity);
}
