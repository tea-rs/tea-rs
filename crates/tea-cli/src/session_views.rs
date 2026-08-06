use serde::Serialize;
use tea::{RuntimeSessionState, SessionStats};
use tea_coding::McpServiceSnapshot;
use tea_mcp::{McpDescriptorDigest, McpErrorCode, McpServerId, McpServerState};
use tea_protocol::{
    ApprovalId, BranchId, ModelId, ProfileId, ProtocolTimestamp, RecordEnvelope, RecordId,
    SessionId, SessionSequence,
};
use tea_session::{SessionCatalogEntry, SessionSnapshot};

/// Maximum records returned by one RPC snapshot page.
pub const MAX_SNAPSHOT_PAGE_RECORDS: usize = 64;

/// One bounded safe MCP server projection for terminal and RPC hosts.
///
/// This intentionally excludes remote tool descriptions and annotations,
/// executable details, environment values, stderr, and server result text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerView {
    server_id: McpServerId,
    state: McpServerState,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<McpErrorCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    descriptor_digest: Option<McpDescriptorDigest>,
    restart_count: u32,
    tools: Vec<String>,
}

impl McpServerView {
    /// Returns the configured server identity.
    #[must_use]
    pub const fn server_id(&self) -> &McpServerId {
        &self.server_id
    }

    /// Returns the host lifecycle state.
    #[must_use]
    pub const fn state(&self) -> McpServerState {
        self.state
    }

    /// Returns the stable health diagnostic code.
    #[must_use]
    pub const fn code(&self) -> Option<McpErrorCode> {
        self.code
    }

    /// Returns the frozen descriptor digest, when initialization completed.
    #[must_use]
    pub const fn descriptor_digest(&self) -> Option<&McpDescriptorDigest> {
        self.descriptor_digest.as_ref()
    }

    /// Returns the bounded reconnect attempt count.
    #[must_use]
    pub const fn restart_count(&self) -> u32 {
        self.restart_count
    }

    /// Returns local frozen aliases owned by this server.
    #[must_use]
    pub fn tools(&self) -> &[String] {
        &self.tools
    }
}

/// Creates safe MCP health/catalog projections in canonical server and alias order.
#[must_use]
pub fn mcp_servers(snapshot: &McpServiceSnapshot) -> Vec<McpServerView> {
    snapshot
        .servers()
        .iter()
        .map(|health| McpServerView {
            server_id: health.server_id().clone(),
            state: health.state(),
            code: health.code(),
            descriptor_digest: health.descriptor_digest().cloned(),
            restart_count: health.restart_count(),
            tools: snapshot
                .catalog()
                .iter()
                .filter(|entry| entry.server_id() == health.server_id())
                .map(|entry| entry.tool_name().as_str().to_owned())
                .collect(),
        })
        .collect()
}

/// One bounded host-facing session catalog row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListItem {
    session_id: SessionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    updated_at: ProtocolTimestamp,
    profile_id: ProfileId,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_id: Option<ModelId>,
    message_count: usize,
    pending_approval_count: usize,
}

impl SessionListItem {
    /// Returns the session identity.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Returns the optional display name.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Returns the active transcript message count.
    #[must_use]
    pub const fn message_count(&self) -> usize {
        self.message_count
    }
}

impl From<&SessionCatalogEntry> for SessionListItem {
    fn from(entry: &SessionCatalogEntry) -> Self {
        Self {
            session_id: entry.session_id(),
            name: entry.name().map(ToString::to_string),
            updated_at: entry.updated_at(),
            profile_id: entry.profile_id().clone(),
            model_id: entry.model_id().cloned(),
            message_count: entry.message_count(),
            pending_approval_count: entry.pending_approval_count(),
        }
    }
}

/// Creates stable catalog projections without exposing adapter types.
#[must_use]
pub fn session_list(entries: &[SessionCatalogEntry]) -> Vec<SessionListItem> {
    entries.iter().map(SessionListItem::from).collect()
}

/// Compact mode-neutral state returned to automation clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStateView {
    session_id: SessionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    profile_id: ProfileId,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_id: Option<ModelId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_branch_id: Option<BranchId>,
    message_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_approval_id: Option<ApprovalId>,
    is_running: bool,
}

impl SessionStateView {
    /// Returns the session identity.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Returns whether the live service owns an active run.
    #[must_use]
    pub const fn is_running(&self) -> bool {
        self.is_running
    }
}

