use std::str::FromStr;
use std::sync::Arc;

use futures_util::stream;
use tea_model::{
    BoxModelStream, ModelCancellation, ModelCapabilities, ModelDisplayName, ModelProvider,
    ModelRef, ModelRegistry, ModelRegistryError, ModelRequest, ModelRouter, ModelSpec, ProviderId,
};
use tea_protocol::{ModelId, TokenCount};

#[derive(Debug)]
struct FixtureProvider {
    provider_id: ProviderId,
    models: Vec<ModelSpec>,
}

impl ModelProvider for FixtureProvider {
    fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    fn models(&self) -> &[ModelSpec] {
        &self.models
    }

    fn stream(&self, _request: ModelRequest, _cancellation: ModelCancellation) -> BoxModelStream {
        Box::pin(stream::empty())
    }
}

fn model(provider_id: &str, model_id: &str) -> ModelSpec {
    ModelSpec::new(
        ModelId::from_str(model_id).unwrap(),
        ProviderId::from_str(provider_id).unwrap(),
        ModelDisplayName::from_str(&format!("{provider_id} {model_id}")).unwrap(),
        TokenCount::new(8_000).unwrap(),
        TokenCount::new(2_000).unwrap(),
        ModelCapabilities::text(),
    )
    .unwrap()
}

fn provider(provider_id: &str, models: Vec<ModelSpec>) -> Arc<dyn ModelProvider> {
    Arc::new(FixtureProvider {
        provider_id: ProviderId::from_str(provider_id).unwrap(),
        models,
    })
}

fn model_ref(provider_id: &str, model_id: &str) -> ModelRef {
    ModelRef::new(
        ProviderId::from_str(provider_id).unwrap(),
        ModelId::from_str(model_id).unwrap(),
    )
}

#[test]
fn registry_rejects_duplicate_provider_identities() {
    let error = ModelRegistry::new([
        provider("one", vec![model("one", "shared")]),
        provider("one", vec![model("one", "other")]),
    ])
    .unwrap_err();

    assert_eq!(
        error,
        ModelRegistryError::DuplicateProvider(ProviderId::from_str("one").unwrap())
    );
}

#[test]
fn registry_rejects_models_owned_by_another_provider() {
    let error = ModelRegistry::new([provider("one", vec![model("two", "shared")])]).unwrap_err();

    assert_eq!(
        error,
        ModelRegistryError::ProviderCatalogMismatch(ProviderId::from_str("one").unwrap())
    );
}

#[test]
fn same_model_id_is_resolved_by_provider_qualified_identity() {
    let registry = ModelRegistry::new([
        provider("one", vec![model("one", "shared")]),
        provider("two", vec![model("two", "shared")]),
    ])
    .unwrap();

    assert_eq!(registry.provider_count(), 2);
    assert_eq!(
        registry
            .model(&model_ref("one", "shared"))
            .unwrap()
            .provider_id()
            .as_str(),
        "one"
    );
    assert_eq!(
        registry
            .model(&model_ref("two", "shared"))
            .unwrap()
            .provider_id()
            .as_str(),
        "two"
    );
    assert!(registry.model(&model_ref("missing", "shared")).is_none());
}
