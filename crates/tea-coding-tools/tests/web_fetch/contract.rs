use std::str::FromStr;
use std::sync::{Arc, Mutex};

use futures_util::StreamExt as _;
use serde_json::json;
use tea_coding_tools::{
    DEFAULT_FETCH_MAX_CHARS, FetchFuture, FetchProvider, FetchProviderError, FetchRedirect,
    FetchRequest, FetchResult, FetchTruncationReason, MAX_FETCH_MAX_CHARS, MAX_FETCH_URL_BYTES,
    WebFetchTool,
};
use tea_control::CancellationScope;
use tea_protocol::{ProtocolMetadata, ToolCallId, ToolPresentation};
use tea_tools::{
    ArgumentResourceResolver, ToolEffect, ToolExecutionEvent, ToolInvocation, ToolName,
    ToolRegistry, ToolRetrySafety,
};

#[derive(Debug, Default)]
struct FakeFetchProvider {
    requests: Mutex<Vec<FetchRequest>>,
}

impl FetchProvider for FakeFetchProvider {
    fn fetch(&self, request: FetchRequest, cancellation: CancellationScope) -> FetchFuture<'_> {
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(FetchProviderError::cancelled());
            }
            self.requests.lock().unwrap().push(request.clone());
            let final_url = "https://example.com/final";
            FetchResult::new(
                request.url(),
                final_url,
                "text/plain; charset=utf-8",
                "tea-rs fetch body",
            )
            .and_then(|result| result.with_title("Tea fetch result"))
            .map(|result| result.with_truncation(FetchTruncationReason::BodyCharacters))
            .and_then(|result| {
                result.with_redirects(vec![FetchRedirect::new(request.url(), final_url, 302)?])
            })
        })
    }
}

fn invocation(arguments: serde_json::Value) -> ToolInvocation {
    ToolInvocation::new(
        ToolCallId::from_str("0195a0b1-5e45-75be-8284-0aa7aa000099").unwrap(),
        ToolName::from_str("web_fetch").unwrap(),
        arguments,
        ProtocolMetadata::default(),
    )
    .unwrap()
}

fn registry(provider: Arc<dyn FetchProvider>) -> ToolRegistry {
    let resolver =
        ArgumentResourceResolver::new("url", "url", tea_tools::ToolResourceAccess::Read).unwrap();
    let mut registry = ToolRegistry::new();
    registry
        .register(
            WebFetchTool::spec().unwrap(),
            Arc::new(resolver),
            Arc::new(WebFetchTool::new(provider)),
        )
        .unwrap();
    registry
}

#[test]
fn request_contract_is_bounded_and_normalizes_root_path() {
    assert!(FetchRequest::new("", DEFAULT_FETCH_MAX_CHARS).is_err());
    assert!(FetchRequest::new("ftp://example.com/file", DEFAULT_FETCH_MAX_CHARS).is_err());
    assert!(FetchRequest::new("http://example.com/file", DEFAULT_FETCH_MAX_CHARS).is_err());
    assert!(FetchRequest::new("https://user:pass@example.com/", DEFAULT_FETCH_MAX_CHARS).is_err());
    assert!(FetchRequest::new("https://example.com/", 0).is_err());
    assert!(FetchRequest::new("https://example.com/", MAX_FETCH_MAX_CHARS + 1).is_err());
    assert!(
        FetchRequest::new("x".repeat(MAX_FETCH_URL_BYTES + 1), DEFAULT_FETCH_MAX_CHARS).is_err()
    );

    let request = FetchRequest::new("https://Example.COM", 1234).unwrap();
    assert_eq!(request.url(), "https://example.com/");
    assert_eq!(request.max_chars(), 1234);
    let request = FetchRequest::new("https://example.com/path#fragment", 1234).unwrap();
    assert_eq!(request.url(), "https://example.com/path");
}

