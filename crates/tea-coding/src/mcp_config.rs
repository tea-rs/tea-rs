//! Trusted MCP settings conversion and late child-environment resolution.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr as _;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tea_mcp::{
    MAX_MCP_ENVIRONMENT_NAME_BYTES, MAX_MCP_ENVIRONMENT_VARIABLES, McpArgumentResource,
    McpLifecyclePolicy, McpLimits, McpReconnectPolicy, McpRemoteToolName, McpServerConfig,
    McpServerId, McpToolDeclaration, McpToolPolicy, McpTransportConfig,
};
use tea_protocol::ToolIdempotency;
use tea_tools::{
    ToolConcurrency, ToolEffect, ToolExecutionSemantics, ToolName, ToolResourceAccess,
    ToolRetrySafety, ToolTimeout,
};

use crate::{CodingError, CodingErrorCode};

/// Maximum explicitly configured MCP servers in one resolved product profile.
pub const MAX_CONFIGURED_MCP_SERVERS: usize = 32;
/// Maximum bytes admitted from one resolved environment value.
pub const MAX_MCP_ENVIRONMENT_VALUE_BYTES: usize = 64 * 1024;
/// Maximum aggregate bytes admitted into one MCP child environment.
pub const MAX_MCP_ENVIRONMENT_VALUE_TOTAL_BYTES: usize = 256 * 1024;

/// Strict serializable settings for one explicitly configured MCP server.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpServerSettings {
    /// Stable canonical server ID.
    pub id: String,
    /// Exact shell-free transport settings.
    pub transport: McpTransportSettings,
    /// Exact environment variable names resolved only at spawn time.
    #[serde(default)]
    pub inherited_environment: Vec<String>,
    /// Host-owned remote tool policies.
    #[serde(default)]
    pub tools: Vec<McpToolSettings>,
    /// Optional protocol and output limit overrides.
    #[serde(default)]
    pub limits: McpLimitsSettings,
    /// Optional lifecycle deadline overrides.
    #[serde(default)]
    pub lifecycle: McpLifecycleSettings,
    /// Explicit reconnect policy; absent means disabled.
    #[serde(default)]
    pub reconnect: Option<McpReconnectSettings>,
}

impl McpServerSettings {
    /// Converts trusted sparse settings into one fully validated pure config.
    ///
    /// # Errors
    ///
    /// Rejects malformed identities, paths, arguments, environment names,
    /// policies, limits, or lifecycle values before any process can start.
    pub fn resolve(&self) -> Result<McpServerConfig, CodingError> {
        let id = McpServerId::from_str(&self.id).map_err(|_| invalid_settings())?;
        let transport = self.transport.resolve()?;
        let tools = self
            .tools
            .iter()
            .map(McpToolSettings::resolve)
            .collect::<Result<Vec<_>, _>>()?;
        let reconnect = self
            .reconnect
            .as_ref()
            .map(McpReconnectSettings::resolve)
            .transpose()?
            .unwrap_or_default();
        McpServerConfig::new(
            id,
            transport,
            self.inherited_environment.clone(),
            tools,
            self.limits.resolve()?,
            self.lifecycle.resolve()?,
            reconnect,
        )
        .map_err(|_| invalid_settings())
    }
}

impl fmt::Debug for McpServerSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpServerSettings")
            .field("id", &self.id)
            .field("transport", &self.transport)
            .field(
                "inherited_environment_count",
                &self.inherited_environment.len(),
            )
            .field("tool_count", &self.tools.len())
            .field("limits", &self.limits)
            .field("lifecycle", &self.lifecycle)
            .field("reconnect", &self.reconnect)
            .finish()
    }
}

/// Supported serializable MCP transport settings.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpTransportSettings {
    /// Exact stdio executable and argument vector.
    Stdio {
        /// Absolute executable path. It is never resolved through `PATH`.
        executable: PathBuf,
        /// Exact UTF-8 argument vector passed without a shell.
        #[serde(default)]
        arguments: Vec<String>,
    },
}

