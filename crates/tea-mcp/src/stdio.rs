use std::{
    collections::{BTreeSet, VecDeque},
    ffi::{OsStr, OsString},
    fmt,
    time::Duration,
};

use tea_tools::ToolTrust;
use tokio::{
    io::{AsyncReadExt, BufReader},
    task::JoinHandle,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use crate::{
    MAX_MCP_ENVIRONMENT_NAME_BYTES, MAX_MCP_ENVIRONMENT_VARIABLES, McpError, McpErrorCode,
    McpExecutableIdentity, McpLifecyclePolicy, McpLimits, McpServerConfig, McpServerId,
    McpServerSnapshot, McpToolCatalog, catalog, process,
    progress::ProgressRouter,
    transport::{BoundedStdioTransport, SdkClient, SdkExecutionHandle, TransportShared},
};

const MAX_ENVIRONMENT_VALUE_BYTES: usize = 64 * 1024;
const MAX_ENVIRONMENT_VALUE_TOTAL_BYTES: usize = 256 * 1024;
const STDERR_CHUNK_BYTES: usize = 8 * 1024;

/// One initialized caller-owned MCP stdio client and its supervised process.
///
/// SDK, JSON-RPC, pipe, process, stderr, and task types remain private. Dropping
/// this value signals cancellation and synchronously kills the owned process
/// tree; call [`shutdown`](Self::shutdown) to await and prove cleanup.
pub struct McpStdioClient {
    server_id: McpServerId,
    client: Option<SdkClient>,
    process: Option<process::OwnedProcess>,
    stderr_task: Option<JoinHandle<StderrCapture>>,
    cancellation: CancellationToken,
    lifecycle: McpLifecyclePolicy,
    limits: McpLimits,
    transport: TransportShared,
}

impl McpStdioClient {
    /// Spawns an exact configured executable in an otherwise empty environment
    /// and completes the MCP initialize/initialized handshake.
    ///
    /// # Errors
    ///
    /// Returns a stable error when environment validation, spawn, handshake,
    /// framing, or bounded startup cleanup fails. Error values never contain
    /// executable paths, arguments, environment values, stderr, or server text.
    pub async fn start<I, K, V>(config: &McpServerConfig, environment: I) -> Result<Self, McpError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        let environment = collect_environment(environment)?;
        let executable_identity =
            McpExecutableIdentity::capture(config.transport().as_stdio().executable())?;
        Self::start_inner(config, environment, &executable_identity, None, None).await
    }

    pub(crate) async fn start_until(
        config: &McpServerConfig,
        environment: Vec<(OsString, OsString)>,
        executable_identity: &McpExecutableIdentity,
        deadline: Instant,
        shutdown: Option<&CancellationToken>,
    ) -> Result<Self, McpError> {
        let environment = collect_environment(environment)?;
        Self::start_inner(
            config,
            environment,
            executable_identity,
            Some(deadline),
            shutdown,
        )
        .await
    }

    async fn start_inner(
        config: &McpServerConfig,
        environment: Vec<(OsString, OsString)>,
        executable_identity: &McpExecutableIdentity,
        deadline: Option<Instant>,
        shutdown: Option<&CancellationToken>,
    ) -> Result<Self, McpError> {
        let lifecycle = config.lifecycle();
        let spawn_timeout = stage_timeout(lifecycle.startup_timeout(), deadline)?;
        let spawned = spawn_process(
            config,
            environment,
            executable_identity,
            spawn_timeout,
            shutdown,
        )
        .await?;
        let process::SpawnedStdioProcess {
            mut owner,
            stdin,
            stdout,
            stderr,
        } = spawned;
        let limits = config.limits();
        let stderr_task = tokio::spawn(drain_stderr(stderr, limits.max_stderr_bytes()));
        let handshake_timeout = match stage_timeout(lifecycle.handshake_timeout(), deadline) {
            Ok(timeout) => timeout,
            Err(error) => {
                let _ = owner.shutdown(lifecycle).await;
                discard_stderr(stderr_task, lifecycle.cancellation_timeout()).await;
                return Err(error);
            }
        };
        let transport =
            TransportShared::new(limits.max_in_flight_requests(), limits.max_notifications());
        let progress =
            ProgressRouter::new(limits.max_in_flight_requests(), limits.max_notifications());
        let adapter = BoundedStdioTransport::new(
            stdout,
            stdin,
            transport.clone(),
            limits.max_frame_bytes(),
            handshake_timeout,
            progress.clone(),
        );
        let cancellation = CancellationToken::new();
        let connection = tokio::time::timeout(
            handshake_timeout,
            Box::pin(SdkClient::connect(
                adapter,
                transport.clone(),
                cancellation.clone(),
                limits.max_in_flight_requests(),
                limits.max_progress_events(),
                lifecycle.cancellation_timeout(),
                progress,
            )),
        );
        tokio::pin!(connection);
        let connection = if let Some(shutdown) = shutdown {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => {
                    cancellation.cancel();
                    let _ = owner.shutdown(lifecycle).await;
                    discard_stderr(stderr_task, lifecycle.cancellation_timeout()).await;
                    return Err(McpError::new(McpErrorCode::Cancellation));
                }
                result = &mut connection => result,
            }
        } else {
            connection.await
        };
        let client = match connection {
            Ok(Ok(client)) => client,
            Err(_) => {
                cancellation.cancel();
                let _ = owner.shutdown(lifecycle).await;
                discard_stderr(stderr_task, lifecycle.cancellation_timeout()).await;
                return Err(McpError::new(McpErrorCode::Timeout));
            }
            Ok(Err(_)) => {
                cancellation.cancel();
                let code = transport.failure_code().unwrap_or_else(|| {
                    if owner.exited().unwrap_or(false) {
                        McpErrorCode::ServerExit
                    } else {
                        McpErrorCode::Handshake
                    }
                });
                let _ = owner.shutdown(lifecycle).await;
                discard_stderr(stderr_task, lifecycle.cancellation_timeout()).await;
                return Err(McpError::new(code));
            }
        };
        Ok(Self {
            server_id: config.id().clone(),
            client: Some(client),
            process: Some(owner),
            stderr_task: Some(stderr_task),
            cancellation,
            lifecycle,
            limits,
            transport,
        })
    }

    /// Returns the stable configured identity without exposing process details.
    #[must_use]
    pub const fn server_id(&self) -> &McpServerId {
        &self.server_id
    }

    pub(crate) fn execution_handle(&self) -> Result<SdkExecutionHandle, McpError> {
        self.client
            .as_ref()
            .map(SdkClient::execution_handle)
            .ok_or_else(|| McpError::new(McpErrorCode::Cancellation))
    }

    pub(crate) const fn lifecycle(&self) -> McpLifecyclePolicy {
        self.lifecycle
    }

    pub(crate) const fn limits(&self) -> McpLimits {
        self.limits
    }

    /// Sends a cancellable protocol ping with one absolute request deadline.
    ///
    /// # Errors
    ///
    /// Returns a stable timeout, cancellation, or transport classification. A
    /// timed-out request sends `notifications/cancelled` before returning.
    pub async fn probe(&self, timeout: Duration) -> Result<(), McpError> {
        if timeout.is_zero() {
            return Err(McpError::new(McpErrorCode::Configuration));
        }
        if let Some(code) = self.transport.failure_code() {
            return Err(McpError::new(code));
        }
        let result = self
            .client
            .as_ref()
            .ok_or_else(|| McpError::new(McpErrorCode::Cancellation))?
            .probe(timeout)
            .await;
        result.map_err(|error| self.transport.failure_code().map_or(error, McpError::new))
    }

    /// Discovers all bounded `tools/list` pages and freezes host-owned bindings.
    ///
    /// # Errors
    ///
    /// Returns a stable failure for a mismatched configuration, transport or
    /// pagination failure, invalid descriptor/schema, collision, or bound.
    pub async fn discover_catalog(
        &self,
        config: &McpServerConfig,
        trust: ToolTrust,
    ) -> Result<McpToolCatalog, McpError> {
        if self.server_id != *config.id() {
            return Err(McpError::new(McpErrorCode::Configuration));
        }
        if let Some(code) = self.transport.failure_code() {
            return Err(McpError::new(code));
        }
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| McpError::new(McpErrorCode::Cancellation))?;
        catalog::discover(client, config, trust)
            .await
            .map_err(|error| self.transport.failure_code().map_or(error, McpError::new))
    }

    pub(crate) fn server_snapshot(
        &self,
        catalog: &McpToolCatalog,
    ) -> Result<McpServerSnapshot, McpError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| McpError::new(McpErrorCode::Cancellation))?;
        McpServerSnapshot::freeze(
            self.server_id.clone(),
            client.handshake_snapshot()?,
            catalog,
        )
    }

    /// Gracefully closes the protocol, drains stderr, and escalates through
    /// process-tree TERM/KILL deadlines when the server does not exit.
    ///
    /// # Errors
    ///
    /// Returns a stable shutdown error unless the process and every owned drain
    /// task have completed. Raw server diagnostics are never returned.
    pub async fn shutdown(mut self) -> Result<McpStdioShutdownReport, McpError> {
        self.cancellation.cancel();
        let client_result = if let Some(mut client) = self.client.take() {
            client.close().await
        } else {
            Ok(())
        };
        let process_result = if let Some(mut process) = self.process.take() {
            process.shutdown(self.lifecycle).await
        } else {
            Err(McpError::new(McpErrorCode::Shutdown))
        };
        let capture = if let Some(task) = self.stderr_task.take() {
            join_stderr(task, self.lifecycle.cancellation_timeout()).await?
        } else {
            return Err(McpError::new(McpErrorCode::Shutdown));
        };
        client_result?;
        let process = process_result?;
        Ok(McpStdioShutdownReport {
            retained_stderr_bytes: capture.bytes.len(),
            dropped_stderr_bytes: capture.dropped_bytes,
            forced_termination: process.forced,
        })
    }
}

