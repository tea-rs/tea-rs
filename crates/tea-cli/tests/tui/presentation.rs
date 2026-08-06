use std::collections::BTreeSet;

use tea_cli::tui::{
    Action, CellContent, CellId, DecisionStatus, LifecycleKind, LifecycleStatus, MessageAuthor,
    MessageCellFacet, NoticeKind, Presentation, QueuedInputKind, StreamCellFacet,
    TimelineDetailKind, ToolCellFacet, TuiState, reduce,
};
use tea_protocol::{
    AgentEvent, CodeChange, CodeChangeKind, EventDelta, HostedToolError, HostedToolOutcome,
    MessageId, ToolCallId, ToolPresentation,
};
use unicode_width::UnicodeWidthStr as _;

use crate::common;

#[tokio::test(flavor = "current_thread")]
#[allow(clippy::too_many_lines)]
async fn presentation_keeps_durable_work_in_order_and_ephemera_outside_history() {
    let snapshot = common::archive_snapshot().await;
    let mut state = TuiState::from_snapshot(&snapshot, common::startup());
    let initial = Presentation::from_state(&state);

    assert!(initial.history().iter().any(|cell| matches!(
        cell.content(),
        CellContent::Message(message) if message.author() == MessageAuthor::User
    )));
    assert!(initial.history().iter().any(|cell| matches!(
        cell.content(),
        CellContent::Message(message) if message.author() == MessageAuthor::Assistant
    )));
    assert!(
        initial
            .history()
            .iter()
            .all(|cell| !matches!(cell.content(), CellContent::Plan(_)))
    );

    let tool_call = initial
        .history()
        .iter()
        .position(|cell| {
            matches!(
                cell.content(),
                CellContent::Lifecycle(lifecycle)
                    if lifecycle.kind() == LifecycleKind::ToolCall
            )
        })
        .expect("fixture contains a durable tool call");
    let tool_result = initial
        .history()
        .iter()
        .position(|cell| matches!(cell.content(), CellContent::Result(_)))
        .expect("fixture contains a durable tool result");
    assert!(
        tool_call < tool_result,
        "tool work must remain chronological"
    );

    let next = state.event_cursor().get() + 1;
    let _ = reduce(
        &mut state,
        Action::Event(Box::new(common::event(
            next,
            401,
            AgentEvent::RunStarted {},
        ))),
    );
    let _ = reduce(
        &mut state,
        Action::Event(Box::new(common::event(
            next + 1,
            402,
            AgentEvent::MessageDelta {
                message_id: common::message_id(),
                content_index: 0,
                delta: EventDelta::TextDelta {
                    text: "streaming tail".to_owned(),
                },
            },
        ))),
    );
    let _ = reduce(
        &mut state,
        Action::QueueSteering("steer after this".to_owned()),
    );
    let _ = reduce(
        &mut state,
        Action::ShowPendingUserPrompt("show this before the model responds".to_owned()),
    );
    let _ = reduce(&mut state, Action::Notify("transient notice".to_owned()));

    let projected = Presentation::from_state(&state);
    let pending_prompt = projected
        .active()
        .iter()
        .position(|cell| {
            matches!(
                cell.content(),
                CellContent::Message(message) if message.author() == MessageAuthor::User
            )
        })
        .expect("accepted prompt is shown before a durable snapshot arrives");
    let stream = projected
        .active()
        .iter()
        .position(|cell| {
            matches!(
                cell.id(),
                CellId::Stream {
                    facet: StreamCellFacet::Message,
                    ..
                }
            )
        })
        .expect("streaming tail is visible");
    assert!(
        pending_prompt < stream,
        "the submitted prompt must precede the response stream"
    );
    assert!(projected.active().iter().any(|cell| {
        matches!(
            cell.content(),
            CellContent::Message(message)
                if message.author() == MessageAuthor::User
                    && message.source() == "show this before the model responds"
        )
    }));
    assert!(projected.active().iter().any(|cell| {
        matches!(
            cell.content(),
            CellContent::Message(message)
                if message.author() == MessageAuthor::Assistant
                    && message.source() == "streaming tail"
        )
    }));
    assert!(projected.active().iter().any(|cell| matches!(
        cell.content(),
        CellContent::QueuedInput(input) if input.kind() == QueuedInputKind::Steering
    )));
    assert!(has_notice(&projected, "transient notice"));
    assert!(
        projected
            .history()
            .iter()
            .all(|cell| cell.text() != "transient notice")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn hosted_search_observations_project_running_and_completed_lifecycles() {
    let snapshot = common::archive_snapshot().await;
    let mut state = TuiState::from_snapshot(&snapshot, common::startup());
    let successful_id = "0195a0b1-7e00-7000-8000-000000000093".parse().unwrap();
    let first = state.event_cursor().get() + 1;
    let _ = reduce(&mut state, Action::StartRunActivity);

    let _ = reduce(
        &mut state,
        Action::Event(Box::new(common::event(
            first,
            510,
            AgentEvent::HostedToolStarted {
                tool_call_id: successful_id,
                tool_name: "web_search".to_owned(),
            },
        ))),
    );
    let _ = reduce(&mut state, Action::AdvanceRunElapsed(1));
    let running = Presentation::from_state(&state);
    let running_cell = running
        .active()
        .iter()
        .find(|cell| cell.tool_call_id() == Some(successful_id))
        .expect("running hosted search is projected immediately");
    assert!(matches!(
        running_cell.content(),
        CellContent::Lifecycle(lifecycle)
            if lifecycle.kind() == LifecycleKind::HostedTool
                && lifecycle.action() == "Searching web"
                && lifecycle.target().is_none()
                && lifecycle.status() == LifecycleStatus::Running
                && lifecycle.tick() == 1
    ));

    let _ = reduce(
        &mut state,
        Action::Event(Box::new(common::event(
            first + 1,
            511,
            AgentEvent::HostedToolCompleted {
                tool_call_id: successful_id,
                tool_name: "web_search".to_owned(),
                arguments: serde_json::json!({"query": "tea-rs hosted search"}),
                outcome: HostedToolOutcome::Success,
                source_count: 3,
            },
        ))),
    );
    let completed = Presentation::from_state(&state);
    let completed_cell = completed
        .active()
        .iter()
        .find(|cell| cell.tool_call_id() == Some(successful_id))
        .expect("completed hosted search remains visible until the durable snapshot arrives");
    assert!(matches!(
        completed_cell.content(),
        CellContent::Lifecycle(lifecycle)
            if lifecycle.kind() == LifecycleKind::HostedTool
            && lifecycle.action() == "Searched web"
            && lifecycle.target() == Some("tea-rs hosted search")
            && lifecycle.status() == LifecycleStatus::Succeeded
            && lifecycle.details().iter().any(|detail| {
                detail.label() == Some("sources") && detail.text() == "3"
            })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn hosted_search_observations_project_failed_lifecycle() {
    let snapshot = common::archive_snapshot().await;
    let mut state = TuiState::from_snapshot(&snapshot, common::startup());
    let failed_id = "0195a0b1-7e00-7000-8000-000000000094".parse().unwrap();
    let first = state.event_cursor().get() + 1;
    for (offset, event) in [
        AgentEvent::HostedToolStarted {
            tool_call_id: failed_id,
            tool_name: "web_search".to_owned(),
        },
        AgentEvent::HostedToolCompleted {
            tool_call_id: failed_id,
            tool_name: "web_search".to_owned(),
            arguments: serde_json::json!({"query": "provider outage"}),
            outcome: HostedToolOutcome::Error(
                HostedToolError::new("provider_error", "search unavailable").unwrap(),
            ),
            source_count: 0,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let _ = reduce(
            &mut state,
            Action::Event(Box::new(common::event(
                first + u64::try_from(offset).unwrap(),
                512 + u16::try_from(offset).unwrap(),
                event,
            ))),
        );
    }
    let failed = Presentation::from_state(&state);
    let failed_cell = failed
        .active()
        .iter()
        .find(|cell| cell.tool_call_id() == Some(failed_id))
        .expect("failed hosted search remains visible");
    assert!(matches!(
        failed_cell.content(),
        CellContent::Lifecycle(lifecycle)
            if lifecycle.kind() == LifecycleKind::HostedTool
            && lifecycle.action() == "Web search failed"
            && lifecycle.status() == LifecycleStatus::Failed
            && lifecycle.details().iter().any(|detail| {
                detail.kind() == TimelineDetailKind::Error
                    && detail.label() == Some("provider_error")
                    && detail.text() == "search unavailable"
            })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn durable_hosted_search_projects_deduplicated_sources_without_continuation_state() {
    let snapshot = common::hosted_search_snapshot().await;
    let state = TuiState::from_snapshot(&snapshot, common::startup());
    let presentation = Presentation::from_state(&state);
    let hosted = presentation
        .history()
        .iter()
        .find(|cell| {
            matches!(
                cell.content(),
                CellContent::Lifecycle(lifecycle)
                    if lifecycle.kind() == LifecycleKind::HostedTool
            )
        })
        .expect("durable hosted activity is projected");
    assert!(matches!(
        hosted.content(),
        CellContent::Lifecycle(lifecycle)
            if lifecycle.action() == "Searched web"
            && lifecycle.target() == Some("tea-rs hosted search architecture")
            && lifecycle.status() == LifecycleStatus::Succeeded
            && lifecycle.details().iter().any(|detail| {
                detail.label() == Some("sources") && detail.text() == "2"
            })
    ));

    let sources = presentation
        .history()
        .iter()
        .find(|cell| matches!(cell.content(), CellContent::Sources(_)))
        .expect("normalized hosted sources are projected");
    let CellContent::Sources(sources_cell) = sources.content() else {
        panic!("hosted source cell must retain structured sources");
    };
    let projected_sources = sources_cell.sources();
    assert_eq!(
        projected_sources[0].destination(),
        Some("https://example.com/docs")
    );
    assert_eq!(
        projected_sources[1].destination(),
        Some("https://docs.example.test/provider-search")
    );
    let raw = sources.raw_text();
    assert_eq!(raw.matches("https://example.com/docs").count(), 1);
    assert_eq!(
        raw.matches("https://docs.example.test/provider-search")
            .count(),
        1
    );
    assert!(raw.contains("Hosted search architecture"));
    assert!(raw.contains("Provider search reference"));
    assert!(!raw.contains("Duplicate citation title"));
    assert!(!raw.contains("Duplicate provider reference"));

    for cell in presentation.history().iter().chain(presentation.active()) {
        let raw = cell.raw_text();
        assert!(!raw.contains("CONTINUATION_MUST_NOT_RENDER"));
        assert!(!raw.contains("CITATION_CONTINUATION_MUST_NOT_RENDER"));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn presentation_replays_persisted_code_change_without_model_result_expansion() {
    let snapshot = common::diff_snapshot().await;
    let state = TuiState::from_snapshot(&snapshot, common::startup());
    let presentation = Presentation::from_state(&state);
    let cell = presentation
        .history()
        .iter()
        .find(|cell| matches!(cell.content(), CellContent::Diff(_)))
        .expect("replayed edit presentation is projected as a typed diff");
    assert!(matches!(
        cell.content(),
        CellContent::Diff(diff)
            if diff.action() == "Updated"
                && diff.change().path() == "src/lib.rs"
                && diff.change().first_changed_line() == Some(2)
    ));
    assert!(cell.raw_text().contains("@@ -1,3 +1,3 @@"));
    assert!(!cell.raw_text().contains('\u{1b}'));
    assert!(cell.raw_text().contains("+    2␛[31m"));
}

#[tokio::test(flavor = "current_thread")]
async fn presentation_replays_normalized_web_fetch_without_raw_or_opaque_state() {
    let snapshot = common::web_fetch_snapshot().await;
    let state = TuiState::from_snapshot(&snapshot, common::startup());
    let presentation = Presentation::from_state(&state);
    let cell = presentation
        .history()
        .iter()
        .find(|cell| {
            matches!(
                cell.content(),
                CellContent::Result(result) if result.action() == "Fetched"
            )
        })
        .expect("replayed web fetch presentation is projected as a typed result");
    let CellContent::Result(result) = cell.content() else {
        panic!("web fetch must use the bounded result surface");
    };
    assert_eq!(result.source_name(), Some("web_fetch"));
    assert_eq!(result.format(), tea_cli::tui::OutputFormat::Plain);
    assert!(!result.is_error());
    let content = result.content();
    assert!(content.contains("URL: https://example.com/final"));
    assert!(content.contains("Content-Type: text/html; charset=utf-8"));
    assert!(content.contains("Title: A fetched page"));
    assert!(content.contains("Truncated: body characters"));
    assert!(content.contains("Redirects: 1"));
    assert!(content.contains("Normalized body"));
    assert!(!content.contains('\u{1b}'));
    assert!(!content.contains("MODEL_RESULT_MUST_NOT_RENDER"));

    let raw = cell.raw_text();
    assert!(raw.contains("https://example.com/final"));
    assert!(!raw.contains("continuation"));
    assert!(!raw.contains("providerCallId"));
    assert!(!raw.contains('\u{1b}'));
}

#[tokio::test(flavor = "current_thread")]
async fn approval_keeps_an_ephemeral_preview_across_snapshot_reload() {
    let snapshot = common::pending_snapshot().await;
    let mut state = TuiState::from_snapshot(&snapshot, common::startup());
    let tool_call_id = state.approval().unwrap().tool_call_id;
    let change = CodeChange::new(
        "notes.txt",
        CodeChangeKind::Update,
        Vec::new(),
        false,
        None,
        None,
        None,
    )
    .unwrap();
    let next = state.event_cursor().get() + 1;
    let _ = reduce(
        &mut state,
        Action::Event(Box::new(common::event(
            next,
            480,
            AgentEvent::ToolExecutionPreview {
                tool_call_id,
                presentation: ToolPresentation::CodeChange(change.clone()),
            },
        ))),
    );
    let _ = reduce(&mut state, Action::SnapshotLoaded(Box::new(snapshot)));

    let projected = Presentation::from_state(&state);
    assert!(projected.active().iter().any(|cell| {
        matches!(
            cell.content(),
            CellContent::Diff(diff)
                if diff.action() == "Preview" && diff.change() == &change
        )
    }));
    assert!(projected.history().iter().all(|cell| {
        !matches!(cell.content(), CellContent::Diff(diff) if diff.action() == "Preview")
    }));
}

fn has_notice(presentation: &Presentation, expected: &str) -> bool {
    presentation.notifications().iter().any(|cell| {
        matches!(
            cell.content(),
            CellContent::Notice(notice)
                if notice.kind() == NoticeKind::General && notice.message() == expected
        )
    })
}

#[tokio::test(flavor = "current_thread")]
async fn presentation_keeps_live_status_out_of_the_transcript_and_preserves_tool_progress() {
    let snapshot = common::archive_snapshot().await;
    let mut state = TuiState::from_snapshot(&snapshot, common::startup());
    let next = state.event_cursor().get() + 1;
    let _ = reduce(
        &mut state,
        Action::Event(Box::new(common::event(
            next,
            451,
            AgentEvent::RunStarted {},
        ))),
    );

    let waiting = Presentation::from_state(&state);
    assert!(waiting.active().iter().all(|cell| !matches!(
        cell.content(),
        CellContent::Lifecycle(lifecycle)
            if lifecycle.kind() == LifecycleKind::RunActivity
    )));

    let _ = reduce(&mut state, Action::AdvanceRunElapsed(1));
    let _ = reduce(&mut state, Action::AdvanceRunElapsed(1));
    let _ = reduce(
        &mut state,
        Action::Event(Box::new(common::event(
            next + 1,
            452,
            AgentEvent::MessageDelta {
                message_id: common::message_id(),
                content_index: 0,
                delta: EventDelta::ThinkingDelta {
                    text: "checking the workspace".to_owned(),
                },
            },
        ))),
    );
    let thinking = Presentation::from_state(&state);
    assert!(thinking.active().iter().all(|cell| !matches!(
        cell.content(),
        CellContent::Lifecycle(lifecycle)
            if lifecycle.kind() == LifecycleKind::RunActivity
    )));

    let _ = reduce(
        &mut state,
        Action::Event(Box::new(common::event(
            next + 2,
            453,
            AgentEvent::ToolCallRequested {
                tool_call_id: common::tool_call_id(),
                tool_name: "index_workspace".to_owned(),
                arguments: serde_json::json!({"path": "."}),
            },
        ))),
    );
    let _ = reduce(
        &mut state,
        Action::Event(Box::new(common::event(
            next + 3,
            454,
            AgentEvent::ToolExecutionProgress {
                tool_call_id: common::tool_call_id(),
                message: "2 files indexed".to_owned(),
                completed_units: 2,
                total_units: Some(5),
            },
        ))),
    );
    let running = Presentation::from_state(&state);
    assert!(running.active().iter().all(|cell| !matches!(
        cell.content(),
        CellContent::Lifecycle(lifecycle)
            if lifecycle.kind() == LifecycleKind::RunActivity
    )));
    assert_eq!(
        state.tools()[&common::tool_call_id()]
            .progress
            .as_ref()
            .map(|progress| progress.message.as_str()),
        Some("2 files indexed"),
        "the status surface consumes actual retained tool progress"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn tool_cells_are_compact_until_explicitly_expanded() {
    let snapshot = common::archive_snapshot().await;
    let mut state = TuiState::from_snapshot(&snapshot, common::startup());
    let tool_call_id = *state.tools().keys().next().expect("fixture has a tool");

    let compact = Presentation::from_state(&state);
    let compact_tool = compact
        .history()
        .iter()
        .find(|cell| cell.tool_call_id() == Some(tool_call_id))
        .expect("tool cell is projected");
    assert!(matches!(
        compact_tool.content(),
        CellContent::Lifecycle(lifecycle)
            if lifecycle.kind() == LifecycleKind::ToolCall
                && lifecycle.target().is_some_and(|target| {
                    target.contains("write_text_file")
                        && target.contains("/workspace/notes.txt")
                })
                && !lifecycle.expanded()
                && lifecycle
                    .details()
                    .iter()
                    .any(|detail| detail.label() == Some("arguments"))
    ));
    assert!(compact_tool.raw_text().contains("arguments"));

    let _ = reduce(&mut state, Action::ToggleTool(tool_call_id));
    let expanded = Presentation::from_state(&state);
    let expanded_tool = expanded
        .history()
        .iter()
        .find(|cell| cell.tool_call_id() == Some(tool_call_id))
        .expect("tool cell is projected");
    assert!(matches!(
        expanded_tool.content(),
        CellContent::Lifecycle(lifecycle)
            if lifecycle.expanded()
                && lifecycle.details().iter().any(|detail| {
                detail.kind() == TimelineDetailKind::Metadata
                    && detail.label() == Some("arguments")
            })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn tool_lifecycle_replaces_active_state_without_duplicate_calls() {
    let snapshot = common::pending_snapshot().await;
    let mut state = TuiState::from_snapshot(&snapshot, common::startup());
    let tool_call_id = *state.tools().keys().next().expect("fixture has a tool");
    let pending = Presentation::from_state(&state);
    let pending_calls = pending
        .history()
        .iter()
        .chain(pending.active())
        .filter(|cell| {
            cell.tool_call_id() == Some(tool_call_id)
                && matches!(
                    cell.content(),
                    CellContent::Lifecycle(lifecycle)
                        if lifecycle.kind() == LifecycleKind::ToolCall
                )
        })
        .collect::<Vec<_>>();
    assert_eq!(pending_calls.len(), 1);
    assert!(matches!(
        pending_calls[0].content(),
        CellContent::Lifecycle(lifecycle)
            if lifecycle.status() == LifecycleStatus::ApprovalPending
    ));

    let completed_snapshot = common::archive_snapshot().await;
    let _ = reduce(
        &mut state,
        Action::SnapshotLoaded(Box::new(completed_snapshot)),
    );
    let completed = Presentation::from_state(&state);
    let completed_calls = completed
        .history()
        .iter()
        .chain(completed.active())
        .filter(|cell| {
            cell.tool_call_id() == Some(tool_call_id)
                && matches!(
                    cell.content(),
                    CellContent::Lifecycle(lifecycle)
                        if lifecycle.kind() == LifecycleKind::ToolCall
                )
        })
        .collect::<Vec<_>>();
    assert_eq!(completed_calls.len(), 1);
    assert!(matches!(
        completed_calls[0].content(),
        CellContent::Lifecycle(lifecycle)
            if lifecycle.status() == LifecycleStatus::Succeeded
    ));
    assert_eq!(
        completed
            .history()
            .iter()
            .filter(|cell| {
                cell.tool_call_id() == Some(tool_call_id)
                    && matches!(cell.content(), CellContent::Result(_))
            })
            .count(),
        1
    );
    assert!(completed.active().iter().all(|cell| {
        cell.tool_call_id() != Some(tool_call_id)
            || !matches!(
                cell.content(),
                CellContent::Lifecycle(lifecycle)
                    if lifecycle.kind() == LifecycleKind::ToolCall
            )
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn durable_approval_decision_appears_once_between_call_and_result() {
    let snapshot = common::pending_snapshot().await;
    let mut state = TuiState::from_snapshot(&snapshot, common::startup());
    let tool_call_id = *state.tools().keys().next().expect("fixture has a tool");
    let pending = Presentation::from_state(&state);

    assert!(pending.history().iter().all(|cell| {
        cell.tool_call_id() != Some(tool_call_id)
            || !matches!(
                cell.content(),
                CellContent::Decision(decision)
                    if matches!(
                        decision.status(),
                        DecisionStatus::Approved | DecisionStatus::Denied
                    )
            )
    }));
    assert_eq!(
        pending
            .active()
            .iter()
            .filter(|cell| {
                cell.tool_call_id() == Some(tool_call_id)
                    && matches!(
                        cell.content(),
                        CellContent::Decision(decision)
                            if matches!(
                                decision.status(),
                                DecisionStatus::Pending | DecisionStatus::Submitting
                            )
                    )
            })
            .count(),
        1
    );

    let completed_snapshot = common::archive_snapshot().await;
    let _ = reduce(
        &mut state,
        Action::SnapshotLoaded(Box::new(completed_snapshot)),
    );
    let completed = Presentation::from_state(&state);
    let decisions = completed
        .history()
        .iter()
        .enumerate()
        .filter(|(_, cell)| {
            cell.tool_call_id() == Some(tool_call_id)
                && matches!(
                    cell.content(),
                    CellContent::Decision(decision)
                        if matches!(
                            decision.status(),
                            DecisionStatus::Approved | DecisionStatus::Denied
                        )
                )
        })
        .collect::<Vec<_>>();
    assert_eq!(decisions.len(), 1);
    let (decision_index, decision) = decisions[0];
    assert!(matches!(
        decision.content(),
        CellContent::Decision(decision)
            if decision.action() == "Approved"
            && decision.subject() == "write_text_file"
            && decision.status() == DecisionStatus::Approved
            && decision.details().iter().any(|detail| {
                detail.label() == Some("scope") && detail.text() == "session"
            })
    ));
    assert!(!decision.raw_text().contains("arguments"));
    assert!(!decision.raw_text().contains("/workspace/notes.txt"));

    let call_index = completed
        .history()
        .iter()
        .position(|cell| {
            cell.tool_call_id() == Some(tool_call_id)
                && matches!(
                    cell.content(),
                    CellContent::Lifecycle(lifecycle)
                        if lifecycle.kind() == LifecycleKind::ToolCall
                )
        })
        .expect("durable tool call is present");
    let result_index = completed
        .history()
        .iter()
        .position(|cell| {
            cell.tool_call_id() == Some(tool_call_id)
                && matches!(cell.content(), CellContent::Result(_))
        })
        .expect("durable tool result is present");
    assert!(call_index < decision_index && decision_index < result_index);
    assert!(completed.active().iter().all(|cell| {
        cell.tool_call_id() != Some(tool_call_id)
            || !matches!(cell.content(), CellContent::Decision(_))
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn queued_inputs_are_typed_bounded_active_only_and_projection_does_not_consume_them() {
    let snapshot = common::archive_snapshot().await;
    let mut state = TuiState::from_snapshot(&snapshot, common::startup());
    let steering = format!("steer\u{1b}[31m\n{}", "宽".repeat(120));
    let _ = reduce(&mut state, Action::QueueSteering(steering));
    let _ = reduce(
        &mut state,
        Action::QueueFollowUp("verify the follow-up path".to_owned()),
    );

    let projected = Presentation::from_state(&state);
    let steering = projected
        .active()
        .iter()
        .find(|cell| {
            matches!(
                cell.content(),
                CellContent::QueuedInput(input) if input.kind() == QueuedInputKind::Steering
            )
        })
        .expect("steering queue row is active");
    assert!(matches!(
        steering.content(),
        CellContent::QueuedInput(input)
            if input.kind() == QueuedInputKind::Steering
                && input.preview().ends_with("...")
                && input.preview().width() <= 160
                && !input.preview().chars().any(char::is_control)
    ));
    let follow_up = projected
        .active()
        .iter()
        .find(|cell| {
            matches!(
                cell.content(),
                CellContent::QueuedInput(input) if input.kind() == QueuedInputKind::FollowUp
            )
        })
        .expect("follow-up queue row is active");
    assert!(matches!(
        follow_up.content(),
        CellContent::QueuedInput(input)
            if input.kind() == QueuedInputKind::FollowUp
                && input.preview() == "verify the follow-up path"
    ));
    assert!(
        projected
            .history()
            .iter()
            .all(|cell| !matches!(cell.content(), CellContent::QueuedInput(_)))
    );

    assert_eq!(projected, Presentation::from_state(&state));
}

#[tokio::test(flavor = "current_thread")]
async fn typed_ids_distinguish_message_blocks_tool_facets_and_assistant_sources() {
    let image_snapshot = common::image_snapshot().await;
    let image_state = TuiState::from_snapshot(&image_snapshot, common::startup());
    let image_message_id: MessageId = "0195a0b1-5e3d-7bb4-863a-0aa7aa000003".parse().unwrap();
    let image_block_ids = Presentation::from_state(&image_state)
        .history()
        .iter()
        .filter_map(|cell| match cell.id() {
            CellId::Message {
                message_id,
                block_index,
                facet: MessageCellFacet::Content,
            } if message_id == image_message_id => Some(block_index),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(image_block_ids, [0, 1, 2]);

    let archive_snapshot = common::archive_snapshot().await;
    let archive_state = TuiState::from_snapshot(&archive_snapshot, common::startup());
    let tool_call_id: ToolCallId = "0195a0b1-5e45-75be-8284-0aa7aa000011".parse().unwrap();
    let tool_facets = Presentation::from_state(&archive_state)
        .history()
        .iter()
        .filter_map(|cell| match cell.id() {
            CellId::Tool {
                tool_call_id: id,
                facet,
                ..
            } if id == tool_call_id => Some(facet),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    assert!(tool_facets.contains(&ToolCellFacet::Call));
    assert!(tool_facets.contains(&ToolCellFacet::Decision));
    assert!(tool_facets.contains(&ToolCellFacet::Result));

    let hosted_snapshot = common::hosted_search_snapshot().await;
    let hosted_state = TuiState::from_snapshot(&hosted_snapshot, common::startup());
    let assistant_message_id: MessageId = "0195a0b1-5e64-76d6-9a5a-0aa7aa000042".parse().unwrap();
    assert!(
        Presentation::from_state(&hosted_state)
            .history()
            .iter()
            .any(|cell| matches!(
                cell.id(),
                CellId::Message {
                    message_id,
                    facet: MessageCellFacet::Sources,
                    ..
                } if message_id == assistant_message_id
            ))
    );
}

#[tokio::test(flavor = "current_thread")]
#[allow(clippy::too_many_lines)]
async fn active_replacement_and_local_lane_ids_are_stable_ordered_and_unique() {
    let snapshot = common::archive_snapshot().await;
    let mut state = TuiState::from_snapshot(&snapshot, common::startup());
    let message_id = common::message_id();
    let next = state.event_cursor().get() + 1;
    let _ = reduce(
        &mut state,
        Action::Event(Box::new(common::event(
            next,
            560,
            AgentEvent::RunStarted {},
        ))),
    );
    let _ = reduce(
        &mut state,
        Action::Event(Box::new(common::event(
            next + 1,
            561,
            AgentEvent::MessageDelta {
                message_id,
                content_index: 0,
                delta: EventDelta::TextDelta {
                    text: "first".to_owned(),
                },
            },
        ))),
    );
    let _ = reduce(&mut state, Action::QueueSteering("steer one".to_owned()));
    let _ = reduce(&mut state, Action::QueueSteering("steer two".to_owned()));
    let _ = reduce(&mut state, Action::QueueFollowUp("follow up".to_owned()));
    let _ = reduce(&mut state, Action::Notify("notice one".to_owned()));
    let _ = reduce(&mut state, Action::Notify("notice two".to_owned()));

    let first = Presentation::from_state(&state);
    let first_stream_id = first
        .active()
        .iter()
        .find(|cell| {
            matches!(
                cell.id(),
                CellId::Stream {
                    message_id: id,
                    block_index: 0,
                    facet: StreamCellFacet::Message,
                } if id == message_id
            )
        })
        .expect("streaming message is projected")
        .id();

    let _ = reduce(
        &mut state,
        Action::Event(Box::new(common::event(
            next + 2,
            562,
            AgentEvent::MessageDelta {
                message_id,
                content_index: 0,
                delta: EventDelta::TextDelta {
                    text: " second".to_owned(),
                },
            },
        ))),
    );
    let second = Presentation::from_state(&state);
    let replaced = second
        .active()
        .iter()
        .find(|cell| cell.id() == first_stream_id)
        .expect("stream replacement retains stable identity");
    assert!(matches!(
        replaced.content(),
        CellContent::Message(message) if message.source() == "first second"
    ));

    let queue_ids = second
        .active()
        .iter()
        .filter(|cell| matches!(cell.content(), CellContent::QueuedInput(_)))
        .map(tea_cli::tui::CellNode::id)
        .collect::<Vec<_>>();
    assert_eq!(
        queue_ids,
        [
            CellId::Queue {
                kind: QueuedInputKind::Steering,
                index: 0,
            },
            CellId::Queue {
                kind: QueuedInputKind::Steering,
                index: 1,
            },
            CellId::Queue {
                kind: QueuedInputKind::FollowUp,
                index: 0,
            },
        ]
    );
    assert_eq!(
        second
            .notifications()
            .iter()
            .map(tea_cli::tui::CellNode::id)
            .collect::<Vec<_>>(),
        [
            CellId::Notification { index: 0 },
            CellId::Notification { index: 1 },
        ]
    );

    let all_cells = second
        .history()
        .iter()
        .chain(second.active())
        .chain(second.notifications())
        .collect::<Vec<_>>();
    let ids = all_cells
        .iter()
        .map(|cell| cell.id())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), all_cells.len());
}

#[tokio::test(flavor = "current_thread")]
async fn mcp_named_tools_keep_the_generic_protocol_backed_lifecycle() {
    let snapshot = common::archive_snapshot().await;
    let mut state = TuiState::from_snapshot(&snapshot, common::startup());
    let tool_call_id = "0195a0b1-7e00-7000-8000-000000000099".parse().unwrap();
    let next = state.event_cursor().get() + 1;
    let _ = reduce(
        &mut state,
        Action::Event(Box::new(common::event(
            next,
            455,
            AgentEvent::ToolCallRequested {
                tool_call_id,
                tool_name: "docs.search".to_owned(),
                arguments: serde_json::json!({"query": "ratatui rendering"}),
            },
        ))),
    );

    let projected = Presentation::from_state(&state);
    let tool = projected
        .active()
        .iter()
        .find(|cell| cell.tool_call_id() == Some(tool_call_id))
        .expect("MCP-backed tool remains visible through the generic lifecycle");
    assert!(matches!(
        tool.content(),
        CellContent::Lifecycle(lifecycle)
            if lifecycle.kind() == LifecycleKind::ToolCall
                && lifecycle.action() == "Requested"
                && lifecycle.target() == Some("docs.search ratatui rendering")
                && lifecycle.status() == LifecycleStatus::Requested
                && lifecycle
                    .details()
                    .iter()
                    .any(|detail| detail.label() == Some("arguments"))
    ));
}
