use std::fmt::Write as _;

use tea_protocol::{CodeChange, MessageId, ToolCallId};

use super::{
    DecisionStatus, LifecycleStatus, NoticeSeverity, OutputFormat, PlanStep, QueuedInputKind,
    TimelineDetail, TimelineSource, append_raw_details, bounded_terminal_text,
    code_change_raw_text, queue_preview, raw_plan_text, terminal_safe_text,
};

/// Author intrinsic to a message payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageAuthor {
    /// User-authored message content.
    User,
    /// Assistant-authored message content.
    Assistant,
}

/// Origin intrinsic to lifecycle activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleKind {
    /// A locally executed tool call.
    ToolCall,
    /// Provider-hosted activity.
    HostedTool,
    /// Overall run activity not owned by one tool.
    RunActivity,
}

/// Origin intrinsic to a notice payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// A general transient notification.
    General,
    /// MCP connection health information.
    McpHealth,
}

/// Typed user or assistant message source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageCell {
    author: MessageAuthor,
    source: String,
    format: OutputFormat,
}

impl MessageCell {
    pub(crate) fn new(author: MessageAuthor, source: &str, format: OutputFormat) -> Self {
        Self {
            author,
            source: bounded_terminal_text(source),
            format,
        }
    }

    /// Returns the intrinsic message author.
    #[must_use]
    pub const fn author(&self) -> MessageAuthor {
        self.author
    }

    /// Returns terminal-safe source retained for reflow and raw copy.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the source rendering format.
    #[must_use]
    pub const fn format(&self) -> OutputFormat {
        self.format
    }
}

/// Typed reasoning source and disclosure state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasoningCell {
    source: String,
    collapsed: bool,
}

impl ReasoningCell {
    pub(crate) fn new(source: &str, collapsed: bool) -> Self {
        Self {
            source: bounded_terminal_text(source),
            collapsed,
        }
    }

    /// Returns terminal-safe reasoning source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns whether the visible reasoning surface is collapsed.
    #[must_use]
    pub const fn collapsed(&self) -> bool {
        self.collapsed
    }
}

/// Typed plan proposal or progress update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanCell {
    title: String,
    source: Option<String>,
    steps: Vec<PlanStep>,
    note: Option<String>,
}

impl PlanCell {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn new(
        title: &str,
        source: Option<&str>,
        steps: Vec<PlanStep>,
        note: Option<&str>,
    ) -> Self {
        Self {
            title: bounded_terminal_text(title),
            source: source.map(bounded_terminal_text),
            steps,
            note: note.map(bounded_terminal_text),
        }
    }

    /// Returns the compact plan action header.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns optional Markdown proposal source.
    #[must_use]
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    /// Returns ordered structured plan steps.
    #[must_use]
    pub fn steps(&self) -> &[PlanStep] {
        &self.steps
    }

    /// Returns an optional progress note.
    #[must_use]
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }
}

/// Typed operational lifecycle activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleCell {
    kind: LifecycleKind,
    action: String,
    target: Option<String>,
    status: LifecycleStatus,
    details: Vec<TimelineDetail>,
    expanded: bool,
    tick: u64,
}

impl LifecycleCell {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        kind: LifecycleKind,
        action: &str,
        target: Option<&str>,
        status: LifecycleStatus,
        details: Vec<TimelineDetail>,
        expanded: bool,
        tick: u64,
    ) -> Self {
        Self {
            kind,
            action: bounded_terminal_text(action),
            target: target.map(bounded_terminal_text),
            status,
            details,
            expanded,
            tick,
        }
    }

    /// Returns the intrinsic lifecycle origin.
    #[must_use]
    pub const fn kind(&self) -> LifecycleKind {
        self.kind
    }

    /// Returns the lifecycle action label.
    #[must_use]
    pub fn action(&self) -> &str {
        &self.action
    }

    /// Returns the optional primary target.
    #[must_use]
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    /// Returns the protocol-backed lifecycle state.
    #[must_use]
    pub const fn status(&self) -> LifecycleStatus {
        self.status
    }

    /// Returns structured progressively disclosed details.
    #[must_use]
    pub fn details(&self) -> &[TimelineDetail] {
        &self.details
    }

    /// Returns whether normally hidden details are expanded.
    #[must_use]
    pub const fn expanded(&self) -> bool {
        self.expanded
    }

    /// Returns the observable animation tick.
    #[must_use]
    pub const fn tick(&self) -> u64 {
        self.tick
    }
}

