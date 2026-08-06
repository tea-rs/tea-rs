use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

use tea_protocol::ProtocolTimestamp;
use tea_tools::{ToolName, ToolTrust};
use tokio::{sync::Mutex, task::JoinSet, time::Instant};
use tokio_util::sync::CancellationToken;

use crate::{
    McpError, McpErrorCode, McpExecutableIdentity, McpServerConfig, McpServerHealth, McpServerId,
    McpServerSnapshot, McpStdioClient, McpStdioShutdownReport, McpToolCatalog, McpToolExecutor,
    reconnect::{McpConnectionSlot, reconnect_backoff},
    stdio,
};

/// Maximum MCP servers admitted into one manager bootstrap.
pub const MAX_MCP_MANAGED_SERVERS: usize = 64;
/// Maximum active MCP aliases accepted from one product profile.
pub const MAX_MCP_ACTIVE_TOOLS: usize = 4_096;
/// Maximum concurrent MCP server initialization operations.
pub const MAX_MCP_STARTUP_CONCURRENCY: usize = 16;

/// One validated server launch with exact already-resolved environment values.
///
/// This value intentionally implements neither `Debug` nor serialization so
/// resolved secret values cannot enter routine diagnostics or persisted state.
pub struct McpServerLaunch {
    config: McpServerConfig,
    trust: ToolTrust,
    environment: Vec<(OsString, OsString)>,
    executable_identity: Option<McpExecutableIdentity>,
}

impl McpServerLaunch {
    /// Creates a launch only when resolved names exactly match the config.
    ///
    /// # Errors
    ///
    /// Rejects missing, extra, duplicate, non-canonical, NUL-containing, or
    /// oversized environment names or values.
    pub fn new<I, K, V>(
        config: McpServerConfig,
        trust: ToolTrust,
        environment: I,
    ) -> Result<Self, McpError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        let environment = stdio::collect_environment(environment)?;
        let mut names = environment
            .iter()
            .map(|(name, _)| {
                name.to_str()
                    .map(str::to_owned)
                    .ok_or_else(|| McpError::new(McpErrorCode::Configuration))
            })
            .collect::<Result<Vec<_>, _>>()?;
        names.sort_unstable();
        if names != config.inherited_environment() {
            return Err(McpError::new(McpErrorCode::Configuration));
        }
        // A missing or unreadable inactive executable remains an unhealthy
        // server diagnostic. Active servers fail later during initialization.
        let executable_identity =
            McpExecutableIdentity::capture(config.transport().as_stdio().executable()).ok();
        Ok(Self {
            config,
            trust,
            environment,
            executable_identity,
        })
    }

    /// Returns the non-secret validated server configuration.
    #[must_use]
    pub const fn config(&self) -> &McpServerConfig {
        &self.config
    }

    /// Returns the host-assigned trust class.
    #[must_use]
    pub const fn trust(&self) -> ToolTrust {
        self.trust
    }
}

impl fmt::Debug for McpServerLaunch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpServerLaunch")
            .field("server_id", self.config.id())
            .field("trust", &self.trust)
            .field("environment", &"<resolved and redacted>")
            .field("environment_count", &self.environment.len())
            .finish_non_exhaustive()
    }
}

/// Owned initialized MCP servers, their frozen combined catalog, and health.
pub struct McpManager {
    servers: BTreeMap<McpServerId, ManagedServer>,
    catalog: McpToolCatalog,
    shutdown: CancellationToken,
    shutdown_started: AtomicBool,
}

