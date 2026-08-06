use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt as _;
use serde_json::json;
use tea_coding_tools::{
    SearchFuture, SearchProvider, SearchProviderErrorCode, SearchRequest, SearchResponse,
    SearchResult, TavilyApiKey, TavilySearchConfig, TavilySearchProvider, WebSearchTool,
};
use tea_control::CancellationScope;
use tea_model::{MAX_WEB_SEARCH_DOMAINS, WebSearchLocation, WebSearchOptions};
use tea_protocol::{ExternalSource, ProtocolMetadata, ToolCallId};
use tea_provider_http::ProviderHttpConfig;
use tea_tools::{
    StaticResourceResolver, ToolEffect, ToolExecutionEvent, ToolInvocation, ToolName, ToolRegistry,
    ToolResourceAccess, ToolRetrySafety,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

const NO_DOMAINS: [&str; 0] = [];

#[derive(Debug)]
struct FakeSearchProvider {
    requests: Mutex<Vec<SearchRequest>>,
}

impl FakeSearchProvider {
    fn new() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl SearchProvider for FakeSearchProvider {
    fn destination(&self) -> &'static str {
        "https://search.example.test/api"
    }

    fn search(&self, request: SearchRequest, cancellation: CancellationScope) -> SearchFuture<'_> {
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(tea_coding_tools::SearchProviderError::cancelled());
            }
            self.requests.lock().unwrap().push(request);
            let source = ExternalSource::new("https://docs.rs/tea")
                .unwrap()
                .with_title("Tea API")
                .unwrap()
                .with_snippet("Portable agent runtime documentation")
                .unwrap();
            SearchResponse::new(vec![SearchResult::new(source, Some(0.9)).unwrap()], false)
        })
    }
}

fn invocation(arguments: serde_json::Value) -> ToolInvocation {
    ToolInvocation::new(
        ToolCallId::from_str("0195a0b1-5e45-75be-8284-0aa7aa000099").unwrap(),
        ToolName::from_str("web_search").unwrap(),
        arguments,
        ProtocolMetadata::default(),
    )
    .unwrap()
}

fn registry(provider: Arc<dyn SearchProvider>, options: WebSearchOptions) -> ToolRegistry {
    let tool = WebSearchTool::new(provider, options);
    let resource = tool.resource().unwrap();
    let mut registry = ToolRegistry::new();
    registry
        .register(
            WebSearchTool::spec().unwrap(),
            Arc::new(StaticResourceResolver::new([resource]).unwrap()),
            Arc::new(tool),
        )
        .unwrap();
    registry
}

#[test]
fn request_contract_bounds_query_domains_and_result_limit() {
    assert!(SearchRequest::new("", NO_DOMAINS, NO_DOMAINS, 5).is_err());
    assert!(SearchRequest::new("x".repeat(2049), NO_DOMAINS, NO_DOMAINS, 5).is_err());
    assert!(SearchRequest::new("tea", NO_DOMAINS, NO_DOMAINS, 0).is_err());
    assert!(SearchRequest::new("tea", NO_DOMAINS, NO_DOMAINS, 21).is_err());
    assert!(SearchRequest::new("tea", ["Example.COM"], NO_DOMAINS, 5).is_err());
    assert!(SearchRequest::new("tea", ["example.com"], ["spam.example"], 5).is_err());
    assert!(
        SearchRequest::new(
            "tea",
            (0..=MAX_WEB_SEARCH_DOMAINS).map(|index| format!("d{index}.example")),
            NO_DOMAINS,
            5,
        )
        .is_err()
    );

    let request = SearchRequest::new(
        "tea rust",
        ["docs.rs", "example.com", "docs.rs"],
        NO_DOMAINS,
        7,
    )
    .unwrap();
    assert_eq!(request.query(), "tea rust");
    assert_eq!(request.allowed_domains(), ["docs.rs", "example.com"]);
    assert!(request.blocked_domains().is_empty());
    assert_eq!(request.limit(), 7);

    let options = WebSearchOptions::new().with_location(
        WebSearchLocation::new()
            .with_country("CN")
            .unwrap()
            .with_timezone("Asia/Shanghai")
            .unwrap(),
    );
    let request = SearchRequest::from_options("tea rust", options, 7).unwrap();
    assert_eq!(request.location().unwrap().country(), Some("CN"));
    assert_eq!(
        request.location().unwrap().timezone(),
        Some("Asia/Shanghai")
    );
}