/// Typed tool, process, artifact, or fallback result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultCell {
    action: String,
    source_name: Option<String>,
    content: String,
    format: OutputFormat,
    is_error: bool,
}

impl ResultCell {
    pub(crate) fn new(
        action: &str,
        source_name: Option<&str>,
        content: &str,
        format: OutputFormat,
        is_error: bool,
    ) -> Self {
        Self {
            action: bounded_terminal_text(action),
            source_name: source_name.map(bounded_terminal_text),
            content: bounded_terminal_text(content),
            format,
            is_error,
        }
    }

    /// Returns the result action label.
    #[must_use]
    pub fn action(&self) -> &str {
        &self.action
    }

    /// Returns the optional source that produced the result.
    #[must_use]
    pub fn source_name(&self) -> Option<&str> {
        self.source_name.as_deref()
    }

    /// Returns canonical result content.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Returns the result rendering format.
    #[must_use]
    pub const fn format(&self) -> OutputFormat {
        self.format
    }

    /// Returns whether the protocol identified an error result.
    #[must_use]
    pub const fn is_error(&self) -> bool {
        self.is_error
    }
}

/// Typed provider-neutral external sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcesCell {
    sources: Vec<TimelineSource>,
}

impl SourcesCell {
    pub(crate) const fn new(sources: Vec<TimelineSource>) -> Self {
        Self { sources }
    }

    /// Returns sources in provider order.
    #[must_use]
    pub fn sources(&self) -> &[TimelineSource] {
        &self.sources
    }
}

/// Typed persisted code change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffCell {
    action: String,
    change: CodeChange,
}

impl DiffCell {
    pub(crate) fn new(action: &str, change: CodeChange) -> Self {
        Self {
            action: bounded_terminal_text(action),
            change,
        }
    }

    /// Returns the code-change action label.
    #[must_use]
    pub fn action(&self) -> &str {
        &self.action
    }

    /// Returns the structured code change retained for reflow and raw copy.
    #[must_use]
    pub const fn change(&self) -> &CodeChange {
        &self.change
    }
}

/// Typed queued steering or follow-up preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedInputCell {
    kind: QueuedInputKind,
    preview: String,
}

impl QueuedInputCell {
    pub(crate) fn new(kind: QueuedInputKind, preview: &str) -> Self {
        Self {
            kind,
            preview: queue_preview(preview),
        }
    }

    /// Returns queue ownership and consumption semantics.
    #[must_use]
    pub const fn kind(&self) -> QueuedInputKind {
        self.kind
    }

    /// Returns the bounded single-line queue preview.
    #[must_use]
    pub fn preview(&self) -> &str {
        &self.preview
    }
}

/// Typed informational, warning, or error notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeCell {
    kind: NoticeKind,
    severity: NoticeSeverity,
    message: String,
    hint: Option<String>,
}

impl NoticeCell {
    pub(crate) fn new(
        kind: NoticeKind,
        severity: NoticeSeverity,
        message: &str,
        hint: Option<&str>,
    ) -> Self {
        Self {
            kind,
            severity,
            message: bounded_terminal_text(message),
            hint: hint.map(bounded_terminal_text),
        }
    }

    /// Returns the intrinsic notice origin.
    #[must_use]
    pub const fn kind(&self) -> NoticeKind {
        self.kind
    }

