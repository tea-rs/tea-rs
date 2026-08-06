use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::content::validate_tool_name;
use crate::envelope::{deserialize_unique_value, validate_read_version};
use crate::{
    ApprovalDecision, ApprovalId, BranchId, CURRENT_PROTOCOL_VERSION, CanonicalMessage,
    CausationId, ContentBlock, CorrelationId, ProfileId, ProtocolError, ProtocolMetadata,
    ProtocolTimestamp, ProtocolVersion, RecordId, RunId, SessionId, SessionSequence, ToolCallId,
    ToolFailure, ToolPresentation, TurnId,
};

/// Maximum result content blocks stored for one tool execution.
pub const MAX_RECORD_CONTENT_BLOCKS: usize = 256;

/// Stable initial durable record discriminators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionRecordType {
    /// A session was created.
    SessionCreated,
    /// A canonical message became durable.
    MessageCommitted,
    /// Durable model/profile configuration changed.
    ConfigurationChanged,
    /// A complete tool request became durable before execution.
    ToolCallRequested,
    /// A policy decision was recorded.
    PolicyDecisionRecorded,
    /// An approval request became durable.
    ApprovalRequested,
    /// A pending approval reached a terminal decision.
    ApprovalResolved,
    /// Tool execution began and may have uncertain recovery state.
    ToolExecutionStarted,
    /// Tool execution reached a durable terminal result.
    ToolExecutionFinished,
    /// Started tool execution was interrupted with uncertain outcome.
    ToolExecutionInterrupted,
    /// A provider run was interrupted before terminal output was durable.
    RunInterrupted,
    /// A run was explicitly cancelled.
    RunCancelled,
    /// A new durable branch was created.
    BranchCreated,
    /// The active durable branch changed.
    ActiveBranchChanged,
    /// Compaction summary and provenance became durable.
    SessionCompacted,
    /// A turn reached the durable boundary required before its next action.
    TurnCheckpointed,
}

impl SessionRecordType {
    /// All initial protocol 1.0 durable record kinds.
    pub const ALL: [Self; 16] = [
        Self::SessionCreated,
        Self::MessageCommitted,
        Self::ConfigurationChanged,
        Self::ToolCallRequested,
        Self::PolicyDecisionRecorded,
        Self::ApprovalRequested,
        Self::ApprovalResolved,
        Self::ToolExecutionStarted,
        Self::ToolExecutionFinished,
        Self::ToolExecutionInterrupted,
        Self::RunInterrupted,
        Self::RunCancelled,
        Self::BranchCreated,
        Self::ActiveBranchChanged,
        Self::SessionCompacted,
        Self::TurnCheckpointed,
    ];

    /// Returns whether replay must understand this record kind.
    #[must_use]
    pub const fn is_required_for_replay(self) -> bool {
        true
    }
}

/// Policy outcome persisted before an approval or execution transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecision {
    /// Policy allows execution without interactive approval.
    Allow,
    /// Policy denies execution.
    Deny,
    /// Policy requires an approval decision.
    RequireApproval,
}

/// Tool executor boundary used for a durable invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTarget {
    /// In-process or operating-system-native executor.
    Native,
    /// Model Context Protocol executor.
    Mcp,
    /// Remote product-defined executor.
    Remote,
}

/// Declared retry semantics of a tool execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolIdempotency {
    /// Repeating the invocation is expected to have the same external effect.
    Idempotent,
    /// Repeating the invocation may duplicate external effects.
    NonIdempotent,
    /// Executor can reconcile an operation by a durable external key.
    ExternallyReconciled,
}

/// Kernel action allowed after a durable turn checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NextTurnAction {
    /// Begin another model request.
    ModelRequest,
    /// Wait for an approval resolution.
    WaitForApproval,
    /// End the current run.
    FinishRun,
}