#[test]
fn web_search_spec_declares_network_effect_and_explicit_retry() {
    let spec = WebSearchTool::spec().unwrap();
    assert_eq!(spec.name().as_str(), "web_search");
    assert_eq!(spec.effects(), &[ToolEffect::NetworkRequest]);
    assert_eq!(
        spec.execution().retry_safety(),
        ToolRetrySafety::ExplicitOnly
    );
    assert!(spec.prompt_hint().is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn tool_applies_configured_domain_policy_and_returns_normalized_results() {
    let provider = Arc::new(FakeSearchProvider::new());
    let options = WebSearchOptions::new()
        .with_allowed_domains(["docs.rs"])
        .unwrap()
        .with_location(WebSearchLocation::new().with_country("CN").unwrap());
    let registry = registry(provider.clone(), options);

    let events = registry
        .execute(
            invocation(json!({"query":"tea runtime","limit":3})),
            CancellationScope::new(),
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    let ToolExecutionEvent::Finished(result) = &events[0] else {
        panic!("expected successful search: {events:?}");
    };
    assert_eq!(result.output()["results"][0]["url"], "https://docs.rs/tea");
    assert_eq!(result.output()["results"][0]["score"], 0.9);
    assert_eq!(result.output()["truncated"], false);
    {
        let captured = provider.requests.lock().unwrap();
        assert_eq!(captured[0].allowed_domains(), ["docs.rs"]);
        assert_eq!(captured[0].limit(), 3);
        assert_eq!(captured[0].location().unwrap().country(), Some("CN"));
    }

    let denied = registry
        .execute(
            invocation(json!({
                "query":"tea runtime",
                "allowed_domains":["example.com"]
            })),
            CancellationScope::new(),
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(denied[0], ToolExecutionEvent::Failed(_)));
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
}

async fn response_server(
    status: &'static str,
    body: &'static str,
    delay: Duration,
) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 2048];
        let header_end = loop {
            let read = stream.read(&mut buffer).await.unwrap();
            if read == 0 {
                return String::from_utf8(request).unwrap();
            }
            request.extend_from_slice(&buffer[..read]);
            if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("content-length: ")
                    .or_else(|| line.strip_prefix("Content-Length: "))
            })
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let read = stream.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
        }
        tokio::time::sleep(delay).await;
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        String::from_utf8(request).unwrap()
    });
    (format!("http://{address}/search"), task)
}

#[tokio::test(flavor = "current_thread")]
async fn tavily_backend_maps_request_auth_and_normalized_response() {
    let response = r#"{
        "query":"tea runtime",
        "results":[
            {"title":"Tea guide","url":"https://example.com/tea","content":"Agent runtime","score":0.91},
            {"title":"Tea API","url":"https://docs.rs/tea","content":"Rust API","score":0.80}
        ]
    }"#;
    let (endpoint, server) = response_server("200 OK", response, Duration::ZERO).await;
    let config = TavilySearchConfig::new(endpoint, Duration::from_secs(1)).unwrap();
    let provider = TavilySearchProvider::new(
        config,
        TavilyApiKey::new("tvly-secret-test").unwrap(),
        &ProviderHttpConfig::new(),
    )
    .unwrap();
    let request =
        SearchRequest::new("tea runtime", ["docs.rs", "example.com"], NO_DOMAINS, 2).unwrap();

    let result = provider
        .search(request, CancellationScope::new())
        .await
        .unwrap();
    let captured = server.await.unwrap();

    assert_eq!(result.results().len(), 2);
    assert_eq!(result.results()[0].source().title(), Some("Tea guide"));
    assert_eq!(
        result.results()[0].source().snippet(),
        Some("Agent runtime")
    );
    assert_eq!(result.results()[0].score(), Some(0.91));
    assert!(
        captured
            .lines()
            .any(|line| line.eq_ignore_ascii_case("authorization: Bearer tvly-secret-test"))
    );
    let body = captured.split("\r\n\r\n").nth(1).unwrap();
    let body: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(body["query"], "tea runtime");
    assert_eq!(body["search_depth"], "basic");
    assert_eq!(body["max_results"], 2);
    assert_eq!(body["include_domains"], json!(["docs.rs", "example.com"]));
    assert_eq!(body["include_answer"], false);
    assert_eq!(body["include_raw_content"], false);
    assert!(!format!("{provider:?}").contains("secret-test"));
}

