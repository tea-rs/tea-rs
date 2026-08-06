#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Bounded MCP adapter configuration and host projection contracts for
//! `tea-rs`.
//!
//! This outward adapter may depend on control, protocol, and tool contracts.
//! Inward runtime, policy, session, coding, and CLI crates must not depend on
//! MCP SDK or transport types.
//!
//! Its internal dependencies are restricted to `tea-control`, `tea-protocol`,
//! and `tea-tools` by the workspace dependency-boundary tests.

mod binding;
mod catalog;
mod config;
mod content;
mod descriptor;
mod error;
mod executor;
mod framing;
mod health;
mod identity;
mod manager;
mod process;
mod progress;
mod reconnect;
mod schema;
mod server;
mod stdio;
mod transport;

pub use binding::McpToolBinding;
pub use catalog::McpToolCatalog;
pub use config::{
    MAX_MCP_ARGUMENT_BYTES, MAX_MCP_ARGUMENT_TOTAL_BYTES, MAX_MCP_ARGUMENTS,
    MAX_MCP_DESCRIPTOR_BYTES, MAX_MCP_ENVIRONMENT_NAME_BYTES, MAX_MCP_ENVIRONMENT_TOTAL_BYTES,
    MAX_MCP_ENVIRONMENT_VARIABLES, MAX_MCP_EXECUTABLE_BYTES, MAX_MCP_FRAME_BYTES,
    MAX_MCP_IN_FLIGHT_REQUESTS, MAX_MCP_LIFECYCLE_TIMEOUT, MAX_MCP_NOTIFICATIONS,
    MAX_MCP_PROGRESS_EVENTS, MAX_MCP_RECONNECT_ATTEMPTS, MAX_MCP_RECONNECT_BACKOFF,
    MAX_MCP_RESULT_BYTES, MAX_MCP_STDERR_BYTES, MAX_MCP_TOOL_EFFECTS, MAX_MCP_TOOL_RESOURCES,
    MAX_MCP_TOOLS_PER_SERVER, McpArgumentResource, McpLifecyclePolicy, McpLimits,
    McpReconnectPolicy, McpServerConfig, McpStdioConfig, McpToolDeclaration, McpToolPolicy,
    McpTransportConfig,
};
pub use descriptor::McpRemoteToolDescriptor;
pub use error::{McpError, McpErrorCode};
pub use executor::McpToolExecutor;
pub use health::{MAX_MCP_RESTART_COUNT, McpServerHealth, McpServerState};
pub(crate) use identity::McpExecutableIdentity;
pub use identity::{
    MAX_MCP_REMOTE_TOOL_NAME_BYTES, MAX_MCP_SERVER_ID_BYTES, McpDescriptorDigest,
    McpRemoteToolName, McpServerId,
};
pub use manager::{
    MAX_MCP_ACTIVE_TOOLS, MAX_MCP_MANAGED_SERVERS, MAX_MCP_STARTUP_CONCURRENCY, McpManager,
    McpManagerShutdownReport, McpServerLaunch,
};
pub use server::{MAX_MCP_PROTOCOL_VERSION_BYTES, McpProtocolVersion, McpServerSnapshot};
pub use stdio::{McpStdioClient, McpStdioShutdownReport};
