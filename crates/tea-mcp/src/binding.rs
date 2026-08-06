use std::{fmt, sync::Arc};

use serde_json::Value;
use tea_tools::{
    ArgumentResourceResolver, MAX_TOOL_RESOURCES, ToolName, ToolResource, ToolResourceAccess,
    ToolResourceError, ToolResourceResolver, ToolSpec,
};

use crate::{McpArgumentResource, McpError, McpErrorCode, McpRemoteToolName, McpServerId};

/// One frozen mapping from an exact remote MCP name to an ordinary tool spec.
#[derive(Clone)]
pub struct McpToolBinding {
    server_id: McpServerId,
    remote_name: McpRemoteToolName,
    spec: ToolSpec,
    resolver: Arc<dyn ToolResourceResolver>,
    remote_annotations: Option<Value>,
}

impl ToolResourceResolver for McpToolBinding {
    fn resolve(
        &self,
        tool_name: &ToolName,
        arguments: &Value,
    ) -> Result<Vec<ToolResource>, ToolResourceError> {
        self.resolver.resolve(tool_name, arguments)
    }
}

impl McpToolBinding {
    pub(crate) fn new(
        server_id: &McpServerId,
        remote_name: McpRemoteToolName,
        spec: ToolSpec,
        resources: &[McpArgumentResource],
        remote_annotations: Option<Value>,
    ) -> Result<Self, McpError> {
        let resolver = McpResourceResolver::new(server_id, &remote_name, resources)?;
        Ok(Self {
            server_id: server_id.clone(),
            remote_name,
            spec,
            resolver: Arc::new(resolver),
            remote_annotations,
        })
    }

    /// Returns the configured server that owns this frozen binding.
    #[must_use]
    pub const fn server_id(&self) -> &McpServerId {
        &self.server_id
    }

    /// Returns the exact remote name used for future `tools/call` requests.
    #[must_use]
    pub const fn remote_name(&self) -> &McpRemoteToolName {
        &self.remote_name
    }

    /// Returns the immutable ordinary tool specification.
    #[must_use]
    pub const fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    /// Returns bounded server annotations retained only for diagnostics.
    #[must_use]
    pub const fn remote_annotations(&self) -> Option<&Value> {
        self.remote_annotations.as_ref()
    }

    /// Resolves the mandatory server execute resource and host argument resources.
    ///
    /// # Errors
    ///
    /// Returns the existing bounded pure resource-resolution failure.
    pub fn resolve_resources(
        &self,
        arguments: &Value,
    ) -> Result<Vec<ToolResource>, ToolResourceError> {
        self.resolver.resolve(self.spec.name(), arguments)
    }

    pub(crate) fn into_parts(self) -> (ToolName, Self) {
        (self.spec.name().clone(), self)
    }
}

impl fmt::Debug for McpToolBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpToolBinding")
            .field("server_id", &self.server_id)
            .field("remote_name", &self.remote_name)
            .field("spec", &self.spec)
            .field(
                "remote_annotations",
                &self.remote_annotations.as_ref().map(|_| "<untrusted>"),
            )
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct McpResourceResolver {
    execute: ToolResource,
    arguments: Vec<ArgumentResourceResolver>,
}

impl McpResourceResolver {
    fn new(
        server_id: &McpServerId,
        remote_name: &McpRemoteToolName,
        resources: &[McpArgumentResource],
    ) -> Result<Self, McpError> {
        let execute = ToolResource::new(
            "mcp-server",
            format!("{}/{}", server_id.as_str(), remote_name.as_str()),
            ToolResourceAccess::Execute,
        )
        .map_err(|_| McpError::new(McpErrorCode::PolicyDeclaration))?;
        let arguments = resources
            .iter()
            .map(|resource| {
                ArgumentResourceResolver::new(
                    resource.argument(),
                    resource.scheme(),
                    resource.access(),
                )
                .map_err(|_| McpError::new(McpErrorCode::PolicyDeclaration))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { execute, arguments })
    }
}

impl ToolResourceResolver for McpResourceResolver {
    fn resolve(
        &self,
        tool_name: &ToolName,
        arguments: &Value,
    ) -> Result<Vec<ToolResource>, ToolResourceError> {
        let mut resources = Vec::with_capacity(self.arguments.len().saturating_add(1));
        resources.push(self.execute.clone());
        for resolver in &self.arguments {
            resources.extend(resolver.resolve(tool_name, arguments)?);
        }
        resources.sort();
        resources.dedup();
        if resources.len() > MAX_TOOL_RESOURCES {
            return Err(ToolResourceError::TooManyResources);
        }
        Ok(resources)
    }
}