impl McpTransportSettings {
    fn resolve(&self) -> Result<McpTransportConfig, CodingError> {
        match self {
            Self::Stdio {
                executable,
                arguments,
            } => {
                McpTransportConfig::stdio(executable.clone(), arguments.iter().map(OsString::from))
                    .map_err(|_| invalid_settings())
            }
        }
    }
}

impl fmt::Debug for McpTransportSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stdio { arguments, .. } => formatter
                .debug_struct("Stdio")
                .field("executable", &"<redacted>")
                .field("argument_count", &arguments.len())
                .finish(),
        }
    }
}

/// Strict settings for one exact remote MCP tool policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpToolSettings {
    /// Exact remote tool name.
    pub remote_name: String,
    /// Optional canonical local alias.
    #[serde(default)]
    pub alias: Option<String>,
    /// Complete declaration that enables the tool; absent keeps it disabled.
    #[serde(default)]
    pub declaration: Option<McpToolDeclarationSettings>,
}

impl McpToolSettings {
    fn resolve(&self) -> Result<McpToolPolicy, CodingError> {
        let remote =
            McpRemoteToolName::new(self.remote_name.clone()).map_err(|_| invalid_settings())?;
        let mut policy = if let Some(declaration) = &self.declaration {
            McpToolPolicy::enabled(remote, declaration.resolve()?)
        } else {
            McpToolPolicy::new(remote)
        };
        if let Some(alias) = &self.alias {
            policy = policy.with_alias(ToolName::from_str(alias).map_err(|_| invalid_settings())?);
        }
        Ok(policy)
    }
}

/// Complete host-owned effects, resources, and execution declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpToolDeclarationSettings {
    /// Authoritative effects; remote annotations cannot narrow these.
    pub effects: Vec<ToolEffect>,
    /// Pure top-level argument resource mappings.
    #[serde(default)]
    pub resources: Vec<McpArgumentResourceSettings>,
    /// Invocation idempotency classification.
    pub idempotency: ToolIdempotency,
    /// Permitted retry behavior.
    pub retry_safety: McpRetrySafetySettings,
    /// Scheduler concurrency behavior.
    pub concurrency: McpConcurrencySettings,
    /// Caller-owned invocation timeout.
    pub timeout_millis: u64,
}

impl McpToolDeclarationSettings {
    fn resolve(&self) -> Result<McpToolDeclaration, CodingError> {
        let resources = self
            .resources
            .iter()
            .map(McpArgumentResourceSettings::resolve)
            .collect::<Result<Vec<_>, _>>()?;
        let timeout =
            ToolTimeout::from_millis(self.timeout_millis).map_err(|_| invalid_settings())?;
        let execution = ToolExecutionSemantics::new(
            self.idempotency,
            self.retry_safety.into(),
            self.concurrency.into(),
            timeout,
        )
        .map_err(|_| invalid_settings())?;
        McpToolDeclaration::new(self.effects.clone(), resources, execution)
            .map_err(|_| invalid_settings())
    }
}

/// Serializable argument-to-resource mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpArgumentResourceSettings {
    /// Top-level string argument name.
    pub argument: String,
    /// Canonical resource scheme.
    pub scheme: String,
    /// Requested resource access.
    pub access: ToolResourceAccess,
}

impl McpArgumentResourceSettings {
    fn resolve(&self) -> Result<McpArgumentResource, CodingError> {
        McpArgumentResource::new(&self.argument, &self.scheme, self.access)
            .map_err(|_| invalid_settings())
    }
}

/// Serializable retry-safety setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpRetrySafetySettings {
    /// Never retry the invocation.
    Never,
    /// Retry only after an explicit informed decision.
    ExplicitOnly,
    /// Permit automatic retry at known-safe boundaries.
    Automatic,
}

impl From<McpRetrySafetySettings> for ToolRetrySafety {
    fn from(value: McpRetrySafetySettings) -> Self {
        match value {
            McpRetrySafetySettings::Never => Self::Never,
            McpRetrySafetySettings::ExplicitOnly => Self::ExplicitOnly,
            McpRetrySafetySettings::Automatic => Self::Automatic,
        }
    }
}

