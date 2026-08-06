use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::envelope::{deserialize_unique_value, validate_read_version};
use crate::metadata::validate_json_bounds;
use crate::{
    ApprovalId, BranchId, CURRENT_PROTOCOL_VERSION, ContentBlock, EventId, ExactCost,
    HostedToolOutcome, MAX_HOSTED_TOOL_SOURCES, MessageId, ProtocolMetadata, ProtocolTimestamp,
    ProtocolVersion, RunId, SessionId, SessionSequence, ToolCallId, ToolPresentation, TurnId,
    Usage,
};

/// Maximum UTF-8 bytes in one streaming delta.
pub const MAX_EVENT_DELTA_BYTES: usize = 64 * 1024;
/// Maximum UTF-8 bytes in one progress diagnostic.
pub const MAX_PROGRESS_MESSAGE_BYTES: usize = 4096;
/// Maximum capabilities or resources in one approval observation.
pub const MAX_APPROVAL_ITEMS: usize = 64;
/// Maximum UTF-8 bytes in one capability or resource string.
pub const MAX_APPROVAL_ITEM_BYTES: usize = 1024;
/// Maximum encoded JSON bytes retained while inspecting an unknown event.
pub const MAX_UNKNOWN_EVENT_BYTES: usize = 64 * 1024;

/// Compatibility classification for observable events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventCompatibility {
    /// Older observers may skip the event with a diagnostic.
    SkippableObservation,
    /// The event carries lifecycle or state information and cannot be skipped.
    RequiredStateBearing,
}

/// Stable known event discriminators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEventType {
    /// A model/tool run started.
    RunStarted,
    /// A bounded streaming message fragment was produced.
    MessageDelta,
    /// A complete canonical tool call was requested.
    ToolCallRequested,
    /// Policy requires an approval decision.
    ApprovalRequested,
    /// A tool emitted non-durable progress.
    ToolExecutionProgress,
    /// A tool produced a non-durable execution preview.
    ToolExecutionPreview,
    /// A provider-hosted tool started executing inside the model response.
    HostedToolStarted,
    /// A provider-hosted tool completed inside the model response.
    HostedToolCompleted,
    /// A retryable model failure entered bounded backoff.
    ModelRetryScheduled,
    /// A scheduled model retry started its next provider request.
    ModelRetryStarted,
    /// A turn reached a durable checkpoint.
    TurnCheckpointed,
    /// A session compaction was committed.
    SessionCompacted,
    /// A session branch was forked.
    SessionForked,
    /// A run reached a terminal state.
    RunFinished,
}

impl AgentEventType {
    /// All currently supported event types.
    pub const ALL: [Self; 14] = [
        Self::RunStarted,
        Self::MessageDelta,
        Self::ToolCallRequested,
        Self::ApprovalRequested,
        Self::ToolExecutionProgress,
        Self::ToolExecutionPreview,
        Self::HostedToolStarted,
        Self::HostedToolCompleted,
        Self::ModelRetryScheduled,
        Self::ModelRetryStarted,
        Self::TurnCheckpointed,
        Self::SessionCompacted,
        Self::SessionForked,
        Self::RunFinished,
    ];

    /// Returns whether an older observer may skip this event kind.
    #[must_use]
    pub const fn compatibility(self) -> EventCompatibility {
        match self {
            Self::MessageDelta
            | Self::ToolExecutionProgress
            | Self::ToolExecutionPreview
            | Self::HostedToolStarted
            | Self::HostedToolCompleted
            | Self::ModelRetryScheduled
            | Self::ModelRetryStarted => EventCompatibility::SkippableObservation,
            Self::RunStarted
            | Self::ToolCallRequested
            | Self::ApprovalRequested
            | Self::TurnCheckpointed
            | Self::SessionCompacted
            | Self::SessionForked
            | Self::RunFinished => EventCompatibility::RequiredStateBearing,
        }
    }
}

/// A bounded streaming content delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventDelta {
    /// Visible text fragment.
    TextDelta {
        /// Bounded UTF-8 fragment.
        text: String,
    },
    /// Reasoning text fragment.
    ThinkingDelta {
        /// Bounded UTF-8 fragment.
        text: String,
    },
}