/// A typed durable fact used for deterministic session replay.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionRecord {
    /// A session was created with its initial profile and extension metadata.
    SessionCreated {
        /// Initial product profile.
        profile_id: ProfileId,
        /// Bounded session metadata.
        metadata: ProtocolMetadata,
    },
    /// A canonical message was committed.
    MessageCommitted {
        /// Durable canonical message.
        message: CanonicalMessage,
    },
    /// Model or profile configuration changed.
    ConfigurationChanged {
        /// New provider-qualified model when changed.
        model: Option<crate::ModelRef>,
        /// New profile when changed.
        profile_id: Option<ProfileId>,
        /// New provider-neutral reasoning effort when changed.
        reasoning_effort: Option<crate::ReasoningEffort>,
    },
    /// A complete tool call became durable before policy/execution.
    ToolCallRequested {
        /// Canonical tool-call identifier.
        tool_call_id: ToolCallId,
        /// Registered tool name.
        tool_name: String,
        /// Provider-neutral JSON object arguments.
        arguments: Value,
    },
    /// Policy produced a durable decision for a tool call.
    PolicyDecisionRecorded {
        /// Tool call evaluated by policy.
        tool_call_id: ToolCallId,
        /// Stable policy outcome.
        decision: PolicyDecision,
    },
    /// Interactive approval became pending.
    ApprovalRequested {
        /// Approval request identifier.
        approval_id: ApprovalId,
        /// Tool call awaiting approval.
        tool_call_id: ToolCallId,
        /// Approval expiry.
        expires_at: ProtocolTimestamp,
    },
    /// Interactive approval reached a terminal decision.
    ApprovalResolved {
        /// Approval request identifier.
        approval_id: ApprovalId,
        /// Stable approval decision.
        decision: ApprovalDecision,
    },
    /// Tool execution started before crossing an uncertain external boundary.
    ToolExecutionStarted {
        /// Tool call being executed.
        tool_call_id: ToolCallId,
        /// Executor boundary.
        execution_target: ExecutionTarget,
        /// Declared recovery/retry semantics.
        idempotency: ToolIdempotency,
    },
    /// Tool execution reached a durable terminal result.
    ToolExecutionFinished {
        /// Tool call that reached a terminal result.
        tool_call_id: ToolCallId,
        /// Whether the result is a failure.
        is_error: bool,
        /// Canonical display/model result content.
        content: Vec<ContentBlock>,
        /// Machine-readable failure, required exactly when `is_error` is true.
        error: Option<ToolFailure>,
        /// Optional bounded UI-only presentation retained out of model context.
        presentation: Option<ToolPresentation>,
    },
    /// Started tool execution was interrupted and its outcome is uncertain.
    ToolExecutionInterrupted {
        /// Tool call with uncertain outcome.
        tool_call_id: ToolCallId,
        /// English technical recovery diagnostic.
        reason: String,
    },
    /// Provider streaming stopped before a terminal message was durable.
    RunInterrupted {
        /// Interrupted run.
        run_id: RunId,
        /// Active turn when interruption occurred.
        turn_id: TurnId,
        /// English technical recovery diagnostic.
        reason: String,
    },
    /// An active run was explicitly cancelled.
    RunCancelled {
        /// Cancelled run.
        run_id: RunId,
    },
    /// A new branch was created without rewriting parent history.
    BranchCreated {
        /// Source branch.
        source_branch_id: BranchId,
        /// New branch.
        branch_id: BranchId,
        /// Source record position.
        from_record_id: RecordId,
    },
    /// Active branch leaf changed durably.
    ActiveBranchChanged {
        /// New active branch.
        branch_id: BranchId,
    },
    /// A compaction summary was committed with source provenance.
    SessionCompacted {
        /// Summary message.
        summary: CanonicalMessage,
        /// Last source record replaced in model context.
        compacted_through_record_id: RecordId,
    },
    /// A turn reached a durable checkpoint before its next action.
    TurnCheckpointed {
        /// Current run.
        run_id: RunId,
        /// Current turn.
        turn_id: TurnId,
        /// Action allowed after this durable boundary.
        next_action: NextTurnAction,
    },
}

impl SessionRecord {
    /// Returns the stable durable record discriminator.
    #[must_use]
    pub const fn record_type(&self) -> SessionRecordType {
        match self {
            Self::SessionCreated { .. } => SessionRecordType::SessionCreated,
            Self::MessageCommitted { .. } => SessionRecordType::MessageCommitted,
            Self::ConfigurationChanged { .. } => SessionRecordType::ConfigurationChanged,
            Self::ToolCallRequested { .. } => SessionRecordType::ToolCallRequested,
            Self::PolicyDecisionRecorded { .. } => SessionRecordType::PolicyDecisionRecorded,
            Self::ApprovalRequested { .. } => SessionRecordType::ApprovalRequested,
            Self::ApprovalResolved { .. } => SessionRecordType::ApprovalResolved,
            Self::ToolExecutionStarted { .. } => SessionRecordType::ToolExecutionStarted,
            Self::ToolExecutionFinished { .. } => SessionRecordType::ToolExecutionFinished,
            Self::ToolExecutionInterrupted { .. } => SessionRecordType::ToolExecutionInterrupted,
            Self::RunInterrupted { .. } => SessionRecordType::RunInterrupted,
            Self::RunCancelled { .. } => SessionRecordType::RunCancelled,
            Self::BranchCreated { .. } => SessionRecordType::BranchCreated,
            Self::ActiveBranchChanged { .. } => SessionRecordType::ActiveBranchChanged,
            Self::SessionCompacted { .. } => SessionRecordType::SessionCompacted,
            Self::TurnCheckpointed { .. } => SessionRecordType::TurnCheckpointed,
        }
    }

