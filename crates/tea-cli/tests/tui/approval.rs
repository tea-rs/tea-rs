use std::collections::BTreeMap;
use std::fs;
use std::str::FromStr as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::Parser as _;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tea_cli::args::{CliArgs, SessionSelection};
use tea_cli::tui::{
    Action, ApprovalChoice, FrameSink, InputEvent, MemoryClipboard, Renderer, Theme, TuiState,
    reduce, run_with_channels,
};
use tea_cli::{BootstrapEnvironment, CliBootstrap};
use tea_model::{
    ModelCapabilities, ModelCompletion, ModelDisplayName, ModelEvent, ModelResponseInfo, ModelSpec,
    ModelStreamIndex, ProviderId, ProviderToolCallId, ToolCallCompleted, ToolCallStarted,
};
use tea_protocol::{ModelId, StopReason, TokenCount};
use tea_session::ToolExecutionState;
use tea_testkit::{ScriptedModelProvider, ScriptedModelResponse};

use crate::common;

#[tokio::test(flavor = "current_thread")]
async fn approval_projection_uses_only_redacted_persisted_arguments() {
    let snapshot = common::pending_snapshot().await;
    let state = TuiState::from_snapshot(&snapshot, common::startup());
    let mut renderer = Renderer::new();
    let text = renderer
        .lines(&state, 120, &Theme::default())
        .iter()
        .map(tea_cli::tui::RenderedLine::text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(text.contains("approval required"));
    assert!(text.contains("write_text_file"));
    assert!(text.contains("/workspace/notes.txt"));
    assert!(text.contains("target"));
    assert!(text.contains("effects"));
    assert!(text.contains("resources"));
    assert!(text.contains("expires"));
    assert!(text.contains("arguments"));
    assert!(text.contains("allow once"));
    assert!(text.contains("allow for session"));
    assert!(text.contains("deny"));
    assert!(text.contains("matching resources this session"));
    assert!(!text.contains("\"content\":\"done\""));
}

#[tokio::test(flavor = "current_thread")]
async fn approval_selection_is_bounded_and_submission_disables_changes() {
    let snapshot = common::pending_snapshot().await;
    let mut state = TuiState::from_snapshot(&snapshot, common::startup());
    assert_eq!(state.approval_choice(), ApprovalChoice::AllowOnce);
    assert!(!state.approval_submitting());

    let _ = reduce(
        &mut state,
        Action::SelectApproval(ApprovalChoice::AllowSession),
    );
    let _ = reduce(&mut state, Action::SetApprovalSubmitting(true));
    assert_eq!(state.approval_choice(), ApprovalChoice::AllowSession);
    assert!(state.approval_submitting());
    assert!(reduce(&mut state, Action::SelectApproval(ApprovalChoice::Deny)).is_empty());
    assert_eq!(state.approval_choice(), ApprovalChoice::AllowSession);

    let resolved = common::archive_snapshot().await;
    let _ = reduce(&mut state, Action::SnapshotLoaded(Box::new(resolved)));
    assert!(state.approval().is_none());
    assert_eq!(state.approval_choice(), ApprovalChoice::AllowOnce);
    assert!(!state.approval_submitting());
}

struct Frames {
    count: usize,
    approval_visible: Arc<AtomicBool>,
    approval_cleared: Arc<AtomicBool>,
    approval_cleared_while_running: Arc<AtomicBool>,
    saw_approval: bool,
}

impl FrameSink for Frames {
    fn render(&mut self, state: &TuiState, _cursor_byte: usize) -> std::io::Result<()> {
        self.count += 1;
        if state.approval().is_some() {
            self.saw_approval = true;
            self.approval_visible.store(true, Ordering::SeqCst);
        } else if self.saw_approval {
            self.approval_cleared.store(true, Ordering::SeqCst);
            if state.is_running() {
                self.approval_cleared_while_running
                    .store(true, Ordering::SeqCst);
            }
        }
        Ok(())
    }
}

fn tool_response() -> ScriptedModelResponse {
    let index = ModelStreamIndex::new(0).unwrap();
    let provider_id = ProviderToolCallId::from_str("edit-ui").unwrap();
    ScriptedModelResponse::events([
        ModelEvent::Started(ModelResponseInfo::new()),
        ModelEvent::ToolCallStarted(
            ToolCallStarted::new(index, provider_id.clone(), "edit").unwrap(),
        ),
        ModelEvent::ToolCallCompleted(
            ToolCallCompleted::new(
                index,
                provider_id,
                "edit",
                serde_json::json!({
                    "path":"file.txt",
                    "oldText":"old",
                    "newText":"new"
                }),
            )
            .unwrap(),
        ),
        ModelEvent::Completed(ModelCompletion::new(StopReason::ToolUse).unwrap()),
    ])
}

#[tokio::test(flavor = "current_thread")]
#[allow(clippy::too_many_lines)] // Keep the real input/service approval sequence visible.
async fn interactive_approval_submits_once_despite_duplicate_enter() {
    let root = std::env::temp_dir().join(format!(
        "tea-cli-approval-{}",
        uuid::Uuid::now_v7().hyphenated()
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("file.txt"), "old\n").unwrap();
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
        [tool_response(), ScriptedModelResponse::await_cancellation()],
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
    let (service, selection) = bootstrap.build(&args).unwrap();
    assert_eq!(selection, SessionSelection::NoSession);
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    let approval_visible = Arc::new(AtomicBool::new(false));
    let observed_approval = Arc::clone(&approval_visible);
    let approval_cleared = Arc::new(AtomicBool::new(false));
    let observed_approval_cleared = Arc::clone(&approval_cleared);
    let approval_cleared_while_running = Arc::new(AtomicBool::new(false));
    let observed_running_after_approval = Arc::clone(&approval_cleared_while_running);
    let drive = async {
        let session_id = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(entry) = service.list_sessions().await.unwrap().into_iter().next() {
                    let snapshot = service.session_snapshot(entry.session_id()).await.unwrap();
                    if !snapshot.state().pending_approvals().is_empty() {
                        break entry.session_id();
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("approval did not become pending");
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !observed_approval.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("approval was not rendered");
        sender
            .send(InputEvent::Key(KeyEvent::new(
                KeyCode::Right,
                KeyModifiers::NONE,
            )))
            .await
            .unwrap();
        for _ in 0..2 {
            sender
                .send(InputEvent::Key(KeyEvent::new(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                )))
                .await
                .unwrap();
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let snapshot = service.session_snapshot(session_id).await.unwrap();
                let finished =
                    snapshot.state().tool_calls().values().all(|tool| {
                        matches!(tool.execution(), ToolExecutionState::Finished { .. })
                    });
                if snapshot.state().pending_approvals().is_empty()
                    && snapshot.grant_journal().len() == 1
                    && finished
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("approval did not complete");
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !observed_approval_cleared.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("resolved approval remained visible");
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !observed_running_after_approval.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("run appeared idle after the resolved approval cleared");
        sender
            .send(InputEvent::Key(KeyEvent::new(
                KeyCode::Char('d'),
                KeyModifiers::CONTROL,
            )))
            .await
            .unwrap();
        session_id
    };
    let mut frames = Frames {
        count: 0,
        approval_visible,
        approval_cleared,
        approval_cleared_while_running,
        saw_approval: false,
    };
    let mut clipboard = MemoryClipboard::default();
    let (state, session_id) = tokio::join!(
        run_with_channels(
            &service,
            selection,
            receiver,
            &mut frames,
            &mut clipboard,
            Some("edit the file".to_owned()),
        ),
        drive
    );
    let state = state.unwrap();
    assert!(state.approval().is_none());
    assert!(!state.approval_submitting());
    assert_eq!(clipboard.contents(), None);
    assert!(frames.count > 2);
    let rendered = Renderer::new()
        .lines(&state, 120, &Theme::default())
        .iter()
        .map(tea_cli::tui::RenderedLine::text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !rendered.contains("event sequence gap detected"),
        "approval resume reported a false event gap: {rendered}"
    );
    let snapshot = service.session_snapshot(session_id).await.unwrap();
    assert_eq!(snapshot.grant_journal().len(), 1);
    assert_eq!(
        snapshot
            .approval_artifacts()
            .iter()
            .filter(|entry| matches!(entry, tea_session::ApprovalArtifactEntry::Resolved { .. }))
            .count(),
        1
    );
    assert_eq!(fs::read_to_string(root.join("file.txt")).unwrap(), "new\n");
    service.shutdown().await;
    fs::remove_dir_all(root).unwrap();
}
