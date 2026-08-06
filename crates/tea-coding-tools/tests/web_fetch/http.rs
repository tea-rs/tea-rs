use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tea_coding_tools::{
    FetchDnsResolver, FetchHttpConfig, FetchHttpLimits, FetchHttpTimeouts, FetchHttpTransport,
    FetchProviderError, FetchProviderErrorCode, FetchResolveFuture,
};
use tea_control::CancellationScope;
use tea_provider_http::ProviderHttpConfig;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

#[derive(Debug)]
struct FakeResolver {
    answers: Mutex<VecDeque<Result<Vec<IpAddr>, FetchProviderError>>>,
    hosts: Mutex<Vec<String>>,
}

impl FakeResolver {
    fn repeated(answer: &[IpAddr], count: usize) -> Self {
        Self {
            answers: Mutex::new((0..count).map(|_| Ok(answer.to_vec())).collect()),
            hosts: Mutex::new(Vec::new()),
        }
    }

    fn hosts(&self) -> Vec<String> {
        self.hosts.lock().unwrap().clone()
    }
}

impl FetchDnsResolver for FakeResolver {
    fn resolve(&self, host: &str, cancellation: CancellationScope) -> FetchResolveFuture<'_> {
        self.hosts.lock().unwrap().push(host.to_owned());
        let answer = self.answers.lock().unwrap().pop_front().unwrap_or_else(|| {
            Err(FetchProviderError::new(
                FetchProviderErrorCode::DnsResolution,
            ))
        });
        Box::pin(async move {
            if cancellation.is_cancelled() {
                Err(FetchProviderError::cancelled())
            } else {
                answer
            }
        })
    }
}

fn loopback() -> IpAddr {
    IpAddr::V4(Ipv4Addr::LOCALHOST)
}

fn config(max_redirects: usize, max_bytes: usize) -> FetchHttpConfig {
    let timeouts = FetchHttpTimeouts::new(
        Duration::from_millis(250),
        Duration::from_millis(250),
        Duration::from_millis(500),
        Duration::from_secs(2),
        Duration::from_millis(500),
        1,
    )
    .unwrap();
    config_with_timeouts(timeouts, max_redirects, max_bytes)
}

fn config_with_timeouts(
    timeouts: FetchHttpTimeouts,
    max_redirects: usize,
    max_bytes: usize,
) -> FetchHttpConfig {
    let limits = FetchHttpLimits::new(max_redirects, max_bytes).unwrap();
    FetchHttpConfig::loopback_tests(timeouts, limits)
}

fn transport(
    resolver: Arc<dyn FetchDnsResolver>,
    max_redirects: usize,
    max_bytes: usize,
) -> FetchHttpTransport {
    FetchHttpTransport::new(
        config(max_redirects, max_bytes),
        resolver,
        ProviderHttpConfig::new(),
    )
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

async fn write_response(stream: &mut TcpStream, response: &str) {
    stream.write_all(response.as_bytes()).await.unwrap();
    stream.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn transport_pins_dns_preserves_host_and_bounds_response_metadata() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_request(&mut stream).await;
        write_response(
            &mut stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
        )
        .await;
        request
    });
    let resolver = Arc::new(FakeResolver::repeated(&[loopback()], 1));
    let response = transport(resolver.clone(), 5, 1024)
        .get(
            &format!("http://localhost:{}/path?token=secret", address.port()),
            CancellationScope::new(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.body(), b"hello");
    assert_eq!(
        response.headers().content_type(),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(response.headers().content_length(), Some(5));
    assert_eq!(resolver.hosts(), ["localhost"]);
    let request = server.await.unwrap();
    assert!(request.starts_with("GET /path?token=secret HTTP/1.1\r\n"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains(&format!("host: localhost:{}", address.port()))
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("accept-encoding: identity")
    );
    let debug = format!("{response:?}");
    assert!(debug.contains("body_bytes: 5"));
    assert!(!debug.contains("hello"));
    assert!(!debug.contains("secret"));
}

#[tokio::test(flavor = "current_thread")]
async fn redirect_is_resolved_revalidated_and_recorded_per_hop() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        let first_request = read_request(&mut first).await;
        write_response(
            &mut first,
            "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        let (mut second, _) = listener.accept().await.unwrap();
        let second_request = read_request(&mut second).await;
        write_response(
            &mut second,
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone",
        )
        .await;
        (first_request, second_request)
    });
    let resolver = Arc::new(FakeResolver::repeated(&[loopback()], 2));
    let response = transport(resolver.clone(), 5, 1024)
        .get(
            &format!("http://localhost:{}/start", address.port()),
            CancellationScope::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.final_url(),
        format!("http://localhost:{}/final", address.port())
    );
    assert_eq!(response.body(), b"done");
    assert_eq!(response.redirects().len(), 1);
    assert_eq!(response.redirects()[0].status(), 302);
    assert_eq!(resolver.hosts(), ["localhost", "localhost"]);
    let (first, second) = server.await.unwrap();
    assert!(first.starts_with("GET /start HTTP/1.1"));
    assert!(second.starts_with("GET /final HTTP/1.1"));
}