impl McpManager {
    /// Initializes validated servers under a bounded concurrent fan-out.
    ///
    /// A failed server without an active profile alias remains as an unhealthy
    /// diagnostic. A failed server owning any active alias aborts bootstrap and
    /// closes every client that initialized successfully.
    ///
    /// # Errors
    ///
    /// Rejects collection bounds, duplicate identities or aliases, unknown
    /// active aliases, invalid concurrency, active startup/discovery failures,
    /// missing active bindings, or cross-server catalog collisions.
    pub async fn start<I, A>(
        launches: I,
        active_tools: A,
        max_concurrent_startups: usize,
        observed_at: ProtocolTimestamp,
    ) -> Result<Self, McpError>
    where
        I: IntoIterator<Item = McpServerLaunch> + Send,
        I::IntoIter: Send,
        A: IntoIterator<Item = ToolName> + Send,
        A::IntoIter: Send,
    {
        if max_concurrent_startups == 0 || max_concurrent_startups > MAX_MCP_STARTUP_CONCURRENCY {
            return Err(McpError::new(McpErrorCode::Configuration));
        }
        let launches = launches.into_iter().collect::<Vec<_>>();
        if launches.len() > MAX_MCP_MANAGED_SERVERS {
            return Err(McpError::new(McpErrorCode::OutputBound));
        }
        let active_tools = collect_active_tools(active_tools)?;
        let required_servers = validate_launches(&launches, &active_tools)?;
        let (outcomes, join_failed) = run_initializations(launches, max_concurrent_startups).await;
        let active_error = required_servers.iter().find_map(|server_id| {
            outcomes
                .get(server_id)
                .and_then(|outcome| outcome.result.as_ref().err())
                .copied()
        });
        if join_failed || active_error.is_some() {
            shutdown_outcomes(outcomes).await;
            return Err(active_error.unwrap_or_else(|| McpError::new(McpErrorCode::Cancellation)));
        }

        let servers = build_servers(outcomes)?;
        let catalog = match McpToolCatalog::combine(
            servers.values().filter_map(|server| server.catalog.clone()),
        ) {
            Ok(catalog) => catalog,
            Err(error) => {
                shutdown_servers(servers).await;
                return Err(error);
            }
        };
        if active_tools
            .iter()
            .any(|alias| catalog.binding(alias.as_str()).is_none())
        {
            shutdown_servers(servers).await;
            return Err(McpError::new(McpErrorCode::Descriptor));
        }
        let manager = Self {
            servers,
            catalog,
            shutdown: CancellationToken::new(),
            shutdown_started: AtomicBool::new(false),
        };
        manager.health(observed_at)?;
        Ok(manager)
    }

    /// Returns the combined immutable catalog in canonical alias order.
    #[must_use]
    pub const fn catalog(&self) -> &McpToolCatalog {
        &self.catalog
    }

    /// Returns server health in canonical server-ID order.
    ///
    /// # Errors
    ///
    /// Fails if a bounded health projection cannot be constructed.
    pub fn health(&self, observed_at: ProtocolTimestamp) -> Result<Vec<McpServerHealth>, McpError> {
        self.servers
            .iter()
            .map(|(server_id, server)| server.health(server_id.clone(), observed_at))
            .collect()
    }

    /// Returns health for one configured server.
    ///
    /// # Errors
    ///
    /// Fails if a bounded health projection cannot be constructed.
    pub fn server_health(
        &self,
        server_id: &McpServerId,
        observed_at: ProtocolTimestamp,
    ) -> Result<Option<McpServerHealth>, McpError> {
        self.servers
            .get(server_id)
            .map(|server| server.health(server_id.clone(), observed_at))
            .transpose()
    }

    /// Returns the frozen initialized snapshot for one healthy server.
    #[must_use]
    pub fn server_snapshot(&self, server_id: &McpServerId) -> Option<&McpServerSnapshot> {
        self.servers
            .get(server_id)
            .and_then(|server| server.snapshot.as_ref())
    }

    /// Creates an executor for one exact frozen tool binding.
    ///
    /// # Errors
    ///
    /// Rejects unknown aliases or inconsistent manager ownership metadata.
    pub fn tool_executor(&self, alias: &ToolName) -> Result<McpToolExecutor, McpError> {
        let binding = self
            .catalog
            .binding(alias.as_str())
            .ok_or_else(|| McpError::new(McpErrorCode::Configuration))?;
        let server = self
            .servers
            .get(binding.server_id())
            .ok_or_else(|| McpError::new(McpErrorCode::Descriptor))?;
        let config = &server.restart.config;
        McpToolExecutor::managed(
            server.connection.clone(),
            binding,
            config.limits().max_result_bytes(),
            config.lifecycle().cancellation_timeout(),
        )
    }