async fn spawn_process(
    config: &McpServerConfig,
    environment: Vec<(OsString, OsString)>,
    executable_identity: &McpExecutableIdentity,
    timeout: Duration,
    shutdown: Option<&CancellationToken>,
) -> Result<process::SpawnedStdioProcess, McpError> {
    let spawned = process::spawn(
        config.transport().as_stdio(),
        environment,
        executable_identity,
        timeout,
        config.lifecycle().kill_timeout(),
    );
    tokio::pin!(spawned);
    if let Some(shutdown) = shutdown {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => Err(McpError::new(McpErrorCode::Cancellation)),
            result = &mut spawned => result,
        }
    } else {
        spawned.await
    }
}

fn stage_timeout(configured: Duration, deadline: Option<Instant>) -> Result<Duration, McpError> {
    let timeout = deadline.map_or(configured, |deadline| {
        configured.min(deadline.saturating_duration_since(Instant::now()))
    });
    if timeout.is_zero() {
        Err(McpError::new(McpErrorCode::Timeout))
    } else {
        Ok(timeout)
    }
}

impl fmt::Debug for McpStdioClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpStdioClient")
            .field("server_id", &self.server_id)
            .field("process", &"<owned>")
            .field("stderr", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl Drop for McpStdioClient {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(client) = &self.client {
            client.cancel();
        }
        if let Some(stderr_task) = &self.stderr_task {
            stderr_task.abort();
        }
    }
}