impl EventDelta {
    fn validate(&self) -> Result<(), EventValidationError> {
        let text = match self {
            Self::TextDelta { text } | Self::ThinkingDelta { text } => text,
        };
        if text.is_empty() || text.len() > MAX_EVENT_DELTA_BYTES || text.contains('\0') {
            Err(EventValidationError::InvalidDelta)
        } else {
            Ok(())
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "EventDelta", tag = "type", rename_all = "snake_case")]
enum EventDeltaDef {
    TextDelta { text: String },
    ThinkingDelta { text: String },
}

impl Serialize for EventDelta {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(serde::ser::Error::custom)?;
        EventDeltaDef::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for EventDelta {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let delta = EventDeltaDef::deserialize(deserializer)?;
        delta.validate().map_err(serde::de::Error::custom)?;
        Ok(delta)
    }
}

/// Terminal status of an observable run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The run completed normally.
    Completed,
    /// The run was cancelled.
    Cancelled,
    /// The run failed with a known error.
    Failed,
    /// The process stopped before a provider/tool operation completed.
    Interrupted,
}

/// A provider- and UI-neutral observable runtime event.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    /// A run started.
    RunStarted {},
    /// A streaming message fragment was produced.
    MessageDelta {
        /// Message receiving the fragment.
        message_id: MessageId,
        /// Zero-based content-block index.
        content_index: u32,
        /// Bounded content fragment.
        delta: EventDelta,
    },
    /// A complete tool call was requested by the model.
    ToolCallRequested {
        /// Canonical tool-call identifier.
        tool_call_id: ToolCallId,
        /// Registered tool name.
        tool_name: String,
        /// Validated JSON object arguments.
        arguments: Value,
    },
    /// A tool call requires approval.
    ApprovalRequested {
        /// Approval request identifier.
        approval_id: ApprovalId,
        /// Tool call awaiting approval.
        tool_call_id: ToolCallId,
        /// Bounded required capability names.
        capabilities: Vec<String>,
        /// Bounded affected resource identifiers.
        resources: Vec<String>,
        /// Decision deadline.
        expires_at: ProtocolTimestamp,
    },
    /// Non-durable tool progress suitable for UI display.
    ToolExecutionProgress {
        /// Tool call reporting progress.
        tool_call_id: ToolCallId,
        /// English technical progress diagnostic.
        message: String,
        /// Completed work units.
        completed_units: u64,
        /// Total work units when known.
        total_units: Option<u64>,
    },
    /// A trusted tool computed a bounded preview before a pending approval.
    ToolExecutionPreview {
        /// Tool call the preview describes.
        tool_call_id: ToolCallId,
        /// Typed UI-only data produced from the tool's current workspace view.
        presentation: ToolPresentation,
    },
    /// A provider-hosted tool started inside the active model response.
    HostedToolStarted {
        /// Kernel-owned activity identifier.
        tool_call_id: ToolCallId,
        /// Stable hosted tool name.
        tool_name: String,
    },
    /// A provider-hosted tool completed inside the active model response.
    HostedToolCompleted {
        /// Kernel-owned activity identifier.
        tool_call_id: ToolCallId,
        /// Stable hosted tool name.
        tool_name: String,
        /// Validated provider-neutral arguments, without continuation state.
        arguments: Value,
        /// Normalized terminal outcome.
        outcome: HostedToolOutcome,
        /// Number of normalized sources retained in the eventual durable block.
        source_count: u32,
    },
    /// A retryable model failure is waiting before another provider request.
    ModelRetryScheduled {
        /// Ephemeral assistant message whose failed partial output is discarded.
        message_id: MessageId,
        /// One-based retry number.
        attempt: u32,
        /// Maximum retry count, excluding the initial request.
        max_retries: u32,
        /// Selected bounded delay in milliseconds.
        delay_ms: u64,
    },
    /// A scheduled retry completed its backoff and is starting another request.
    ModelRetryStarted {
        /// Ephemeral assistant message reused by the next attempt.
        message_id: MessageId,
        /// One-based retry number.
        attempt: u32,
        /// Maximum retry count, excluding the initial request.
        max_retries: u32,
    },
    /// A turn reached a durable checkpoint.
    TurnCheckpointed {},
    /// Session history was compacted through a source message.
    SessionCompacted {
        /// Summary message introduced by compaction.
        summary_message_id: MessageId,
        /// Last source message covered by the summary.
        compacted_through_message_id: MessageId,
    },
    /// A new branch was forked from an existing branch/message.
    SessionForked {
        /// Source branch identifier.
        source_branch_id: BranchId,
        /// New branch identifier.
        branch_id: BranchId,
        /// Message at the fork point.
        from_message_id: MessageId,
    },
    /// A run reached a terminal state.
    RunFinished {
        /// Terminal run status.
        status: RunStatus,
        /// Provider-neutral usage when available.
        usage: Option<Usage>,
        /// Exact billable cost when available.
        cost: Option<ExactCost>,
    },
}

