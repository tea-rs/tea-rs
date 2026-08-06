//! Live Anthropic Messages `ModelProvider` backed by `reqwest`.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures_util::StreamExt;
use tea_control::CancellationScope;
use tea_model::{
    ModelEvent, ModelFailure, ModelFailureCode, ModelProvider, ModelRequest, ModelSpec, ProviderId,
};
use tea_protocol::RetryClass;
use tea_provider_http::{
    ProviderHttpConfig, UserAgent, read_bounded_error_body, retry_after_delay,
};

use crate::catalog::default_catalog;
use crate::credential::{AnthropicConfig, CredentialResolver, EnvCredentialResolver};
use crate::error::{AnthropicError, AnthropicErrorCode};
use crate::request::{build_messages_body, messages_url, request_headers};
use crate::sse::{SseEvent, SseParser};
use crate::stream::{AnthropicReducer, map_http_failure};

/// Anthropic Messages streaming provider adapter.
#[derive(Debug, Clone)]
pub struct AnthropicProvider {
    config: Arc<AnthropicConfig>,
    client: reqwest::Client,
    catalog: Vec<ModelSpec>,
}

impl AnthropicProvider {
    /// Creates a provider from an immutable connection config and model catalog.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be constructed.
    pub fn new(
        config: Arc<AnthropicConfig>,
        catalog: Vec<ModelSpec>,
    ) -> Result<Self, AnthropicError> {
        Self::new_with_http_config(config, catalog, &ProviderHttpConfig::new())
    }

    fn new_with_http_config(
        config: Arc<AnthropicConfig>,
        catalog: Vec<ModelSpec>,
        http_config: &ProviderHttpConfig,
    ) -> Result<Self, AnthropicError> {
        let client = http_config
            .build_client(Duration::from_millis(config.timeout_millis()))
            .map_err(|_| AnthropicError::new(AnthropicErrorCode::Transport, "client failed"))?;
        Ok(Self {
            config,
            client,
            catalog,
        })
    }

    /// Returns the connection configuration.
    #[must_use]
    pub fn config(&self) -> &AnthropicConfig {
        &self.config
    }
}

impl ModelProvider for AnthropicProvider {
    fn provider_id(&self) -> &ProviderId {
        self.config.provider_id()
    }

    fn models(&self) -> &[ModelSpec] {
        &self.catalog
    }

    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationScope,
    ) -> tea_model::BoxModelStream {
        let config = Arc::clone(&self.config);
        let client = self.client.clone();
        Box::pin(async_stream::stream! {
            let body = match build_messages_body(&request, &config) {
                Ok(body) => body,
                Err(error) => {
                    yield ModelEvent::Failed(adapter_failure(&error));
                    return;
                }
            };
            let mut request_builder = client.post(messages_url(&config)).json(&body);
            for (header, value) in request_headers(&config) {
                request_builder = request_builder.header(header, value);
            }
            let response = match request_builder.send().await {
                Ok(response) => response,
                Err(error) => {
                    yield ModelEvent::Failed(transport_failure(&error));
                    return;
                }
            };
            if !response.status().is_success() {
                let status = response.status().as_u16();
                let retry_after = retry_after_delay(response.headers(), SystemTime::now());
                let body = read_bounded_error_body(response).await;
                let mut failure = map_http_failure(status, &body);
                if let Some(retry_after) = retry_after {
                    failure = failure.with_retry_after(retry_after);
                }
                yield ModelEvent::Failed(failure);
                return;
            }
            let mut bytes = response.bytes_stream();
            let mut parser = SseParser::new();
            let mut reducer = AnthropicReducer::new();
            while !reducer.terminal_emitted() {
                let chunk = tokio::select! {
                    biased;
                    () = cancellation.cancelled() => {
                        yield ModelEvent::Failed(ModelFailure::new(
                            ModelFailureCode::Cancelled,
                            "model request was cancelled",
                            RetryClass::Never,
                        ).unwrap_or_else(|_| ModelFailure::internal_adapter_failure()));
                        return;
                    }
                    chunk = bytes.next() => match chunk {
                        Some(Ok(bytes)) => bytes,
                        None => break,
                        Some(Err(error)) => {
                            yield ModelEvent::Failed(transport_failure(&error));
                            return;
                        }
                    },
                };
                for event in parser.feed(&chunk) {
                    match map_sse_event(&mut reducer, event) {
                        Ok(events) => {
                            for event in events {
                                let terminal = matches!(event, ModelEvent::Completed(_) | ModelEvent::Failed(_));
                                yield event;
                                if terminal {
                                    return;
                                }
                            }
                        }
                        Err(error) => {
                            yield ModelEvent::Failed(adapter_failure(&error));
                            return;
                        }
                    }
                }
            }
            for event in parser.finish() {
                    match map_sse_event(&mut reducer, event) {
                    Ok(events) => {
                        for event in events {
                            let terminal = matches!(event, ModelEvent::Completed(_) | ModelEvent::Failed(_));
                            yield event;
                            if terminal {
                                return;
                            }
                        }
                    }
                    Err(error) => {
                        yield ModelEvent::Failed(adapter_failure(&error));
                        return;
                    }
                }
            }
            match reducer.finish() {
                Ok(events) => {
                    for event in events {
                        yield event;
                    }
                }
                Err(error) => yield ModelEvent::Failed(adapter_failure(&error)),
            }
        })
    }
}