/// Secret-independent proof returned after awaited stdio shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpStdioShutdownReport {
    retained_stderr_bytes: usize,
    dropped_stderr_bytes: u64,
    forced_termination: bool,
}

impl McpStdioShutdownReport {
    /// Returns how many private stderr bytes were retained before destruction.
    #[must_use]
    pub const fn retained_stderr_bytes(self) -> usize {
        self.retained_stderr_bytes
    }

    /// Returns how many stderr bytes were dropped by the bounded ring.
    #[must_use]
    pub const fn dropped_stderr_bytes(self) -> u64 {
        self.dropped_stderr_bytes
    }

    /// Returns whether shutdown escalated beyond waiting for stdin EOF.
    #[must_use]
    pub const fn forced_termination(self) -> bool {
        self.forced_termination
    }
}

#[derive(Debug, Default)]
struct StderrCapture {
    bytes: VecDeque<u8>,
    dropped_bytes: u64,
}

impl StderrCapture {
    fn push(&mut self, chunk: &[u8], max_bytes: usize) {
        let overflow = self
            .bytes
            .len()
            .saturating_add(chunk.len())
            .saturating_sub(max_bytes);
        let from_retained = overflow.min(self.bytes.len());
        self.bytes.drain(..from_retained);
        let from_chunk = overflow.saturating_sub(from_retained).min(chunk.len());
        self.bytes.extend(&chunk[from_chunk..]);
        self.dropped_bytes = self
            .dropped_bytes
            .saturating_add(u64::try_from(overflow).unwrap_or(u64::MAX));
    }
}