    fn validate(&self) -> Result<(), RecordValidationError> {
        match self {
            Self::ConfigurationChanged {
                model,
                profile_id,
                reasoning_effort,
            } if model.is_none() && profile_id.is_none() && reasoning_effort.is_none() => {
                Err(RecordValidationError::EmptyConfigurationChange)
            }
            Self::ToolCallRequested {
                tool_name,
                arguments,
                ..
            } => {
                validate_tool_name(tool_name)?;
                if !arguments.is_object() {
                    return Err(RecordValidationError::ToolArgumentsMustBeObject);
                }
                crate::metadata::validate_json_bounds(
                    arguments,
                    crate::MAX_TOOL_ARGUMENT_BYTES,
                    crate::MAX_TOOL_ARGUMENT_DEPTH,
                )?;
                Ok(())
            }
            Self::ToolExecutionFinished {
                is_error,
                content,
                error,
                presentation,
                ..
            } => {
                validate_result_content(content)?;
                if *is_error != error.is_some() {
                    return Err(RecordValidationError::InconsistentToolFailure);
                }
                if *is_error && presentation.is_some() {
                    return Err(RecordValidationError::PresentationOnFailure);
                }
                Ok(())
            }
            Self::ToolExecutionInterrupted { reason, .. } | Self::RunInterrupted { reason, .. } => {
                validate_reason(reason)
            }
            Self::SessionCreated { .. }
            | Self::MessageCommitted { .. }
            | Self::ConfigurationChanged { .. }
            | Self::PolicyDecisionRecorded { .. }
            | Self::ApprovalRequested { .. }
            | Self::ApprovalResolved { .. }
            | Self::ToolExecutionStarted { .. }
            | Self::RunCancelled { .. }
            | Self::BranchCreated { .. }
            | Self::ActiveBranchChanged { .. }
            | Self::SessionCompacted { .. }
            | Self::TurnCheckpointed { .. } => Ok(()),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(
    remote = "SessionRecord",
    tag = "type",
    content = "payload",
    rename_all = "snake_case"
)]
enum SessionRecordDef {
    SessionCreated {
        #[serde(rename = "profileId")]
        profile_id: ProfileId,
        #[serde(default, skip_serializing_if = "ProtocolMetadata::is_empty")]
        metadata: ProtocolMetadata,
    },
    MessageCommitted {
        message: CanonicalMessage,
    },
    ConfigurationChanged {
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<crate::ModelRef>,
        #[serde(rename = "profileId", skip_serializing_if = "Option::is_none")]
        profile_id: Option<ProfileId>,
        #[serde(
            rename = "reasoningEffort",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        reasoning_effort: Option<crate::ReasoningEffort>,
    },
    ToolCallRequested {
        #[serde(rename = "toolCallId")]
        tool_call_id: ToolCallId,
        #[serde(rename = "toolName")]
        tool_name: String,
        arguments: Value,
    },
    PolicyDecisionRecorded {
        #[serde(rename = "toolCallId")]
        tool_call_id: ToolCallId,
        decision: PolicyDecision,
    },
    ApprovalRequested {
        #[serde(rename = "approvalId")]
        approval_id: ApprovalId,
        #[serde(rename = "toolCallId")]
        tool_call_id: ToolCallId,
        #[serde(rename = "expiresAt")]
        expires_at: ProtocolTimestamp,
    },
    ApprovalResolved {
        #[serde(rename = "approvalId")]
        approval_id: ApprovalId,
        decision: ApprovalDecision,
    },
    ToolExecutionStarted {
        #[serde(rename = "toolCallId")]
        tool_call_id: ToolCallId,
        #[serde(rename = "executionTarget")]
        execution_target: ExecutionTarget,
        idempotency: ToolIdempotency,
    },
    ToolExecutionFinished {
        #[serde(rename = "toolCallId")]
        tool_call_id: ToolCallId,
        #[serde(rename = "isError")]
        is_error: bool,
        content: Vec<ContentBlock>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<ToolFailure>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        presentation: Option<ToolPresentation>,
    },
    ToolExecutionInterrupted {
        #[serde(rename = "toolCallId")]
        tool_call_id: ToolCallId,
        reason: String,
    },
    RunInterrupted {
        #[serde(rename = "runId")]
        run_id: RunId,
        #[serde(rename = "turnId")]
        turn_id: TurnId,
        reason: String,
    },
    RunCancelled {
        #[serde(rename = "runId")]
        run_id: RunId,
    },
    BranchCreated {
        #[serde(rename = "sourceBranchId")]
        source_branch_id: BranchId,
        #[serde(rename = "branchId")]
        branch_id: BranchId,
        #[serde(rename = "fromRecordId")]
        from_record_id: RecordId,
    },
    ActiveBranchChanged {
        #[serde(rename = "branchId")]
        branch_id: BranchId,
    },
    SessionCompacted {
        summary: CanonicalMessage,
        #[serde(rename = "compactedThroughRecordId")]
        compacted_through_record_id: RecordId,
    },
    TurnCheckpointed {
        #[serde(rename = "runId")]
        run_id: RunId,
        #[serde(rename = "turnId")]
        turn_id: TurnId,
        #[serde(rename = "nextAction")]
        next_action: NextTurnAction,
    },
}

impl Serialize for SessionRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(serde::ser::Error::custom)?;
        SessionRecordDef::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for SessionRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let record = SessionRecordDef::deserialize(deserializer)?;
        record.validate().map_err(serde::de::Error::custom)?;
        Ok(record)
    }
}

