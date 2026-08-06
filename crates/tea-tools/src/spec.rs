use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
#[cfg(feature = "model-projection")]
use tea_model::{ModelRequestError, ModelToolDefinition};
use tea_protocol::ToolIdempotency;
use thiserror::Error;

use crate::{ToolEffect, ToolSource};

const MAX_TOOL_NAME_BYTES: usize = 128;
const MAX_TOOL_LABEL_BYTES: usize = 256;
const MAX_TOOL_DESCRIPTION_BYTES: usize = 16 * 1024;
const MAX_TOOL_HINT_BYTES: usize = 16 * 1024;
const MAX_TOOL_PROMPT_GUIDELINES: usize = 16;
const MAX_TOOL_PROMPT_GUIDELINE_BYTES: usize = 1024;
const MAX_TOOL_PROMPT_GUIDELINES_BYTES: usize = 16 * 1024;
const MAX_RENDERER_ID_BYTES: usize = 128;
const MAX_TOOL_SCHEMA_BYTES: usize = 256 * 1024;
const MAX_TOOL_SCHEMA_DEPTH: usize = 32;
const MAX_TOOL_EFFECTS: usize = 64;
const MAX_TOOL_TIMEOUT_MILLIS: u64 = 86_400_000;

/// Stable canonical tool name used for registry lookup.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ToolName(String);

impl ToolName {
    /// Returns the canonical name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ToolName {
    type Err = ToolIdentityParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut bytes = value.bytes();
        if value.len() > MAX_TOOL_NAME_BYTES
            || !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            || !bytes.all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'_' | b'-' | b'.')
            })
        {
            return Err(ToolIdentityParseError::InvalidName);
        }
        Ok(Self(value.to_owned()))
    }
}

impl Serialize for ToolName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ToolName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for ToolName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Semantic version of one tool contract.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolVersion(Version);

impl ToolVersion {
    /// Returns the parsed semantic version.
    #[must_use]
    pub const fn as_semver(&self) -> &Version {
        &self.0
    }
}

impl FromStr for ToolVersion {
    type Err = ToolIdentityParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let version = Version::parse(value).map_err(|_| ToolIdentityParseError::InvalidVersion)?;
        if version.to_string() != value {
            return Err(ToolIdentityParseError::InvalidVersion);
        }
        Ok(Self(version))
    }
}

impl fmt::Display for ToolVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Error returned when parsing tool identity values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ToolIdentityParseError {
    /// Tool name is empty, oversized, or not canonical lowercase ASCII.
    #[error("tool name is not canonical")]
    InvalidName,
    /// Tool version is not canonical semantic version text.
    #[error("tool version is not canonical semantic version text")]
    InvalidVersion,
}

/// Whether and by whom a failed or interrupted invocation may be retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRetrySafety {
    /// The invocation must never be retried automatically or explicitly.
    Never,
    /// A higher layer may retry only after an explicit informed decision.
    ExplicitOnly,
    /// The runtime may automatically retry a known-safe failure boundary.
    Automatic,
}

/// Declared concurrency constraint for one tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolConcurrency {
    /// Independent invocations may execute concurrently.
    Parallel,
    /// Invocations execute serially with mutation/unknown work.
    Serial,
    /// Invocation requires an exclusive scheduler lane.
    Exclusive,
}

/// Bounded tool timeout metadata consumed by a future kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ToolTimeout(u64);

impl ToolTimeout {
    /// Creates timeout metadata from milliseconds.
    ///
    /// # Errors
    ///
    /// Returns an error for zero or values above 24 hours.
    pub const fn from_millis(value: u64) -> Result<Self, ToolSpecError> {
        if value == 0 || value > MAX_TOOL_TIMEOUT_MILLIS {
            Err(ToolSpecError::InvalidTimeout)
        } else {
            Ok(Self(value))
        }
    }

    /// Returns timeout milliseconds.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0
    }
}

/// Recovery, retry, concurrency, and timeout semantics for a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolExecutionSemantics {
    idempotency: ToolIdempotency,
    retry_safety: ToolRetrySafety,
    concurrency: ToolConcurrency,
    timeout: ToolTimeout,
}

