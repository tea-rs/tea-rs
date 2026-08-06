use std::collections::{BTreeMap, BTreeSet};

use tea_protocol::{
    ApprovalDecision, ApprovalId, BranchId, CanonicalMessage, MessageId, RecordEnvelope, RecordId,
    SessionRecord, SessionSequence, ToolCallId,
};

use crate::error::SessionReplayError;
use crate::state::{
    MaterializedSessionState, RunRecoveryState, ToolExecutionState, TurnCheckpoint,
    commit_tool_result, declared_tool_calls, finish_tool, interrupt_tool, message_id,
    new_pending_approval, new_state, new_tool_call, resolve_approval, set_approval, set_compaction,
    set_model, set_policy, set_profile, set_reasoning_effort, start_tool,
};

#[derive(Debug, Clone, PartialEq)]
struct DeclaredToolCall {
    tool_name: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
struct BranchProjection {
    configuration: crate::SessionConfiguration,
    messages: Vec<CanonicalMessage>,
    pending_approvals: BTreeMap<ApprovalId, crate::PendingApproval>,
    tool_calls: BTreeMap<ToolCallId, crate::ToolCallState>,
    run_recovery: BTreeMap<tea_protocol::RunId, RunRecoveryState>,
    latest_checkpoint: Option<TurnCheckpoint>,
    latest_compaction: Option<crate::SessionCompaction>,
}

impl BranchProjection {
    fn capture(state: &MaterializedSessionState) -> Self {
        Self {
            configuration: state.configuration.clone(),
            messages: state.messages.clone(),
            pending_approvals: state.pending_approvals.clone(),
            tool_calls: state.tool_calls.clone(),
            run_recovery: state.run_recovery.clone(),
            latest_checkpoint: state.latest_checkpoint.clone(),
            latest_compaction: state.latest_compaction.clone(),
        }
    }

    fn install(&self, state: &mut MaterializedSessionState) {
        state.configuration.clone_from(&self.configuration);
        state.messages.clone_from(&self.messages);
        state.pending_approvals.clone_from(&self.pending_approvals);
        state.tool_calls.clone_from(&self.tool_calls);
        state.run_recovery.clone_from(&self.run_recovery);
        state.latest_checkpoint.clone_from(&self.latest_checkpoint);
        state.latest_compaction.clone_from(&self.latest_compaction);
    }
}

#[derive(Debug, Clone, PartialEq)]
struct BranchSnapshot {
    projection: BranchProjection,
    history: BTreeSet<RecordId>,
    declared_tool_calls: BTreeSet<ToolCallId>,
}

impl BranchSnapshot {
    fn safe_to_fork(&self) -> bool {
        self.projection.pending_approvals.is_empty()
            && self.declared_tool_calls.iter().all(|tool_call_id| {
                self.projection
                    .tool_calls
                    .get(tool_call_id)
                    .is_some_and(|tool| {
                        matches!(tool.execution(), ToolExecutionState::Finished { .. })
                            && tool.result_message_id().is_some()
                    })
            })
    }
}

#[derive(Debug, Default)]
struct HistoricalSnapshots {
    required: BTreeSet<RecordId>,
    captured: BTreeMap<RecordId, BranchSnapshot>,
}

impl HistoricalSnapshots {
    fn for_prefix(records: &[RecordEnvelope], target: RecordId) -> Option<(Self, usize)> {
        let target_index = records
            .iter()
            .position(|record| record.record_id() == target)?;
        let mut required = BTreeSet::from([target]);
        for record in &records[..=target_index] {
            if let SessionRecord::BranchCreated { from_record_id, .. } = record.record() {
                required.insert(*from_record_id);
            }
        }
        Some((
            Self {
                required,
                captured: BTreeMap::new(),
            },
            target_index,
        ))
    }

    fn capture(&mut self, record_id: RecordId, snapshot: &BranchSnapshot) {
        if self.required.contains(&record_id) {
            self.captured.insert(record_id, snapshot.clone());
        }
    }

