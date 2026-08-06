use tea_protocol::ToolCallId;
use tea_tools::{SchedulerClass, ValidatedToolInvocation};

use crate::{KernelError, KernelErrorCode};

/// Effect-aware tool scheduler producing lane assignments only.
///
/// The scheduler never executes tools. It partitions validated invocations by
/// `SchedulerClass` so the kernel can run parallel-safe work concurrently while
/// serializing mutation and granting exclusive work its own lane. Source order
/// within a lane is preserved; the kernel commits results in canonical source
/// order regardless of completion order.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scheduler;

/// One scheduler lane containing invocations that share an execution rule.
#[derive(Debug, Clone)]
pub struct Lane<'a> {
    pub(crate) class: SchedulerClass,
    pub(crate) invocations: Vec<&'a ValidatedToolInvocation>,
}

impl<'a> Lane<'a> {
    /// Returns the lane's scheduler class.
    #[must_use]
    pub const fn class(&self) -> SchedulerClass {
        self.class
    }
    /// Returns the invocations assigned to this lane in source order.
    #[must_use]
    pub fn invocations(&self) -> &[&'a ValidatedToolInvocation] {
        &self.invocations
    }
    /// Returns whether invocations in this lane may execute concurrently.
    #[must_use]
    pub fn allows_parallel(&self) -> bool {
        self.class.allows_parallel_execution()
    }
}

/// Immutable plan of lane assignments for one model turn.
#[derive(Debug, Clone)]
pub struct SchedulePlan<'a> {
    lanes: Vec<Lane<'a>>,
    source_order: Vec<ToolCallId>,
}

impl<'a> SchedulePlan<'a> {
    /// Returns the assigned lanes.
    #[must_use]
    pub fn lanes(&self) -> &[Lane<'a>] {
        &self.lanes
    }
    /// Returns the count of scheduled invocations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.source_order.len()
    }
    /// Returns whether the plan scheduled no invocations.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.source_order.is_empty()
    }
    /// Returns the canonical source-order tool-call identifiers.
    #[must_use]
    pub fn source_order(&self) -> &[ToolCallId] {
        &self.source_order
    }
}

impl Scheduler {
    /// Partitions validated invocations into scheduler lanes.
    ///
    /// # Errors
    ///
    /// Returns a `SchedulerConflict` for any invocation whose
    /// `SchedulerClass::requires_policy()` is true (unknown effects). The
    /// scheduler never auto-schedules policy-required work.
    pub fn plan<'a>(
        &self,
        invocations: &'a [&'a ValidatedToolInvocation],
    ) -> Result<SchedulePlan<'a>, KernelError> {
        if invocations
            .iter()
            .any(|invocation| invocation.scheduler_class().requires_policy())
        {
            return Err(KernelError::new(
                KernelErrorCode::SchedulerConflict,
                "tool invocation requires policy before scheduling",
            ));
        }
        let source_order = invocations
            .iter()
            .map(|invocation| *invocation.tool_call_id())
            .collect();
        let mut parallel: Vec<&ValidatedToolInvocation> = Vec::new();
        let mut serial: Vec<&ValidatedToolInvocation> = Vec::new();
        let mut exclusive: Vec<&ValidatedToolInvocation> = Vec::new();
        for invocation in invocations {
            match invocation.scheduler_class() {
                SchedulerClass::ParallelReadOnly | SchedulerClass::ParallelRetrySafe => {
                    parallel.push(invocation);
                }
                SchedulerClass::Serial => serial.push(invocation),
                SchedulerClass::Exclusive => exclusive.push(invocation),
                SchedulerClass::PolicyRequired => {
                    return Err(KernelError::new(
                        KernelErrorCode::SchedulerConflict,
                        "tool invocation requires policy before scheduling",
                    ));
                }
            }
        }
        let mut lanes = Vec::new();
        if !parallel.is_empty() {
            lanes.push(Lane {
                class: SchedulerClass::ParallelReadOnly,
                invocations: parallel,
            });
        }
        if !serial.is_empty() {
            lanes.push(Lane {
                class: SchedulerClass::Serial,
                invocations: serial,
            });
        }
        for invocation in exclusive {
            lanes.push(Lane {
                class: SchedulerClass::Exclusive,
                invocations: vec![invocation],
            });
        }
        Ok(SchedulePlan {
            lanes,
            source_order,
        })
    }
}
