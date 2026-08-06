#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Provider-neutral model port for `tea-rs`.
//!
//! This crate defines model contracts but does not contain a live provider
//! adapter or network transport.
//!
//! # Example
//!
//! ```
//! use std::str::FromStr;
//!
//! use tea_model::{
//!     ModelCapabilities, ModelDisplayName, ModelSpec, ProviderId,
//! };
//! use tea_protocol::{ModelId, TokenCount};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let model = ModelSpec::new(
//!     ModelId::from_str("example/model")?,
//!     ProviderId::from_str("example")?,
//!     ModelDisplayName::from_str("Example Model")?,
//!     TokenCount::new(32_000)?,
//!     TokenCount::new(8_000)?,
//!     ModelCapabilities::text().with_reasoning().with_tools(true),
//! )?;
//!
//! assert!(model.capabilities().supports_parallel_tool_calls());
//! # Ok(())
//! # }
//! ```

mod failure;
mod hosted;
mod provider;
mod request;
mod router;
mod spec;
mod stream;

pub use failure::{ModelFailure, ModelFailureCode};
pub use hosted::{
    HostedToolKind, HostedToolOptions, MAX_WEB_SEARCH_DOMAIN_BYTES, MAX_WEB_SEARCH_DOMAINS,
    MAX_WEB_SEARCH_LOCATION_FIELD_BYTES, WebSearchLocation, WebSearchOptions,
};
pub use provider::{BoxModelStream, ModelProvider, ModelStream};
pub use request::{
    FunctionToolDefinition, HostedToolDefinition, MAX_MODEL_TOOLS, MAX_REQUEST_MESSAGES,
    MAX_SYSTEM_PROMPT_BYTES, MAX_TOOL_DESCRIPTION_BYTES, MAX_TOOL_SCHEMA_BYTES,
    MAX_TOOL_SCHEMA_DEPTH, ModelRequest, ModelRequestError, ModelToolDefinition, ReasoningOptions,
};
pub use router::{ModelRegistry, ModelRegistryError, ModelRouter};
pub use spec::{
    ModelCapabilities, ModelDisplayName, ModelSpec, ModelSpecError, ModelTextParseError,
    ReasoningProfile, ReasoningResolution,
};
pub use stream::{
    HostedToolCompleted, HostedToolStarted, MAX_MODEL_DELTA_BYTES, MAX_MODEL_STREAM_INDEX,
    MAX_PROVIDER_OPAQUE_ID_BYTES, ModelCompletion, ModelEvent, ModelResponseInfo,
    ModelSourceCitation, ModelStreamIndex, ModelStreamSummary, ModelStreamValidator,
    ModelStreamValueError, ModelStreamViolation, ProviderResponseId, ProviderToolCallId,
    ToolArgumentsDelta, ToolCallCompleted, ToolCallStarted, Utf8Delta,
};
pub use tea_control::CancellationScope as ModelCancellation;
pub use tea_protocol::{ModelRef, ProviderId, ReasoningEffort};