    /// Returns the notice severity.
    #[must_use]
    pub const fn severity(&self) -> NoticeSeverity {
        self.severity
    }

    /// Returns the primary notice message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns optional supporting text.
    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }
}

/// Typed approval or another explicit durable decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionCell {
    action: String,
    subject: String,
    status: DecisionStatus,
    details: Vec<TimelineDetail>,
}

impl DecisionCell {
    pub(crate) fn new(
        action: &str,
        subject: &str,
        status: DecisionStatus,
        details: Vec<TimelineDetail>,
    ) -> Self {
        Self {
            action: bounded_terminal_text(action),
            subject: bounded_terminal_text(subject),
            status,
            details,
        }
    }

    /// Returns the decision action label.
    #[must_use]
    pub fn action(&self) -> &str {
        &self.action
    }

    /// Returns the redacted decision subject.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// Returns the current decision status.
    #[must_use]
    pub const fn status(&self) -> DecisionStatus {
        self.status
    }

    /// Returns structured redacted effects and scope.
    #[must_use]
    pub fn details(&self) -> &[TimelineDetail] {
        &self.details
    }
}

/// Closed family of legal first-party cell payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellContent {
    /// User or assistant message content.
    Message(MessageCell),
    /// Reasoning or reasoning-summary content.
    Reasoning(ReasoningCell),
    /// Plan proposal or progress content.
    Plan(PlanCell),
    /// Active or finalized operational work.
    Lifecycle(LifecycleCell),
    /// Tool, process, artifact, or fallback output.
    Result(ResultCell),
    /// Provider-neutral external sources.
    Sources(SourcesCell),
    /// Persisted code change.
    Diff(DiffCell),
    /// Queued steering or follow-up input.
    QueuedInput(QueuedInputCell),
    /// Informational, warning, or error notice.
    Notice(NoticeCell),
    /// Approval or another durable decision.
    Decision(DecisionCell),
}

impl CellContent {
    /// Returns whether this payload belongs in the non-modal live timeline.
    #[must_use]
    pub(crate) const fn is_live_timeline_visible(&self) -> bool {
        !matches!(self, Self::Decision(_))
    }

    /// Returns the primary source text for compatibility and search.
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Self::Message(cell) => cell.source(),
            Self::Reasoning(cell) => cell.source(),
            Self::Plan(cell) => cell.source().unwrap_or_else(|| cell.title()),
            Self::Lifecycle(cell) => cell.action(),
            Self::Result(cell) => cell.action(),
            Self::Sources(_) => "Sources",
            Self::Diff(cell) => cell.action(),
            Self::QueuedInput(cell) => cell.preview(),
            Self::Notice(cell) => cell.message(),
            Self::Decision(cell) => cell.action(),
        }
    }

    /// Returns a renderer-independent, copy-friendly representation.
    #[must_use]
    pub fn raw_text(&self) -> String {
        let raw = match self {
            Self::Message(cell) => cell.source().to_owned(),
            Self::Reasoning(cell) => cell.source().to_owned(),
            Self::Plan(cell) => {
                raw_plan_text(cell.title(), cell.source(), cell.steps(), cell.note())
            }
            Self::Lifecycle(cell) => {
                let mut raw = cell.action().to_owned();
                if let Some(target) = cell.target() {
                    raw.push(' ');
                    raw.push_str(target);
                }
                append_raw_details(&mut raw, cell.details());
                raw
            }
            Self::Result(cell) => cell.content().to_owned(),
            Self::Sources(cell) => {
                let mut raw = String::new();
                for source in cell.sources() {
                    if !raw.is_empty() {
                        raw.push('\n');
                    }
                    write!(raw, "- {} <{}>", source.label(), source.url())
                        .expect("writing to a String cannot fail");
                }
                raw
            }
            Self::Diff(cell) => code_change_raw_text(cell.change()),
            Self::QueuedInput(cell) => cell.preview().to_owned(),
            Self::Notice(cell) => {
                let mut raw = cell.message().to_owned();
                if let Some(hint) = cell.hint() {
                    raw.push('\n');
                    raw.push_str(hint);
                }
                raw
            }
            Self::Decision(cell) => {
                let mut raw = cell.action().to_owned();
                if !cell.subject().is_empty() {
                    raw.push(' ');
                    raw.push_str(cell.subject());
                }
                append_raw_details(&mut raw, cell.details());
                raw
            }
        };
        terminal_safe_text(&raw)
    }
}