/// Versioned durable session-record envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordEnvelope {
    protocol_version: ProtocolVersion,
    record_id: RecordId,
    session_id: SessionId,
    sequence: SessionSequence,
    timestamp: ProtocolTimestamp,
    causation_id: Option<CausationId>,
    correlation_id: Option<CorrelationId>,
    branch_id: Option<BranchId>,
    metadata: ProtocolMetadata,
    record: SessionRecord,
}

impl RecordEnvelope {
    /// Creates a validated current-version durable record envelope.
    ///
    /// # Errors
    ///
    /// Returns an error when the record payload or branch references are invalid.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        record_id: RecordId,
        session_id: SessionId,
        sequence: SessionSequence,
        timestamp: ProtocolTimestamp,
        causation_id: Option<CausationId>,
        correlation_id: Option<CorrelationId>,
        branch_id: Option<BranchId>,
        metadata: ProtocolMetadata,
        record: SessionRecord,
    ) -> Result<Self, RecordValidationError> {
        let envelope = Self {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            record_id,
            session_id,
            sequence,
            timestamp,
            causation_id,
            correlation_id,
            branch_id,
            metadata,
            record,
        };
        envelope.validate()?;
        Ok(envelope)
    }

    /// Decodes untrusted JSON while preserving unsupported-record classification.
    ///
    /// # Errors
    ///
    /// Returns [`RecordDecodeError::UnsupportedType`] for an unknown canonical
    /// discriminator and [`RecordDecodeError::Invalid`] for malformed data.
    pub fn decode_value(value: Value) -> Result<Self, RecordDecodeError> {
        let version = decode_version(&value).map_err(RecordDecodeError::Invalid)?;
        if validate_read_version(version).is_err() {
            return Err(RecordDecodeError::UnsupportedVersion { version });
        }
        let discriminator = value
            .as_object()
            .and_then(|object| object.get("type"))
            .and_then(Value::as_str)
            .ok_or_else(|| RecordDecodeError::Invalid("missing record type".to_owned()))?
            .to_owned();
        if SessionRecordTypeText::from_str(&discriminator).is_err() {
            if valid_discriminator(&discriminator) {
                return Err(RecordDecodeError::UnsupportedType {
                    record_type: discriminator,
                });
            }
            return Err(RecordDecodeError::Invalid("invalid record type".to_owned()));
        }
        serde_json::from_value(value).map_err(|error| RecordDecodeError::Invalid(error.to_string()))
    }

    /// Returns the protocol version.
    #[must_use]
    pub const fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    /// Returns the record identifier.
    #[must_use]
    pub const fn record_id(&self) -> RecordId {
        self.record_id
    }

    /// Returns the owning session.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Returns the authoritative replay sequence.
    #[must_use]
    pub const fn sequence(&self) -> SessionSequence {
        self.sequence
    }

    /// Returns the record timestamp.
    #[must_use]
    pub const fn timestamp(&self) -> ProtocolTimestamp {
        self.timestamp
    }

    /// Returns the optional immediate cause.
    #[must_use]
    pub const fn causation_id(&self) -> Option<CausationId> {
        self.causation_id
    }

    /// Returns the optional operation correlation identifier.
    #[must_use]
    pub const fn correlation_id(&self) -> Option<CorrelationId> {
        self.correlation_id
    }

    /// Returns the branch receiving this fact when branch-scoped.
    #[must_use]
    pub const fn branch_id(&self) -> Option<BranchId> {
        self.branch_id
    }

    /// Returns bounded extension metadata.
    #[must_use]
    pub const fn metadata(&self) -> &ProtocolMetadata {
        &self.metadata
    }

    /// Returns the typed durable fact.
    #[must_use]
    pub const fn record(&self) -> &SessionRecord {
        &self.record
    }

    /// Returns the stable durable record discriminator.
    #[must_use]
    pub const fn record_type(&self) -> SessionRecordType {
        self.record.record_type()
    }

    fn validate(&self) -> Result<(), RecordValidationError> {
        self.record.validate()?;
        match &self.record {
            SessionRecord::BranchCreated { branch_id, .. }
            | SessionRecord::ActiveBranchChanged { branch_id }
                if self.branch_id != Some(*branch_id) =>
            {
                Err(RecordValidationError::BranchReferenceMismatch)
            }
            _ => Ok(()),
        }
    }
}

