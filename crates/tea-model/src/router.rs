use std::collections::BTreeMap;
use std::sync::Arc;

use tea_protocol::{ModelRef, ProviderId};
use thiserror::Error;

use crate::{ModelProvider, ModelSpec};

/// Object-safe lookup port that routes provider-qualified model identities.
pub trait ModelRouter: std::fmt::Debug + Send + Sync {
    /// Returns the provider registered under the canonical identity.
    fn provider(&self, provider_id: &ProviderId) -> Option<&dyn ModelProvider>;

    /// Returns all advertised models in deterministic provider/model order.
    fn models(&self) -> &[ModelSpec];

    /// Resolves one complete model identity.
    fn model(&self, model_ref: &ModelRef) -> Option<&ModelSpec> {
        self.provider(model_ref.provider_id())?
            .model(model_ref.model_id())
            .filter(|model| model.model_ref() == model_ref)
    }
}

impl<T: ModelProvider> ModelRouter for T {
    fn provider(&self, provider_id: &ProviderId) -> Option<&dyn ModelProvider> {
        (self.provider_id() == provider_id).then_some(self)
    }

    fn models(&self) -> &[ModelSpec] {
        ModelProvider::models(self)
    }
}

/// Immutable reference registry for a fixed runtime provider generation.
#[derive(Debug)]
pub struct ModelRegistry {
    providers: BTreeMap<ProviderId, Arc<dyn ModelProvider>>,
    models: Vec<ModelSpec>,
}

impl ModelRegistry {
    /// Builds a validated registry from one immutable provider generation.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate provider identities or a provider that
    /// advertises a model owned by another provider.
    pub fn new(
        providers: impl IntoIterator<Item = Arc<dyn ModelProvider>>,
    ) -> Result<Self, ModelRegistryError> {
        let mut by_id = BTreeMap::new();
        for provider in providers {
            let provider_id = provider.provider_id().clone();
            if provider
                .models()
                .iter()
                .any(|model| model.provider_id() != &provider_id)
            {
                return Err(ModelRegistryError::ProviderCatalogMismatch(provider_id));
            }
            if by_id.insert(provider_id.clone(), provider).is_some() {
                return Err(ModelRegistryError::DuplicateProvider(provider_id));
            }
        }
        if by_id.is_empty() {
            return Err(ModelRegistryError::Empty);
        }
        let mut models = by_id
            .values()
            .flat_map(|provider| provider.models().iter().cloned())
            .collect::<Vec<_>>();
        models.sort_by(|left, right| left.model_ref().cmp(right.model_ref()));
        Ok(Self {
            providers: by_id,
            models,
        })
    }

    /// Returns the registered provider count.
    #[must_use]
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    /// Returns registered provider identities in canonical order.
    #[must_use]
    pub fn provider_ids(&self) -> Vec<ProviderId> {
        self.providers.keys().cloned().collect()
    }
}

impl ModelRouter for ModelRegistry {
    fn provider(&self, provider_id: &ProviderId) -> Option<&dyn ModelProvider> {
        self.providers.get(provider_id).map(AsRef::as_ref)
    }

    fn models(&self) -> &[ModelSpec] {
        &self.models
    }
}

/// Invalid immutable provider-registry composition.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ModelRegistryError {
    /// At least one provider is required.
    #[error("model registry requires at least one provider")]
    Empty,
    /// Two adapters claimed the same provider identity.
    #[error("model provider {0} is registered more than once")]
    DuplicateProvider(ProviderId),
    /// An adapter advertised a model owned by a different provider.
    #[error("model provider {0} advertises a model owned by another provider")]
    ProviderCatalogMismatch(ProviderId),
}