    /// Replaces one stale or unhealthy connection only when discovery exactly
    /// matches its frozen initialized snapshot.
    ///
    /// # Errors
    ///
    /// Rejects unknown or never-initialized servers, disabled reconnect policy,
    /// concurrent calls or reconnect owners, snapshot drift, and exhausted
    /// bounded startup attempts.
    pub async fn reconnect(
        &self,
        server_id: &McpServerId,
        observed_at: ProtocolTimestamp,
    ) -> Result<McpServerHealth, McpError> {
        if self.shutdown.is_cancelled() {
            return Err(McpError::new(McpErrorCode::Unavailable));
        }
        let server = self
            .servers
            .get(server_id)
            .ok_or_else(|| McpError::new(McpErrorCode::Unavailable))?;
        let policy = server.restart.config.reconnect();
        let frozen = server
            .snapshot
            .as_ref()
            .ok_or_else(|| McpError::new(McpErrorCode::Unavailable))?;
        if !policy.is_enabled() {
            return Err(McpError::new(McpErrorCode::Unavailable));
        }
        let guard = server.connection.begin_reconnect()?;
        let _operation = server.operation.lock().await;
        if self.shutdown.is_cancelled() {
            guard.stop();
            return Err(McpError::new(McpErrorCode::Cancellation));
        }

        let old_client = server.client.lock().await.take();
        if let Some(client) = old_client {
            let _ = client.shutdown().await;
        }
        if self.shutdown.is_cancelled() {
            guard.stop();
            return Err(McpError::new(McpErrorCode::Cancellation));
        }

        for attempt in 1..=policy.max_attempts() {
            if let Err(error) = guard.record_attempt() {
                guard.fail(error);
                return Err(error);
            }
            let candidate = initialize_restart(&server.restart, Some(&self.shutdown)).await;
            if self.shutdown.is_cancelled() {
                shutdown_candidate(candidate).await;
                guard.stop();
                return Err(McpError::new(McpErrorCode::Cancellation));
            }
            match candidate {
                Ok(candidate) if candidate.snapshot != *frozen => {
                    let _ = candidate.client.shutdown().await;
                    let error = McpError::new(McpErrorCode::Descriptor);
                    guard.fail(error);
                    return Err(error);
                }
                Ok(candidate) => {
                    let connection = match candidate.client.execution_handle() {
                        Ok(connection) => connection,
                        Err(error) => {
                            let _ = candidate.client.shutdown().await;
                            if attempt == policy.max_attempts() {
                                guard.fail(error);
                                return Err(error);
                            }
                            if self
                                .wait_reconnect_backoff(reconnect_backoff(policy, attempt))
                                .await
                                .is_err()
                            {
                                guard.stop();
                                return Err(McpError::new(McpErrorCode::Cancellation));
                            }
                            continue;
                        }
                    };
                    if guard.complete(connection).is_err() {
                        let _ = candidate.client.shutdown().await;
                        return Err(McpError::new(McpErrorCode::Cancellation));
                    }
                    *server.client.lock().await = Some(candidate.client);
                    return ready_health(
                        &server.connection,
                        server_id.clone(),
                        frozen,
                        observed_at,
                    );
                }
                Err(error) if attempt == policy.max_attempts() => {
                    guard.fail(error);
                    return Err(error);
                }
                Err(_) => {
                    if self
                        .wait_reconnect_backoff(reconnect_backoff(policy, attempt))
                        .await
                        .is_err()
                    {
                        guard.stop();
                        return Err(McpError::new(McpErrorCode::Cancellation));
                    }
                }
            }
        }

        let error = McpError::new(McpErrorCode::Unavailable);
        guard.fail(error);
        Err(error)
    }

    /// Stops all managed servers, rejects new work, drains active calls, and
    /// awaits every owned client, service task, stderr drain, and process tree.
    ///
    /// The returned report contains only bounded aggregate counts and never
    /// includes process arguments, environment values, stderr, or server text.
    ///
    /// # Errors
    ///
    /// Returns unavailable when another caller already owns manager shutdown.
    pub async fn shutdown(&self) -> Result<McpManagerShutdownReport, McpError> {
        if self
            .shutdown_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(McpError::new(McpErrorCode::Unavailable));
        }
        self.shutdown.cancel();
        for server in self.servers.values() {
            server.connection.begin_shutdown();
        }

