use serde::{Deserialize, Deserializer, Serialize};
use tea_protocol::ProtocolTimestamp;

use crate::{McpDescriptorDigest, McpError, McpErrorCode, McpServerId};

/// Maximum restart count exposed by one bounded service health snapshot.
pub const MAX_MCP_RESTART_COUNT: u32 = 1_024;

/// Host-only MCP server lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpServerState {
    /// Configuration is accepted but startup has not begun.
    Configured,
    /// The owned process or protocol service is starting.
    Starting,
    /// The frozen server and tool catalog are ready.
    Ready,
    /// The server announced catalog drift and calls are blocked.
    Stale,
    /// The server is unavailable or failed a lifecycle operation.
    Unhealthy,
    /// One caller owns a bounded reconnect attempt.
    Reconnecting,
    /// Shutdown completed and no owned server work remains.
    Stopped,
}

impl McpServerState {
    /// Returns the initial state of one freshly constructed server lifecycle.
    #[must_use]
    pub const fn fresh() -> Self {
        Self::Configured
    }

    /// Returns whether the lifecycle permits the requested next state.
    ///
    /// `Stopped` is terminal. Rebuilding a service creates a fresh
    /// `Configured` lifecycle instead of transitioning a stopped instance.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Configured, Self::Starting | Self::Stopped)
                | (
                    Self::Starting | Self::Reconnecting,
                    Self::Ready | Self::Unhealthy | Self::Stopped
                )
                | (Self::Ready, Self::Stale | Self::Unhealthy | Self::Stopped)
                | (
                    Self::Stale | Self::Unhealthy,
                    Self::Reconnecting | Self::Stopped
                )
        )
    }
}

/// Bounded serializable host projection for one MCP server.
///
/// This value intentionally contains no executable, argument, environment
/// value, stderr, raw server error, or protocol payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerHealth {
    server_id: McpServerId,
    state: McpServerState,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<McpErrorCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    descriptor_digest: Option<McpDescriptorDigest>,
    restart_count: u32,
    observed_at: ProtocolTimestamp,
}

impl McpServerHealth {
    /// Creates a validated health snapshot.
    ///
    /// # Errors
    ///
    /// Rejects restart counts outside the host projection bound.
    pub fn new(
        server_id: McpServerId,
        state: McpServerState,
        code: Option<McpErrorCode>,
        descriptor_digest: Option<McpDescriptorDigest>,
        restart_count: u32,
        observed_at: ProtocolTimestamp,
    ) -> Result<Self, McpError> {
        if restart_count > MAX_MCP_RESTART_COUNT {
            return Err(McpError::new(McpErrorCode::OutputBound));
        }
        Ok(Self {
            server_id,
            state,
            code,
            descriptor_digest,
            restart_count,
            observed_at,
        })
    }

    /// Returns the configured server identity.
    #[must_use]
    pub const fn server_id(&self) -> &McpServerId {
        &self.server_id
    }

    /// Returns the current host lifecycle state.
    #[must_use]
    pub const fn state(&self) -> McpServerState {
        self.state
    }

    /// Returns the stable current diagnostic code, when one is present.
    #[must_use]
    pub const fn code(&self) -> Option<McpErrorCode> {
        self.code
    }

    /// Returns the frozen descriptor digest when one has been established.
    #[must_use]
    pub const fn descriptor_digest(&self) -> Option<&McpDescriptorDigest> {
        self.descriptor_digest.as_ref()
    }

    /// Returns the bounded restart count.
    #[must_use]
    pub const fn restart_count(&self) -> u32 {
        self.restart_count
    }

    /// Returns when this immutable host snapshot was observed.
    #[must_use]
    pub const fn observed_at(&self) -> ProtocolTimestamp {
        self.observed_at
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawMcpServerHealth {
    server_id: McpServerId,
    state: McpServerState,
    code: Option<McpErrorCode>,
    descriptor_digest: Option<McpDescriptorDigest>,
    restart_count: u32,
    observed_at: ProtocolTimestamp,
}

impl<'de> Deserialize<'de> for McpServerHealth {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawMcpServerHealth::deserialize(deserializer)?;
        Self::new(
            raw.server_id,
            raw.state,
            raw.code,
            raw.descriptor_digest,
            raw.restart_count,
            raw.observed_at,
        )
        .map_err(serde::de::Error::custom)
    }
}
