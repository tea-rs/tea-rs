use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tea_coding_tools::{
    FetchBodyDecoder, FetchBodyLimits, FetchCacheConfig, FetchCacheScope, FetchHttpConfig,
    FetchHttpLimits, FetchHttpTimeouts, FetchHttpTransport, FetchProvider, FetchProviderErrorCode,
    FetchRequest, FetchResultCache, FetchRetryDisposition, FetchUrlPolicy, HttpFetchProvider,
    SystemFetchDnsResolver,
};
use tea_control::CancellationScope;
use tea_provider_http::ProviderHttpConfig;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone)]
enum ResponseBody {
    Fixed(&'static str),
    RequestPath,
}

#[derive(Clone)]
struct FixtureResponse {
    status: u16,
    content_type: &'static str,
    body: ResponseBody,
}

struct FixtureServer {
    address: SocketAddr,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl FixtureServer {
    async fn spawn(response: FixtureResponse) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let request_counter = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let request = read_request(&mut stream).await;
                request_counter.fetch_add(1, Ordering::SeqCst);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let body = match response.body {
                    ResponseBody::Fixed(body) => body.to_owned(),
                    ResponseBody::RequestPath => path.to_owned(),
                };
                write_response(&mut stream, &response, &body).await;
            }
        });
        Self {
            address,
            requests,
            task,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.address.port())
    }

    fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream.read(&mut buffer).await.unwrap();
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(request).unwrap()
}

async fn write_response(stream: &mut TcpStream, response: &FixtureResponse, body: &str) {
    let reason = match response.status {
        200 => "OK",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Fixture",
    };
    let head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        response.content_type,
        body.len(),
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body.as_bytes()).await.unwrap();
    stream.shutdown().await.unwrap();
}

fn transport() -> FetchHttpTransport {
    let timeouts = FetchHttpTimeouts::new(
        Duration::from_millis(250),
        Duration::from_millis(250),
        Duration::from_millis(500),
        Duration::from_secs(2),
        Duration::from_millis(500),
        1,
    )
    .unwrap();
    let limits = FetchHttpLimits::new(5, 16 * 1024).unwrap();
    FetchHttpTransport::new(
        FetchHttpConfig::loopback_tests(timeouts, limits),
        Arc::new(SystemFetchDnsResolver),
        ProviderHttpConfig::new(),
    )
}

fn scope(workspace: &str, profile: &str) -> FetchCacheScope {
    FetchCacheScope::new(workspace, profile).unwrap()
}

fn make_provider(scope: FetchCacheScope, cache: Arc<FetchResultCache>) -> HttpFetchProvider {
    HttpFetchProvider::new(transport(), FetchBodyDecoder::default(), scope, cache)
}

fn make_provider_with_decoder(
    scope: FetchCacheScope,
    cache: Arc<FetchResultCache>,
    decoder: FetchBodyDecoder,
) -> HttpFetchProvider {
    HttpFetchProvider::new(transport(), decoder, scope, cache)
}

