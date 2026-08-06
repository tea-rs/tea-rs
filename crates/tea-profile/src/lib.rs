#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Versioned product profile schema, validation, composition, and example
//! profiles for `tea-rs`.
//!
//! This pure crate contains no model provider, tool executor, session store,
//! clock, filesystem, process, network, database, or async runtime. It depends
//! only on protocol selectors, tool metadata, policy environment types, Serde,
//! and `SemVer`.
//!
//! # Example
//!
//! ```
//! use std::str::FromStr;
//!
//! use tea_profile::AgentProfile;
//! use tea_protocol::{ModelRef, ProviderId};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let profile = AgentProfile::builder(
//!     "minimal-assistant".parse()?,
//!     "Minimal Assistant".parse()?,
//!     ModelRef::new(ProviderId::from_str("fake")?, "fake/model".parse()?),
//! )
//! .environment(tea_policy::PolicyEnvironment::new(
//!     tea_policy::ExecutionSurface::Cli,
//!     tea_policy::PolicyExecutionTarget::Native,
//!     tea_protocol::ProtocolMetadata::default(),
//! ))
//! .build()?;
//! assert_eq!(profile.profile_id().as_str(), "minimal-assistant");
//! assert!(profile.active_tool_names().is_empty());
//! # Ok(())
//! # }
//! ```

mod budget;
mod composition;
mod error;
mod identity;
mod limits;
mod profile;
mod version;
mod workspace;

pub use budget::{MAX_PROFILE_PROMPT_BYTES, MAX_PROFILE_PROMPT_TOKENS, ProfilePromptBudget};
pub use composition::{ProfileOverlay, example_workspace_instruction};
pub use error::{ProfileError, ProfileErrorCode};
pub use identity::{
    MAX_PROFILE_DESCRIPTION_BYTES, MAX_PROFILE_DISPLAY_NAME_BYTES, MAX_PROFILE_SEGMENT_ID_BYTES,
    ProfileDescription, ProfileDisplayName, ProfileRuleId, ProfileSegmentId, ProfileTextError,
    ProfileTrustLevel,
};
pub use limits::{
    MAX_PROFILE_ASSISTANT_OUTPUT_BYTES, MAX_PROFILE_ELAPSED, MAX_PROFILE_EVENTS,
    MAX_PROFILE_QUEUED_MESSAGES, MAX_PROFILE_TOOL_ITERATIONS, ProfileRunLimits,
};
pub use profile::{
    AgentProfile, AgentProfileBuilder, MAX_PROFILE_ACTIVE_TOOLS, MAX_PROFILE_APPROVAL_TTL,
    MAX_PROFILE_POLICY_RULES, parse_rule_id, parse_segment_id,
};
pub use version::{
    CURRENT_PROFILE_SCHEMA_VERSION, PROFILE_SCHEMA_V1_0_0, ProfileSchemaVersion,
    ProfileSchemaVersionParseError,
};
pub use workspace::{
    MAX_PROFILE_WORKSPACE_INSTRUCTIONS, MAX_WORKSPACE_CONTENT_BYTES, MAX_WORKSPACE_LOCATOR_BYTES,
    ProfileWorkspaceInstruction,
};