/// Builder for [`AnthropicProvider`] backed by the environment contract.
#[derive(Debug, Default)]
pub struct AnthropicProviderBuilder {
    config: Option<Arc<AnthropicConfig>>,
    catalog: Option<Vec<ModelSpec>>,
    resolver: Option<Arc<dyn CredentialResolver>>,
    http_config: ProviderHttpConfig,
}

impl AnthropicProviderBuilder {
    /// Creates an empty builder that resolves config from the environment.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a builder pre-populated from the `TEA_ANTHROPIC_*` environment contract.
    ///
    /// # Errors
    ///
    /// Returns an error when required environment values are missing or invalid.
    pub fn from_env() -> Result<Self, AnthropicError> {
        let config = Arc::new(EnvCredentialResolver::new().resolve()?);
        Ok(Self {
            config: Some(config),
            catalog: None,
            resolver: None,
            http_config: ProviderHttpConfig::new(),
        })
    }

    /// Overrides the connection configuration.
    #[must_use]
    pub fn with_config(mut self, config: Arc<AnthropicConfig>) -> Self {
        self.config = Some(config);
        self
    }

    /// Overrides the credential resolver.
    #[must_use]
    pub fn with_resolver(mut self, resolver: Arc<dyn CredentialResolver>) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// Overrides the advertised model catalog.
    #[must_use]
    pub fn with_catalog(mut self, catalog: Vec<ModelSpec>) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// Sets the shared HTTP client policy used by model requests.
    #[must_use]
    pub fn with_http_config(mut self, http_config: ProviderHttpConfig) -> Self {
        self.http_config = http_config;
        self
    }

    /// Sets the application identity sent with model requests.
    #[must_use]
    pub fn with_user_agent(mut self, user_agent: UserAgent) -> Self {
        self.http_config = self.http_config.with_user_agent(user_agent);
        self
    }

    /// Builds the provider.
    ///
    /// # Errors
    ///
    /// Returns an error when config resolution, catalog construction, or client
    /// construction fails.
    pub fn build(self) -> Result<AnthropicProvider, AnthropicError> {
        let config = if let Some(config) = self.config {
            config
        } else {
            let resolver = self
                .resolver
                .unwrap_or_else(|| Arc::new(EnvCredentialResolver::new()));
            Arc::new(resolver.resolve()?)
        };
        let catalog = self.catalog.map_or_else(|| default_catalog(&config), Ok)?;
        AnthropicProvider::new_with_http_config(config, catalog, &self.http_config)
    }
}

fn map_sse_event(
    reducer: &mut AnthropicReducer,
    event: SseEvent,
) -> Result<Vec<ModelEvent>, AnthropicError> {
    match event {
        SseEvent::Data(payload) => serde_json::from_str(&payload)
            .map_err(|_| {
                AnthropicError::new(AnthropicErrorCode::MalformedResponse, "invalid sse json")
            })
            .and_then(|value| reducer.map_chunk(&value)),
        SseEvent::Done => reducer.finish(),
    }
}

