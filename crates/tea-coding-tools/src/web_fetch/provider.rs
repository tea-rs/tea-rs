use std::fmt;
use std::sync::Arc;

use tea_control::CancellationScope;
use tea_provider_http::ProviderHttpConfig;

use super::cache::{FetchCacheKey, FetchCachePolicyKey};
use super::http::{FETCH_ACCEPT_ENCODING_VALUE, FETCH_ACCEPT_VALUE};
use super::{
    FetchBodyDecoder, FetchCacheScope, FetchFuture, FetchHttpConfig, FetchHttpResponse,
    FetchHttpTransport, FetchProvider, FetchProviderError, FetchProviderErrorCode, FetchRequest,
    FetchResult, FetchResultCache, SystemFetchDnsResolver,
};

/// Version mixed into cache keys whenever fetch security semantics change.
pub const FETCH_SECURITY_POLICY_VERSION: u32 = 1;

/// Production client fetch backend composed from transport, decoding, and cache layers.
#[derive(Clone)]
pub struct HttpFetchProvider {
    transport: FetchHttpTransport,
    decoder: FetchBodyDecoder,
    scope: FetchCacheScope,
    cache: Arc<FetchResultCache>,
}

impl HttpFetchProvider {
    /// Creates a provider from explicit independently testable layers.
    #[must_use]
    pub const fn new(
        transport: FetchHttpTransport,
        decoder: FetchBodyDecoder,
        scope: FetchCacheScope,
        cache: Arc<FetchResultCache>,
    ) -> Self {
        Self {
            transport,
            decoder,
            scope,
            cache,
        }
    }

    /// Creates the production HTTPS/public-network provider.
    #[must_use]
    pub fn production(
        scope: FetchCacheScope,
        cache: Arc<FetchResultCache>,
        http: &ProviderHttpConfig,
    ) -> Self {
        let transport = FetchHttpTransport::new(
            FetchHttpConfig::production(),
            Arc::new(SystemFetchDnsResolver),
            http.clone(),
        );
        Self::new(transport, FetchBodyDecoder::default(), scope, cache)
    }

    async fn execute(
        &self,
        request: FetchRequest,
        cancellation: CancellationScope,
    ) -> Result<FetchResult, FetchProviderError> {
        if cancellation.is_cancelled() {
            return Err(FetchProviderError::cancelled());
        }
        let policy = self.transport.url_policy();
        let url = policy
            .validate(request.url())
            .map_err(|_| FetchProviderError::new(FetchProviderErrorCode::InvalidRequest))?;
        let key = FetchCacheKey::new(
            self.scope.clone(),
            url.as_str().to_owned(),
            request.max_chars(),
            FetchCachePolicyKey::new(
                FETCH_SECURITY_POLICY_VERSION,
                self.transport.config(),
                self.decoder.limits(),
                FETCH_ACCEPT_VALUE,
                FETCH_ACCEPT_ENCODING_VALUE,
            ),
        );
        if let Some(result) = self.cache.get(&key) {
            return if cancellation.is_cancelled() {
                Err(FetchProviderError::cancelled())
            } else {
                Ok(result)
            };
        }

        let response = self
            .transport
            .get(url.as_str(), cancellation.child())
            .await?;
        if cancellation.is_cancelled() {
            return Err(FetchProviderError::cancelled());
        }
        if !(200..=299).contains(&response.status()) {
            return Err(status_error(response.status()));
        }
        let result = self.normalize_response(&response, request.max_chars())?;
        if cancellation.is_cancelled() {
            return Err(FetchProviderError::cancelled());
        }
        self.cache.insert(key, result.clone());
        Ok(result)
    }

    fn normalize_response(
        &self,
        response: &FetchHttpResponse,
        max_chars: usize,
    ) -> Result<FetchResult, FetchProviderError> {
        let decoded = self
            .decoder
            .decode(response.headers(), response.body(), max_chars)?;
        let mut result = FetchResult::new_with_policy(
            response.requested_url(),
            response.final_url(),
            decoded.mime_type(),
            decoded.body(),
            &self.transport.url_policy(),
        )?;
        if let Some(title) = decoded.title() {
            result = result.with_title(title)?;
        }
        if let Some(truncation) = decoded.truncation() {
            result = result.with_truncation(truncation);
        }
        result.with_redirects(response.redirects().to_vec())
    }
}

impl FetchProvider for HttpFetchProvider {
    fn fetch(&self, request: FetchRequest, cancellation: CancellationScope) -> FetchFuture<'_> {
        Box::pin(self.execute(request, cancellation))
    }
}

impl fmt::Debug for HttpFetchProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpFetchProvider")
            .field("transport", &self.transport)
            .field("decoder", &self.decoder)
            .field("scope", &self.scope)
            .field("cache", &self.cache)
            .finish_non_exhaustive()
    }
}

const fn status_error(status: u16) -> FetchProviderError {
    let code = match status {
        400..=499 => FetchProviderErrorCode::InvalidRequest,
        500..=599 => FetchProviderErrorCode::Transport,
        _ => FetchProviderErrorCode::MalformedResponse,
    };
    FetchProviderError::new(code)
}