/// Serializable tool concurrency setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpConcurrencySettings {
    /// Independent invocations may execute concurrently.
    Parallel,
    /// Invocations share a serial lane.
    Serial,
    /// Invocations require an exclusive lane.
    Exclusive,
}

impl From<McpConcurrencySettings> for ToolConcurrency {
    fn from(value: McpConcurrencySettings) -> Self {
        match value {
            McpConcurrencySettings::Parallel => Self::Parallel,
            McpConcurrencySettings::Serial => Self::Serial,
            McpConcurrencySettings::Exclusive => Self::Exclusive,
        }
    }
}

/// Sparse protocol and output limit overrides.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpLimitsSettings {
    /// Maximum raw frame bytes.
    pub max_frame_bytes: Option<usize>,
    /// Maximum frozen descriptor bytes.
    pub max_descriptor_bytes: Option<usize>,
    /// Maximum terminal result bytes.
    pub max_result_bytes: Option<usize>,
    /// Maximum private retained stderr bytes.
    pub max_stderr_bytes: Option<usize>,
    /// Maximum tools admitted from the descriptor.
    pub max_tools: Option<usize>,
    /// Maximum notifications between lifecycle checkpoints.
    pub max_notifications: Option<usize>,
    /// Maximum progress events for one invocation.
    pub max_progress_events: Option<usize>,
    /// Maximum concurrent requests to one server.
    pub max_in_flight_requests: Option<usize>,
}

impl McpLimitsSettings {
    fn resolve(&self) -> Result<McpLimits, CodingError> {
        let mut limits = McpLimits::default();
        macro_rules! apply_limit {
            ($field:ident, $method:ident) => {
                if let Some(value) = self.$field {
                    limits = limits.$method(value).map_err(|_| invalid_settings())?;
                }
            };
        }
        apply_limit!(max_frame_bytes, with_max_frame_bytes);
        apply_limit!(max_descriptor_bytes, with_max_descriptor_bytes);
        apply_limit!(max_result_bytes, with_max_result_bytes);
        apply_limit!(max_stderr_bytes, with_max_stderr_bytes);
        apply_limit!(max_tools, with_max_tools);
        apply_limit!(max_notifications, with_max_notifications);
        apply_limit!(max_progress_events, with_max_progress_events);
        apply_limit!(max_in_flight_requests, with_max_in_flight_requests);
        Ok(limits)
    }
}

/// Sparse lifecycle timeout overrides in milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpLifecycleSettings {
    /// Process startup timeout.
    pub startup_timeout_millis: Option<u64>,
    /// Protocol handshake timeout.
    pub handshake_timeout_millis: Option<u64>,
    /// Cooperative cancellation timeout.
    pub cancellation_timeout_millis: Option<u64>,
    /// Graceful service shutdown timeout.
    pub graceful_shutdown_timeout_millis: Option<u64>,
    /// Process termination timeout.
    pub termination_timeout_millis: Option<u64>,
    /// Forced-kill timeout.
    pub kill_timeout_millis: Option<u64>,
}

impl McpLifecycleSettings {
    fn resolve(&self) -> Result<McpLifecyclePolicy, CodingError> {
        let defaults = McpLifecyclePolicy::default();
        McpLifecyclePolicy::new(
            selected_duration(self.startup_timeout_millis, defaults.startup_timeout())?,
            selected_duration(self.handshake_timeout_millis, defaults.handshake_timeout())?,
            selected_duration(
                self.cancellation_timeout_millis,
                defaults.cancellation_timeout(),
            )?,
            selected_duration(
                self.graceful_shutdown_timeout_millis,
                defaults.graceful_shutdown_timeout(),
            )?,
            selected_duration(
                self.termination_timeout_millis,
                defaults.termination_timeout(),
            )?,
            selected_duration(self.kill_timeout_millis, defaults.kill_timeout())?,
        )
        .map_err(|_| invalid_settings())
    }
}