impl AgentEvent {
    /// Returns the stable event discriminator.
    #[must_use]
    pub const fn event_type(&self) -> AgentEventType {
        match self {
            Self::RunStarted {} => AgentEventType::RunStarted,
            Self::MessageDelta { .. } => AgentEventType::MessageDelta,
            Self::ToolCallRequested { .. } => AgentEventType::ToolCallRequested,
            Self::ApprovalRequested { .. } => AgentEventType::ApprovalRequested,
            Self::ToolExecutionProgress { .. } => AgentEventType::ToolExecutionProgress,
            Self::ToolExecutionPreview { .. } => AgentEventType::ToolExecutionPreview,
            Self::HostedToolStarted { .. } => AgentEventType::HostedToolStarted,
            Self::HostedToolCompleted { .. } => AgentEventType::HostedToolCompleted,
            Self::ModelRetryScheduled { .. } => AgentEventType::ModelRetryScheduled,
            Self::ModelRetryStarted { .. } => AgentEventType::ModelRetryStarted,
            Self::TurnCheckpointed {} => AgentEventType::TurnCheckpointed,
            Self::SessionCompacted { .. } => AgentEventType::SessionCompacted,
            Self::SessionForked { .. } => AgentEventType::SessionForked,
            Self::RunFinished { .. } => AgentEventType::RunFinished,
        }
    }