fn adapter_failure(error: &AnthropicError) -> ModelFailure {
    ModelFailure::new(
        error.code().into_model_failure_code(),
        error.message(),
        RetryClass::Never,
    )
    .unwrap_or_else(|_| ModelFailure::internal_adapter_failure())
}

fn transport_failure(error: &reqwest::Error) -> ModelFailure {
    let code = if error.is_timeout() || error.is_connect() || error.is_request() {
        ModelFailureCode::Transport
    } else {
        ModelFailureCode::Internal
    };
    ModelFailure::new(code, "anthropic transport error", RetryClass::Immediate)
        .unwrap_or_else(|_| ModelFailure::internal_adapter_failure())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::str::FromStr;

    use tea_protocol::{CanonicalMessage, ContentBlock, MessageId, ModelId, ProtocolTimestamp};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    use super::*;
    use crate::credential::MapCredentialResolver;

    fn provider(user_agent: Option<UserAgent>) -> AnthropicProvider {
        let config = MapCredentialResolver::new(BTreeMap::from([
            ("TEA_ANTHROPIC_API_KEY".to_owned(), "sk-ant-test".to_owned()),
            (
                "TEA_ANTHROPIC_MODEL".to_owned(),
                "claude-sonnet-4-20250514".to_owned(),
            ),
        ]))
        .resolve()
        .unwrap();
        let mut builder = AnthropicProviderBuilder::new()
            .with_config(Arc::new(config))
            .with_catalog(Vec::new());
        if let Some(user_agent) = user_agent {
            builder = builder.with_user_agent(user_agent);
        }
        builder.build().unwrap()
    }

    async fn captured_headers(provider: &AnthropicProvider) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream.read(&mut buffer).await.unwrap();
                assert_ne!(read, 0, "request closed before headers were sent");
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        let response = provider
            .client
            .post(format!("http://{address}/v1/messages"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
        server.await.unwrap()
    }

    #[tokio::test]
    async fn client_sends_configured_user_agent_and_omits_it_by_default() {
        let headers =
            captured_headers(&provider(Some(UserAgent::new("tea-cli/0.1.0").unwrap()))).await;
        assert!(
            headers
                .lines()
                .any(|header| header.eq_ignore_ascii_case("user-agent: tea-cli/0.1.0"))
        );

        let headers = captured_headers(&provider(None)).await;
        assert!(
            !headers
                .lines()
                .any(|header| header.to_ascii_lowercase().starts_with("user-agent:"))
        );
    }

    #[tokio::test]
    async fn http_failure_propagates_retry_after_hint() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            let body = r#"{"type":"error","error":{"type":"overloaded_error","message":"busy"}}"#;
            let response = format!(
                "HTTP/1.1 503 Service Unavailable\r\nRetry-After: 11\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let config = MapCredentialResolver::new(BTreeMap::from([
            (
                "TEA_ANTHROPIC_API_KEY".to_owned(),
                "sk-ant-test-key".to_owned(),
            ),
            (
                "TEA_ANTHROPIC_MODEL".to_owned(),
                "claude-sonnet-4-20250514".to_owned(),
            ),
            (
                "TEA_ANTHROPIC_BASE_URL".to_owned(),
                format!("http://{address}"),
            ),
        ]))
        .resolve()
        .unwrap();
        let provider = AnthropicProviderBuilder::new()
            .with_config(Arc::new(config))
            .with_catalog(Vec::new())
            .build()
            .unwrap();
        let message = CanonicalMessage::user(
            MessageId::from_str("0195a0b1-5e52-74b2-8c25-0aa7aa000045").unwrap(),
            vec![ContentBlock::text("hello").unwrap()],
            ProtocolTimestamp::from_str("2026-07-23T09:30:12.125Z").unwrap(),
        )
        .unwrap();
        let request = ModelRequest::new(
            ModelId::from_str("claude-sonnet-4-20250514").unwrap(),
            vec![message],
        )
        .unwrap();
        let events = provider
            .stream(request, CancellationScope::new())
            .collect::<Vec<_>>()
            .await;
        let failure = events
            .iter()
            .find_map(|event| match event {
                ModelEvent::Failed(failure) => Some(failure),
                _ => None,
            })
            .unwrap();
        assert_eq!(failure.retry_after(), Some(Duration::from_secs(11)));
        server.await.unwrap();
    }
}
