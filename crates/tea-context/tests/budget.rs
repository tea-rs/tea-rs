use crate::common;

use common::{module, segment};
use tea_context::{
    BudgetBehavior, ContextErrorCode, PromptAuthority, PromptBudget, PromptCompiler,
    PromptDiagnosticCode, SegmentDisposition, TRUNCATION_MARKER, estimate_tokens,
};

#[test]
fn required_overflow_fails_and_omit_is_diagnostic() {
    let required = PromptCompiler
        .compile(
            [module(
                "required",
                PromptAuthority::Kernel,
                0,
                vec![segment("required", "12345", BudgetBehavior::Required)],
            )],
            PromptBudget::new(4, 100).unwrap(),
        )
        .unwrap_err();
    assert_eq!(required.code(), ContextErrorCode::BudgetExceeded);

    let prompt = PromptCompiler
        .compile(
            [module(
                "mixed",
                PromptAuthority::Product,
                0,
                vec![
                    segment("keep", "ok", BudgetBehavior::Required),
                    segment("omit", "too long", BudgetBehavior::Omit),
                ],
            )],
            PromptBudget::new(4, 100).unwrap(),
        )
        .unwrap();
    assert_eq!(prompt.text(), "ok");
    assert_eq!(
        prompt.diagnostics()[0].code(),
        PromptDiagnosticCode::OmittedForBudget
    );
}

#[test]
fn truncation_is_utf8_safe_and_uses_fixed_marker() {
    let prompt = PromptCompiler
        .compile(
            [module(
                "truncate",
                PromptAuthority::Workspace,
                0,
                vec![segment(
                    "unicode",
                    "你好abcdefghijklmnop",
                    BudgetBehavior::Truncate,
                )],
            )],
            PromptBudget::new(TRUNCATION_MARKER.len() + 5, 100).unwrap(),
        )
        .unwrap();
    assert!(prompt.text().ends_with(TRUNCATION_MARKER));
    assert!(std::str::from_utf8(prompt.text().as_bytes()).is_ok());
    assert_eq!(
        prompt.diagnostics()[0].code(),
        PromptDiagnosticCode::TruncatedForBudget
    );
    assert_eq!(
        prompt.inspection()[0].disposition(),
        SegmentDisposition::Truncated
    );
}

#[test]
fn separators_count_toward_byte_and_token_budgets() {
    let modules = [module(
        "two",
        PromptAuthority::Product,
        0,
        vec![
            segment("one", "abc", BudgetBehavior::Required),
            segment("two", "def", BudgetBehavior::Required),
        ],
    )];
    let prompt = PromptCompiler
        .compile(modules.clone(), PromptBudget::new(8, 3).unwrap())
        .unwrap();
    assert_eq!(prompt.text(), "abc\n\ndef");
    assert_eq!(prompt.bytes(), 8);
    assert_eq!(prompt.estimated_tokens(), 3);
    assert_eq!(estimate_tokens(8), 3);
    assert_eq!(
        PromptCompiler
            .compile(modules, PromptBudget::new(7, 3).unwrap())
            .unwrap_err()
            .code(),
        ContextErrorCode::BudgetExceeded
    );
}

#[test]
fn fully_optional_prompt_may_compile_empty_with_diagnostics() {
    let prompt = PromptCompiler
        .compile(
            [module(
                "optional",
                PromptAuthority::Session,
                0,
                vec![segment("optional", "too long", BudgetBehavior::Omit)],
            )],
            PromptBudget::new(1, 1).unwrap(),
        )
        .unwrap();
    assert!(prompt.text().is_empty());
    assert_eq!(prompt.estimated_tokens(), 0);
    assert_eq!(
        prompt.diagnostics()[0].code(),
        PromptDiagnosticCode::OmittedForBudget
    );
}

#[test]
fn truncation_without_marker_space_omits_deterministically() {
    let prompt = PromptCompiler
        .compile(
            [module(
                "mixed",
                PromptAuthority::Product,
                0,
                vec![
                    segment("keep", "x", BudgetBehavior::Required),
                    segment("truncate", "abcdef", BudgetBehavior::Truncate),
                ],
            )],
            PromptBudget::new(4, 100).unwrap(),
        )
        .unwrap();
    assert_eq!(prompt.text(), "x");
    assert_eq!(
        prompt.inspection()[1].disposition(),
        SegmentDisposition::OmittedForBudget
    );
}
