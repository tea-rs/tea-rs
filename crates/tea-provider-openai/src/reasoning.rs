//! OpenAI-compatible wire mapping for provider-neutral reasoning efforts.

use std::collections::BTreeMap;

use tea_model::{ModelRequest, ReasoningEffort, ReasoningProfile};

use crate::{OpenAiError, OpenAiErrorCode};

const MAX_WIRE_EFFORT_BYTES: usize = 64;

/// Validated mapping from canonical efforts to one model's wire values.
///
/// `off` is deliberately absent: explicit disable is represented by omitting
/// the provider reasoning field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiReasoningEffortMap {
    entries: BTreeMap<ReasoningEffort, String>,
}

impl OpenAiReasoningEffortMap {
    /// Creates a strict model-level wire mapping.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate levels, an `off` mapping, too many
    /// entries, or an invalid wire value.
    pub fn new(
        entries: impl IntoIterator<Item = (ReasoningEffort, String)>,
    ) -> Result<Self, OpenAiError> {
        let mut mapped = BTreeMap::new();
        for (effort, wire) in entries {
            if effort == ReasoningEffort::Off
                || wire.is_empty()
                || wire.len() > MAX_WIRE_EFFORT_BYTES
                || !wire
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
                || mapped.insert(effort, wire).is_some()
            {
                return Err(OpenAiError::new(
                    OpenAiErrorCode::InvalidRequest,
                    "reasoning effort wire map is invalid",
                ));
            }
        }
        if mapped.len() > ReasoningEffort::ALL.len() - 1 {
            return Err(OpenAiError::new(
                OpenAiErrorCode::InvalidRequest,
                "reasoning effort wire map is too large",
            ));
        }
        Ok(Self { entries: mapped })
    }

    /// Builds an identity mapping for every non-off level in a model profile.
    pub(crate) fn for_profile(profile: &ReasoningProfile) -> Result<Self, OpenAiError> {
        Self::new(
            profile
                .supported_efforts()
                .iter()
                .copied()
                .filter(|effort| *effort != ReasoningEffort::Off)
                .map(|effort| (effort, effort.as_str().to_owned())),
        )
    }

    /// Returns the provider wire value for one canonical effort.
    ///
    /// # Errors
    ///
    /// Returns an error when a non-off effort has no validated mapping.
    pub fn wire_effort(&self, effort: ReasoningEffort) -> Result<Option<&str>, OpenAiError> {
        if effort == ReasoningEffort::Off {
            return Ok(None);
        }
        self.entries
            .get(&effort)
            .map(String::as_str)
            .map(Some)
            .ok_or_else(|| {
                OpenAiError::new(
                    OpenAiErrorCode::InvalidRequest,
                    "reasoning effort is not mapped for the selected model",
                )
            })
    }

    pub(crate) fn efforts(&self) -> impl Iterator<Item = ReasoningEffort> + '_ {
        self.entries.keys().copied()
    }
}

impl Default for OpenAiReasoningEffortMap {
    fn default() -> Self {
        Self::new(
            ReasoningEffort::ALL
                .into_iter()
                .filter(|effort| *effort != ReasoningEffort::Off)
                .map(|effort| (effort, effort.as_str().to_owned())),
        )
        .expect("canonical OpenAI reasoning wire values are valid")
    }
}

pub(crate) fn request_wire_effort<'a>(
    request: &ModelRequest,
    map: Option<&'a OpenAiReasoningEffortMap>,
) -> Result<Option<&'a str>, OpenAiError> {
    let Some(options) = request.reasoning() else {
        return Ok(None);
    };
    if options.effort() == ReasoningEffort::Off {
        return Ok(None);
    }
    map.ok_or_else(|| {
        OpenAiError::new(
            OpenAiErrorCode::InvalidRequest,
            "selected model has no reasoning effort wire map",
        )
    })?
    .wire_effort(options.effort())
}