    fn get(&self, record_id: RecordId) -> Option<&BranchSnapshot> {
        self.captured.get(&record_id)
    }
}

/// Pure deterministic reducer for canonical durable session records.
#[derive(Debug, Clone, Default)]
pub struct SessionReducer {
    state: Option<MaterializedSessionState>,
    records: Vec<RecordEnvelope>,
    record_ids: BTreeSet<RecordId>,
    message_ids: BTreeSet<MessageId>,
    declared_tool_calls: BTreeMap<ToolCallId, DeclaredToolCall>,
    requested_tool_call_ids: BTreeSet<ToolCallId>,
    approval_ids: BTreeSet<ApprovalId>,
    branch_heads: BTreeMap<BranchId, BranchSnapshot>,
}

impl SessionReducer {
    /// Creates an empty reducer awaiting a sequence-zero creation record.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: None,
            records: Vec::new(),
            record_ids: BTreeSet::new(),
            message_ids: BTreeSet::new(),
            declared_tool_calls: BTreeMap::new(),
            requested_tool_call_ids: BTreeSet::new(),
            approval_ids: BTreeSet::new(),
            branch_heads: BTreeMap::new(),
        }
    }

    /// Rebuilds materialized state from canonical record order.
    ///
    /// # Errors
    ///
    /// Returns a deterministic corruption/reference/transition error when the
    /// complete record stream cannot represent one valid session.
    pub fn replay(
        records: impl IntoIterator<Item = RecordEnvelope>,
    ) -> Result<MaterializedSessionState, SessionReplayError> {
        let reducer = Self::replay_reducer(records)?;
        reducer.state.ok_or(SessionReplayError::EmptyLog)
    }

    /// Rebuilds a reusable reducer from canonical record order.
    ///
    /// # Errors
    ///
    /// Returns the first deterministic replay failure.
    pub fn replay_reducer(
        records: impl IntoIterator<Item = RecordEnvelope>,
    ) -> Result<Self, SessionReplayError> {
        let mut reducer = Self::new();
        for record in records {
            reducer.apply(&record)?;
        }
        if reducer.state.is_none() {
            return Err(SessionReplayError::EmptyLog);
        }
        Ok(reducer)
    }

    /// Applies one next canonical record atomically to this reducer.
    ///
    /// # Errors
    ///
    /// Returns an error without changing reducer state when sequence,
    /// identity, reference, or lifecycle invariants fail.
    pub fn apply(&mut self, envelope: &RecordEnvelope) -> Result<(), SessionReplayError> {
        if let Err(error) = self.apply_inner(envelope) {
            self.restore_durable_state();
            return Err(error);
        }
        self.records.push(envelope.clone());
        Ok(())
    }

    /// Returns current materialized state, or `None` before creation.
    #[must_use]
    pub const fn state(&self) -> Option<&MaterializedSessionState> {
        self.state.as_ref()
    }

    fn restore_durable_state(&mut self) {
        let records = std::mem::take(&mut self.records);
        let mut restored = Self::new();
        for record in records {
            restored
                .apply_inner(&record)
                .expect("previously accepted records remain replayable");
            restored.records.push(record);
        }
        *self = restored;
    }

    fn apply_inner(&mut self, envelope: &RecordEnvelope) -> Result<(), SessionReplayError> {
        self.apply_inner_with_history(envelope, None)
    }

    fn apply_inner_with_history(
        &mut self,
        envelope: &RecordEnvelope,
        historical: Option<&mut HistoricalSnapshots>,
    ) -> Result<(), SessionReplayError> {
        if self.record_ids.contains(&envelope.record_id()) {
            return Err(SessionReplayError::DuplicateRecord {
                record_id: envelope.record_id(),
            });
        }
        if self.state.is_none() {
            return self.apply_creation(envelope, historical);
        }

        self.validate_envelope(envelope)?;
        if matches!(envelope.record(), SessionRecord::SessionCreated { .. }) {
            return Err(SessionReplayError::InvalidCreation);
        }
        self.validate_branch_scope(envelope)?;
        match envelope.record() {
            SessionRecord::BranchCreated {
                source_branch_id,
                branch_id,
                from_record_id,
            } => {
                self.apply_branch_created(
                    envelope,
                    *source_branch_id,
                    *branch_id,
                    *from_record_id,
                    historical,
                )?;
            }
            SessionRecord::ActiveBranchChanged { branch_id } => {
                self.apply_active_branch_changed(envelope, *branch_id, historical)?;
            }
            _ => {
                self.apply_record(envelope)?;
                self.capture_active_branch(envelope.record_id(), historical)?;
            }
        }

        let state = self
            .state
            .as_mut()
            .ok_or(SessionReplayError::InvalidTransition {
                transition: "missing_materialized_state",
            })?;
        state.tail_sequence = envelope.sequence();
        state.tail_record_id = envelope.record_id();
        self.record_ids.insert(envelope.record_id());
        Ok(())
    }

    fn apply_creation(
        &mut self,
        envelope: &RecordEnvelope,
        historical: Option<&mut HistoricalSnapshots>,
    ) -> Result<(), SessionReplayError> {
        if envelope.sequence() != SessionSequence::new(0) {
            return Err(SessionReplayError::SequenceMismatch {
                expected: SessionSequence::new(0),
                actual: envelope.sequence(),
            });
        }
        let SessionRecord::SessionCreated {
            profile_id,
            metadata,
        } = envelope.record()
        else {
            return Err(SessionReplayError::InvalidCreation);
        };
        self.state = Some(new_state(
            envelope.record_id(),
            envelope.session_id(),
            envelope.sequence(),
            profile_id.clone(),
            metadata.clone(),
            envelope.branch_id(),
        ));
        if let Some(branch_id) = envelope.branch_id() {
            let state = self
                .state
                .as_ref()
                .ok_or(SessionReplayError::InvalidCreation)?;
            let snapshot = BranchSnapshot {
                projection: BranchProjection::capture(state),
                history: BTreeSet::from([envelope.record_id()]),
                declared_tool_calls: BTreeSet::new(),
            };
            if let Some(historical) = historical {
                historical.capture(envelope.record_id(), &snapshot);
            }
            self.branch_heads.insert(branch_id, snapshot);
        }
        self.record_ids.insert(envelope.record_id());
        Ok(())
    }

    fn validate_envelope(&self, envelope: &RecordEnvelope) -> Result<(), SessionReplayError> {
        let state = self
            .state
            .as_ref()
            .ok_or(SessionReplayError::InvalidTransition {
                transition: "missing_materialized_state",
            })?;
        if envelope.session_id() != state.session_id {
            return Err(SessionReplayError::SessionMismatch {
                expected: state.session_id,
                actual: envelope.session_id(),
            });
        }
        let expected = state
            .tail_sequence
            .checked_next()
            .ok_or(SessionReplayError::SequenceOverflow)?;
        if envelope.sequence() != expected {
            return Err(SessionReplayError::SequenceMismatch {
                expected,
                actual: envelope.sequence(),
            });
        }
        Ok(())
    }

    fn validate_branch_scope(&self, envelope: &RecordEnvelope) -> Result<(), SessionReplayError> {
        let state = self
            .state
            .as_ref()
            .ok_or(SessionReplayError::InvalidTransition {
                transition: "missing_materialized_state",
            })?;
        match envelope.record() {
            SessionRecord::BranchCreated { .. } | SessionRecord::ActiveBranchChanged { .. } => {
                Ok(())
            }
            _ if envelope.branch_id() == state.active_branch_id => Ok(()),
            _ => Err(SessionReplayError::InvalidReference {
                reference: "inactive_branch",
            }),
        }
    }

    fn capture_active_branch(
        &mut self,
        record_id: RecordId,
        historical: Option<&mut HistoricalSnapshots>,
    ) -> Result<(), SessionReplayError> {
        let state = self
            .state
            .as_mut()
            .ok_or(SessionReplayError::InvalidTransition {
                transition: "missing_materialized_state",
            })?;
        let Some(branch_id) = state.active_branch_id else {
            return Ok(());
        };
        let head =
            self.branch_heads
                .get_mut(&branch_id)
                .ok_or(SessionReplayError::InvalidReference {
                    reference: "active_branch",
                })?;
        head.history.insert(record_id);
        head.declared_tool_calls = state
            .messages
            .iter()
            .flat_map(declared_tool_calls)
            .map(|(tool_call_id, _, _)| tool_call_id)
            .collect();
        head.projection = BranchProjection::capture(state);
        state
            .branches
            .get_mut(&branch_id)
            .ok_or(SessionReplayError::InvalidReference {
                reference: "active_branch_summary",
            })?
            .set_leaf(record_id);
        if let Some(historical) = historical {
            historical.capture(record_id, head);
        }
        Ok(())
    }

    fn apply_branch_created(
        &mut self,
        envelope: &RecordEnvelope,
        source_branch_id: BranchId,
        branch_id: BranchId,
        from_record_id: RecordId,
        historical: Option<&mut HistoricalSnapshots>,
    ) -> Result<(), SessionReplayError> {
        if self.branch_heads.contains_key(&branch_id) {
            return Err(SessionReplayError::DuplicateEntity { entity: "branch" });
        }
        let source = self.branch_heads.get(&source_branch_id).ok_or(
            SessionReplayError::InvalidReference {
                reference: "source_branch",
            },
        )?;
        if !source.history.contains(&from_record_id) {
            return Err(SessionReplayError::InvalidReference {
                reference: "fork_point_outside_source_branch",
            });
        }
        let source_snapshot = match historical.as_deref() {
            Some(historical) => historical.get(from_record_id).cloned(),
            None => self.replay_branch_snapshot(source_branch_id, from_record_id)?,
        }
        .ok_or(SessionReplayError::InvalidReference {
            reference: "fork_record_snapshot",
        })?;
        if !source_snapshot.safe_to_fork() {
            return Err(SessionReplayError::InvalidTransition {
                transition: "unsafe_fork_point",
            });
        }
        let mut new_snapshot = source_snapshot;
        new_snapshot.history.insert(envelope.record_id());
        if let Some(historical) = historical {
            historical.capture(envelope.record_id(), &new_snapshot);
        }
        self.branch_heads.insert(branch_id, new_snapshot);
        self.state
            .as_mut()
            .ok_or(SessionReplayError::InvalidTransition {
                transition: "missing_materialized_state",
            })?
            .branches
            .insert(
                branch_id,
                crate::BranchSummary::new(
                    branch_id,
                    Some(source_branch_id),
                    from_record_id,
                    envelope.record_id(),
                ),
            );
        Ok(())
    }

    fn apply_active_branch_changed(
        &mut self,
        envelope: &RecordEnvelope,
        branch_id: BranchId,
        historical: Option<&mut HistoricalSnapshots>,
    ) -> Result<(), SessionReplayError> {
        let head =
            self.branch_heads
                .get_mut(&branch_id)
                .ok_or(SessionReplayError::InvalidReference {
                    reference: "active_branch_change",
                })?;
        let state = self
            .state
            .as_mut()
            .ok_or(SessionReplayError::InvalidTransition {
                transition: "missing_materialized_state",
            })?;
        head.projection.install(state);
        head.history.insert(envelope.record_id());
        state.active_branch_id = Some(branch_id);
        state
            .branches
            .get_mut(&branch_id)
            .ok_or(SessionReplayError::InvalidReference {
                reference: "active_branch_summary",
            })?
            .set_leaf(envelope.record_id());
        if let Some(historical) = historical {
            historical.capture(envelope.record_id(), head);
        }
        Ok(())
    }

    fn replay_branch_snapshot(
        &self,
        source_branch_id: BranchId,
        from_record_id: RecordId,
    ) -> Result<Option<BranchSnapshot>, SessionReplayError> {
        let Some((mut historical, target_index)) =
            HistoricalSnapshots::for_prefix(&self.records, from_record_id)
        else {
            return Ok(None);
        };
        let mut replay = Self::new();
        for record in &self.records[..=target_index] {
            replay.apply_inner_with_history(record, Some(&mut historical))?;
            replay.records.push(record.clone());
        }
        Ok(historical
            .get(from_record_id)
            .filter(|snapshot| {
                replay
                    .branch_heads
                    .get(&source_branch_id)
                    .is_some_and(|source| source.history.contains(&from_record_id))
                    && snapshot.history.contains(&from_record_id)
            })
            .cloned())
    }

    #[allow(clippy::too_many_lines)]
    fn apply_record(&mut self, envelope: &RecordEnvelope) -> Result<(), SessionReplayError> {
        let state = self
            .state
            .as_mut()
            .ok_or(SessionReplayError::InvalidTransition {
                transition: "missing_materialized_state",
            })?;
        match envelope.record() {
            SessionRecord::SessionCreated { .. } => Err(SessionReplayError::InvalidCreation),
            SessionRecord::MessageCommitted { message } => {
                let id = message_id(message);
                if !self.message_ids.insert(id) {
                    return Err(SessionReplayError::DuplicateEntity { entity: "message" });
                }
                for (tool_call_id, tool_name, arguments) in declared_tool_calls(message) {
                    if self
                        .declared_tool_calls
                        .insert(
                            tool_call_id,
                            DeclaredToolCall {
                                tool_name: tool_name.to_owned(),
                                arguments: arguments.clone(),
                            },
                        )
                        .is_some()
                        || state.tool_calls.contains_key(&tool_call_id)
                    {
                        return Err(SessionReplayError::DuplicateEntity {
                            entity: "tool_call",
                        });
                    }
                }
                if let CanonicalMessage::ToolResult {
                    tool_call_id,
                    tool_name,
                    content,
                    is_error,
                    error,
                    ..
                } = message
                {
                    let tool = state.tool_calls.get_mut(tool_call_id).ok_or(
                        SessionReplayError::InvalidReference {
                            reference: "tool_result_tool_call",
                        },
                    )?;
                    if tool.result_message_id().is_some() {
                        return Err(SessionReplayError::InvalidTransition {
                            transition: "duplicate_tool_result_message",
                        });
                    }
                    match tool.execution() {
                        ToolExecutionState::Finished {
                            is_error: terminal_error,
                            content: terminal_content,
                            error: terminal_failure,
                            ..
                        } if terminal_error == is_error
                            && terminal_content == content
                            && terminal_failure == error
                            && tool.tool_name() == tool_name => {}
                        ToolExecutionState::NotStarted
                        | ToolExecutionState::Started { .. }
                        | ToolExecutionState::Interrupted { .. }
                        | ToolExecutionState::Finished { .. } => {
                            return Err(SessionReplayError::InvalidTransition {
                                transition: "tool_result_before_matching_terminal",
                            });
                        }
                    }
                    commit_tool_result(tool, id);
                }
                state.messages.push(message.clone());
                Ok(())
            }
            SessionRecord::ConfigurationChanged {
                model,
                profile_id,
                reasoning_effort,
            } => {
                if let Some(model) = model {
                    set_model(&mut state.configuration, model.clone());
                }
                if let Some(profile_id) = profile_id {
                    set_profile(&mut state.configuration, profile_id.clone());
                }
                if let Some(reasoning_effort) = reasoning_effort {
                    set_reasoning_effort(&mut state.configuration, *reasoning_effort);
                }
                Ok(())
            }
            SessionRecord::ToolCallRequested {
                tool_call_id,
                tool_name,
                arguments,
            } => {
                let visible_on_branch = state.active_branch_id.is_none_or(|branch_id| {
                    self.branch_heads
                        .get(&branch_id)
                        .is_some_and(|head| head.declared_tool_calls.contains(tool_call_id))
                });
                let declared = self
                    .declared_tool_calls
                    .get(tool_call_id)
                    .filter(|_| visible_on_branch)
                    .ok_or(SessionReplayError::InvalidReference {
                        reference: "undeclared_tool_call",
                    })?;
                if declared.tool_name != *tool_name || declared.arguments != *arguments {
                    return Err(SessionReplayError::InvalidReference {
                        reference: "tool_call_declaration_mismatch",
                    });
                }
                if !self.requested_tool_call_ids.insert(*tool_call_id)
                    || state.tool_calls.contains_key(tool_call_id)
                {
                    return Err(SessionReplayError::DuplicateEntity {
                        entity: "tool_call_request",
                    });
                }
                state.tool_calls.insert(
                    *tool_call_id,
                    new_tool_call(*tool_call_id, tool_name.clone(), arguments.clone()),
                );
                Ok(())
            }
            SessionRecord::PolicyDecisionRecorded {
                tool_call_id,
                decision,
            } => {
                let tool = state.tool_calls.get_mut(tool_call_id).ok_or(
                    SessionReplayError::InvalidReference {
                        reference: "policy_tool_call",
                    },
                )?;
                if tool.policy_decision().is_some() {
                    return Err(SessionReplayError::InvalidTransition {
                        transition: "duplicate_policy_decision",
                    });
                }
                set_policy(tool, *decision);
                Ok(())
            }
            SessionRecord::ApprovalRequested {
                approval_id,
                tool_call_id,
                expires_at,
            } => {
                if !self.approval_ids.insert(*approval_id) {
                    return Err(SessionReplayError::DuplicateEntity { entity: "approval" });
                }
                let tool = state.tool_calls.get_mut(tool_call_id).ok_or(
                    SessionReplayError::InvalidReference {
                        reference: "approval_tool_call",
                    },
                )?;
                if tool.policy_decision() != Some(tea_protocol::PolicyDecision::RequireApproval)
                    || tool.approval_id().is_some()
                    || *expires_at <= envelope.timestamp()
                {
                    return Err(SessionReplayError::InvalidTransition {
                        transition: "approval_request",
                    });
                }
                set_approval(tool, *approval_id);
                state.pending_approvals.insert(
                    *approval_id,
                    new_pending_approval(
                        *approval_id,
                        *tool_call_id,
                        *expires_at,
                        envelope.timestamp(),
                    ),
                );
                Ok(())
            }
            SessionRecord::ApprovalResolved {
                approval_id,
                decision,
            } => {
                let pending = state.pending_approvals.remove(approval_id).ok_or(
                    SessionReplayError::InvalidReference {
                        reference: "pending_approval",
                    },
                )?;
                if envelope.timestamp() < pending.requested_at()
                    || envelope.timestamp() >= pending.expires_at()
                {
                    return Err(SessionReplayError::InvalidTransition {
                        transition: "expired_approval_resolution",
                    });
                }
                let tool = state.tool_calls.get_mut(&pending.tool_call_id()).ok_or(
                    SessionReplayError::InvalidReference {
                        reference: "approval_resolution_tool_call",
                    },
                )?;
                resolve_approval(tool, *decision);
                Ok(())
            }
            SessionRecord::ToolExecutionStarted {
                tool_call_id,
                execution_target,
                idempotency,
            } => {
                let tool = state.tool_calls.get_mut(tool_call_id).ok_or(
                    SessionReplayError::InvalidReference {
                        reference: "execution_tool_call",
                    },
                )?;
                let authorized = match tool.policy_decision() {
                    Some(tea_protocol::PolicyDecision::Allow) => true,
                    Some(tea_protocol::PolicyDecision::RequireApproval) => matches!(
                        tool.approval_decision(),
                        Some(ApprovalDecision::AllowOnce | ApprovalDecision::AllowSession)
                    ),
                    Some(tea_protocol::PolicyDecision::Deny) | None => false,
                };
                if !authorized || !matches!(tool.execution(), ToolExecutionState::NotStarted) {
                    return Err(SessionReplayError::InvalidTransition {
                        transition: "tool_execution_start",
                    });
                }
                start_tool(tool, *execution_target, *idempotency);
                Ok(())
            }
            SessionRecord::ToolExecutionFinished {
                tool_call_id,
                is_error,
                content,
                error,
                presentation,
            } => {
                let tool = state.tool_calls.get_mut(tool_call_id).ok_or(
                    SessionReplayError::InvalidReference {
                        reference: "tool_terminal_tool_call",
                    },
                )?;
                let denied_without_execution =
                    matches!(tool.execution(), ToolExecutionState::NotStarted)
                        && *is_error
                        && (tool.policy_decision() == Some(tea_protocol::PolicyDecision::Deny)
                            || tool.approval_decision() == Some(ApprovalDecision::Deny));
                if !matches!(tool.execution(), ToolExecutionState::Started { .. })
                    && !denied_without_execution
                {
                    return Err(SessionReplayError::InvalidTransition {
                        transition: "tool_execution_finish",
                    });
                }
                finish_tool(
                    tool,
                    *is_error,
                    content.clone(),
                    error.clone(),
                    presentation.clone(),
                );
                Ok(())
            }
            SessionRecord::ToolExecutionInterrupted {
                tool_call_id,
                reason,
            } => {
                let tool = state.tool_calls.get_mut(tool_call_id).ok_or(
                    SessionReplayError::InvalidReference {
                        reference: "tool_interruption_tool_call",
                    },
                )?;
                if !matches!(tool.execution(), ToolExecutionState::Started { .. }) {
                    return Err(SessionReplayError::InvalidTransition {
                        transition: "tool_execution_interruption",
                    });
                }
                interrupt_tool(tool, reason.clone());
                Ok(())
            }
            SessionRecord::RunInterrupted {
                run_id,
                turn_id,
                reason,
            } => {
                if state.run_recovery.contains_key(run_id) {
                    return Err(SessionReplayError::InvalidTransition {
                        transition: "duplicate_run_terminal",
                    });
                }
                state.run_recovery.insert(
                    *run_id,
                    RunRecoveryState::Interrupted {
                        turn_id: *turn_id,
                        reason: reason.clone(),
                    },
                );
                Ok(())
            }
            SessionRecord::RunCancelled { run_id } => {
                if state
                    .run_recovery
                    .insert(*run_id, RunRecoveryState::Cancelled)
                    .is_some()
                {
                    return Err(SessionReplayError::InvalidTransition {
                        transition: "duplicate_run_terminal",
                    });
                }
                Ok(())
            }
            SessionRecord::BranchCreated { .. } | SessionRecord::ActiveBranchChanged { .. } => {
                Err(SessionReplayError::InvalidTransition {
                    transition: "branching_not_applied",
                })
            }
            SessionRecord::SessionCompacted {
                summary,
                compacted_through_record_id,
            } => {
                if !matches!(summary, CanonicalMessage::Assistant { .. }) {
                    return Err(SessionReplayError::InvalidTransition {
                        transition: "compaction_summary_role",
                    });
                }
                let source_is_active = state.active_branch_id.map_or_else(
                    || self.record_ids.contains(compacted_through_record_id),
                    |branch_id| {
                        self.branch_heads
                            .get(&branch_id)
                            .is_some_and(|head| head.history.contains(compacted_through_record_id))
                    },
                );
                if !source_is_active {
                    return Err(SessionReplayError::InvalidReference {
                        reference: "compaction_source",
                    });
                }
                let id = message_id(summary);
                if !self.message_ids.insert(id) {
                    return Err(SessionReplayError::DuplicateEntity { entity: "message" });
                }
                set_compaction(state, summary.clone(), *compacted_through_record_id);
                Ok(())
            }
            SessionRecord::TurnCheckpointed {
                run_id,
                turn_id,
                next_action,
            } => {
                state.latest_checkpoint = Some(TurnCheckpoint::new(
                    *run_id,
                    *turn_id,
                    envelope.record_id(),
                    envelope.sequence(),
                    *next_action,
                ));
                Ok(())
            }
        }
    }
}
