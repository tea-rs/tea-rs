use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};

/// Stable machine-readable MCP adapter failure classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpErrorCode {
    /// A pure server configuration value was invalid.
    Configuration,
    /// An owned server process could not start safely.
    Startup,
    /// Protocol initialization or capability negotiation failed.
    Handshake,
    /// A server or tool descriptor was invalid or changed.
    Descriptor,
    /// The configured server executable changed after startup preparation.
    Identity,
    /// A remote tool schema was invalid or unsupported.
    Schema,
    /// A host capability or resource declaration was incomplete.
    PolicyDeclaration,
    /// The bounded transport failed.
    Transport,
    /// A response or progress notification violated the MCP protocol contract.
    Protocol,
    /// A remote tool reported an expected execution failure.
    Execution,
    /// A remote terminal result could not be mapped safely.
    InvalidResult,
    /// An operation exceeded its caller-owned deadline.
    Timeout,
    /// Work was cooperatively cancelled.
    Cancellation,
    /// A frozen tool catalog is stale.
    StaleCatalog,
    /// A server is not ready for a new independent operation.
    Unavailable,
    /// A frame, descriptor, result, progress, or diagnostic exceeded its bound.
    OutputBound,
    /// The owned server process exited unexpectedly.
    ServerExit,
    /// Graceful or escalated shutdown failed.
    Shutdown,
}

impl McpErrorCode {
    const fn message(self) -> &'static str {
        match self {
            Self::Configuration => "MCP configuration is invalid",
            Self::Startup => "MCP server startup failed",
            Self::Handshake => "MCP handshake failed",
            Self::Descriptor => "MCP descriptor is invalid",
            Self::Identity => "MCP server identity changed",
            Self::Schema => "MCP tool schema is invalid",
            Self::PolicyDeclaration => "MCP host policy declaration is invalid",
            Self::Transport => "MCP transport failed",
            Self::Protocol => "MCP protocol response is invalid",
            Self::Execution => "MCP tool execution failed",
            Self::InvalidResult => "MCP tool result is invalid",
            Self::Timeout => "MCP operation timed out",
            Self::Cancellation => "MCP operation was cancelled",
            Self::StaleCatalog => "MCP catalog is stale",
            Self::Unavailable => "MCP server is unavailable",
            Self::OutputBound => "MCP output exceeded a configured bound",
            Self::ServerExit => "MCP server exited",
            Self::Shutdown => "MCP server shutdown failed",
        }
    }
}

/// Secret-independent MCP adapter failure.
///
/// The error deliberately carries no server text, path, argument, environment
/// value, stderr, or protocol payload. Private diagnostics stay at the adapter
/// boundary while callers receive only a stable classification and message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpError {
    code: McpErrorCode,
}

impl McpError {
    /// Creates an error from one stable classification.
    #[must_use]
    pub const fn new(code: McpErrorCode) -> Self {
        Self { code }
    }

    /// Returns the stable machine-readable classification.
    #[must_use]
    pub const fn code(self) -> McpErrorCode {
        self.code
    }
}

impl fmt::Display for McpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code.message())
    }
}

impl Error for McpError {}
