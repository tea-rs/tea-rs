use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::str::FromStr as _;
use std::sync::Arc;

use clap::Parser as _;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tea_cli::args::{CliArgs, SessionSelection};
use tea_cli::session_views::{
    MAX_SNAPSHOT_PAGE_RECORDS, session_list, session_tree, snapshot_page,
};
use tea_cli::tui::{FrameSink, InputEvent, MemoryClipboard, TuiState, run_with_channels};
use tea_cli::{BootstrapEnvironment, CliBootstrap};
use tea_model::{ModelCapabilities, ModelDisplayName, ModelSpec, ProviderId};
use tea_protocol::{ModelId, TokenCount};
use tea_session::{InMemorySessionStore, SessionArchive, SessionCatalogEntry, SessionStore};
use tea_testkit::{ScriptedModelProvider, ScriptedModelResponse};

async fn fixture_snapshot() -> tea_session::SessionSnapshot {
    let archive = SessionArchive::decode_json(include_str!(
        "../../tea-session/tests/fixtures/v1/session-archive.json"
    ))
    .unwrap();
    let session_id = archive.session_id();
    let store = InMemorySessionStore::new();
    archive.import_into(&store).await.unwrap();
    store.load(session_id).await.unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn catalog_and_tree_views_are_stable_host_projections() {
    let snapshot = fixture_snapshot().await;
    let entry = SessionCatalogEntry::from_snapshot(&snapshot, None).unwrap();
    let list = session_list(&[entry]);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].session_id(), snapshot.state().session_id());
    assert_eq!(list[0].message_count(), snapshot.state().messages().len());

    let tree = session_tree(&snapshot);
    assert!(!tree.branches().is_empty());
    assert_eq!(
        tree.branches()
            .iter()
            .filter(|branch| branch.is_active())
            .count(),
        1
    );
    let json = serde_json::to_value(&tree).unwrap();
    assert_eq!(json["sessionId"], snapshot.state().session_id().to_string());
    assert!(json.get("branches").is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn snapshot_replay_is_cursor_based_and_page_bounded() {
    let snapshot = fixture_snapshot().await;
    let first = snapshot_page(&snapshot, None, 1);
    assert_eq!(first.records().len(), 1);
    assert_eq!(first.has_more(), snapshot.records().len() > 1);

    let cursor = first.records()[0].sequence();
    let rest = snapshot_page(&snapshot, Some(cursor), usize::MAX);
    assert!(rest.records().len() <= MAX_SNAPSHOT_PAGE_RECORDS);
    assert!(
        rest.records()
            .iter()
            .all(|record| record.sequence() > cursor)
    );
}

#[derive(Debug, Default)]
struct Frames;

impl FrameSink for Frames {
    fn render(&mut self, _state: &TuiState, _cursor_byte: usize) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn live_service_rebind_preserves_session_scoped_ephemeral_queue() {
    let root = std::env::temp_dir().join(format!(
        "tea-session-ux-{}",
        uuid::Uuid::now_v7().hyphenated()
    ));
    fs::create_dir_all(&root).unwrap();
    let provider_id = ProviderId::from_str("fake").unwrap();
    let provider = Arc::new(ScriptedModelProvider::new(
        provider_id.clone(),
        vec![
            ModelSpec::new(
                ModelId::from_str("fake/model").unwrap(),
                provider_id,
                ModelDisplayName::from_str("Fake Model").unwrap(),
                TokenCount::new(32_000).unwrap(),
                TokenCount::new(4_000).unwrap(),
                ModelCapabilities::text().with_tools(true),
            )
            .unwrap(),
        ],
        [ScriptedModelResponse::await_cancellation()],
    ));
    let args = CliArgs::try_parse_from([
        "tea",
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
    .with_provider(provider);
    let (service, _) = bootstrap.build(&args).unwrap();
    let original = service.create_session().await.unwrap();
    let (sender, receiver) = tokio::sync::mpsc::channel(32);
    let producer = tokio::spawn(async move {
        for input in [
            InputEvent::Paste("initial".to_owned()),
            InputEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        ] {
            sender.send(input).await.unwrap();
        }
        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
        for input in [
            InputEvent::Paste("steering".to_owned()),
            InputEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            InputEvent::Paste("/new".to_owned()),
            InputEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            InputEvent::Paste(format!("/resume {original}")),
            InputEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            InputEvent::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
        ] {
            sender.send(input).await.unwrap();
            tokio::task::yield_now().await;
        }
    });
    let mut frames = Frames;
    let mut clipboard = MemoryClipboard::default();
    let state = Box::pin(run_with_channels(
        &service,
        SessionSelection::Existing(original),
        receiver,
        &mut frames,
        &mut clipboard,
        None,
    ))
    .await
    .unwrap();
    producer.await.unwrap();
    assert_eq!(state.session_id(), original);
    assert_eq!(state.queued_message_count(), 1);
    service.shutdown().await;
    fs::remove_dir_all(root).unwrap();
}
