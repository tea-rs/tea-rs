use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Provider-neutral reasoning effort ordered from disabled to maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    /// Explicitly disable model reasoning.
    Off,
    /// Smallest non-zero reasoning effort.
    Minimal,
    /// Low reasoning effort.
    Low,
    /// Medium reasoning effort.
    Medium,
    /// High reasoning effort.
    High,
    /// Extended high reasoning effort, encoded as `xhigh`.
    #[serde(rename = "xhigh")]
    ExtraHigh,
    /// Provider/model maximum reasoning effort, encoded as `max`.
    #[serde(rename = "max")]
    Maximum,
}

impl ReasoningEffort {
    /// Every canonical level in ascending effort order.
    pub const ALL: [Self; 7] = [
        Self::Off,
        Self::Minimal,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::ExtraHigh,
        Self::Maximum,
    ];

    /// Levels eligible for quick shortcut cycling.
    pub const SHORTCUT_LEVELS: [Self; 5] = [
        Self::Off,
        Self::Minimal,
        Self::Low,
        Self::Medium,
        Self::High,
    ];

    /// Returns the stable configuration and protocol spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::ExtraHigh => "xhigh",
            Self::Maximum => "max",
        }
    }
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ReasoningEffort {
    type Err = ReasoningEffortParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "off" => Ok(Self::Off),
            "minimal" => Ok(Self::Minimal),
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "xhigh" => Ok(Self::ExtraHigh),
            "max" => Ok(Self::Maximum),
            _ => Err(ReasoningEffortParseError),
        }
    }
}

/// Error returned for an unknown reasoning effort spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("reasoning effort is invalid")]
pub struct ReasoningEffortParseError;