        let mut tasks = JoinSet::new();
        for server in self.servers.values() {
            let _operation = server.operation.lock().await;
            let client = server.client.lock().await.take();
            let connection = server.connection.clone();
            let drain_timeout = server.restart.config.lifecycle().cancellation_timeout();
            tasks.spawn(async move {
                let client = match client {
                    Some(client) => Some(client.shutdown().await),
                    None => None,
                };
                let drained = connection.wait_for_drain(drain_timeout).await;
                ShutdownOutcome { client, drained }
            });
        }

        let mut report = McpManagerShutdownReport::new(self.servers.len());
        while let Some(outcome) = tasks.join_next().await {
            match outcome {
                Ok(outcome) => report.record(&outcome),
                Err(_) => report.failed_shutdown_count += 1,
            }
        }
        Ok(report)
    }

    async fn wait_reconnect_backoff(&self, duration: std::time::Duration) -> Result<(), McpError> {
        tokio::select! {
            biased;
            () = self.shutdown.cancelled() => Err(McpError::new(McpErrorCode::Cancellation)),
            () = tokio::time::sleep(duration) => Ok(()),
        }
    }
}

impl fmt::Debug for McpManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpManager")
            .field("server_count", &self.servers.len())
            .field("catalog_tools", &self.catalog.len())
            .finish_non_exhaustive()
    }
}

struct ManagedServer {
    client: Mutex<Option<McpStdioClient>>,
    operation: Mutex<()>,
    restart: RestartConfig,
    connection: McpConnectionSlot,
    catalog: Option<McpToolCatalog>,
    snapshot: Option<McpServerSnapshot>,
}

struct ShutdownOutcome {
    client: Option<Result<McpStdioShutdownReport, McpError>>,
    drained: bool,
}

/// Bounded secret-independent proof of awaited manager shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpManagerShutdownReport {
    server_count: usize,
    client_shutdown_count: usize,
    failed_shutdown_count: usize,
    forced_termination_count: usize,
    undrained_server_count: usize,
    retained_stderr_bytes: usize,
    dropped_stderr_bytes: u64,
}

impl McpManagerShutdownReport {
    const fn new(server_count: usize) -> Self {
        Self {
            server_count,
            client_shutdown_count: 0,
            failed_shutdown_count: 0,
            forced_termination_count: 0,
            undrained_server_count: 0,
            retained_stderr_bytes: 0,
            dropped_stderr_bytes: 0,
        }
    }

    fn record(&mut self, outcome: &ShutdownOutcome) {
        match outcome.client {
            Some(Ok(client)) => {
                self.client_shutdown_count += 1;
                self.forced_termination_count += usize::from(client.forced_termination());
                self.retained_stderr_bytes = self
                    .retained_stderr_bytes
                    .saturating_add(client.retained_stderr_bytes());
                self.dropped_stderr_bytes = self
                    .dropped_stderr_bytes
                    .saturating_add(client.dropped_stderr_bytes());
            }
            Some(Err(_)) => self.failed_shutdown_count += 1,
            None => {}
        }
        self.undrained_server_count += usize::from(!outcome.drained);
    }

    /// Returns how many configured servers entered stopped state.
    #[must_use]
    pub const fn server_count(self) -> usize {
        self.server_count
    }

    /// Returns how many owned clients completed awaited shutdown successfully.
    #[must_use]
    pub const fn client_shutdown_count(self) -> usize {
        self.client_shutdown_count
    }

    /// Returns how many owned client shutdown operations failed.
    #[must_use]
    pub const fn failed_shutdown_count(self) -> usize {
        self.failed_shutdown_count
    }

    /// Returns how many owned processes required TERM or KILL escalation.
    #[must_use]
    pub const fn forced_termination_count(self) -> usize {
        self.forced_termination_count
    }

