#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Pure policy, approval, grant, and redaction contracts for `tea-rs`.
//!
//! # Example
//!
//! ```
//! use std::str::FromStr;
//!
//! use tea_policy::{ActorId, PolicyRedactor, ResourcePattern};
//! use tea_tools::ToolResourceAccess;
//! use serde_json::json;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let actor = ActorId::from_str("user:alice")?;
//! let pattern = ResourcePattern::new(
//!     "file",
//!     "/workspace/",
//!     Some(ToolResourceAccess::Write),
//! )?;
//! let presentation = PolicyRedactor.redact_arguments(&json!({
//!     "path": "/workspace/notes.txt",
//!     "apiKey": "secret"
//! }))?;
//!
//! assert_eq!(actor.as_str(), "user:alice");
//! assert_eq!(pattern.locator_prefix(), "/workspace/");
//! assert_eq!(presentation.value()["apiKey"], "[REDACTED]");
//! # Ok(())
//! # }
//! ```

mod approval;
mod decision;
mod engine;
mod grant;
mod identity;
mod input;
mod policies;
mod redaction;

pub use approval::{ApprovalError, ApprovalPresentation, ApprovalRequest, ApprovalResolution};
pub use decision::{ApprovalRequirement, PolicyDecision, PolicyDecisionError, PolicyRuleDecision};
pub use engine::{
    PolicyEngine, PolicyEvaluation, PolicyLayer, PolicyRule, PolicyRuleError, PolicyTraceEntry,
};
pub use grant::{GrantScope, PolicyGrant, PolicyGrantError, ResourcePattern};
pub use identity::{
    ActorId, ExecutionSurface, GrantId, PolicyEnvironment, PolicyExecutionTarget,
    PolicyIdentityParseError, WorkspaceId,
};
pub use input::{MAX_POLICY_GRANTS, PolicyInput, PolicyInputError};
pub use policies::{
    CodingWorkspacePolicy, DesktopPolicy, ExternalSourcePolicy, FilesystemReadPolicy,
    UnknownEffectPolicy,
};
pub use redaction::{PolicyRedactor, RedactedArguments, RedactionError};
