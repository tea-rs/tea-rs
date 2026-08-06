use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tea_cli::tui::{Action, Renderer, Theme, TuiState, reduce};
use tea_protocol::{AgentEvent, EventDelta};

use crate::common;

fn render_rows(
    renderer: &mut Renderer,
    state: &TuiState,
    area: Rect,
    theme: &Theme,
) -> Vec<String> {
    let mut buffer = Buffer::empty(area);
    renderer.render(state, area, &mut buffer, theme);
    buffer
        .content
        .chunks(usize::from(area.width))
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        })
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn transcript_scroll_preserves_browsing_position_and_marks_new_output() {
    let snapshot = common::archive_snapshot().await;
    let mut state = TuiState::from_snapshot(&snapshot, common::startup());
    let _ = reduce(&mut state, Action::ScrollTranscriptUp { rows: 3 });
    assert!(!state.transcript_viewport().follows_tail());
    assert_eq!(state.transcript_viewport().offset_from_tail_rows(), 3);

    let next = state.event_cursor().get() + 1;
    let _ = reduce(
        &mut state,
        Action::Event(Box::new(common::event(
            next,
            610,
            AgentEvent::RunStarted {},
        ))),
    );
    let _ = reduce(
        &mut state,
        Action::Event(Box::new(common::event(
            next + 1,
            611,
            AgentEvent::MessageDelta {
                message_id: common::message_id(),
                content_index: 0,
                delta: EventDelta::TextDelta {
                    text: "new streamed output".to_owned(),
                },
            },
        ))),
    );
    assert_eq!(state.transcript_viewport().unread_items(), 1);
    assert_eq!(state.transcript_viewport().offset_from_tail_rows(), 4);

    let area = Rect::new(0, 0, 80, 12);
    let mut renderer = Renderer::new();
    let rows = render_rows(&mut renderer, &state, area, &Theme::default());
    let frame = rows.join("\n");
    assert!(frame.contains("new output (1)"), "frame: {frame:?}");
    assert!(frame.contains("› Ask Tea to do anything"));
    assert!(
        rows.iter()
            .all(|row| !row.ends_with('│') && !row.ends_with('█'))
    );

    let _ = reduce(
        &mut state,
        Action::Resize {
            width: 40,
            height: 16,
        },
    );
    assert_eq!(state.transcript_viewport().unread_items(), 1);
    assert!(!state.transcript_viewport().follows_tail());
    let resized_rows = render_rows(
        &mut renderer,
        &state,
        Rect::new(0, 0, 40, 16),
        &Theme::default(),
    );
    let resized_frame = resized_rows.join("\n");
    assert!(
        resized_frame.contains("new output (1)"),
        "resized frame: {resized_frame:?}"
    );
    assert_eq!(state.transcript_viewport().offset_from_tail_rows(), 4);
    assert_eq!(state.transcript_viewport().unread_items(), 1);
    assert!(!state.transcript_viewport().follows_tail());

    let _ = reduce(&mut state, Action::FollowTranscriptTail);
    assert!(state.transcript_viewport().follows_tail());
    assert_eq!(state.transcript_viewport().offset_from_tail_rows(), 0);
    assert_eq!(state.transcript_viewport().unread_items(), 0);
}