/// Explicit bounded reconnect settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpReconnectSettings {
    /// Maximum reconnect attempts.
    pub max_attempts: u32,
    /// Initial reconnect delay.
    pub initial_backoff_millis: u64,
    /// Maximum reconnect delay.
    pub max_backoff_millis: u64,
}

impl McpReconnectSettings {
    fn resolve(&self) -> Result<McpReconnectPolicy, CodingError> {
        McpReconnectPolicy::bounded(
            self.max_attempts,
            Duration::from_millis(self.initial_backoff_millis),
            Duration::from_millis(self.max_backoff_millis),
        )
        .map_err(|_| invalid_settings())
    }
}

/// Stable failure classification for late MCP environment resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpEnvironmentErrorCode {
    /// The configured variable was absent from the resolver.
    MissingVariable,
    /// A variable name was invalid for a child process.
    InvalidName,
    /// A resolved value was invalid or exceeded a hard bound.
    InvalidValue,
    /// The resolver failed without exposing its private diagnostic.
    Resolution,
}

/// Name-only, value-independent MCP environment resolution failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpEnvironmentError {
    code: McpEnvironmentErrorCode,
    name: String,
}

impl McpEnvironmentError {
    /// Creates a value-independent error for one requested variable name.
    ///
    /// Invalid names are replaced with a constant placeholder so external
    /// resolver diagnostics cannot inject control text.
    #[must_use]
    pub fn new(code: McpEnvironmentErrorCode, name: &str) -> Self {
        Self {
            code,
            name: safe_environment_name(name),
        }
    }

    /// Returns the stable failure classification.
    #[must_use]
    pub const fn code(&self) -> McpEnvironmentErrorCode {
        self.code
    }

    /// Returns only the configured variable name, never its value.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for McpEnvironmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:?}: MCP environment variable {} could not be resolved",
            self.code, self.name
        )
    }
}

impl Error for McpEnvironmentError {}

/// Bounded environment value whose debug output is always redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct McpEnvironmentValue(OsString);

impl McpEnvironmentValue {
    /// Creates a bounded value for one validated variable name.
    ///
    /// # Errors
    ///
    /// Rejects invalid names, NUL-containing values, or values over 64 KiB.
    pub fn try_new(name: &str, value: impl Into<OsString>) -> Result<Self, McpEnvironmentError> {
        if !valid_environment_name(name) {
            return Err(McpEnvironmentError::new(
                McpEnvironmentErrorCode::InvalidName,
                name,
            ));
        }
        let value = value.into();
        let (bytes, has_nul) = os_bytes(&value);
        if bytes > MAX_MCP_ENVIRONMENT_VALUE_BYTES || has_nul {
            return Err(McpEnvironmentError::new(
                McpEnvironmentErrorCode::InvalidValue,
                name,
            ));
        }
        Ok(Self(value))
    }

    /// Exposes the value only at the child-process environment boundary.
    #[must_use]
    pub fn as_os_str(&self) -> &OsStr {
        &self.0
    }

    fn into_os_string(self) -> OsString {
        self.0
    }
}

impl fmt::Debug for McpEnvironmentValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("**REDACTED**")
    }
}

/// Object-safe late resolver for explicitly allowlisted environment names.
pub trait McpEnvironmentResolver: fmt::Debug + Send + Sync {
    /// Resolves one exact name at process-spawn time.
    ///
    /// # Errors
    ///
    /// Returns only stable name/code diagnostics without the private value.
    fn resolve(&self, name: &str) -> Result<Option<McpEnvironmentValue>, McpEnvironmentError>;
}

/// Hermetic map-backed resolver for embedders and tests.
#[derive(Clone)]
pub struct StaticMcpEnvironmentResolver {
    values: BTreeMap<String, McpEnvironmentValue>,
}

