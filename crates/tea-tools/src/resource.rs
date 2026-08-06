use std::fmt::Debug;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::ToolName;

/// Maximum resolved resources for one invocation.
pub const MAX_TOOL_RESOURCES: usize = 128;

/// Access requested for a resolved resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResourceAccess {
    /// Read-only access.
    Read,
    /// Create or modify access.
    Write,
    /// Delete access.
    Delete,
    /// Execute or spawn access.
    Execute,
    /// Access semantics unknown to this runtime.
    Unknown,
}

/// Canonical resource affected by a tool invocation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResource {
    scheme: String,
    locator: String,
    access: ToolResourceAccess,
}

impl ToolResource {
    /// Creates a bounded canonical resource.
    ///
    /// # Errors
    ///
    /// Rejects invalid schemes or empty/control/oversized locators.
    pub fn new(
        scheme: impl Into<String>,
        locator: impl Into<String>,
        access: ToolResourceAccess,
    ) -> Result<Self, ToolResourceError> {
        let scheme = scheme.into();
        let locator = locator.into();
        let mut bytes = scheme.bytes();
        if scheme.len() > 64
            || !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            || !bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(ToolResourceError::InvalidScheme);
        }
        if locator.is_empty() || locator.len() > 2048 || locator.chars().any(char::is_control) {
            return Err(ToolResourceError::InvalidLocator);
        }
        Ok(Self {
            scheme,
            locator,
            access,
        })
    }

    /// Returns the resource scheme.
    #[must_use]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// Returns the opaque bounded locator.
    #[must_use]
    pub fn locator(&self) -> &str {
        &self.locator
    }

    /// Returns requested access.
    #[must_use]
    pub const fn access(&self) -> ToolResourceAccess {
        self.access
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawToolResource {
    scheme: String,
    locator: String,
    access: ToolResourceAccess,
}

impl<'de> Deserialize<'de> for ToolResource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawToolResource::deserialize(deserializer)?;
        Self::new(raw.scheme, raw.locator, raw.access).map_err(serde::de::Error::custom)
    }
}

/// Pure resource resolver used before policy and execution.
pub trait ToolResourceResolver: Debug + Send + Sync {
    /// Resolves affected resources from schema-validated arguments.
    ///
    /// # Errors
    ///
    /// Returns a bounded resolution error without performing a side effect.
    fn resolve(
        &self,
        tool_name: &ToolName,
        arguments: &Value,
    ) -> Result<Vec<ToolResource>, ToolResourceError>;
}

/// Resolver reading one string argument as a resource locator.
#[derive(Debug, Clone)]
pub struct ArgumentResourceResolver {
    argument: String,
    scheme: String,
    access: ToolResourceAccess,
}

impl ArgumentResourceResolver {
    /// Creates a resolver for one canonical top-level string argument.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid argument names or resource schemes.
    pub fn new(
        argument: impl Into<String>,
        scheme: impl Into<String>,
        access: ToolResourceAccess,
    ) -> Result<Self, ToolResourceError> {
        let argument = argument.into();
        let scheme = scheme.into();
        if argument.is_empty()
            || argument.len() > 128
            || !argument
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(ToolResourceError::Unresolved);
        }
        ToolResource::new(&scheme, "validation", access)?;
        Ok(Self {
            argument,
            scheme,
            access,
        })
    }
}

impl ToolResourceResolver for ArgumentResourceResolver {
    fn resolve(
        &self,
        _tool_name: &ToolName,
        arguments: &Value,
    ) -> Result<Vec<ToolResource>, ToolResourceError> {
        let locator = arguments
            .get(&self.argument)
            .and_then(Value::as_str)
            .ok_or(ToolResourceError::Unresolved)?;
        Ok(vec![ToolResource::new(&self.scheme, locator, self.access)?])
    }
}

/// Resolver returning one deterministic static resource set.
#[derive(Debug, Clone)]
pub struct StaticResourceResolver {
    resources: Vec<ToolResource>,
}

impl StaticResourceResolver {
    /// Creates a sorted, deduplicated bounded static resolver.
    ///
    /// # Errors
    ///
    /// Returns an error when the deduplicated resource count exceeds bounds.
    pub fn new(
        resources: impl IntoIterator<Item = ToolResource>,
    ) -> Result<Self, ToolResourceError> {
        let mut resources = resources.into_iter().collect::<Vec<_>>();
        resources.sort();
        resources.dedup();
        if resources.len() > MAX_TOOL_RESOURCES {
            return Err(ToolResourceError::TooManyResources);
        }
        Ok(Self { resources })
    }

    /// Returns sorted static resources.
    #[must_use]
    pub fn resources(&self) -> &[ToolResource] {
        &self.resources
    }
}

impl ToolResourceResolver for StaticResourceResolver {
    fn resolve(
        &self,
        _tool_name: &ToolName,
        _arguments: &Value,
    ) -> Result<Vec<ToolResource>, ToolResourceError> {
        Ok(self.resources.clone())
    }
}

/// Resource construction or resolution failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ToolResourceError {
    /// Scheme is not canonical lowercase ASCII.
    #[error("tool resource scheme is invalid")]
    InvalidScheme,
    /// Locator is empty, oversized, or contains controls.
    #[error("tool resource locator is invalid")]
    InvalidLocator,
    /// Resolver returned too many resources.
    #[error("tool invocation resolves too many resources")]
    TooManyResources,
    /// Arguments do not identify a required resource.
    #[error("tool resource cannot be resolved from arguments")]
    Unresolved,
}