impl Serialize for RecordEnvelope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(serde::ser::Error::custom)?;
        let mut value = serde_json::to_value(&self.record).map_err(serde::ser::Error::custom)?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| serde::ser::Error::custom("record must encode as object"))?;
        object.insert("protocolVersion".to_owned(), json!(self.protocol_version));
        object.insert("recordId".to_owned(), json!(self.record_id));
        object.insert("sessionId".to_owned(), json!(self.session_id));
        object.insert("sequence".to_owned(), json!(self.sequence));
        object.insert("timestamp".to_owned(), json!(self.timestamp));
        if let Some(causation_id) = self.causation_id {
            object.insert("causationId".to_owned(), json!(causation_id));
        }
        if let Some(correlation_id) = self.correlation_id {
            object.insert("correlationId".to_owned(), json!(correlation_id));
        }
        if let Some(branch_id) = self.branch_id {
            object.insert("branchId".to_owned(), json!(branch_id));
        }
        if !self.metadata.is_empty() {
            object.insert("metadata".to_owned(), json!(self.metadata));
        }
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RecordEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut value = deserialize_unique_value(deserializer)?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| serde::de::Error::custom("record envelope must be an object"))?;
        let protocol_version = take(object, "protocolVersion").map_err(serde::de::Error::custom)?;
        validate_read_version(protocol_version).map_err(serde::de::Error::custom)?;
        let envelope = Self {
            protocol_version,
            record_id: take(object, "recordId").map_err(serde::de::Error::custom)?,
            session_id: take(object, "sessionId").map_err(serde::de::Error::custom)?,
            sequence: take(object, "sequence").map_err(serde::de::Error::custom)?,
            timestamp: take(object, "timestamp").map_err(serde::de::Error::custom)?,
            causation_id: take_optional(object, "causationId").map_err(serde::de::Error::custom)?,
            correlation_id: take_optional(object, "correlationId")
                .map_err(serde::de::Error::custom)?,
            branch_id: take_optional(object, "branchId").map_err(serde::de::Error::custom)?,
            metadata: take_optional(object, "metadata")
                .map_err(serde::de::Error::custom)?
                .unwrap_or_default(),
            record: SessionRecord::deserialize(Value::Object(std::mem::take(object)))
                .map_err(serde::de::Error::custom)?,
        };
        envelope.validate().map_err(serde::de::Error::custom)?;
        Ok(envelope)
    }
}

/// Failure while decoding an untrusted durable record.
#[derive(Debug, Error)]
pub enum RecordDecodeError {
    /// The protocol major is unsupported and takes precedence over record type.
    #[error("unsupported protocol version: {version}")]
    UnsupportedVersion {
        /// Received canonical protocol version.
        version: ProtocolVersion,
    },
    /// Replay cannot safely understand this required record kind.
    #[error("unsupported required record type: {record_type}")]
    UnsupportedType {
        /// Bounded canonical unknown discriminator.
        record_type: String,
    },
    /// The known record or envelope is malformed.
    #[error("invalid durable record: {0}")]
    Invalid(String),
}