    fn validate(&self) -> Result<(), EventValidationError> {
        match self {
            Self::MessageDelta { delta, .. } => delta.validate(),
            Self::ToolCallRequested {
                tool_call_id,
                tool_name,
                arguments,
            } => {
                ContentBlock::tool_call(*tool_call_id, tool_name.clone(), arguments.clone())?;
                Ok(())
            }
            Self::ApprovalRequested {
                capabilities,
                resources,
                ..
            } => {
                validate_items(capabilities)?;
                validate_items(resources)
            }
            Self::ToolExecutionProgress {
                message,
                completed_units,
                total_units,
                ..
            } => {
                if message.is_empty()
                    || message.len() > MAX_PROGRESS_MESSAGE_BYTES
                    || message.contains('\0')
                    || *completed_units > crate::MAX_SAFE_INTEGER
                    || total_units.is_some_and(|total| {
                        total > crate::MAX_SAFE_INTEGER || *completed_units > total
                    })
                {
                    return Err(EventValidationError::InvalidProgress);
                }
                Ok(())
            }
            Self::HostedToolStarted {
                tool_call_id,
                tool_name,
            } => {
                ContentBlock::tool_call(*tool_call_id, tool_name.clone(), json!({}))?;
                Ok(())
            }
            Self::HostedToolCompleted {
                tool_call_id,
                tool_name,
                arguments,
                source_count,
                ..
            } => {
                ContentBlock::tool_call(*tool_call_id, tool_name.clone(), arguments.clone())?;
                let Ok(source_count) = usize::try_from(*source_count) else {
                    return Err(EventValidationError::InvalidHostedToolObservation);
                };
                if source_count > MAX_HOSTED_TOOL_SOURCES {
                    return Err(EventValidationError::InvalidHostedToolObservation);
                }
                Ok(())
            }
            Self::ModelRetryScheduled {
                attempt,
                max_retries,
                delay_ms,
                ..
            } => validate_model_retry(*attempt, *max_retries, Some(*delay_ms)),
            Self::ModelRetryStarted {
                attempt,
                max_retries,
                ..
            } => validate_model_retry(*attempt, *max_retries, None),
            Self::ToolExecutionPreview { .. }
            | Self::RunStarted {}
            | Self::TurnCheckpointed {}
            | Self::SessionCompacted { .. }
            | Self::SessionForked { .. }
            | Self::RunFinished { .. } => Ok(()),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(
    remote = "AgentEvent",
    tag = "type",
    content = "payload",
    rename_all = "snake_case"
)]
enum AgentEventDef {
    RunStarted {},
    MessageDelta {
        #[serde(rename = "messageId")]
        message_id: MessageId,
        #[serde(rename = "contentIndex")]
        content_index: u32,
        delta: EventDelta,
    },
    ToolCallRequested {
        #[serde(rename = "toolCallId")]
        tool_call_id: ToolCallId,
        #[serde(rename = "toolName")]
        tool_name: String,
        arguments: Value,
    },
    ApprovalRequested {
        #[serde(rename = "approvalId")]
        approval_id: ApprovalId,
        #[serde(rename = "toolCallId")]
        tool_call_id: ToolCallId,
        capabilities: Vec<String>,
        resources: Vec<String>,
        #[serde(rename = "expiresAt")]
        expires_at: ProtocolTimestamp,
    },
    ToolExecutionProgress {
        #[serde(rename = "toolCallId")]
        tool_call_id: ToolCallId,
        message: String,
        #[serde(rename = "completedUnits")]
        completed_units: u64,
        #[serde(rename = "totalUnits", skip_serializing_if = "Option::is_none")]
        total_units: Option<u64>,
    },
    ToolExecutionPreview {
        #[serde(rename = "toolCallId")]
        tool_call_id: ToolCallId,
        presentation: ToolPresentation,
    },
    HostedToolStarted {
        #[serde(rename = "toolCallId")]
        tool_call_id: ToolCallId,
        #[serde(rename = "toolName")]
        tool_name: String,
    },
    HostedToolCompleted {
        #[serde(rename = "toolCallId")]
        tool_call_id: ToolCallId,
        #[serde(rename = "toolName")]
        tool_name: String,
        arguments: Value,
        outcome: HostedToolOutcome,
        #[serde(rename = "sourceCount")]
        source_count: u32,
    },
    ModelRetryScheduled {
        #[serde(rename = "messageId")]
        message_id: MessageId,
        attempt: u32,
        #[serde(rename = "maxRetries")]
        max_retries: u32,
        #[serde(rename = "delayMs")]
        delay_ms: u64,
    },
    ModelRetryStarted {
        #[serde(rename = "messageId")]
        message_id: MessageId,
        attempt: u32,
        #[serde(rename = "maxRetries")]
        max_retries: u32,
    },
    TurnCheckpointed {},
    SessionCompacted {
        #[serde(rename = "summaryMessageId")]
        summary_message_id: MessageId,
        #[serde(rename = "compactedThroughMessageId")]
        compacted_through_message_id: MessageId,
    },
    SessionForked {
        #[serde(rename = "sourceBranchId")]
        source_branch_id: BranchId,
        #[serde(rename = "branchId")]
        branch_id: BranchId,
        #[serde(rename = "fromMessageId")]
        from_message_id: MessageId,
    },
    RunFinished {
        status: RunStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost: Option<ExactCost>,
    },
}

impl Serialize for AgentEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(serde::ser::Error::custom)?;
        AgentEventDef::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for AgentEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let event = AgentEventDef::deserialize(deserializer)?;
        event.validate().map_err(serde::de::Error::custom)?;
        Ok(event)
    }
}

/// A versioned observable event envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct EventEnvelope {
    protocol_version: ProtocolVersion,
    event_id: EventId,
    session_id: SessionId,
    run_id: Option<RunId>,
    turn_id: Option<TurnId>,
    sequence: SessionSequence,
    timestamp: ProtocolTimestamp,
    metadata: ProtocolMetadata,
    event: AgentEvent,
}