impl ToolExecutionSemantics {
    /// Creates validated execution semantics.
    ///
    /// # Errors
    ///
    /// Rejects automatic retry for non-idempotent invocations.
    pub const fn new(
        idempotency: ToolIdempotency,
        retry_safety: ToolRetrySafety,
        concurrency: ToolConcurrency,
        timeout: ToolTimeout,
    ) -> Result<Self, ToolSpecError> {
        if matches!(idempotency, ToolIdempotency::NonIdempotent)
            && matches!(retry_safety, ToolRetrySafety::Automatic)
        {
            return Err(ToolSpecError::UnsafeAutomaticRetry);
        }
        Ok(Self {
            idempotency,
            retry_safety,
            concurrency,
            timeout,
        })
    }

    /// Returns idempotency/reconciliation semantics.
    #[must_use]
    pub const fn idempotency(self) -> ToolIdempotency {
        self.idempotency
    }

    /// Returns retry safety.
    #[must_use]
    pub const fn retry_safety(self) -> ToolRetrySafety {
        self.retry_safety
    }

    /// Returns concurrency constraint.
    #[must_use]
    pub const fn concurrency(self) -> ToolConcurrency {
        self.concurrency
    }

    /// Returns timeout metadata.
    #[must_use]
    pub const fn timeout(self) -> ToolTimeout {
        self.timeout
    }
}

/// Conservative scheduler lane derived only from declared tool metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerClass {
    /// Known read-only, parallel-safe work.
    ParallelReadOnly,
    /// Known mutation that is idempotent/reconciled and automatically retry-safe.
    ParallelRetrySafe,
    /// Work must share a serial lane.
    Serial,
    /// Work requires an exclusive lane.
    Exclusive,
    /// Unknown effects require policy and serial scheduling.
    PolicyRequired,
}

impl SchedulerClass {
    /// Returns whether explicit policy evaluation is mandatory.
    #[must_use]
    pub const fn requires_policy(self) -> bool {
        matches!(self, Self::PolicyRequired)
    }

    /// Returns whether this class permits concurrent execution.
    #[must_use]
    pub const fn allows_parallel_execution(self) -> bool {
        matches!(self, Self::ParallelReadOnly | Self::ParallelRetrySafe)
    }
}

/// Portable specification separated from tool executor behavior.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    name: ToolName,
    version: ToolVersion,
    label: Option<String>,
    description: String,
    input_schema: Value,
    output_schema: Value,
    effects: Vec<ToolEffect>,
    source: ToolSource,
    execution: ToolExecutionSemantics,
    prompt_snippet: Option<String>,
    prompt_guidelines: Vec<String>,
    ui_renderer: Option<String>,
}