impl RecordDecodeError {
    /// Converts a decode failure to a safe protocol error.
    #[must_use]
    pub fn into_protocol_error(self, correlation_id: CorrelationId) -> ProtocolError {
        match self {
            Self::UnsupportedVersion { version } => {
                ProtocolError::unsupported_protocol_version(correlation_id, version)
            }
            Self::UnsupportedType { record_type } => {
                ProtocolError::unsupported_record(correlation_id, &record_type)
            }
            Self::Invalid(_) => ProtocolError::invalid_record(correlation_id),
        }
    }
}

/// Error returned when validating a durable record payload.
#[derive(Debug, Error)]
pub enum RecordValidationError {
    /// A configuration record does not change any typed setting.
    #[error("configuration_changed must include modelId or profileId")]
    EmptyConfigurationChange,
    /// Tool name or arguments are invalid.
    #[error("tool call is invalid: {0}")]
    InvalidToolCall(#[from] crate::ContentValidationError),
    /// Tool arguments are not a JSON object.
    #[error("tool arguments must be a JSON object")]
    ToolArgumentsMustBeObject,
    /// Tool arguments exceed protocol JSON bounds.
    #[error("tool arguments exceed protocol bounds: {0}")]
    ToolArgumentsOutOfBounds(#[from] crate::ProtocolMetadataError),
    /// Tool terminal content is empty, excessive, or contains invalid blocks.
    #[error("tool terminal content is invalid")]
    InvalidToolResultContent,
    /// Branch-scoped envelope and payload identifiers disagree.
    #[error("record envelope branchId must match the branch payload")]
    BranchReferenceMismatch,
    /// Tool error flag does not match machine-readable failure presence.
    #[error("tool terminal isError must match error presence")]
    InconsistentToolFailure,
    /// A failed execution cannot claim a successful change presentation.
    #[error("failed tool execution cannot include a presentation")]
    PresentationOnFailure,
    /// Interruption diagnostic is empty, oversized, or contains a null character.
    #[error("interruption reason is invalid")]
    InvalidInterruptionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SessionRecordTypeText;

impl FromStr for SessionRecordTypeText {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if [
            "session_created",
            "message_committed",
            "configuration_changed",
            "tool_call_requested",
            "policy_decision_recorded",
            "approval_requested",
            "approval_resolved",
            "tool_execution_started",
            "tool_execution_finished",
            "tool_execution_interrupted",
            "run_interrupted",
            "run_cancelled",
            "branch_created",
            "active_branch_changed",
            "session_compacted",
            "turn_checkpointed",
        ]
        .contains(&value)
        {
            Ok(Self)
        } else {
            Err(())
        }
    }
}

fn decode_version(value: &Value) -> Result<ProtocolVersion, String> {
    let version = value
        .as_object()
        .and_then(|object| object.get("protocolVersion"))
        .cloned()
        .ok_or_else(|| "missing protocolVersion".to_owned())?;
    serde_json::from_value(version).map_err(|error| error.to_string())
}

fn validate_result_content(content: &[ContentBlock]) -> Result<(), RecordValidationError> {
    if content.is_empty()
        || content.len() > MAX_RECORD_CONTENT_BLOCKS
        || !content.iter().all(ContentBlock::valid_for_tool_result)
    {
        return Err(RecordValidationError::InvalidToolResultContent);
    }
    for block in content {
        block
            .validate()
            .map_err(RecordValidationError::InvalidToolCall)?;
    }
    Ok(())
}

fn validate_reason(reason: &str) -> Result<(), RecordValidationError> {
    if reason.is_empty() || reason.len() > 4096 || reason.contains('\0') {
        Err(RecordValidationError::InvalidInterruptionReason)
    } else {
        Ok(())
    }
}

fn valid_discriminator(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn take<T>(object: &mut Map<String, Value>, key: &str) -> Result<T, serde_json::Error>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(object.remove(key).unwrap_or(Value::Null))
}

fn take_optional<T>(
    object: &mut Map<String, Value>,
    key: &str,
) -> Result<Option<T>, serde_json::Error>
where
    T: for<'de> Deserialize<'de>,
{
    object.remove(key).map_or(Ok(None), serde_json::from_value)
}
