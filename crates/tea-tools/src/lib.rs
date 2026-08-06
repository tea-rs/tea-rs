#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Portable tool specifications and execution contracts for `tea-rs`.
//!
//! # Example
//!
//! ```
//! use std::str::FromStr;
//!
//! use tea_protocol::ToolIdempotency;
//! use tea_tools::{
//!     SchedulerClass, ToolConcurrency, ToolEffect, ToolExecutionSemantics,
//!     ToolName, ToolRetrySafety, ToolSpec, ToolTimeout, ToolVersion,
//! };
//! use serde_json::json;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let tool = ToolSpec::new(
//!     ToolName::from_str("read_file")?,
//!     ToolVersion::from_str("1.0.0")?,
//!     "Reads one workspace file.",
//!     json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
//!     json!({"type":"object","properties":{"content":{"type":"string"}},"required":["content"]}),
//!     [ToolEffect::FsRead],
//!     ToolExecutionSemantics::new(
//!         ToolIdempotency::Idempotent,
//!         ToolRetrySafety::Automatic,
//!         ToolConcurrency::Parallel,
//!         ToolTimeout::from_millis(5_000)?,
//!     )?,
//! )?;
//! assert_eq!(tool.scheduler_class(), SchedulerClass::ParallelReadOnly);
//! # Ok(())
//! # }
//! ```

mod audit;
mod effect;
#[cfg(feature = "execution")]
mod executor;
mod invocation;
#[cfg(feature = "execution")]
mod registry;
mod resource;
mod result;
#[cfg(feature = "schema")]
mod schema;
mod source;
mod spec;

pub use audit::{
    TOOL_AUDIT_METADATA_NAMESPACE, ToolAuditMetadata, ToolAuditMetadataError, ToolAuditResource,
};
pub use effect::{ToolEffect, ToolEffectParseError};
#[cfg(feature = "execution")]
pub use executor::{
    BoxToolExecutionStream, ToolExecutionEvent, ToolExecutionStream, ToolExecutor, ToolProgress,
    ToolStreamValidator, ToolStreamViolation,
};
pub use invocation::{ToolInvocation, ToolInvocationError, ValidatedToolInvocation};
#[cfg(all(feature = "execution", feature = "model-projection"))]
pub use registry::ToolRoutePreference;
#[cfg(feature = "execution")]
pub use registry::{ToolBinding, ToolRegistry, ToolRegistryError};
pub use resource::{
    ArgumentResourceResolver, MAX_TOOL_RESOURCES, StaticResourceResolver, ToolResource,
    ToolResourceAccess, ToolResourceError, ToolResourceResolver,
};
pub use result::{ToolExecutionFailure, ToolExecutionFailureCode, ToolResult, ToolResultError};
#[cfg(feature = "schema")]
pub use schema::{
    CompiledToolSchema, MAX_SCHEMA_ERRORS, MAX_TOOL_VALUE_BYTES, MAX_TOOL_VALUE_DEPTH,
    SchemaCompilationError, SchemaValidationError, SchemaValidationFailure,
};
pub use source::{ToolSource, ToolSourceError, ToolSourceKind, ToolTrust};
pub use spec::{
    SchedulerClass, ToolConcurrency, ToolExecutionSemantics, ToolIdentityParseError, ToolName,
    ToolRetrySafety, ToolSpec, ToolSpecError, ToolTimeout, ToolVersion,
};