impl ToolSpec {
    /// Creates a validated portable tool specification.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid text, schemas, effects, or unsafe execution
    /// semantics.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: ToolName,
        version: ToolVersion,
        description: impl Into<String>,
        input_schema: Value,
        output_schema: Value,
        effects: impl IntoIterator<Item = ToolEffect>,
        execution: ToolExecutionSemantics,
    ) -> Result<Self, ToolSpecError> {
        let description = description.into();
        validate_text(&description, MAX_TOOL_DESCRIPTION_BYTES)
            .map_err(|()| ToolSpecError::InvalidDescription)?;
        validate_object_schema(&input_schema)?;
        validate_object_schema(&output_schema)?;
        let effects = effects.into_iter().collect::<BTreeSet<_>>();
        if effects.is_empty() {
            return Err(ToolSpecError::MissingEffects);
        }
        if effects.len() > MAX_TOOL_EFFECTS {
            return Err(ToolSpecError::TooManyEffects);
        }
        Ok(Self {
            name,
            version,
            label: None,
            description,
            input_schema,
            output_schema,
            effects: effects.into_iter().collect(),
            source: ToolSource::native_product(),
            execution,
            prompt_snippet: None,
            prompt_guidelines: Vec::new(),
            ui_renderer: None,
        })
    }

    /// Replaces the default native provenance with one validated frozen source.
    #[must_use]
    pub fn with_source(mut self, source: ToolSource) -> Self {
        self.source = source;
        self
    }

    /// Adds a bounded human-readable label kept out of model tool definitions.
    ///
    /// # Errors
    ///
    /// Returns an error for empty, oversized, or null-containing text.
    pub fn with_label(mut self, label: impl Into<String>) -> Result<Self, ToolSpecError> {
        let label = label.into();
        validate_text(&label, MAX_TOOL_LABEL_BYTES).map_err(|()| ToolSpecError::InvalidLabel)?;
        self.label = Some(label);
        Ok(self)
    }

    /// Adds a bounded model prompt hint.
    ///
    /// # Errors
    ///
    /// Returns an error for empty, oversized, or null-containing text.
    pub fn with_prompt_hint(mut self, hint: impl Into<String>) -> Result<Self, ToolSpecError> {
        let hint = hint.into();
        validate_text(&hint, MAX_TOOL_HINT_BYTES).map_err(|()| ToolSpecError::InvalidPromptHint)?;
        self.prompt_snippet = Some(hint);
        Ok(self)
    }

    /// Adds one bounded model prompt snippet.
    ///
    /// # Errors
    ///
    /// Returns an error for empty, oversized, or null-containing text.
    pub fn with_prompt_snippet(
        mut self,
        snippet: impl Into<String>,
    ) -> Result<Self, ToolSpecError> {
        let snippet = snippet.into();
        validate_text(&snippet, MAX_TOOL_HINT_BYTES)
            .map_err(|()| ToolSpecError::InvalidPromptSnippet)?;
        self.prompt_snippet = Some(snippet);
        Ok(self)
    }

    /// Adds bounded model prompt guidelines.
    ///
    /// # Errors
    ///
    /// Returns an error when a guideline is invalid or the collection exceeds
    /// its documented bounds.
    pub fn with_prompt_guidelines<I, S>(mut self, guidelines: I) -> Result<Self, ToolSpecError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut bounded = Vec::new();
        let mut total_bytes = 0;
        for guideline in guidelines {
            if bounded.len() == MAX_TOOL_PROMPT_GUIDELINES {
                return Err(ToolSpecError::TooManyPromptGuidelines);
            }
            let guideline = guideline.into();
            validate_text(&guideline, MAX_TOOL_PROMPT_GUIDELINE_BYTES)
                .map_err(|()| ToolSpecError::InvalidPromptGuideline)?;
            total_bytes += guideline.len();
            if total_bytes > MAX_TOOL_PROMPT_GUIDELINES_BYTES {
                return Err(ToolSpecError::TooManyPromptGuidelines);
            }
            bounded.push(guideline);
        }
        self.prompt_guidelines = bounded;
        Ok(self)
    }

    /// Adds a bounded renderer selector with no UI implementation dependency.
    ///
    /// # Errors
    ///
    /// Returns an error when the selector is not canonical.
    pub fn with_ui_renderer(mut self, renderer: impl Into<String>) -> Result<Self, ToolSpecError> {
        let renderer = renderer.into();
        if renderer.is_empty()
            || renderer.len() > MAX_RENDERER_ID_BYTES
            || !renderer.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b'_' | b'.')
            })
        {
            return Err(ToolSpecError::InvalidRenderer);
        }
        self.ui_renderer = Some(renderer);
        Ok(self)
    }

    /// Projects this specification into the model-facing tool contract.
    ///
    /// # Errors
    ///
    /// Returns an error if model-layer bounds are stricter than this contract.
    #[cfg(feature = "model-projection")]
    pub fn to_model_definition(&self) -> Result<ModelToolDefinition, ToolSpecError> {
        ModelToolDefinition::new(
            self.name.as_str(),
            self.description.clone(),
            self.input_schema.clone(),
        )
        .map_err(ToolSpecError::ModelProjection)
    }

    /// Derives conservative scheduler behavior from metadata only.
    #[must_use]
    pub fn scheduler_class(&self) -> SchedulerClass {
        if self.effects.iter().any(ToolEffect::is_unknown) {
            return SchedulerClass::PolicyRequired;
        }
        if matches!(self.execution.concurrency, ToolConcurrency::Exclusive) {
            return SchedulerClass::Exclusive;
        }
        if matches!(self.execution.concurrency, ToolConcurrency::Serial)
            || matches!(self.execution.idempotency, ToolIdempotency::NonIdempotent)
        {
            return SchedulerClass::Serial;
        }
        if self.effects.iter().all(ToolEffect::is_read_only) {
            return SchedulerClass::ParallelReadOnly;
        }
        if matches!(self.execution.retry_safety, ToolRetrySafety::Automatic) {
            SchedulerClass::ParallelRetrySafe
        } else {
            SchedulerClass::Serial
        }
    }

    /// Returns the stable tool name.
    #[must_use]
    pub const fn name(&self) -> &ToolName {
        &self.name
    }

    /// Returns the semantic contract version.
    #[must_use]
    pub const fn version(&self) -> &ToolVersion {
        &self.version
    }

    /// Returns the optional human-readable label.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Returns the model-visible description.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns the input object schema.
    #[must_use]
    pub const fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    /// Returns the output object schema.
    #[must_use]
    pub const fn output_schema(&self) -> &Value {
        &self.output_schema
    }

    /// Returns sorted, deduplicated effects.
    #[must_use]
    pub fn effects(&self) -> &[ToolEffect] {
        &self.effects
    }

    /// Returns frozen provider-neutral tool provenance.
    #[must_use]
    pub const fn source(&self) -> &ToolSource {
        &self.source
    }

    /// Returns execution semantics.
    #[must_use]
    pub const fn execution(&self) -> ToolExecutionSemantics {
        self.execution
    }

    /// Returns the optional model prompt hint.
    #[must_use]
    pub fn prompt_hint(&self) -> Option<&str> {
        self.prompt_snippet()
    }

    /// Returns the optional model prompt snippet.
    #[must_use]
    pub fn prompt_snippet(&self) -> Option<&str> {
        self.prompt_snippet.as_deref()
    }

    /// Returns bounded model prompt guidelines in declaration order.
    #[must_use]
    pub fn prompt_guidelines(&self) -> &[String] {
        &self.prompt_guidelines
    }

    /// Returns the optional UI renderer selector.
    #[must_use]
    pub fn ui_renderer(&self) -> Option<&str> {
        self.ui_renderer.as_deref()
    }
}

