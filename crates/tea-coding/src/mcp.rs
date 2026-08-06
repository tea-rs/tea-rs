use tea_mcp::{McpManager, McpServerHealth, McpServerId};
use tea_protocol::ProtocolTimestamp;
use tea_tools::ToolName;

/// One safe immutable MCP tool binding exposed to product hosts.
///
/// Remote descriptions, annotations, executable details, environment values,
/// stderr, and result bodies intentionally do not cross this boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpCatalogEntry {
    server_id: McpServerId,
    tool_name: ToolName,
}

impl McpCatalogEntry {
    pub(crate) fn new(server_id: McpServerId, tool_name: ToolName) -> Self {
        Self {
            server_id,
            tool_name,
        }
    }

    /// Returns the configured server that owns this exact tool binding.
    #[must_use]
    pub const fn server_id(&self) -> &McpServerId {
        &self.server_id
    }

    /// Returns the frozen local tool alias.
    #[must_use]
    pub const fn tool_name(&self) -> &ToolName {
        &self.tool_name
    }
}

/// Safe immutable MCP lifecycle and catalog projection for product hosts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct McpServiceSnapshot {
    servers: Vec<McpServerHealth>,
    catalog: Vec<McpCatalogEntry>,
}

impl McpServiceSnapshot {
    pub(crate) fn new(servers: Vec<McpServerHealth>, catalog: Vec<McpCatalogEntry>) -> Self {
        Self { servers, catalog }
    }

    /// Returns server health in canonical server-ID order.
    #[must_use]
    pub fn servers(&self) -> &[McpServerHealth] {
        &self.servers
    }

    /// Returns enabled frozen aliases in canonical local-alias order.
    #[must_use]
    pub fn catalog(&self) -> &[McpCatalogEntry] {
        &self.catalog
    }
}

pub(crate) fn snapshot(
    manager: Option<&McpManager>,
    observed_at: ProtocolTimestamp,
) -> Result<McpServiceSnapshot, tea_mcp::McpError> {
    let Some(manager) = manager else {
        return Ok(McpServiceSnapshot::default());
    };
    let servers = manager.health(observed_at)?;
    let catalog = manager
        .catalog()
        .bindings()
        .map(|binding| {
            McpCatalogEntry::new(binding.server_id().clone(), binding.spec().name().clone())
        })
        .collect();
    Ok(McpServiceSnapshot::new(servers, catalog))
}