    /// Returns how many servers retained an external active-call lease at the
    /// end of their configured cancellation deadline.
    #[must_use]
    pub const fn undrained_server_count(self) -> usize {
        self.undrained_server_count
    }

    /// Returns the aggregate private stderr bytes destroyed after shutdown.
    #[must_use]
    pub const fn retained_stderr_bytes(self) -> usize {
        self.retained_stderr_bytes
    }

    /// Returns the aggregate stderr bytes discarded by bounded rings.
    #[must_use]
    pub const fn dropped_stderr_bytes(self) -> u64 {
        self.dropped_stderr_bytes
    }
}

impl ManagedServer {
    fn health(
        &self,
        server_id: McpServerId,
        observed_at: ProtocolTimestamp,
    ) -> Result<McpServerHealth, McpError> {
        self.connection.health(
            server_id,
            self.snapshot
                .as_ref()
                .map(|snapshot| snapshot.catalog_digest().clone()),
            observed_at,
        )
    }
}

#[derive(Clone)]
struct RestartConfig {
    config: McpServerConfig,
    trust: ToolTrust,
    environment: Vec<(OsString, OsString)>,
    executable_identity: Option<McpExecutableIdentity>,
}

impl From<McpServerLaunch> for RestartConfig {
    fn from(launch: McpServerLaunch) -> Self {
        let McpServerLaunch {
            config,
            trust,
            environment,
            executable_identity,
        } = launch;
        Self {
            config,
            trust,
            environment,
            executable_identity,
        }
    }
}

struct InitializedServer {
    client: McpStdioClient,
    catalog: McpToolCatalog,
    snapshot: McpServerSnapshot,
}

struct InitializationOutcome {
    restart: RestartConfig,
    result: Result<InitializedServer, McpError>,
}

async fn run_initializations(
    launches: Vec<McpServerLaunch>,
    max_concurrent_startups: usize,
) -> (BTreeMap<McpServerId, InitializationOutcome>, bool) {
    let mut pending = launches.into_iter();
    let mut tasks = JoinSet::new();
    while tasks.len() < max_concurrent_startups {
        let Some(launch) = pending.next() else {
            break;
        };
        tasks.spawn(initialize_launch(launch));
    }

    let mut outcomes = BTreeMap::new();
    let mut join_failed = false;
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok((server_id, outcome)) => {
                outcomes.insert(server_id, outcome);
            }
            Err(_) => join_failed = true,
        }
        if let Some(launch) = pending.next() {
            tasks.spawn(initialize_launch(launch));
        }
    }
    (outcomes, join_failed)
}

fn build_servers(
    outcomes: BTreeMap<McpServerId, InitializationOutcome>,
) -> Result<BTreeMap<McpServerId, ManagedServer>, McpError> {
    outcomes
        .into_iter()
        .map(|(server_id, outcome)| {
            let server = match outcome.result {
                Ok(initialized) => ManagedServer {
                    connection: McpConnectionSlot::ready(initialized.client.execution_handle()?),
                    client: Mutex::new(Some(initialized.client)),
                    operation: Mutex::new(()),
                    restart: outcome.restart,
                    catalog: Some(initialized.catalog),
                    snapshot: Some(initialized.snapshot),
                },
                Err(error) => ManagedServer {
                    client: Mutex::new(None),
                    operation: Mutex::new(()),
                    restart: outcome.restart,
                    connection: McpConnectionSlot::unhealthy(error.code()),
                    catalog: None,
                    snapshot: None,
                },
            };
            Ok((server_id, server))
        })
        .collect()
}

async fn initialize_launch(launch: McpServerLaunch) -> (McpServerId, InitializationOutcome) {
    let restart = RestartConfig::from(launch);
    let server_id = restart.config.id().clone();
    let result = initialize_restart(&restart, None).await;
    (server_id, InitializationOutcome { restart, result })
}