impl StaticMcpEnvironmentResolver {
    /// Validates and captures a bounded injected environment.
    ///
    /// # Errors
    ///
    /// Rejects invalid/duplicate names, too many values, or oversized values.
    pub fn new(values: BTreeMap<String, OsString>) -> Result<Self, McpEnvironmentError> {
        if values.len() > MAX_MCP_ENVIRONMENT_VARIABLES {
            return Err(McpEnvironmentError::new(
                McpEnvironmentErrorCode::InvalidValue,
                "<collection>",
            ));
        }
        let mut resolved = BTreeMap::new();
        let mut canonical = BTreeSet::new();
        let mut total = 0usize;
        for (name, value) in values {
            if !canonical.insert(canonical_environment_name(&name)) {
                return Err(McpEnvironmentError::new(
                    McpEnvironmentErrorCode::InvalidName,
                    &name,
                ));
            }
            let value = McpEnvironmentValue::try_new(&name, value)?;
            total = total
                .checked_add(os_bytes(value.as_os_str()).0)
                .ok_or_else(|| {
                    McpEnvironmentError::new(McpEnvironmentErrorCode::InvalidValue, &name)
                })?;
            if total > MAX_MCP_ENVIRONMENT_VALUE_TOTAL_BYTES {
                return Err(McpEnvironmentError::new(
                    McpEnvironmentErrorCode::InvalidValue,
                    &name,
                ));
            }
            resolved.insert(canonical_environment_name(&name), value);
        }
        Ok(Self { values: resolved })
    }
}

impl fmt::Debug for StaticMcpEnvironmentResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StaticMcpEnvironmentResolver")
            .field("value_count", &self.values.len())
            .finish()
    }
}

impl McpEnvironmentResolver for StaticMcpEnvironmentResolver {
    fn resolve(&self, name: &str) -> Result<Option<McpEnvironmentValue>, McpEnvironmentError> {
        if !valid_environment_name(name) {
            return Err(McpEnvironmentError::new(
                McpEnvironmentErrorCode::InvalidName,
                name,
            ));
        }
        Ok(self.values.get(&canonical_environment_name(name)).cloned())
    }
}

/// Resolver that reads the live process environment only when called.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessMcpEnvironmentResolver;

impl McpEnvironmentResolver for ProcessMcpEnvironmentResolver {
    fn resolve(&self, name: &str) -> Result<Option<McpEnvironmentValue>, McpEnvironmentError> {
        if !valid_environment_name(name) {
            return Err(McpEnvironmentError::new(
                McpEnvironmentErrorCode::InvalidName,
                name,
            ));
        }
        std::env::var_os(name)
            .map(|value| McpEnvironmentValue::try_new(name, value))
            .transpose()
    }
}

/// Exact, bounded child environment resolved from an empty map.
pub struct McpChildEnvironment {
    values: BTreeMap<String, McpEnvironmentValue>,
}

impl McpChildEnvironment {
    /// Returns sorted exact names without exposing values.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }

    /// Clears inherited process state and applies only these reviewed values.
    pub fn apply_to(&self, command: &mut Command) {
        command.env_clear();
        command.envs(
            self.values
                .iter()
                .map(|(name, value)| (name, value.as_os_str())),
        );
    }

    /// Consumes the environment into exact process name/value pairs.
    pub fn into_variables(self) -> impl Iterator<Item = (String, OsString)> {
        self.values
            .into_iter()
            .map(|(name, value)| (name, value.into_os_string()))
    }
}

