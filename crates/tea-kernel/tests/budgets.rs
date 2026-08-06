use std::time::Duration;

use tea_kernel::{KernelErrorCode, RunLimits};

fn assert_invalid(
    max_tool_iterations: u32,
    max_elapsed: Duration,
    max_assistant_output_bytes: usize,
    max_events: u64,
    max_queued_messages: usize,
) {
    let error = RunLimits::new(
        max_tool_iterations,
        max_elapsed,
        max_assistant_output_bytes,
        max_events,
        max_queued_messages,
    )
    .unwrap_err();
    assert_eq!(error.code(), KernelErrorCode::InvalidRequest);
}

#[test]
fn default_budget_is_a_stable_release_contract() {
    let limits = RunLimits::default();

    assert_eq!(limits.max_tool_iterations(), 16);
    assert_eq!(limits.max_elapsed(), Duration::from_mins(5));
    assert_eq!(limits.max_assistant_output_bytes(), 4 * 1024 * 1024);
    assert_eq!(limits.max_events(), 100_000);
    assert_eq!(limits.max_queued_messages(), 64);
}

#[test]
fn hard_budget_boundaries_fail_closed_with_stable_errors() {
    assert!(
        RunLimits::new(
            u32::MAX,
            Duration::from_hours(24),
            16 * 1024 * 1024,
            1_000_000,
            1024,
        )
        .is_ok()
    );

    assert_invalid(0, Duration::from_secs(1), 1, 1, 1);
    assert_invalid(1, Duration::ZERO, 1, 1, 1);
    assert_invalid(1, Duration::from_secs(86_401), 1, 1, 1);
    assert_invalid(1, Duration::from_secs(1), 16 * 1024 * 1024 + 1, 1, 1);
    assert_invalid(1, Duration::from_secs(1), 1, 1_000_001, 1);
    assert_invalid(1, Duration::from_secs(1), 1, 1, 1025);
}
