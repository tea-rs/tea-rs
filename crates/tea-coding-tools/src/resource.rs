use serde_json::Value;
use tea_tools::{
    ToolName, ToolResource, ToolResourceAccess, ToolResourceError, ToolResourceResolver,
};

use crate::path::ValidatedRelativePath;

/// Pure resolver mapping one validated `path` argument to a workspace file resource.
#[derive(Debug, Clone)]
pub struct WorkspaceFileResourceResolver {
    accesses: Vec<ToolResourceAccess>,
}

impl WorkspaceFileResourceResolver {
    /// Creates a resolver with one declared resource access.
    #[must_use]
    pub fn new(access: ToolResourceAccess) -> Self {
        Self {
            accesses: vec![access],
        }
    }

    /// Creates a resolver declaring both read and write access to one file.
    #[must_use]
    pub fn read_write() -> Self {
        Self {
            accesses: vec![ToolResourceAccess::Read, ToolResourceAccess::Write],
        }
    }
}

impl ToolResourceResolver for WorkspaceFileResourceResolver {
    fn resolve(
        &self,
        _tool_name: &ToolName,
        arguments: &Value,
    ) -> Result<Vec<ToolResource>, ToolResourceError> {
        let path = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let relative =
            ValidatedRelativePath::parse(path).map_err(|_| ToolResourceError::Unresolved)?;
        let locator = if relative.display() == "." {
            "/workspace".to_owned()
        } else {
            format!("/workspace/{}", relative.display())
        };
        self.accesses
            .iter()
            .copied()
            .map(|access| ToolResource::new("file", &locator, access))
            .collect()
    }
}