impl fmt::Debug for McpChildEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpChildEnvironment")
            .field("names", &self.values.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Resolves one server's exact allowlist into an otherwise empty environment.
///
/// On Windows, `SYSTEMROOT` is the sole reviewed platform necessity. No path,
/// proxy, credential, home, or loader variable is inherited implicitly.
///
/// # Errors
///
/// Fails closed when any required name is absent or a value exceeds bounds.
pub fn resolve_mcp_environment(
    server: &McpServerConfig,
    resolver: &dyn McpEnvironmentResolver,
) -> Result<McpChildEnvironment, McpEnvironmentError> {
    let names = PLATFORM_ENVIRONMENT_NAMES
        .iter()
        .copied()
        .chain(server.inherited_environment().iter().map(String::as_str));
    let mut values = BTreeMap::new();
    let mut canonical = BTreeSet::new();
    let mut total = 0usize;
    for name in names {
        if !canonical.insert(canonical_environment_name(name)) {
            continue;
        }
        let value = resolver.resolve(name)?.ok_or_else(|| {
            McpEnvironmentError::new(McpEnvironmentErrorCode::MissingVariable, name)
        })?;
        total = total
            .checked_add(os_bytes(value.as_os_str()).0)
            .ok_or_else(|| McpEnvironmentError::new(McpEnvironmentErrorCode::InvalidValue, name))?;
        if total > MAX_MCP_ENVIRONMENT_VALUE_TOTAL_BYTES {
            return Err(McpEnvironmentError::new(
                McpEnvironmentErrorCode::InvalidValue,
                name,
            ));
        }
        values.insert(name.to_owned(), value);
    }
    Ok(McpChildEnvironment { values })
}

pub(crate) fn merge_mcp_server_settings(
    resolved: &mut Vec<McpServerConfig>,
    layer: &[McpServerSettings],
) -> Result<(), CodingError> {
    if layer.len() > MAX_CONFIGURED_MCP_SERVERS {
        return Err(invalid_settings());
    }
    let mut servers = resolved
        .iter()
        .cloned()
        .map(|server| (server.id().as_str().to_owned(), server))
        .collect::<BTreeMap<_, _>>();
    let mut layer_ids = BTreeSet::new();
    for settings in layer {
        let server = settings.resolve()?;
        let id = server.id().as_str().to_owned();
        if !layer_ids.insert(id.clone()) {
            return Err(invalid_settings());
        }
        servers.insert(id, server);
    }
    if servers.len() > MAX_CONFIGURED_MCP_SERVERS {
        return Err(invalid_settings());
    }
    *resolved = servers.into_values().collect();
    Ok(())
}

pub(crate) fn validate_mcp_server_aliases(servers: &[McpServerConfig]) -> Result<(), CodingError> {
    let mut aliases = BTreeSet::new();
    for server in servers {
        for tool in server.tools() {
            if let Some(alias) = tool.resolved_alias(server.id())
                && !aliases.insert(alias)
            {
                return Err(invalid_settings());
            }
        }
    }
    Ok(())
}

fn selected_duration(value: Option<u64>, default: Duration) -> Result<Duration, CodingError> {
    let millis =
        value.unwrap_or(u64::try_from(default.as_millis()).map_err(|_| invalid_settings())?);
    Ok(Duration::from_millis(millis))
}

fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= MAX_MCP_ENVIRONMENT_NAME_BYTES
        && bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn safe_environment_name(name: &str) -> String {
    if valid_environment_name(name) {
        name.to_owned()
    } else {
        "<invalid>".to_owned()
    }
}

#[cfg(windows)]
fn canonical_environment_name(name: &str) -> String {
    name.to_ascii_uppercase()
}

#[cfg(not(windows))]
fn canonical_environment_name(name: &str) -> String {
    name.to_owned()
}

#[cfg(windows)]
const PLATFORM_ENVIRONMENT_NAMES: &[&str] = &["SYSTEMROOT"];
#[cfg(not(windows))]
const PLATFORM_ENVIRONMENT_NAMES: &[&str] = &[];

#[cfg(unix)]
fn os_bytes(value: &OsStr) -> (usize, bool) {
    use std::os::unix::ffi::OsStrExt as _;

    let bytes = value.as_bytes();
    (bytes.len(), bytes.contains(&0))
}

#[cfg(windows)]
fn os_bytes(value: &OsStr) -> (usize, bool) {
    use std::os::windows::ffi::OsStrExt as _;

    value
        .encode_wide()
        .fold((0usize, false), |(bytes, nul), unit| {
            (bytes.saturating_add(2), nul || unit == 0)
        })
}

#[cfg(not(any(unix, windows)))]
fn os_bytes(value: &OsStr) -> (usize, bool) {
    let value = value.to_string_lossy();
    (value.len(), value.contains('\0'))
}

fn invalid_settings() -> CodingError {
    CodingError::new(CodingErrorCode::InvalidInput, "MCP settings are invalid")
}
