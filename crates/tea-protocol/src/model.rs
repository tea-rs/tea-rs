use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{ModelId, ProviderId};

/// Complete canonical identity of one model advertised by one provider.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelRef {
    provider_id: ProviderId,
    model_id: ModelId,
}

impl ModelRef {
    /// Creates a provider-qualified model identity.
    #[must_use]
    pub const fn new(provider_id: ProviderId, model_id: ModelId) -> Self {
        Self {
            provider_id,
            model_id,
        }
    }

    /// Returns the provider selector.
    #[must_use]
    pub const fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    /// Returns the provider-local model selector.
    #[must_use]
    pub const fn model_id(&self) -> &ModelId {
        &self.model_id
    }
}

impl fmt::Display for ModelRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.provider_id, self.model_id)
    }
}
