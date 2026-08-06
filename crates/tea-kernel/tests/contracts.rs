use std::str::FromStr;

use tea_kernel::{
    DiscardEventSink, KernelClock, KernelError, KernelErrorCode, KernelEventSink, KernelIdSource,
    RunLimits, RunState, TokioKernelClock, UuidV7KernelIdSource,
};
use tea_protocol::{
    AgentEvent, EventEnvelope, ProtocolMetadata, ProtocolTimestamp, SessionId, SessionSequence,
};

fn assert_ports(_clock: &dyn KernelClock, _ids: &dyn KernelIdSource, _sink: &dyn KernelEventSink) {}

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn stable_error_codes_round_trip() {
    let cases = [
        (KernelErrorCode::InvalidModel, "invalid_model"),
        (KernelErrorCode::InvalidRequest, "invalid_request"),
        (KernelErrorCode::InvalidState, "invalid_state"),
        (KernelErrorCode::ModelFailure, "model_failure"),
        (KernelErrorCode::ToolFailure, "tool_failure"),
        (KernelErrorCode::PolicyFailure, "policy_failure"),
        (KernelErrorCode::SessionFailure, "session_failure"),
        (KernelErrorCode::EventSinkFailure, "event_sink_failure"),
        (KernelErrorCode::Cancelled, "cancelled"),
        (KernelErrorCode::LimitExceeded, "limit_exceeded"),
        (KernelErrorCode::IdExhausted, "id_exhausted"),
        (KernelErrorCode::ClockFailure, "clock_failure"),
        (KernelErrorCode::ContextOverflow, "context_overflow"),
        (KernelErrorCode::RetryExhausted, "retry_exhausted"),
        (KernelErrorCode::SchedulerConflict, "scheduler_conflict"),
    ];
    for (code, wire) in cases {
        assert_eq!(serde_json::to_value(code).unwrap(), wire);
        assert_eq!(
            serde_json::from_str::<KernelErrorCode>(&format!("\"{wire}\"")).unwrap(),
            code
        );
    }
}

#[test]
fn errors_bound_and_sanitize_diagnostics() {
    let error = KernelError::new(
        KernelErrorCode::InvalidRequest,
        format!("{}\0", "x".repeat(5000)),
    );
    assert_eq!(error.code(), KernelErrorCode::InvalidRequest);
    assert!(error.message().len() <= 4096);
    assert!(!error.message().contains('\0'));
}

#[tokio::test]
async fn runtime_ports_are_object_safe_and_send_sync() {
    let clock = TokioKernelClock;
    let ids = UuidV7KernelIdSource;
    let sink = DiscardEventSink;
    assert_ports(&clock, &ids, &sink);
    assert_send_sync::<TokioKernelClock>();
    assert_send_sync::<UuidV7KernelIdSource>();
    assert_send_sync::<DiscardEventSink>();

    let timestamp = clock.now().unwrap();
    let session_id = SessionId::from_str("0195a0b1-5e3a-7d72-a902-c4e85d828bf1").unwrap();
    let event = EventEnvelope::new(
        ids.next_event_id().unwrap(),
        session_id,
        Some(ids.next_run_id().unwrap()),
        None,
        SessionSequence::new(1),
        timestamp,
        ProtocolMetadata::default(),
        AgentEvent::RunStarted {},
    )
    .unwrap();
    sink.emit(event).await.unwrap();
}

#[test]
fn production_id_source_returns_typed_uuid_v7_values() {
    let ids = UuidV7KernelIdSource;
    assert_ne!(
        ids.next_record_id().unwrap().to_string(),
        ids.next_record_id().unwrap().to_string()
    );
    assert_ne!(
        ids.next_message_id().unwrap().to_string(),
        ids.next_tool_call_id().unwrap().to_string()
    );
    ids.next_turn_id().unwrap();
    ids.next_approval_id().unwrap();
    ids.next_grant_id().unwrap();
}

#[test]
fn run_state_allows_only_explicit_transitions() {
    let state = RunState::Idle
        .transition(RunState::PreparingContext)
        .unwrap();
    let state = state.transition(RunState::StreamingModel).unwrap();
    let state = state.transition(RunState::PlanningToolCalls).unwrap();
    let state = state.transition(RunState::EvaluatingPolicy).unwrap();
    let state = state.transition(RunState::WaitingApproval).unwrap();
    assert!(state.is_terminal());
    assert!(RunState::Idle.transition(RunState::ExecutingTool).is_err());
    assert!(
        RunState::Completed
            .transition(RunState::StreamingModel)
            .is_err()
    );
}

