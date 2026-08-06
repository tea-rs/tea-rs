use std::collections::BTreeMap;
use std::pin::Pin;
use std::str::FromStr as _;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use clap::Parser as _;
use tea_cli::args::CliArgs;
use tea_cli::rpc::{RpcLineWriter, RpcWriteError};
use tea_cli::{BootstrapEnvironment, CliBootstrap, ExitCategory};
use tea_model::{ModelCapabilities, ModelDisplayName, ModelSpec, ProviderId};
use tea_protocol::{ModelId, TokenCount};
use tea_testkit::{ScriptedModelProvider, ScriptedModelResponse};
use tokio::io::{AsyncWrite, AsyncWriteExt as _};

#[derive(Debug, Default)]
struct PendingOutput;

impl AsyncWrite for PendingOutput {
    fn poll_write(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        _bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Poll::Pending
    }

    fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Pending
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[derive(Debug, Default)]
struct FirstFrameThenPending {
    wrote_first_frame: bool,
}

impl AsyncWrite for FirstFrameThenPending {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.wrote_first_frame {
            Poll::Pending
        } else {
            self.wrote_first_frame = true;
            Poll::Ready(Ok(bytes.len()))
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test(flavor = "current_thread")]
async fn response_queue_and_writer_latency_remain_bounded() {
    let writer = Arc::new(RpcLineWriter::spawn_with_deadline(
        PendingOutput,
        Duration::from_millis(20),
    ));
    let mut tasks = Vec::new();
    for index in 0..128 {
        let writer = Arc::clone(&writer);
        tasks.push(tokio::spawn(async move {
            writer.write(&serde_json::json!({"index":index})).await
        }));
    }
    let results = tokio::time::timeout(Duration::from_secs(1), async {
        let mut results = Vec::new();
        for write in tasks {
            results.push(write.await.unwrap());
        }
        results
    })
    .await
    .expect("bounded writers must all terminate");
    assert!(results.iter().all(Result::is_err));
    assert!(
        results.iter().any(|result| {
            matches!(result, Err(RpcWriteError::Deadline | RpcWriteError::Closed))
        })
    );
    let writer = Arc::try_unwrap(writer).expect("all write tasks joined");
    assert!(writer.shutdown().await.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn slow_client_cancels_owned_work_and_returns_within_deadline() {
    let root = std::env::temp_dir().join(format!(
        "tea-rpc-slow-{}",
        uuid::Uuid::now_v7().hyphenated()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let provider_id = ProviderId::from_str("fake").unwrap();
    let model = ModelSpec::new(
        ModelId::from_str("fake/model").unwrap(),
        provider_id.clone(),
        ModelDisplayName::from_str("Fake Model").unwrap(),
        TokenCount::new(32_000).unwrap(),
        TokenCount::new(4_000).unwrap(),
        ModelCapabilities::text().with_tools(true),
    )
    .unwrap();
    let provider = Arc::new(ScriptedModelProvider::new(
        provider_id,
        vec![model],
        [ScriptedModelResponse::await_cancellation()],
    ));
    let args = CliArgs::try_parse_from([
        "tea",
        "--rpc",
        "--no-session",
        "--provider",
        "fake",
        "--model",
        "fake/model",
        "--trust",
        "ignore",
        "--cwd",
        root.to_str().unwrap(),
        "--config-dir",
        root.join("config").to_str().unwrap(),
        "--state-dir",
        root.join("state").to_str().unwrap(),
        "--data-dir",
        root.join("data").to_str().unwrap(),
    ])
    .unwrap();
    let bootstrap = CliBootstrap::new(BootstrapEnvironment::new(
        &root,
        Some(root.clone()),
        BTreeMap::new(),
    ))
    .with_provider(Arc::clone(&provider) as Arc<dyn tea_model::ModelProvider>);
    let (mut client, server_input) = tokio::io::duplex(4096);
    client
        .write_all(
            b"{\"rpcVersion\":\"1.0\",\"id\":\"prompt\",\"type\":\"prompt\",\"payload\":{\"text\":\"work\"}}\n",
        )
        .await
        .unwrap();
    client.shutdown().await.unwrap();

    let error = Box::pin(tokio::time::timeout(
        Duration::from_secs(2),
        tea_cli::rpc::run(
            &args,
            &bootstrap,
            server_input,
            FirstFrameThenPending::default(),
        ),
    ))
    .await
    .expect("slow client policy must terminate")
    .unwrap_err();
    assert_eq!(error.category(), ExitCategory::Cancelled);
    assert_eq!(provider.captured_requests().unwrap().len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}
