use tea_context::{ContextError, ContextErrorCode};

#[test]
fn stable_context_error_codes_round_trip() {
    let cases = [
        (ContextErrorCode::InvalidValue, "invalid_value"),
        (ContextErrorCode::BoundsExceeded, "bounds_exceeded"),
        (ContextErrorCode::DuplicateIdentity, "duplicate_identity"),
        (ContextErrorCode::AmbiguousConflict, "ambiguous_conflict"),
        (ContextErrorCode::BudgetExceeded, "budget_exceeded"),
        (ContextErrorCode::ProviderFailure, "provider_failure"),
    ];
    for (code, wire) in cases {
        assert_eq!(serde_json::to_value(code).unwrap(), wire);
        assert_eq!(
            serde_json::from_str::<ContextErrorCode>(&format!("\"{wire}\"")).unwrap(),
            code
        );
    }
}

#[test]
fn context_errors_bound_safe_diagnostics() {
    let error = ContextError::new(
        ContextErrorCode::InvalidValue,
        format!("{}\0", "x".repeat(5000)),
    );
    assert_eq!(error.code(), ContextErrorCode::InvalidValue);
    assert!(!error.message().contains('\0'));
    assert!(error.message().len() <= 4096);
}