#[test]
fn result_contract_bounds_metadata_and_redirects() {
    let redirect =
        FetchRedirect::new("https://example.com/old", "https://example.com/new", 301).unwrap();
    let result = FetchResult::new(
        "https://example.com/old",
        "https://example.com/new",
        "TEXT/PLAIN; charset=utf-8",
        "body",
    )
    .unwrap()
    .with_title("Tea")
    .unwrap()
    .with_truncation(FetchTruncationReason::BodyCharacters)
    .with_redirects(vec![redirect])
    .unwrap();
    assert_eq!(result.mime_type(), "text/plain; charset=utf-8");
    assert_eq!(result.title(), Some("Tea"));
    assert_eq!(
        result.truncation(),
        Some(FetchTruncationReason::BodyCharacters)
    );
    assert_eq!(result.redirects().len(), 1);
    assert!(FetchRedirect::new("https://example.com/", "https://example.com/", 200).is_err());
}

#[test]
fn spec_declares_network_effect_and_explicit_retry() {
    let spec = WebFetchTool::spec().unwrap();
    assert_eq!(spec.name().as_str(), "web_fetch");
    assert_eq!(spec.effects(), &[ToolEffect::NetworkRequest]);
    assert_eq!(
        spec.execution().retry_safety(),
        ToolRetrySafety::ExplicitOnly
    );
    assert!(spec.prompt_hint().is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn tool_executes_only_after_registry_resource_validation() {
    let provider = Arc::new(FakeFetchProvider::default());
    let registry = registry(provider.clone());
    let events = registry
        .execute(
            invocation(json!({"url":"https://example.com/guide"})),
            CancellationScope::new(),
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    let ToolExecutionEvent::Finished(result) = &events[0] else {
        panic!("expected successful fetch: {events:?}");
    };
    assert_eq!(
        result.output()["requested_url"],
        "https://example.com/guide"
    );
    assert_eq!(result.output()["final_url"], "https://example.com/final");
    assert_eq!(result.output()["title"], "Tea fetch result");
    assert_eq!(result.output()["mime_type"], "text/plain; charset=utf-8");
    assert_eq!(result.output()["body"], "tea-rs fetch body");
    assert_eq!(result.output()["truncated"], "body_characters");
    assert_eq!(result.output()["redirects"][0]["status"], 302);
    let Some(ToolPresentation::WebFetch(presentation)) = result.presentation() else {
        panic!("successful fetch must include a normalized durable presentation");
    };
    assert_eq!(presentation.final_url(), "https://example.com/final");
    assert_eq!(presentation.title(), Some("Tea fetch result"));
    assert_eq!(presentation.mime_type(), "text/plain; charset=utf-8");
    assert_eq!(presentation.body(), "tea-rs fetch body");
    assert_eq!(
        presentation.truncation(),
        Some(tea_protocol::WebFetchTruncation::BodyCharacters)
    );
    assert_eq!(presentation.redirects().len(), 1);
    let [tea_protocol::ContentBlock::Text { text }] = result.content() else {
        panic!("web fetch model content must remain one normalized text block");
    };
    assert!(text.contains("Title: Tea fetch result"));
    assert!(text.contains("Truncated: body_characters"));
    assert!(text.contains("Redirects: 1"));
    assert!(!format!("{presentation:?}").contains("tea-rs fetch body"));
    assert_eq!(provider.requests.lock().unwrap().len(), 1);

    let invalid = registry
        .execute(
            invocation(json!({"url":"https://user:pass@example.com/guide"})),
            CancellationScope::new(),
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(
        invalid.as_slice(),
        [ToolExecutionEvent::Failed(_)]
    ));
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_invocation_never_calls_provider() {
    let provider = Arc::new(FakeFetchProvider::default());
    let registry = registry(provider.clone());
    let cancellation = CancellationScope::new();
    cancellation.cancel();
    let events = registry
        .execute(
            invocation(json!({"url":"https://example.com/guide"})),
            cancellation,
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(events.as_slice(), [ToolExecutionEvent::Failed(_)]));
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn caller_controlled_headers_cookies_and_authorization_are_rejected() {
    let provider = Arc::new(FakeFetchProvider::default());
    let registry = registry(provider.clone());
    let Err(error) = registry.execute(
        invocation(json!({
            "url": "https://example.com/guide",
            "headers": {"Authorization": "Bearer secret"},
            "cookies": "session=secret"
        })),
        CancellationScope::new(),
    ) else {
        panic!("caller-controlled request metadata must be rejected");
    };
    assert!(provider.requests.lock().unwrap().is_empty());
    let diagnostic = format!("{error:?}");
    assert!(diagnostic.contains("additionalProperties"));
    assert!(!diagnostic.contains("secret"));
}