/// Presentation lane used only when a protocol identity is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CellLane {
    /// Durable history projection.
    History,
    /// Active streaming or local projection.
    Active,
    /// Transient notification projection.
    Notifications,
}

/// Identity facet for one canonical message block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessageCellFacet {
    /// Primary content associated with the block index.
    Content,
    /// External sources associated with the message.
    Sources,
}

/// Identity facet for one tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolCellFacet {
    /// Tool-call lifecycle activity.
    Call,
    /// Approval or another explicit decision.
    Decision,
    /// Canonical tool result.
    Result,
    /// Structured code change.
    Diff,
    /// External sources associated with hosted work.
    Sources,
    /// Transient approval preview.
    ApprovalPreview,
}

/// Identity facet for active message streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StreamCellFacet {
    /// Visible assistant message delta.
    Message,
    /// Visible reasoning delta.
    Reasoning,
}

/// Stable typed identity for one projected cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CellId {
    /// One block or facet owned by a canonical message.
    Message {
        /// Stable canonical message identifier.
        message_id: MessageId,
        /// Bounded index of the canonical content block.
        block_index: u16,
        /// Message-owned identity facet.
        facet: MessageCellFacet,
    },
    /// One facet owned by a canonical tool call.
    Tool {
        /// Stable canonical tool-call identifier.
        tool_call_id: ToolCallId,
        /// Tool-owned identity facet.
        facet: ToolCellFacet,
        /// Bounded part index within the facet.
        part_index: u16,
    },
    /// One active streaming block.
    Stream {
        /// Stable canonical streaming message identifier.
        message_id: MessageId,
        /// Bounded streaming block index.
        block_index: u16,
        /// Stream-owned identity facet.
        facet: StreamCellFacet,
    },
    /// One queued local input.
    Queue {
        /// Queue ownership and consumption semantics.
        kind: QueuedInputKind,
        /// Bounded index within that queue.
        index: u16,
    },
    /// One local MCP health row.
    McpHealth {
        /// Bounded row index.
        index: u16,
    },
    /// One transient notification row.
    Notification {
        /// Bounded row index.
        index: u16,
    },
    /// One lane-local value without a protocol identity.
    Synthetic {
        /// Owning presentation lane.
        lane: CellLane,
        /// Bounded lane-local index.
        index: u16,
    },
}

/// One immutable typed presentation cell with stable identity and ownership metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellNode {
    id: CellId,
    content: CellContent,
    tool_call_id: Option<ToolCallId>,
}

impl CellNode {
    pub(crate) const fn new(
        id: CellId,
        content: CellContent,
        tool_call_id: Option<ToolCallId>,
    ) -> Self {
        Self {
            id,
            content,
            tool_call_id,
        }
    }

    /// Returns the stable typed cell identity.
    #[must_use]
    pub const fn id(&self) -> CellId {
        self.id
    }

    /// Returns the typed semantic payload.
    #[must_use]
    pub const fn content(&self) -> &CellContent {
        &self.content
    }

    /// Returns the primary source text for compatibility and search.
    #[must_use]
    pub fn text(&self) -> &str {
        self.content.text()
    }

    /// Returns a renderer-independent, copy-friendly representation.
    #[must_use]
    pub fn raw_text(&self) -> String {
        self.content.raw_text()
    }

    /// Returns the owning tool call independently of payload family.
    #[must_use]
    pub const fn tool_call_id(&self) -> Option<ToolCallId> {
        self.tool_call_id
    }
}
