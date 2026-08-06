//! Public contract tests for runtime stable errors and health inspection.

use tea::{RuntimeError, RuntimeErrorCode, RuntimeHealth};

#[test]
fn core_contract_namespaces_are_reexported() {
    fn assert_public_type<T>() {}

    assert_public_type::<tea::context::PromptBudget>();
    assert_public_type::<tea::control::CancellationScope>();
    assert_public_type::<tea::kernel::RunState>();
    assert_public_type::<tea::model::ModelSpec>();
    assert_public_type::<tea::policy::ActorId>();
    assert_public_type::<tea::profile::AgentProfile>();
    assert_public_type::<tea::protocol::CommandEnvelope>();
    assert_public_type::<tea::session::SessionSnapshot>();
    assert_public_type::<tea::tools::ToolSpec>();
}

#[test]
fn error_codes_are_stable_discriminants() {
    for code in [
        RuntimeErrorCode::InvalidRequest,
        RuntimeErrorCode::UnknownProfile,
        RuntimeErrorCode::UnknownProvider,
        RuntimeErrorCode::UnknownModel,
        RuntimeErrorCode::UnknownTool,
        RuntimeErrorCode::UnknownPolicyRule,
        RuntimeErrorCode::RunAlreadyActive,
        RuntimeErrorCode::NoActiveRun,
        RuntimeErrorCode::SessionFailure,
        RuntimeErrorCode::PolicyFailure,
        RuntimeErrorCode::KernelFailure,
        RuntimeErrorCode::ProviderFailure,
        RuntimeErrorCode::Cancelled,
        RuntimeErrorCode::UnsupportedCommand,
        RuntimeErrorCode::EventSinkClosed,
        RuntimeErrorCode::BoundsExceeded,
        RuntimeErrorCode::DuplicateEntry,
    ] {
        let error = RuntimeError::new(code, "example");
        assert_eq!(error.code(), code);
        assert!(!error.message().is_empty());
        assert!(!error.message().contains('\0'));
    }
}

#[test]
fn error_message_is_bounded_and_null_free() {
    let long = "x".repeat(8192);
    let error = RuntimeError::new(RuntimeErrorCode::BoundsExceeded, long);
    assert!(error.message().len() <= 4096);
    assert!(!error.message().is_empty());
}

#[test]
fn health_reports_empty_runtime_state() {
    let health = RuntimeHealth::empty();
    assert!(health.profile_ids().is_empty());
    assert!(health.policy_rule_ids().is_empty());
    assert_eq!(health.model_count(), 0);
    assert_eq!(health.tool_count(), 0);
    assert_eq!(health.session_count(), 0);
    assert!(health.provider_id().is_none());
}