fn request(server: &FixtureServer, path: &str, max_chars: usize) -> FetchRequest {
    FetchRequest::new_with_policy(
        server.url(path),
        max_chars,
        &FetchUrlPolicy::loopback_tests(),
    )
    .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn provider_normalizes_success_and_reuses_only_normalized_cache_entries() {
    let server = FixtureServer::spawn(FixtureResponse {
        status: 200,
        content_type: "text/html; charset=utf-8",
        body: ResponseBody::Fixed(
            "<html><head><title>Tea docs</title></head><body>cached body</body></html>",
        ),
    })
    .await;
    let cache = Arc::new(FetchResultCache::new(FetchCacheConfig::default()));
    let provider = make_provider(scope("private-workspace", "private-profile"), cache.clone());
    let request = request(&server, "/guide?token=secret", 100);

    let first = provider
        .fetch(request.clone(), CancellationScope::new())
        .await
        .unwrap();
    let second = provider
        .fetch(request, CancellationScope::new())
        .await
        .unwrap();

    assert_eq!(first, second);
    assert_eq!(first.title(), Some("Tea docs"));
    assert_eq!(first.body(), "cached body");
    assert_eq!(server.request_count(), 1);
    assert_eq!(cache.stats().entries(), 1);
    let debug = format!("{provider:?}");
    assert!(!debug.contains("private-workspace"));
    assert!(!debug.contains("private-profile"));
    assert!(!debug.contains("secret"));
    assert!(!debug.contains("cached body"));
}

#[tokio::test(flavor = "current_thread")]
async fn shared_cache_is_isolated_by_workspace_and_profile() {
    let server = FixtureServer::spawn(FixtureResponse {
        status: 200,
        content_type: "text/plain",
        body: ResponseBody::Fixed("isolated"),
    })
    .await;
    let cache = Arc::new(FetchResultCache::new(FetchCacheConfig::default()));
    let providers = [
        make_provider(scope("workspace-a", "profile-a"), cache.clone()),
        make_provider(scope("workspace-b", "profile-a"), cache.clone()),
        make_provider(scope("workspace-a", "profile-b"), cache.clone()),
    ];
    let request = request(&server, "/same", 100);

    for provider in &providers {
        provider
            .fetch(request.clone(), CancellationScope::new())
            .await
            .unwrap();
    }
    for provider in &providers {
        provider
            .fetch(request.clone(), CancellationScope::new())
            .await
            .unwrap();
    }

    let stricter_decoder = FetchBodyDecoder::new(FetchBodyLimits::new(1024, 1024, 100).unwrap());
    make_provider_with_decoder(
        scope("workspace-a", "profile-a"),
        cache.clone(),
        stricter_decoder,
    )
    .fetch(request.clone(), CancellationScope::new())
    .await
    .unwrap();

    assert_eq!(server.request_count(), 4);
    assert_eq!(cache.stats().entries(), 4);
}

#[tokio::test(flavor = "current_thread")]
async fn cache_evicts_least_recently_used_and_skips_oversized_entries() {
    let server = FixtureServer::spawn(FixtureResponse {
        status: 200,
        content_type: "text/plain",
        body: ResponseBody::RequestPath,
    })
    .await;
    let cache = Arc::new(FetchResultCache::new(
        FetchCacheConfig::new(Duration::from_mins(1), 2, 4096, 2048).unwrap(),
    ));
    let provider = make_provider(scope("workspace", "profile"), cache.clone());

    for path in ["/one", "/two", "/one", "/three", "/two"] {
        provider
            .fetch(request(&server, path, 100), CancellationScope::new())
            .await
            .unwrap();
    }
    assert_eq!(server.request_count(), 4);
    assert_eq!(cache.stats().entries(), 2);
    assert!(cache.stats().logical_bytes() <= 4096);

    let tiny_cache = Arc::new(FetchResultCache::new(
        FetchCacheConfig::new(Duration::from_mins(1), 2, 128, 128).unwrap(),
    ));
    let tiny_provider = make_provider(scope("workspace", "tiny"), tiny_cache.clone());
    let oversized = request(&server, "/oversized", 100);
    tiny_provider
        .fetch(oversized.clone(), CancellationScope::new())
        .await
        .unwrap();
    tiny_provider
        .fetch(oversized, CancellationScope::new())
        .await
        .unwrap();
    assert_eq!(server.request_count(), 6);
    assert_eq!(tiny_cache.stats().entries(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_wins_even_when_a_normalized_result_is_cached() {
    let server = FixtureServer::spawn(FixtureResponse {
        status: 200,
        content_type: "text/plain",
        body: ResponseBody::Fixed("cached"),
    })
    .await;
    let cache = Arc::new(FetchResultCache::new(FetchCacheConfig::default()));
    let provider = make_provider(scope("workspace", "profile"), cache);
    let request = request(&server, "/cancel", 100);
    provider
        .fetch(request.clone(), CancellationScope::new())
        .await
        .unwrap();

    let cancellation = CancellationScope::new();
    cancellation.cancel();
    let error = provider.fetch(request, cancellation).await.unwrap_err();
    assert_eq!(error.code(), FetchProviderErrorCode::Cancelled);
    assert_eq!(server.request_count(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn only_pre_response_transport_failures_allow_explicit_retry() {
    let unused = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable_address = unused.local_addr().unwrap();
    drop(unused);
    let cache = Arc::new(FetchResultCache::new(FetchCacheConfig::default()));
    let provider = make_provider(scope("workspace", "profile"), cache.clone());
    let unavailable = FetchRequest::new_with_policy(
        format!("http://127.0.0.1:{}/", unavailable_address.port()),
        100,
        &FetchUrlPolicy::loopback_tests(),
    )
    .unwrap();
    let error = provider
        .fetch(unavailable, CancellationScope::new())
        .await
        .unwrap_err();
    assert_eq!(error.code(), FetchProviderErrorCode::Transport);
    assert_eq!(error.retry_disposition(), FetchRetryDisposition::Explicit);

    let server = FixtureServer::spawn(FixtureResponse {
        status: 500,
        content_type: "text/plain",
        body: ResponseBody::Fixed("failure body"),
    })
    .await;
    let failed_request = request(&server, "/failure", 100);
    for _ in 0..2 {
        let error = provider
            .fetch(failed_request.clone(), CancellationScope::new())
            .await
            .unwrap_err();
        assert_eq!(error.code(), FetchProviderErrorCode::Transport);
        assert_eq!(error.retry_disposition(), FetchRetryDisposition::Never);
        assert!(!format!("{error:?}").contains("failure body"));
    }
    assert_eq!(server.request_count(), 2);
    assert_eq!(cache.stats().entries(), 0);
}