impl EventEnvelope {
    /// Creates a validated current-version event envelope.
    ///
    /// # Errors
    ///
    /// Returns an error when references do not match the event kind or the
    /// payload exceeds protocol bounds.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_id: EventId,
        session_id: SessionId,
        run_id: Option<RunId>,
        turn_id: Option<TurnId>,
        sequence: SessionSequence,
        timestamp: ProtocolTimestamp,
        metadata: ProtocolMetadata,
        event: AgentEvent,
    ) -> Result<Self, EventValidationError> {
        let envelope = Self {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            event_id,
            session_id,
            run_id,
            turn_id,
            sequence,
            timestamp,
            metadata,
            event,
        };
        envelope.validate()?;
        Ok(envelope)
    }

    /// Inspects untrusted event JSON with explicit unknown-event behavior.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed known events, unknown state-bearing
    /// events, invalid compatibility markers, or unsupported protocol majors.
    pub fn inspect_value(value: Value) -> Result<EventInspection, EventDecodeError> {
        let discriminator = value
            .as_object()
            .and_then(|object| object.get("type"))
            .and_then(Value::as_str)
            .ok_or_else(|| EventDecodeError::Invalid("missing event type".to_owned()))?
            .to_owned();
        if AgentEventTypeText::from_str(&discriminator).is_ok() {
            return serde_json::from_value(value)
                .map(EventInspection::Known)
                .map_err(|error| EventDecodeError::Invalid(error.to_string()));
        }
        inspect_unknown(&value, discriminator)
    }

    /// Returns the envelope protocol version.
    #[must_use]
    pub const fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    /// Returns the observable event identifier.
    #[must_use]
    pub const fn event_id(&self) -> EventId {
        self.event_id
    }

    /// Returns the session identifier.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Returns the related run identifier when present.
    #[must_use]
    pub const fn run_id(&self) -> Option<RunId> {
        self.run_id
    }

    /// Returns the related turn identifier when present.
    #[must_use]
    pub const fn turn_id(&self) -> Option<TurnId> {
        self.turn_id
    }

    /// Returns the authoritative session-local event sequence.
    #[must_use]
    pub const fn sequence(&self) -> SessionSequence {
        self.sequence
    }

    /// Returns the event timestamp.
    #[must_use]
    pub const fn timestamp(&self) -> ProtocolTimestamp {
        self.timestamp
    }

    /// Returns bounded extension metadata.
    #[must_use]
    pub const fn metadata(&self) -> &ProtocolMetadata {
        &self.metadata
    }

    /// Returns the typed event payload.
    #[must_use]
    pub const fn event(&self) -> &AgentEvent {
        &self.event
    }

    /// Returns the stable event discriminator.
    #[must_use]
    pub const fn event_type(&self) -> AgentEventType {
        self.event.event_type()
    }

    fn validate(&self) -> Result<(), EventValidationError> {
        self.event.validate()?;
        let (requires_run, requires_turn) = match self.event.event_type() {
            AgentEventType::RunStarted | AgentEventType::RunFinished => (true, false),
            AgentEventType::MessageDelta
            | AgentEventType::ToolCallRequested
            | AgentEventType::ApprovalRequested
            | AgentEventType::ToolExecutionProgress
            | AgentEventType::ToolExecutionPreview
            | AgentEventType::HostedToolStarted
            | AgentEventType::HostedToolCompleted
            | AgentEventType::ModelRetryScheduled
            | AgentEventType::ModelRetryStarted
            | AgentEventType::TurnCheckpointed => (true, true),
            AgentEventType::SessionCompacted | AgentEventType::SessionForked => (false, false),
        };
        if self.run_id.is_some() != requires_run || self.turn_id.is_some() != requires_turn {
            return Err(EventValidationError::InvalidReferences);
        }
        Ok(())
    }
}