/// Error returned when constructing tool specifications.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ToolSpecError {
    /// Description is empty, oversized, or contains a null character.
    #[error("tool description is invalid")]
    InvalidDescription,
    /// Input or output schema must be a bounded object schema.
    #[error("tool schema must be a bounded JSON object schema")]
    InvalidSchema,
    /// Tool must declare at least one effect.
    #[error("tool must declare at least one effect")]
    MissingEffects,
    /// Tool declares too many effects.
    #[error("tool declares too many effects")]
    TooManyEffects,
    /// Timeout is zero or above 24 hours.
    #[error("tool timeout is outside supported bounds")]
    InvalidTimeout,
    /// Automatic retry is unsafe for non-idempotent execution.
    #[error("non-idempotent tool cannot allow automatic retry")]
    UnsafeAutomaticRetry,
    /// Prompt hint is invalid.
    #[error("tool prompt hint is invalid")]
    InvalidPromptHint,
    /// Human-readable tool label is invalid.
    #[error("tool label is invalid")]
    InvalidLabel,
    /// Prompt snippet is invalid.
    #[error("tool prompt snippet is invalid")]
    InvalidPromptSnippet,
    /// One prompt guideline is invalid.
    #[error("tool prompt guideline is invalid")]
    InvalidPromptGuideline,
    /// Prompt guideline collection exceeds documented bounds.
    #[error("tool has too many prompt guidelines")]
    TooManyPromptGuidelines,
    /// Renderer selector is invalid.
    #[error("tool renderer selector is invalid")]
    InvalidRenderer,
    /// Model projection rejected the tool definition.
    #[cfg(feature = "model-projection")]
    #[error("tool cannot be projected to model definition: {0}")]
    ModelProjection(ModelRequestError),
}

fn validate_text(value: &str, max_bytes: usize) -> Result<(), ()> {
    if value.is_empty() || value.len() > max_bytes || value.contains('\0') {
        Err(())
    } else {
        Ok(())
    }
}

fn validate_object_schema(value: &Value) -> Result<(), ToolSpecError> {
    let object = value.as_object().ok_or(ToolSpecError::InvalidSchema)?;
    if object.get("type").and_then(Value::as_str) != Some("object")
        || serde_json::to_vec(value)
            .map_err(|_| ToolSpecError::InvalidSchema)?
            .len()
            > MAX_TOOL_SCHEMA_BYTES
        || json_depth(value) > MAX_TOOL_SCHEMA_DEPTH
    {
        return Err(ToolSpecError::InvalidSchema);
    }
    Ok(())
}

fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => 1 + values.iter().map(json_depth).max().unwrap_or(0),
        Value::Object(values) => 1 + values.values().map(json_depth).max().unwrap_or(0),
        _ => 1,
    }
}
