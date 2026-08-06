use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use tea_protocol::ProtocolTimestamp;
use tokio::{sync::Notify, time::Instant};

use crate::{
    MAX_MCP_RESTART_COUNT, McpDescriptorDigest, McpError, McpErrorCode, McpReconnectPolicy,
    McpServerHealth, McpServerId, McpServerState, transport::SdkExecutionHandle,
};

#[derive(Clone)]
pub(crate) struct McpConnectionSlot {
    inner: Arc<ConnectionInner>,
}

impl McpConnectionSlot {
    pub(crate) fn ready(connection: SdkExecutionHandle) -> Self {
        Self::new(McpServerState::Ready, None, Some(connection))
    }

    pub(crate) fn unhealthy(code: McpErrorCode) -> Self {
        Self::new(McpServerState::Unhealthy, Some(code), None)
    }

    fn new(
        state: McpServerState,
        code: Option<McpErrorCode>,
        connection: Option<SdkExecutionHandle>,
    ) -> Self {
        Self {
            inner: Arc::new(ConnectionInner {
                state: Mutex::new(ConnectionState {
                    state,
                    code,
                    connection,
                    in_flight: 0,
                    restart_count: 0,
                }),
                drained: Notify::new(),
            }),
        }
    }

    pub(crate) fn acquire_call(&self) -> Result<McpConnectionLease, McpError> {
        let mut state = self.lock();
        synchronize(&mut state);
        if state.state != McpServerState::Ready {
            return Err(McpError::new(match state.state {
                McpServerState::Stale => McpErrorCode::StaleCatalog,
                _ => McpErrorCode::Unavailable,
            }));
        }
        let connection = state
            .connection
            .clone()
            .ok_or_else(|| McpError::new(McpErrorCode::Unavailable))?;
        state.in_flight = state
            .in_flight
            .checked_add(1)
            .ok_or_else(|| McpError::new(McpErrorCode::OutputBound))?;
        Ok(McpConnectionLease {
            slot: self.clone(),
            connection,
        })
    }

    pub(crate) fn begin_reconnect(&self) -> Result<McpReconnectGuard, McpError> {
        let mut state = self.lock();
        synchronize(&mut state);
        if state.in_flight != 0
            || !matches!(
                state.state,
                McpServerState::Stale | McpServerState::Unhealthy
            )
        {
            return Err(McpError::new(McpErrorCode::Unavailable));
        }
        state.state = McpServerState::Reconnecting;
        state.code = None;
        state.connection = None;
        Ok(McpReconnectGuard {
            slot: self.clone(),
            active: true,
        })
    }

    pub(crate) fn health(
        &self,
        server_id: McpServerId,
        descriptor_digest: Option<McpDescriptorDigest>,
        observed_at: ProtocolTimestamp,
    ) -> Result<McpServerHealth, McpError> {
        let mut state = self.lock();
        synchronize(&mut state);
        McpServerHealth::new(
            server_id,
            state.state,
            state.code,
            descriptor_digest,
            state.restart_count,
            observed_at,
        )
    }

    pub(crate) fn begin_shutdown(&self) {
        let drained = {
            let mut state = self.lock();
            state.state = McpServerState::Stopped;
            state.code = None;
            state.connection = None;
            state.in_flight == 0
        };
        if drained {
            self.inner.drained.notify_one();
        }
    }

    pub(crate) async fn wait_for_drain(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let notified = self.inner.drained.notified();
            if self.lock().in_flight == 0 {
                return true;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return false;
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, ConnectionState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

struct ConnectionInner {
    state: Mutex<ConnectionState>,
    drained: Notify,
}

struct ConnectionState {
    state: McpServerState,
    code: Option<McpErrorCode>,
    connection: Option<SdkExecutionHandle>,
    in_flight: usize,
    restart_count: u32,
}

fn synchronize(state: &mut ConnectionState) {
    if state.state != McpServerState::Ready {
        return;
    }
    let Some(connection) = &state.connection else {
        state.state = McpServerState::Unhealthy;
        state.code = Some(McpErrorCode::Unavailable);
        return;
    };
    if let Some(code) = connection.failure_code() {
        state.state = McpServerState::Unhealthy;
        state.code = Some(code);
    } else if connection.catalog_stale() {
        state.state = McpServerState::Stale;
        state.code = Some(McpErrorCode::StaleCatalog);
    }
}

pub(crate) struct McpConnectionLease {
    slot: McpConnectionSlot,
    connection: SdkExecutionHandle,
}

impl McpConnectionLease {
    pub(crate) const fn connection(&self) -> &SdkExecutionHandle {
        &self.connection
    }
}

impl Drop for McpConnectionLease {
    fn drop(&mut self) {
        let drained = {
            let mut state = self.slot.lock();
            state.in_flight = state.in_flight.saturating_sub(1);
            state.in_flight == 0
        };
        if drained {
            self.slot.inner.drained.notify_one();
        }
    }
}

pub(crate) struct McpReconnectGuard {
    slot: McpConnectionSlot,
    active: bool,
}

impl McpReconnectGuard {
    pub(crate) fn record_attempt(&self) -> Result<(), McpError> {
        let mut state = self.slot.lock();
        if state.restart_count >= MAX_MCP_RESTART_COUNT {
            return Err(McpError::new(McpErrorCode::OutputBound));
        }
        state.restart_count += 1;
        Ok(())
    }

    pub(crate) fn complete(mut self, connection: SdkExecutionHandle) -> Result<(), McpError> {
        let mut state = self.slot.lock();
        if state.state != McpServerState::Reconnecting {
            self.active = false;
            return Err(McpError::new(McpErrorCode::Cancellation));
        }
        state.state = McpServerState::Ready;
        state.code = None;
        state.connection = Some(connection);
        self.active = false;
        Ok(())
    }

    pub(crate) fn fail(mut self, error: McpError) {
        self.set_failure(error.code());
        self.active = false;
    }

    pub(crate) fn stop(mut self) {
        let mut state = self.slot.lock();
        state.state = McpServerState::Stopped;
        state.code = None;
        state.connection = None;
        self.active = false;
    }

    fn set_failure(&self, code: McpErrorCode) {
        let mut state = self.slot.lock();
        if state.state == McpServerState::Stopped {
            return;
        }
        state.state = McpServerState::Unhealthy;
        state.code = Some(code);
        state.connection = None;
    }
}

impl Drop for McpReconnectGuard {
    fn drop(&mut self) {
        if self.active {
            self.set_failure(McpErrorCode::Cancellation);
        }
    }
}

pub(crate) fn reconnect_backoff(policy: McpReconnectPolicy, failed_attempts: u32) -> Duration {
    let mut backoff = policy.initial_backoff();
    for _ in 1..failed_attempts {
        backoff = backoff
            .checked_mul(2)
            .unwrap_or(policy.max_backoff())
            .min(policy.max_backoff());
    }
    backoff
}