#[test]
fn run_limits_fail_closed_and_have_bounded_defaults() {
    let limits = RunLimits::default();
    assert!(limits.max_tool_iterations() > 0);
    assert!(limits.max_elapsed().as_secs() <= 86_400);
    assert!(limits.max_assistant_output_bytes() <= 16 * 1024 * 1024);
    assert!(limits.max_events() <= 1_000_000);
    assert!(limits.max_queued_messages() <= 1024);
    assert!(RunLimits::new(0, std::time::Duration::from_secs(1), 1, 1, 1).is_err());
}

#[test]
fn clock_returns_canonical_millisecond_timestamp() {
    let timestamp = TokioKernelClock.now().unwrap();
    let encoded = timestamp.to_string();
    assert!(ProtocolTimestamp::from_str(&encoded).is_ok());
}

#[test]
fn retry_policy_is_validated_and_deterministic() {
    use std::time::Duration;
    use tea_kernel::ModelRetryPolicy;

    let policy =
        ModelRetryPolicy::new(3, Duration::from_millis(100), Duration::from_secs(2)).unwrap();
    assert_eq!(policy.max_attempts(), 3);
    assert_eq!(policy.base_delay(), Duration::from_millis(100));
    assert_eq!(policy.max_delay(), Duration::from_secs(2));
    // Exponential, non-decreasing, capped.
    let d1 = policy.next_delay(1);
    let d2 = policy.next_delay(2);
    let d3 = policy.next_delay(3);
    assert!(d1 <= d2);
    assert!(d2 <= d3);
    assert!(d3 <= policy.max_delay());
    assert!(d1 >= policy.base_delay());
}

#[test]
fn retry_policy_rejects_invalid_bounds() {
    use std::time::Duration;
    use tea_kernel::{KernelErrorCode, ModelRetryPolicy};

    assert!(ModelRetryPolicy::new(0, Duration::from_millis(10), Duration::from_secs(1)).is_err());
    assert!(ModelRetryPolicy::new(3, Duration::ZERO, Duration::from_secs(1)).is_err());
    // Base greater than max is invalid.
    assert!(ModelRetryPolicy::new(3, Duration::from_secs(5), Duration::from_secs(1)).is_err());
    let err =
        ModelRetryPolicy::new(99, Duration::from_millis(10), Duration::from_secs(1)).unwrap_err();
    assert_eq!(err.code(), KernelErrorCode::InvalidRequest);
}

#[test]
fn retry_policy_should_retry_classifies_failures() {
    use std::time::Duration;
    use tea_kernel::ModelRetryPolicy;
    use tea_model::{ModelFailure, ModelFailureCode};
    use tea_protocol::RetryClass;

    let policy =
        ModelRetryPolicy::new(3, Duration::from_millis(10), Duration::from_secs(1)).unwrap();
    let retryable = ModelFailure::new(
        ModelFailureCode::Unavailable,
        "transient",
        RetryClass::AfterBackoff,
    )
    .unwrap();
    let immediate = ModelFailure::new(
        ModelFailureCode::RateLimited,
        "slow down",
        RetryClass::Immediate,
    )
    .unwrap();
    let never =
        ModelFailure::new(ModelFailureCode::InvalidRequest, "bad", RetryClass::Never).unwrap();
    assert_eq!(
        policy.should_retry(&retryable),
        Some(ModelFailureCode::Unavailable)
    );
    assert_eq!(
        policy.should_retry(&immediate),
        Some(ModelFailureCode::RateLimited)
    );
    assert_eq!(policy.should_retry(&never), None);
}

#[test]
fn retry_policy_prefers_bounded_server_delay() {
    use std::time::Duration;
    use tea_kernel::ModelRetryPolicy;
    use tea_model::{ModelFailure, ModelFailureCode};
    use tea_protocol::RetryClass;

    let policy = ModelRetryPolicy::new(4, Duration::from_secs(2), Duration::from_mins(1)).unwrap();
    let hinted = ModelFailure::new(
        ModelFailureCode::Unavailable,
        "temporarily unavailable",
        RetryClass::AfterBackoff,
    )
    .unwrap()
    .with_retry_after(Duration::from_secs(17));
    assert_eq!(hinted.retry_after(), Some(Duration::from_secs(17)));
    assert_eq!(
        policy.delay_for_failure(&hinted, 1),
        Duration::from_secs(17)
    );

    let capped = hinted.with_retry_after(Duration::from_secs(90));
    assert_eq!(policy.delay_for_failure(&capped, 1), Duration::from_mins(1));
}
