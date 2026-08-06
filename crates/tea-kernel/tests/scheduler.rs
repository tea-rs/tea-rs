//! Effect-aware parallel tool scheduler lane assignment tests.

use std::str::FromStr;
use std::sync::Arc;

use serde_json::json;
use tea_kernel::Scheduler;
use tea_protocol::{ToolCallId, ToolIdempotency};
use tea_tools::{
    ArgumentResourceResolver, SchedulerClass, ToolConcurrency, ToolEffect, ToolExecutionSemantics,
    ToolExecutor, ToolName, ToolResourceAccess, ToolSpec, ToolTimeout, ToolVersion,
};

fn invocation(
    tool_call_id: &str,
    name: &str,
    effect: ToolEffect,
    idempotency: ToolIdempotency,
    concurrency: ToolConcurrency,
) -> tea_tools::ValidatedToolInvocation {
    let spec = ToolSpec::new(
        ToolName::from_str(name).unwrap(),
        ToolVersion::from_str("1.0.0").unwrap(),
        format!("Tool {name}."),
        json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        json!({"type":"object","properties":{"content":{"type":"string"}},"required":["content"]}),
        [effect],
        ToolExecutionSemantics::new(
            idempotency,
            if idempotency == ToolIdempotency::Idempotent {
                tea_tools::ToolRetrySafety::Automatic
            } else {
                tea_tools::ToolRetrySafety::ExplicitOnly
            },
            concurrency,
            ToolTimeout::from_millis(1_000).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let mut registry = tea_tools::ToolRegistry::new();
    registry
        .register(
            spec,
            std::sync::Arc::new(
                ArgumentResourceResolver::new("path", "file", ToolResourceAccess::Read).unwrap(),
            ),
            std::sync::Arc::new(NoopExecutor),
        )
        .unwrap();
    let invocation = tea_tools::ToolInvocation::new(
        ToolCallId::from_str(tool_call_id).unwrap(),
        ToolName::from_str(name).unwrap(),
        json!({"path":"/x"}),
        tea_protocol::ProtocolMetadata::default(),
    )
    .unwrap();
    registry.validate(invocation).unwrap()
}

#[derive(Debug)]
struct NoopExecutor;
impl ToolExecutor for NoopExecutor {
    fn execute(
        &self,
        _invocation: tea_tools::ValidatedToolInvocation,
        _cancellation: tea_control::CancellationScope,
    ) -> tea_tools::BoxToolExecutionStream {
        Box::pin(futures_util::stream::empty::<tea_tools::ToolExecutionEvent>())
    }
}

#[test]
fn read_only_tools_share_parallel_lane() {
    let invocations = [
        invocation(
            "0195a0b1-0001-7000-8000-0000000000a1",
            "read_a",
            ToolEffect::FsRead,
            ToolIdempotency::Idempotent,
            ToolConcurrency::Parallel,
        ),
        invocation(
            "0195a0b1-0001-7000-8000-0000000000a2",
            "read_b",
            ToolEffect::FsRead,
            ToolIdempotency::Idempotent,
            ToolConcurrency::Parallel,
        ),
    ];
    let refs: Vec<&tea_tools::ValidatedToolInvocation> = invocations.iter().collect();
    let plan = Scheduler.plan(&refs).unwrap();
    assert_eq!(plan.len(), 2);
    assert!(
        plan.lanes()
            .iter()
            .all(|lane| lane.class() == SchedulerClass::ParallelReadOnly)
    );
    assert_eq!(
        plan.lanes().len(),
        1,
        "parallel read-only tools share one lane"
    );
}

#[test]
fn serial_tools_share_one_lane() {
    let invocations = [
        invocation(
            "0195a0b1-0001-7000-8000-0000000000a1",
            "write_a",
            ToolEffect::FsWrite,
            ToolIdempotency::NonIdempotent,
            ToolConcurrency::Serial,
        ),
        invocation(
            "0195a0b1-0001-7000-8000-0000000000a2",
            "write_b",
            ToolEffect::FsWrite,
            ToolIdempotency::NonIdempotent,
            ToolConcurrency::Serial,
        ),
    ];
    let refs: Vec<&tea_tools::ValidatedToolInvocation> = invocations.iter().collect();
    let plan = Scheduler.plan(&refs).unwrap();
    assert_eq!(plan.lanes().len(), 1);
    assert_eq!(plan.lanes()[0].class(), SchedulerClass::Serial);
    assert_eq!(plan.lanes()[0].invocations().len(), 2);
}

#[test]
fn exclusive_tools_get_dedicated_lanes() {
    let invocations = [
        invocation(
            "0195a0b1-0001-7000-8000-0000000000a1",
            "lock_a",
            ToolEffect::ExternalMutation,
            ToolIdempotency::NonIdempotent,
            ToolConcurrency::Exclusive,
        ),
        invocation(
            "0195a0b1-0001-7000-8000-0000000000a2",
            "lock_b",
            ToolEffect::ExternalMutation,
            ToolIdempotency::NonIdempotent,
            ToolConcurrency::Exclusive,
        ),
    ];
    let refs: Vec<&tea_tools::ValidatedToolInvocation> = invocations.iter().collect();
    let plan = Scheduler.plan(&refs).unwrap();
    assert_eq!(plan.lanes().len(), 2);
    assert!(
        plan.lanes()
            .iter()
            .all(|lane| lane.class() == SchedulerClass::Exclusive)
    );
}

fn invocation_with_effect(
    tool_call_id: &str,
    name: &str,
    effect: ToolEffect,
) -> tea_tools::ValidatedToolInvocation {
    invocation(
        tool_call_id,
        name,
        effect,
        ToolIdempotency::Idempotent,
        ToolConcurrency::Parallel,
    )
}

#[test]
fn unknown_effects_are_rejected() {
    let mystery = ToolEffect::from_str("custom.mystery.effect").unwrap();
    let invocations = [invocation_with_effect(
        "0195a0b1-0001-7000-8000-0000000000a1",
        "mystery",
        mystery,
    )];
    let refs: Vec<&tea_tools::ValidatedToolInvocation> = invocations.iter().collect();
    let err = Scheduler.plan(&refs).unwrap_err();
    assert_eq!(err.code(), tea_kernel::KernelErrorCode::SchedulerConflict);
}

#[test]
fn mixed_classes_partition_into_separate_lanes() {
    let invocations = [
        invocation(
            "0195a0b1-0001-7000-8000-0000000000a1",
            "read_a",
            ToolEffect::FsRead,
            ToolIdempotency::Idempotent,
            ToolConcurrency::Parallel,
        ),
        invocation(
            "0195a0b1-0001-7000-8000-0000000000a2",
            "write_b",
            ToolEffect::FsWrite,
            ToolIdempotency::NonIdempotent,
            ToolConcurrency::Serial,
        ),
        invocation(
            "0195a0b1-0001-7000-8000-0000000000a3",
            "lock_c",
            ToolEffect::ExternalMutation,
            ToolIdempotency::NonIdempotent,
            ToolConcurrency::Exclusive,
        ),
    ];
    let refs: Vec<&tea_tools::ValidatedToolInvocation> = invocations.iter().collect();
    let plan = Scheduler.plan(&refs).unwrap();
    // Three distinct lanes: one parallel read-only, one serial, one exclusive.
    assert_eq!(plan.lanes().len(), 3);
    let classes: Vec<_> = plan.lanes().iter().map(tea_kernel::Lane::class).collect();
    assert!(classes.contains(&SchedulerClass::ParallelReadOnly));
    assert!(classes.contains(&SchedulerClass::Serial));
    assert!(classes.contains(&SchedulerClass::Exclusive));
}

// Keep the resource resolver import reachable; the registry validate path uses it.
#[allow(dead_code)]
fn _unused() -> Result<(), Box<dyn std::error::Error>> {
    let _ = ArgumentResourceResolver::new("path", "file", ToolResourceAccess::Read)?;
    let _ = Arc::new(());
    Ok(())
}