impl Serialize for EventEnvelope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(serde::ser::Error::custom)?;
        let mut value = serde_json::to_value(&self.event).map_err(serde::ser::Error::custom)?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| serde::ser::Error::custom("event must encode as object"))?;
        object.insert("protocolVersion".to_owned(), json!(self.protocol_version));
        object.insert("eventId".to_owned(), json!(self.event_id));
        object.insert("sessionId".to_owned(), json!(self.session_id));
        if let Some(run_id) = self.run_id {
            object.insert("runId".to_owned(), json!(run_id));
        }
        if let Some(turn_id) = self.turn_id {
            object.insert("turnId".to_owned(), json!(turn_id));
        }
        object.insert("sequence".to_owned(), json!(self.sequence));
        object.insert("timestamp".to_owned(), json!(self.timestamp));
        if self.event_type().compatibility() == EventCompatibility::SkippableObservation {
            object.insert(
                "compatibility".to_owned(),
                json!(EventCompatibility::SkippableObservation),
            );
        }
        if !self.metadata.is_empty() {
            object.insert("metadata".to_owned(), json!(self.metadata));
        }
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EventEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut value = deserialize_unique_value(deserializer)?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| serde::de::Error::custom("event envelope must be an object"))?;
        let protocol_version = take(object, "protocolVersion").map_err(serde::de::Error::custom)?;
        validate_read_version(protocol_version).map_err(serde::de::Error::custom)?;
        let event_id = take(object, "eventId").map_err(serde::de::Error::custom)?;
        let session_id = take(object, "sessionId").map_err(serde::de::Error::custom)?;
        let run_id = take_optional(object, "runId").map_err(serde::de::Error::custom)?;
        let turn_id = take_optional(object, "turnId").map_err(serde::de::Error::custom)?;
        let sequence = take(object, "sequence").map_err(serde::de::Error::custom)?;
        let timestamp = take(object, "timestamp").map_err(serde::de::Error::custom)?;
        let metadata = take_optional(object, "metadata")
            .map_err(serde::de::Error::custom)?
            .unwrap_or_default();
        object.remove("compatibility");
        let event = AgentEvent::deserialize(Value::Object(std::mem::take(object)))
            .map_err(serde::de::Error::custom)?;
        let envelope = Self {
            protocol_version,
            event_id,
            session_id,
            run_id,
            turn_id,
            sequence,
            timestamp,
            metadata,
            event,
        };
        envelope.validate().map_err(serde::de::Error::custom)?;
        Ok(envelope)
    }
}

/// Result of inspecting a known or forward-compatible observable event.
#[derive(Debug, Clone, PartialEq)]
pub enum EventInspection {
    /// A fully understood typed event.
    Known(EventEnvelope),
    /// An explicitly skippable unknown observational event.
    UnknownSkippable(UnknownSkippableEvent),
}

/// Validated common fields retained when skipping an unknown observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownSkippableEvent {
    event_type: String,
    protocol_version: ProtocolVersion,
    event_id: EventId,
    session_id: SessionId,
    run_id: Option<RunId>,
    turn_id: Option<TurnId>,
    sequence: SessionSequence,
    timestamp: ProtocolTimestamp,
}

impl UnknownSkippableEvent {
    /// Returns the unknown canonical discriminator for bounded diagnostics.
    #[must_use]
    pub fn event_type(&self) -> &str {
        &self.event_type
    }

    /// Returns the event's protocol version.
    #[must_use]
    pub const fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    /// Returns the authoritative sequence that observers must still advance.
    #[must_use]
    pub const fn sequence(&self) -> SessionSequence {
        self.sequence
    }

    /// Returns the event identifier.
    #[must_use]
    pub const fn event_id(&self) -> EventId {
        self.event_id
    }

    /// Returns the session identifier.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Returns the related run identifier when present.
    #[must_use]
    pub const fn run_id(&self) -> Option<RunId> {
        self.run_id
    }

    /// Returns the related turn identifier when present.
    #[must_use]
    pub const fn turn_id(&self) -> Option<TurnId> {
        self.turn_id
    }

    /// Returns the event timestamp.
    #[must_use]
    pub const fn timestamp(&self) -> ProtocolTimestamp {
        self.timestamp
    }
}

/// Failure while inspecting an untrusted observable event.
#[derive(Debug, Error)]
pub enum EventDecodeError {
    /// The unknown event is not explicitly safe for observers to skip.
    #[error("unsupported state-bearing event type: {event_type}")]
    UnsupportedStateBearing {
        /// Bounded canonical unknown discriminator.
        event_type: String,
    },
    /// The envelope, payload, or compatibility marker is malformed.
    #[error("invalid event: {0}")]
    Invalid(String),
}