async fn drain_stderr(stderr: tokio::process::ChildStderr, max_bytes: usize) -> StderrCapture {
    let mut reader = BufReader::new(stderr);
    let mut capture = StderrCapture::default();
    let mut chunk = [0_u8; STDERR_CHUNK_BYTES];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => return capture,
            Ok(count) => capture.push(&chunk[..count], max_bytes),
        }
    }
}

async fn join_stderr(
    mut task: JoinHandle<StderrCapture>,
    timeout: Duration,
) -> Result<StderrCapture, McpError> {
    match tokio::time::timeout(timeout, &mut task).await {
        Ok(Ok(capture)) => Ok(capture),
        Ok(Err(_)) => Err(McpError::new(McpErrorCode::Shutdown)),
        Err(_) => {
            task.abort();
            let _ = task.await;
            Err(McpError::new(McpErrorCode::Shutdown))
        }
    }
}

async fn discard_stderr(task: JoinHandle<StderrCapture>, timeout: Duration) {
    let mut task = task;
    if tokio::time::timeout(timeout, &mut task).await.is_err() {
        task.abort();
        let _ = task.await;
    }
}

pub(crate) fn collect_environment<I, K, V>(
    environment: I,
) -> Result<Vec<(OsString, OsString)>, McpError>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<OsString>,
    V: Into<OsString>,
{
    let mut variables = Vec::new();
    let mut names = BTreeSet::new();
    let mut total_value_bytes = 0usize;
    for (name, value) in environment {
        let name = name.into();
        let value = value.into();
        let Some(name_text) = name.to_str() else {
            return Err(McpError::new(McpErrorCode::Configuration));
        };
        let name_key = canonical_environment_name(name_text);
        let value_bytes = os_bytes(&value);
        total_value_bytes = total_value_bytes
            .checked_add(value_bytes)
            .ok_or_else(|| McpError::new(McpErrorCode::Configuration))?;
        if variables.len() >= max_environment_variables()
            || !valid_environment_name(name_text)
            || !names.insert(name_key)
            || value_bytes > MAX_ENVIRONMENT_VALUE_BYTES
            || total_value_bytes > MAX_ENVIRONMENT_VALUE_TOTAL_BYTES
            || os_has_nul(&value)
        {
            return Err(McpError::new(McpErrorCode::Configuration));
        }
        variables.push((name, value));
    }
    Ok(variables)
}

fn valid_environment_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_MCP_ENVIRONMENT_NAME_BYTES
        && name
            .bytes()
            .all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

#[cfg(windows)]
fn canonical_environment_name(name: &str) -> String {
    name.to_ascii_uppercase()
}

#[cfg(not(windows))]
fn canonical_environment_name(name: &str) -> String {
    name.to_owned()
}

const fn max_environment_variables() -> usize {
    if cfg!(windows) {
        MAX_MCP_ENVIRONMENT_VARIABLES + 1
    } else {
        MAX_MCP_ENVIRONMENT_VARIABLES
    }
}

#[cfg(unix)]
fn os_bytes(value: &OsStr) -> usize {
    use std::os::unix::ffi::OsStrExt as _;
    value.as_bytes().len()
}

#[cfg(windows)]
fn os_bytes(value: &OsStr) -> usize {
    use std::os::windows::ffi::OsStrExt as _;
    value.encode_wide().count().saturating_mul(2)
}

#[cfg(unix)]
fn os_has_nul(value: &OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt as _;
    value.as_bytes().contains(&0)
}

#[cfg(windows)]
fn os_has_nul(value: &OsStr) -> bool {
    use std::os::windows::ffi::OsStrExt as _;
    value.encode_wide().any(|unit| unit == 0)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{StderrCapture, collect_environment};

    #[test]
    fn stderr_ring_retains_only_the_newest_bounded_bytes() {
        let mut capture = StderrCapture::default();
        capture.push(b"abcd", 5);
        capture.push(b"efgh", 5);
        assert_eq!(capture.bytes.iter().copied().collect::<Vec<_>>(), b"defgh");
        assert_eq!(capture.dropped_bytes, 3);
    }

    #[test]
    fn child_environment_rejects_duplicate_names_and_nul_values() {
        let duplicate = collect_environment([
            (OsString::from("TOKEN"), OsString::from("one")),
            (OsString::from("TOKEN"), OsString::from("two")),
        ]);
        assert!(duplicate.is_err());
        assert!(
            collect_environment([(OsString::from("TOKEN"), OsString::from("bad\0value"))]).is_err()
        );
    }
}