impl From<&RuntimeSessionState> for SessionStateView {
    fn from(state: &RuntimeSessionState) -> Self {
        Self {
            session_id: state.session_id(),
            name: state.name().map(ToString::to_string),
            profile_id: state.profile_id().clone(),
            model_id: state.model_id().cloned(),
            active_branch_id: state.active_branch_id(),
            message_count: state.message_count(),
            pending_approval_id: state.pending_approval_id(),
            is_running: state.is_running(),
        }
    }
}

/// Rebuildable transcript statistics returned by host queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatsView {
    message_count: usize,
    user_messages: usize,
    assistant_messages: usize,
    tool_result_messages: usize,
    tool_calls: usize,
}

impl From<SessionStats> for SessionStatsView {
    fn from(stats: SessionStats) -> Self {
        Self {
            message_count: stats.message_count(),
            user_messages: stats.user_messages(),
            assistant_messages: stats.assistant_messages(),
            tool_result_messages: stats.tool_result_messages(),
            tool_calls: stats.tool_calls(),
        }
    }
}

/// One durable branch in the append-only session tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchView {
    branch_id: BranchId,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_branch_id: Option<BranchId>,
    from_record_id: RecordId,
    leaf_record_id: RecordId,
    active: bool,
}

impl BranchView {
    /// Returns branch identity.
    #[must_use]
    pub const fn branch_id(self) -> BranchId {
        self.branch_id
    }

    /// Returns the source branch, or `None` for the root.
    #[must_use]
    pub const fn source_branch_id(self) -> Option<BranchId> {
        self.source_branch_id
    }

    /// Returns the durable fork point.
    #[must_use]
    pub const fn from_record_id(self) -> RecordId {
        self.from_record_id
    }

    /// Returns the current durable leaf.
    #[must_use]
    pub const fn leaf_record_id(self) -> RecordId {
        self.leaf_record_id
    }

    /// Returns whether this is the active branch.
    #[must_use]
    pub const fn is_active(self) -> bool {
        self.active
    }
}

/// Deterministic tree projection over one canonical snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTreeView {
    session_id: SessionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_branch_id: Option<BranchId>,
    branches: Vec<BranchView>,
}

impl SessionTreeView {
    /// Returns branches in stable branch-ID order.
    #[must_use]
    pub fn branches(&self) -> &[BranchView] {
        &self.branches
    }
}

/// Projects the append-only branch graph for terminal and RPC consumers.
#[must_use]
pub fn session_tree(snapshot: &SessionSnapshot) -> SessionTreeView {
    let state = snapshot.state();
    let active_branch_id = state.active_branch_id();
    let branches = state
        .branches()
        .values()
        .map(|branch| BranchView {
            branch_id: branch.branch_id(),
            source_branch_id: branch.source_branch_id(),
            from_record_id: branch.from_record_id(),
            leaf_record_id: branch.leaf_record_id(),
            active: active_branch_id == Some(branch.branch_id()),
        })
        .collect();
    SessionTreeView {
        session_id: state.session_id(),
        active_branch_id,
        branches,
    }
}

/// One bounded page of canonical durable records plus the authoritative tail.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshotPage {
    session_id: SessionId,
    after_sequence: Option<SessionSequence>,
    tail_sequence: SessionSequence,
    records: Vec<RecordEnvelope>,
    has_more: bool,
}

impl SessionSnapshotPage {
    /// Returns records after the requested cursor.
    #[must_use]
    pub fn records(&self) -> &[RecordEnvelope] {
        &self.records
    }

    /// Returns whether another page is required to reach the tail.
    #[must_use]
    pub const fn has_more(&self) -> bool {
        self.has_more
    }
}

/// Creates a bounded canonical replay page after an optional durable cursor.
#[must_use]
pub fn snapshot_page(
    snapshot: &SessionSnapshot,
    after_sequence: Option<SessionSequence>,
    requested_limit: usize,
) -> SessionSnapshotPage {
    let limit = requested_limit.clamp(1, MAX_SNAPSHOT_PAGE_RECORDS);
    let mut matching = snapshot
        .records()
        .iter()
        .filter(|record| after_sequence.is_none_or(|cursor| record.sequence() > cursor));
    let records = matching.by_ref().take(limit).cloned().collect::<Vec<_>>();
    let has_more = matching.next().is_some();
    SessionSnapshotPage {
        session_id: snapshot.state().session_id(),
        after_sequence,
        tail_sequence: snapshot.state().tail_sequence(),
        records,
        has_more,
    }
}