#[tokio::test(flavor = "current_thread")]
async fn mixed_dns_answers_are_rejected_before_connection() {
    let resolver = Arc::new(FakeResolver::repeated(
        &[loopback(), "10.0.0.1".parse().unwrap()],
        1,
    ));
    let error = transport(resolver, 5, 1024)
        .get("http://localhost:8080/", CancellationScope::new())
        .await
        .unwrap_err();
    assert_eq!(error.code(), FetchProviderErrorCode::DnsResolution);
}

#[tokio::test(flavor = "current_thread")]
async fn redirect_loop_is_rejected_without_a_second_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_request(&mut stream).await;
        write_response(
            &mut stream,
            "HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        request
    });
    let resolver = Arc::new(FakeResolver::repeated(&[loopback()], 1));
    let error = transport(resolver, 5, 1024)
        .get(
            &format!("http://localhost:{}/loop", address.port()),
            CancellationScope::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), FetchProviderErrorCode::RedirectRejected);
    assert!(server.await.unwrap().starts_with("GET /loop HTTP/1.1"));
}

#[tokio::test(flavor = "current_thread")]
async fn redirect_to_private_target_is_rejected_before_new_dns_resolution() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_request(&mut stream).await;
        write_response(
            &mut stream,
            "HTTP/1.1 302 Found\r\nLocation: http://192.168.1.1:8080/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        request
    });
    let resolver = Arc::new(FakeResolver::repeated(&[loopback()], 1));
    let error = transport(resolver.clone(), 5, 1024)
        .get(
            &format!("http://localhost:{}/start", address.port()),
            CancellationScope::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), FetchProviderErrorCode::RedirectRejected);
    assert_eq!(resolver.hosts(), ["localhost"]);
    server.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn redirect_dns_rebinding_to_private_address_is_rejected_before_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await;
        write_response(
            &mut stream,
            "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
    });
    let resolver = Arc::new(FakeResolver {
        answers: Mutex::new(VecDeque::from([
            Ok(vec![loopback()]),
            Ok(vec!["10.0.0.1".parse().unwrap()]),
        ])),
        hosts: Mutex::new(Vec::new()),
    });
    let error = transport(resolver.clone(), 5, 1024)
        .get(
            &format!("http://localhost:{}/start", address.port()),
            CancellationScope::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), FetchProviderErrorCode::DnsResolution);
    assert_eq!(resolver.hosts(), ["localhost", "localhost"]);
    server.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn content_length_above_the_raw_body_limit_fails_before_reading_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await;
        write_response(
            &mut stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 100\r\nConnection: close\r\n\r\n",
        )
        .await;
    });
    let resolver = Arc::new(FakeResolver::repeated(&[loopback()], 1));
    let error = transport(resolver, 5, 8)
        .get(
            &format!("http://localhost:{}/large", address.port()),
            CancellationScope::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), FetchProviderErrorCode::ResponseTooLarge);
    server.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn first_byte_timeout_is_bounded_without_waiting_for_server_shutdown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    });
    let timeouts = FetchHttpTimeouts::new(
        Duration::from_millis(100),
        Duration::from_millis(100),
        Duration::from_millis(40),
        Duration::from_millis(250),
        Duration::from_millis(100),
        1,
    )
    .unwrap();
    let resolver = Arc::new(FakeResolver::repeated(&[loopback()], 1));
    let transport = FetchHttpTransport::new(
        config_with_timeouts(timeouts, 5, 1024),
        resolver,
        ProviderHttpConfig::new(),
    );
    let error = transport
        .get(
            &format!("http://localhost:{}/slow-headers", address.port()),
            CancellationScope::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), FetchProviderErrorCode::Timeout);
    server.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn low_speed_body_timeout_is_bounded_by_window_and_byte_threshold() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await;
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\na",
            )
            .await
            .unwrap();
        stream.flush().await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });
    let timeouts = FetchHttpTimeouts::new(
        Duration::from_millis(100),
        Duration::from_millis(100),
        Duration::from_millis(100),
        Duration::from_millis(250),
        Duration::from_millis(40),
        10,
    )
    .unwrap();
    let resolver = Arc::new(FakeResolver::repeated(&[loopback()], 1));
    let transport = FetchHttpTransport::new(
        config_with_timeouts(timeouts, 5, 1024),
        resolver,
        ProviderHttpConfig::new(),
    );
    let error = transport
        .get(
            &format!("http://localhost:{}/slow-body", address.port()),
            CancellationScope::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), FetchProviderErrorCode::Timeout);
    server.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_interrupts_a_waiting_first_byte() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    });
    let resolver = Arc::new(FakeResolver::repeated(&[loopback()], 1));
    let cancellation = CancellationScope::new();
    let canceller = cancellation.clone();
    let task = tokio::spawn(async move {
        transport(resolver, 5, 1024)
            .get(
                &format!("http://localhost:{}/slow", address.port()),
                cancellation,
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    canceller.cancel();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code(), FetchProviderErrorCode::Cancelled);
    server.abort();
}