#[tokio::test(flavor = "current_thread")]
async fn tavily_backend_classifies_cancellation_timeout_and_provider_errors() {
    let cancelled = CancellationScope::new();
    cancelled.cancel();
    let provider = TavilySearchProvider::new(
        TavilySearchConfig::new("http://127.0.0.1:9/search", Duration::from_millis(20)).unwrap(),
        TavilyApiKey::new("tvly-test").unwrap(),
        &ProviderHttpConfig::new(),
    )
    .unwrap();
    let error = provider
        .search(
            SearchRequest::new("tea", NO_DOMAINS, NO_DOMAINS, 1).unwrap(),
            cancelled,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), SearchProviderErrorCode::Cancelled);

    let (endpoint, server) = response_server(
        "200 OK",
        r#"{"query":"tea","results":[]}"#,
        Duration::from_millis(100),
    )
    .await;
    let provider = TavilySearchProvider::new(
        TavilySearchConfig::new(endpoint, Duration::from_millis(20)).unwrap(),
        TavilyApiKey::new("tvly-test").unwrap(),
        &ProviderHttpConfig::new(),
    )
    .unwrap();
    let error = provider
        .search(
            SearchRequest::new("tea", NO_DOMAINS, NO_DOMAINS, 1).unwrap(),
            CancellationScope::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), SearchProviderErrorCode::Timeout);
    server.await.unwrap();

    let (endpoint, server) = response_server("429 Too Many Requests", "{}", Duration::ZERO).await;
    let provider = TavilySearchProvider::new(
        TavilySearchConfig::new(endpoint, Duration::from_secs(1)).unwrap(),
        TavilyApiKey::new("tvly-test").unwrap(),
        &ProviderHttpConfig::new(),
    )
    .unwrap();
    let error = provider
        .search(
            SearchRequest::new("tea", NO_DOMAINS, NO_DOMAINS, 1).unwrap(),
            CancellationScope::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), SearchProviderErrorCode::RateLimited);
    server.await.unwrap();
}

#[test]
fn tavily_configuration_and_credentials_are_validated_and_redacted() {
    assert!(TavilyApiKey::new("").is_err());
    assert!(TavilyApiKey::new("bad\nkey").is_err());
    let key = TavilyApiKey::new("tvly-secret-test").unwrap();
    assert!(!format!("{key:?}").contains("secret-test"));
    assert!(TavilySearchConfig::new("http://example.com/search", Duration::from_secs(1)).is_err());
    assert!(
        TavilySearchConfig::new(
            "https://user:password@example.com/search",
            Duration::from_secs(1)
        )
        .is_err()
    );
}

#[test]
fn resource_disclosure_uses_the_configured_search_destination() {
    let provider: Arc<dyn SearchProvider> = Arc::new(FakeSearchProvider::new());
    let tool = WebSearchTool::new(provider, WebSearchOptions::new());
    let resource = tool.resource().unwrap();
    assert_eq!(resource.scheme(), "url");
    assert_eq!(resource.locator(), "https://search.example.test/api");
    assert_eq!(resource.access(), ToolResourceAccess::Read);
}
