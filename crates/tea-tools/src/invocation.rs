use std::sync::Arc;

use serde_json::Value;
use tea_protocol::{ProtocolMetadata, ToolCallId};
use thiserror::Error;

use crate::{SchedulerClass, ToolName, ToolResource, ToolSource, ToolSpec};

/// Untrusted complete tool invocation before registry validation.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInvocation {
    tool_call_id: ToolCallId,
    name: ToolName,
    arguments: Value,
    metadata: ProtocolMetadata,
}

impl ToolInvocation {
    /// Creates a bounded invocation with object arguments.
    ///
    /// # Errors
    ///
    /// Returns an error for non-object, oversized, or deeply nested arguments.
    pub fn new(
        tool_call_id: ToolCallId,
        name: ToolName,
        arguments: Value,
        metadata: ProtocolMetadata,
    ) -> Result<Self, ToolInvocationError> {
        if !arguments.is_object() {
            return Err(ToolInvocationError::ArgumentsMustBeObject);
        }
        if serde_json::to_vec(&arguments)
            .map_err(|_| ToolInvocationError::ArgumentsOutOfBounds)?
            .len()
            > 256 * 1024
            || json_depth(&arguments) > 32
        {
            return Err(ToolInvocationError::ArgumentsOutOfBounds);
        }
        Ok(Self {
            tool_call_id,
            name,
            arguments,
            metadata,
        })
    }

    /// Returns canonical tool-call ID.
    #[must_use]
    pub const fn tool_call_id(&self) -> &ToolCallId {
        &self.tool_call_id
    }
    /// Returns requested tool name.
    #[must_use]
    pub const fn name(&self) -> &ToolName {
        &self.name
    }
    /// Returns untrusted object arguments.
    #[must_use]
    pub const fn arguments(&self) -> &Value {
        &self.arguments
    }
    /// Returns bounded invocation metadata.
    #[must_use]
    pub const fn metadata(&self) -> &ProtocolMetadata {
        &self.metadata
    }
}

/// Invocation proven valid against a registered tool schema and resource resolver.
#[derive(Debug, Clone)]
pub struct ValidatedToolInvocation {
    invocation: ToolInvocation,
    spec: Arc<ToolSpec>,
    source: ToolSource,
    resources: Vec<ToolResource>,
}

impl ValidatedToolInvocation {
    #[cfg(feature = "execution")]
    pub(crate) fn new(
        invocation: ToolInvocation,
        spec: Arc<ToolSpec>,
        resources: Vec<ToolResource>,
    ) -> Self {
        let source = spec.source().clone();
        Self {
            invocation,
            spec,
            source,
            resources,
        }
    }

    /// Returns canonical tool-call ID.
    #[must_use]
    pub const fn tool_call_id(&self) -> &ToolCallId {
        self.invocation.tool_call_id()
    }
    /// Returns registered tool name.
    #[must_use]
    pub const fn name(&self) -> &ToolName {
        self.invocation.name()
    }
    /// Returns schema-validated object arguments.
    #[must_use]
    pub const fn arguments(&self) -> &Value {
        self.invocation.arguments()
    }
    /// Returns bounded invocation metadata.
    #[must_use]
    pub const fn metadata(&self) -> &ProtocolMetadata {
        self.invocation.metadata()
    }
    /// Returns registered tool specification.
    #[must_use]
    pub fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    /// Returns provenance frozen when registry validation succeeded.
    #[must_use]
    pub const fn source(&self) -> &ToolSource {
        &self.source
    }
    /// Returns sorted resolved resources.
    #[must_use]
    pub fn resources(&self) -> &[ToolResource] {
        &self.resources
    }
    /// Returns metadata-derived scheduler class.
    #[must_use]
    pub fn scheduler_class(&self) -> SchedulerClass {
        self.spec.scheduler_class()
    }
}

/// Error constructing an untrusted invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ToolInvocationError {
    /// Arguments must be a JSON object.
    #[error("tool arguments must be a JSON object")]
    ArgumentsMustBeObject,
    /// Arguments exceed byte or nesting bounds.
    #[error("tool arguments exceed supported bounds")]
    ArgumentsOutOfBounds,
}

fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => 1 + values.iter().map(json_depth).max().unwrap_or(0),
        Value::Object(values) => 1 + values.values().map(json_depth).max().unwrap_or(0),
        _ => 1,
    }
}