/// Error returned when validating event data.
#[derive(Debug, Error)]
pub enum EventValidationError {
    /// A streaming delta is empty, oversized, or contains a null character.
    #[error("streaming delta is invalid")]
    InvalidDelta,
    /// Tool-call payload validation failed.
    #[error("tool-call event payload is invalid: {0}")]
    InvalidToolCall(#[from] crate::ContentValidationError),
    /// Approval capabilities or resources exceed collection/string bounds.
    #[error("approval event items are invalid")]
    InvalidApprovalItems,
    /// Progress details, units, or unit relationship are invalid.
    #[error("tool progress event is invalid")]
    InvalidProgress,
    /// Hosted-tool observation source count exceeds the durable protocol bound.
    #[error("hosted tool observation is invalid")]
    InvalidHostedToolObservation,
    /// Model retry attempt counts or delay exceed protocol bounds.
    #[error("model retry observation is invalid")]
    InvalidModelRetry,
    /// Run and turn references do not match the event kind.
    #[error("event runId/turnId references do not match the event kind")]
    InvalidReferences,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AgentEventTypeText;

impl FromStr for AgentEventTypeText {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if [
            "run_started",
            "message_delta",
            "tool_call_requested",
            "approval_requested",
            "tool_execution_progress",
            "tool_execution_preview",
            "hosted_tool_started",
            "hosted_tool_completed",
            "model_retry_scheduled",
            "model_retry_started",
            "turn_checkpointed",
            "session_compacted",
            "session_forked",
            "run_finished",
        ]
        .contains(&value)
        {
            Ok(Self)
        } else {
            Err(())
        }
    }
}

fn inspect_unknown(value: &Value, event_type: String) -> Result<EventInspection, EventDecodeError> {
    if !valid_discriminator(&event_type) {
        return Err(EventDecodeError::Invalid("invalid event type".to_owned()));
    }
    validate_json_bounds(value, MAX_UNKNOWN_EVENT_BYTES, 32)
        .map_err(|error| EventDecodeError::Invalid(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| EventDecodeError::Invalid("event must be an object".to_owned()))?;
    let compatibility =
        object
            .get("compatibility")
            .ok_or_else(|| EventDecodeError::UnsupportedStateBearing {
                event_type: event_type.clone(),
            })?;
    let compatibility: EventCompatibility = serde_json::from_value(compatibility.clone())
        .map_err(|error| EventDecodeError::Invalid(error.to_string()))?;
    if compatibility != EventCompatibility::SkippableObservation {
        return Err(EventDecodeError::UnsupportedStateBearing { event_type });
    }
    let protocol_version = field(object, "protocolVersion")?;
    validate_read_version(protocol_version)
        .map_err(|message| EventDecodeError::Invalid(message.to_owned()))?;
    let payload = object
        .get("payload")
        .ok_or_else(|| EventDecodeError::Invalid("missing event payload".to_owned()))?;
    validate_json_bounds(payload, MAX_UNKNOWN_EVENT_BYTES, 32)
        .map_err(|error| EventDecodeError::Invalid(error.to_string()))?;
    Ok(EventInspection::UnknownSkippable(UnknownSkippableEvent {
        event_type,
        protocol_version,
        event_id: field(object, "eventId")?,
        session_id: field(object, "sessionId")?,
        run_id: optional_field(object, "runId")?,
        turn_id: optional_field(object, "turnId")?,
        sequence: field(object, "sequence")?,
        timestamp: field(object, "timestamp")?,
    }))
}

fn validate_items(items: &[String]) -> Result<(), EventValidationError> {
    if items.is_empty()
        || items.len() > MAX_APPROVAL_ITEMS
        || items.iter().any(|item| {
            item.is_empty()
                || item.len() > MAX_APPROVAL_ITEM_BYTES
                || item.chars().any(char::is_control)
        })
    {
        Err(EventValidationError::InvalidApprovalItems)
    } else {
        Ok(())
    }
}

fn validate_model_retry(
    attempt: u32,
    max_retries: u32,
    delay_ms: Option<u64>,
) -> Result<(), EventValidationError> {
    if attempt == 0
        || max_retries == 0
        || attempt > max_retries
        || delay_ms.is_some_and(|delay| delay > crate::MAX_SAFE_INTEGER)
    {
        Err(EventValidationError::InvalidModelRetry)
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

fn field<T>(object: &Map<String, Value>, key: &str) -> Result<T, EventDecodeError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(object.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|error| EventDecodeError::Invalid(error.to_string()))
}

fn optional_field<T>(object: &Map<String, Value>, key: &str) -> Result<Option<T>, EventDecodeError>
where
    T: for<'de> Deserialize<'de>,
{
    object
        .get(key)
        .cloned()
        .map_or(Ok(None), serde_json::from_value)
        .map_err(|error| EventDecodeError::Invalid(error.to_string()))
}
