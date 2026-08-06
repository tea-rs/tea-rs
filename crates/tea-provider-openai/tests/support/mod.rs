//! Test support: a fixture-backed `ModelProvider` that plays recorded `SSE` bytes
//! through the real `SseParser` + `ChunkReducer` pipeline (no HTTP).

use std::path::PathBuf;
use std::sync::Arc;

use tea_control::CancellationScope;
use tea_model::{ModelEvent, ModelFailure, ModelProvider, ModelRequest, ModelSpec, ProviderId};
use tea_protocol::RetryClass;

use tea_provider_openai::sse::{SseEvent, SseParser};
use tea_provider_openai::stream::ChunkReducer;

#[allow(dead_code)]
#[derive(Debug)]
pub struct FixtureProvider {
    provider_id: ProviderId,
    catalog: Vec<ModelSpec>,
    fixture: Vec<u8>,
}

impl FixtureProvider {
    #[allow(dead_code)]
    pub fn from_fixture_name(provider_id: &str, model: ModelSpec, name: &str) -> Self {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let fixture = std::fs::read(manifest.join("tests/fixtures").join(name)).unwrap();
        Self {
            provider_id: ProviderId::from_str(provider_id).unwrap(),
            catalog: vec![model],
            fixture,
        }
    }
}

use std::str::FromStr;

impl ModelProvider for FixtureProvider {
    fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }
    fn models(&self) -> &[ModelSpec] {
        &self.catalog
    }
    fn stream(
        &self,
        _request: ModelRequest,
        cancellation: CancellationScope,
    ) -> tea_model::BoxModelStream {
        let fixture = self.fixture.clone();
        Box::pin(async_stream::stream! {
            let mut parser = SseParser::new();
            let mut reducer = ChunkReducer::new();
            for chunk in fixture.chunks(16) {
                if cancellation.is_cancelled() {
                    yield ModelEvent::Failed(
                        ModelFailure::new(
                            tea_model::ModelFailureCode::Cancelled,
                            "model request was cancelled",
                            RetryClass::Never,
                        )
                        .unwrap_or_else(|_| ModelFailure::internal_adapter_failure()),
                    );
                    return;
                }
                for sse in parser.feed(chunk) {
                    match sse {
                        SseEvent::Data(payload) => {
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload)
                                && let Ok(events) = reducer.map_chunk(&value) {
                                    for event in events {
                                        let terminal = matches!(
                                            event,
                                            ModelEvent::Completed(_) | ModelEvent::Failed(_)
                                        );
                                        yield event;
                                        if terminal {
                                            return;
                                        }
                                    }
                                }
                        }
                        SseEvent::Done => {
                            if let Ok(Some(terminal)) = reducer.finish() {
                                yield terminal;
                            }
                            return;
                        }
                    }
                }
            }
            for sse in parser.finish() {
                if let SseEvent::Data(payload) = sse
                    && let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload)
                        && let Ok(events) = reducer.map_chunk(&value) {
                            for event in events {
                                yield event;
                            }
                        }
            }
            if let Ok(Some(terminal)) = reducer.finish() {
                yield terminal;
            }
        })
    }
}

#[allow(dead_code)]
pub fn _unused(_arc: Arc<()>, _ext: futures_util::stream::Next<tea_model::BoxModelStream>) {}