async fn initialize_restart(
    restart: &RestartConfig,
    cancellation: Option<&CancellationToken>,
) -> Result<InitializedServer, McpError> {
    let deadline = Instant::now() + restart.config.lifecycle().startup_timeout();
    let executable_identity = restart
        .executable_identity
        .as_ref()
        .ok_or_else(|| McpError::new(McpErrorCode::Startup))?;
    let client = McpStdioClient::start_until(
        &restart.config,
        restart.environment.clone(),
        executable_identity,
        deadline,
        cancellation,
    )
    .await?;
    let discovered = {
        let discovery = tokio::time::timeout_at(
            deadline,
            client.discover_catalog(&restart.config, restart.trust),
        );
        tokio::pin!(discovery);
        if let Some(cancellation) = cancellation {
            tokio::select! {
                biased;
                () = cancellation.cancelled() => None,
                result = &mut discovery => Some(result),
            }
        } else {
            Some(discovery.await)
        }
    };
    let Some(discovered) = discovered else {
        let _ = client.shutdown().await;
        return Err(McpError::new(McpErrorCode::Cancellation));
    };
    let catalog = match discovered {
        Ok(Ok(catalog)) => catalog,
        Ok(Err(error)) => {
            let _ = client.shutdown().await;
            return Err(error);
        }
        Err(_) => {
            let _ = client.shutdown().await;
            return Err(McpError::new(McpErrorCode::Timeout));
        }
    };
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        let _ = client.shutdown().await;
        return Err(McpError::new(McpErrorCode::Cancellation));
    }
    let snapshot = match client.server_snapshot(&catalog) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = client.shutdown().await;
            return Err(error);
        }
    };
    Ok(InitializedServer {
        client,
        catalog,
        snapshot,
    })
}

async fn shutdown_candidate(candidate: Result<InitializedServer, McpError>) {
    if let Ok(candidate) = candidate {
        let _ = candidate.client.shutdown().await;
    }
}

fn ready_health(
    connection: &McpConnectionSlot,
    server_id: McpServerId,
    snapshot: &McpServerSnapshot,
    observed_at: ProtocolTimestamp,
) -> Result<McpServerHealth, McpError> {
    let health = connection.health(
        server_id,
        Some(snapshot.catalog_digest().clone()),
        observed_at,
    )?;
    if health.state() != crate::McpServerState::Ready {
        return Err(McpError::new(McpErrorCode::Cancellation));
    }
    Ok(health)
}

fn collect_active_tools(
    active_tools: impl IntoIterator<Item = ToolName>,
) -> Result<BTreeSet<ToolName>, McpError> {
    let mut active = BTreeSet::new();
    for alias in active_tools {
        if active.len() >= MAX_MCP_ACTIVE_TOOLS || !active.insert(alias) {
            return Err(McpError::new(McpErrorCode::Configuration));
        }
    }
    Ok(active)
}

fn validate_launches(
    launches: &[McpServerLaunch],
    active_tools: &BTreeSet<ToolName>,
) -> Result<BTreeSet<McpServerId>, McpError> {
    let mut server_ids = BTreeSet::new();
    let mut aliases = BTreeMap::new();
    for launch in launches {
        if !server_ids.insert(launch.config.id().clone()) {
            return Err(McpError::new(McpErrorCode::Configuration));
        }
        for policy in launch.config.tools() {
            if !policy.is_enabled() {
                continue;
            }
            let alias = policy
                .resolved_alias(launch.config.id())
                .ok_or_else(|| McpError::new(McpErrorCode::PolicyDeclaration))?;
            if aliases.insert(alias, launch.config.id().clone()).is_some() {
                return Err(McpError::new(McpErrorCode::Configuration));
            }
        }
    }
    active_tools
        .iter()
        .map(|alias| {
            aliases
                .get(alias)
                .cloned()
                .ok_or_else(|| McpError::new(McpErrorCode::Configuration))
        })
        .collect()
}

async fn shutdown_outcomes(outcomes: BTreeMap<McpServerId, InitializationOutcome>) {
    for initialized in outcomes
        .into_values()
        .filter_map(|outcome| outcome.result.ok())
    {
        let _ = initialized.client.shutdown().await;
    }
}

async fn shutdown_servers(servers: BTreeMap<McpServerId, ManagedServer>) {
    for server in servers.into_values() {
        if let Some(client) = server.client.into_inner() {
            let _ = client.shutdown().await;
        }
    }
}
