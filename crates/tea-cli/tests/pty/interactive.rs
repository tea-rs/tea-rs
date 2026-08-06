use super::PtyHarness;
use std::time::Duration;

const MOUSE_ENABLE: &[u8] = b"\x1b[?1000h";
const MOUSE_DISABLE: &[u8] = b"\x1b[?1006l";
const EVENT_SEQUENCE_GAP: &[u8] = b"event sequence gap detected";
const OSC8_PREFIX: &[u8] = b"\x1b]8;";
const OSC8_DOCS_OPEN: &[u8] = b"\x1b]8;;https://e.test/\x1b\\";
const OSC8_CLOSE: &[u8] = b"\x1b]8;;\x1b\\";
const TRUECOLOR_FOREGROUND: &[u8] = b"\x1b[38;2;";
const ANSI256_FOREGROUND: &[u8] = b"\x1b[38;5;";
const KITTY_IMAGE_PREFIX: &[u8] = b"\x1b_G";
const SIXEL_IMAGE_PREFIX: &[u8] = b"\x1bPq";
const ITERM_IMAGE_PREFIX: &[u8] = b"\x1b]1337;File=";

fn assert_no_event_sequence_gap(harness: &PtyHarness) {
    assert!(
        !harness
            .raw()
            .windows(EVENT_SEQUENCE_GAP.len())
            .any(|bytes| bytes == EVENT_SEQUENCE_GAP),
        "normal execution must not report event loss: {:?}",
        harness.screen()
    );
}

fn submit_unicode_prompt(harness: &mut PtyHarness) {
    harness.wait_for_raw(b"\x1b[?2004h");
    harness.send(b"\x1b[I");
    harness.paste("请读取 mixed-width e\u{301}");
    harness.send(b"\x1b[O");
    harness.send(b"\r");
}

#[test]
fn workspace_trust_acceptance_is_persisted_before_main_tui() {
    let mut harness = PtyHarness::spawn("trust-reopen");
    harness.wait_for_screen("workspace trust required");
    let prompt = harness.screen();
    assert!(
        prompt.contains("Yes, trust this folder"),
        "screen: {prompt:?}"
    );
    assert!(prompt.contains("No, exit"), "screen: {prompt:?}");
    assert!(
        prompt.contains("read, edit, and execute files"),
        "screen: {prompt:?}"
    );

    harness.send(b"\x1b[A");
    harness.send(b"\r");
    harness.wait_for_raw_occurrences(b"\x1b[>1u", 2);
    harness.send(&[0x04]);
    harness.wait_for_raw(b"TEA_PTY_TRUST_REOPEN");
    harness.wait_for_raw_occurrences(b"\x1b[>1u", 3);
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
}

#[test]
fn workspace_trust_rejection_exits_before_main_tui() {
    let mut harness = PtyHarness::spawn("trust-reject");
    harness.wait_for_screen("workspace trust required");
    harness.send(b"\r");
    harness.wait_for_raw(b"TEA_PTY_TRUST_REJECTED");
    assert!(harness.wait_for_exit().success());
    assert_eq!(harness.raw_occurrences(b"\x1b[>1u"), 1);
}

#[test]
fn shift_enter_inserts_a_newline_on_a_real_pty() {
    let mut harness = PtyHarness::spawn("fullscreen");
    harness.wait_for_raw(b"\x1b[>1u");
    harness.send(b"first line");
    harness.send(b"\x1b[13;2u");
    harness.send(b"second line");
    harness.wait_for_screen("second line");

    let screen = harness.screen();
    let lines = screen.lines().collect::<Vec<_>>();
    let first = lines
        .iter()
        .position(|line| line.contains("first line"))
        .unwrap();
    let second = lines
        .iter()
        .position(|line| line.contains("second line"))
        .unwrap();
    assert_eq!(second, first + 1, "screen: {screen:?}");
    assert!(!screen.contains("PTY final"), "Shift+Enter submitted input");

    harness.send(&[0x0c]);
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
}

#[test]
fn codex_style_editor_shortcuts_work_on_a_real_pty() {
    let mut harness = PtyHarness::spawn("fullscreen");
    harness.wait_for_raw(b"\x1b[>1u");
    harness.send(b"alpha beta");
    harness.send(&[0x01]); // Ctrl+A
    harness.send(b"x");
    harness.send(&[0x05]); // Ctrl+E
    harness.send(b"z");
    harness.send(&[0x15]); // Ctrl+U
    harness.send(&[0x19]); // Ctrl+Y
    harness.send(&[0x01]); // Ctrl+A
    harness.send(b"\x1bf"); // Alt+F
    harness.send(b"|");
    harness.send(&[0x04]); // Ctrl+D deletes instead of exiting while input is non-empty.
    harness.send(&[0x0b]); // Ctrl+K
    harness.send(&[0x19]); // Ctrl+Y
    harness.wait_for_screen("xalpha|betaz");
    assert!(!harness.screen().contains("PTY final"));

    harness.send(&[0x15]);
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
}

#[test]
fn fullscreen_streaming_unicode_paste_focus_and_escape_render_on_a_real_pty() {
    let mut harness = PtyHarness::spawn("fullscreen");
    harness.wait_for_raw(b"\x1b[?2004h");
    harness.resize(13, 100);
    submit_unicode_prompt(&mut harness);
    harness.wait_for_screen("PTY final");
    harness.settle(Duration::from_millis(150));
    let screen = harness.screen();
    assert!(
        screen.contains("PTY final: 你好 uses e\u{301} and styled text"),
        "screen: {screen:?}"
    );
    assert!(
        screen.contains("fn main() { println!(\"wide response\"); }"),
        "screen: {screen:?}"
    );
    assert!(
        screen.contains("docs (https://e.test/)"),
        "screen: {screen:?}"
    );
    assert!(
        harness
            .raw()
            .windows(OSC8_DOCS_OPEN.len())
            .any(|bytes| bytes == OSC8_DOCS_OPEN),
        "guarded OSC8 destination was not emitted"
    );
    for leaked_label in ["you:", "assistant:", "rust:", "**", "`"] {
        assert!(
            !screen.contains(leaked_label),
            "conversation/Markdown label {leaked_label:?} leaked: {screen:?}"
        );
    }
    assert!(
        screen
            .lines()
            .all(|row| !row.ends_with('│') && !row.ends_with('█')),
        "the main viewport must not draw an application scrollbar: {screen:?}"
    );
    assert!(
        !harness
            .raw()
            .windows(MOUSE_ENABLE.len())
            .any(|bytes| bytes == MOUSE_ENABLE)
    );
    assert!(
        harness.frame_count() <= 32,
        "render storm: {} frames",
        harness.frame_count()
    );
    assert_no_event_sequence_gap(&harness);
    harness.send(b"\x1b");
    harness.settle(Duration::from_millis(50));
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
    assert!(harness.raw_occurrences(OSC8_CLOSE) >= harness.raw_occurrences(OSC8_DOCS_OPEN));
    assert!(
        !harness
            .raw()
            .windows(MOUSE_DISABLE.len())
            .any(|bytes| bytes == MOUSE_DISABLE)
    );
}

#[test]
fn dumb_terminal_keeps_fenced_code_plain_and_copyable() {
    let mut harness = PtyHarness::spawn_with_term("fullscreen", "dumb");
    submit_unicode_prompt(&mut harness);
    harness.wait_for_screen("fn main() { println!(\"wide response\"); }");
    harness.settle(Duration::from_millis(100));

    for color_prefix in [TRUECOLOR_FOREGROUND, ANSI256_FOREGROUND] {
        assert!(
            !harness
                .raw()
                .windows(color_prefix.len())
                .any(|bytes| bytes == color_prefix),
            "no-color terminal received a color sequence: {color_prefix:?}"
        );
    }
    assert_eq!(harness.raw_occurrences(OSC8_PREFIX), 0);
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
}

#[test]
fn fullscreen_table_uses_the_structured_grid_on_a_real_pty() {
    let mut harness = PtyHarness::spawn("fullscreen");
    harness.resize(20, 100);
    submit_unicode_prompt(&mut harness);
    harness.wait_for_screen("Safe");
    harness.settle(Duration::from_millis(100));
    let screen = harness.screen();

    for expected in ["Name", "Status", "Tea", "Ready", "Stable", "Safe"] {
        assert!(screen.contains(expected), "screen: {screen:?}");
    }
    assert!(screen.contains('━'), "screen: {screen:?}");
    assert!(!screen.contains("|---|"), "screen: {screen:?}");
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
}

#[test]
fn inline_mode_preserves_native_screen_and_never_uses_the_alternate_screen() {
    let mut harness = PtyHarness::spawn("inline");
    harness.wait_for_raw(b"\x1b[?2004h");
    harness.resize(8, 100);
    submit_unicode_prompt(&mut harness);
    harness.wait_for_screen("PTY final");
    harness.wait_for_screen("Worked for");
    harness.settle(Duration::from_millis(100));
    let live = harness.screen();
    assert!(
        harness
            .raw()
            .windows(b"\x1b[1;".len())
            .any(|bytes| bytes == b"\x1b[1;"),
        "Codex-style history insertion did not establish a top-anchored scroll region"
    );
    assert!(
        live.lines()
            .all(|row| !row.ends_with('│') && !row.ends_with('█')),
        "inline mode must not draw an application scrollbar: {live:?}"
    );
    assert_eq!(harness.cursor_query_count(), 1);
    assert!(
        !harness
            .raw()
            .windows(8)
            .any(|bytes| bytes == b"\x1b[?1049h")
    );
    assert!(
        !harness
            .raw()
            .windows(MOUSE_ENABLE.len())
            .any(|bytes| bytes == MOUSE_ENABLE)
    );
    assert!(
        harness.frame_count() <= 16,
        "inline render storm: {} frames",
        harness.frame_count()
    );
    assert_no_event_sequence_gap(&harness);
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
    assert!(
        !harness
            .raw()
            .windows(MOUSE_DISABLE.len())
            .any(|bytes| bytes == MOUSE_DISABLE)
    );
}

#[test]
fn inline_mode_keeps_screen_compatible_term_on_the_normal_buffer() {
    let mut harness = PtyHarness::spawn_with_term("inline", "screen-256color");
    submit_unicode_prompt(&mut harness);
    harness.wait_for_screen("PTY final");
    assert!(
        harness
            .raw()
            .windows(b"\x1b[1;".len())
            .any(|bytes| bytes == b"\x1b[1;")
    );
    assert!(
        !harness
            .raw()
            .windows(b"\x1b[?1049h".len())
            .any(|bytes| bytes == b"\x1b[?1049h")
    );
    assert!(
        !harness
            .raw()
            .windows(MOUSE_ENABLE.len())
            .any(|bytes| bytes == MOUSE_ENABLE)
    );
    assert_eq!(harness.raw_occurrences(OSC8_PREFIX), 0);
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
}

#[test]
fn inline_history_emits_balanced_guarded_hyperlinks() {
    let mut harness = PtyHarness::spawn("inline");
    submit_unicode_prompt(&mut harness);
    harness.wait_for_screen("PTY final");
    harness.settle(Duration::from_millis(100));
    assert!(harness.raw_occurrences(OSC8_DOCS_OPEN) >= 1);

    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
    assert!(harness.raw_occurrences(OSC8_CLOSE) >= harness.raw_occurrences(OSC8_DOCS_OPEN));
}

#[test]
fn tmux_terminal_keeps_plain_text_link_fallback_without_osc8() {
    let mut harness = PtyHarness::spawn_with_term("fullscreen", "tmux-256color");
    submit_unicode_prompt(&mut harness);
    harness.wait_for_screen("docs (https://e.test/)");
    harness.settle(Duration::from_millis(100));
    assert_eq!(harness.raw_occurrences(OSC8_PREFIX), 0);

    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
}

#[test]
fn inline_first_frame_clears_visible_terminal_residue_without_purging_scrollback() {
    let mut harness = PtyHarness::spawn("inline-stale");
    harness.wait_for_screen("Ask Tea to do anything");
    let screen = harness.screen();
    assert!(
        !screen.contains("STALE_TERMINAL_CONTENT"),
        "stale terminal content leaked into the first frame: {screen:?}"
    );
    assert!(
        harness
            .raw()
            .windows(b"\x1b[2J".len())
            .any(|bytes| bytes == b"\x1b[2J")
    );
    assert!(
        !harness
            .raw()
            .windows(b"\x1b[3J".len())
            .any(|bytes| bytes == b"\x1b[3J"),
        "startup must preserve terminal-owned scrollback"
    );
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
}

#[test]
fn inline_quit_clears_the_composer_before_returning_to_the_shell() {
    let mut harness = PtyHarness::spawn("inline-quit");
    harness.wait_for_screen("Ask Tea to do anything");
    let raw_before_quit = harness.raw().len();
    harness.paste("/quit");
    harness.send(b"\r");
    harness.wait_for_screen("TEA_SHELL_PROMPT");

    let screen = harness.screen();
    let shell_row = screen
        .lines()
        .find(|row| row.contains("TEA_SHELL_PROMPT"))
        .expect("shell marker must be visible");
    assert_eq!(shell_row.trim(), "TEA_SHELL_PROMPT", "screen: {screen:?}");
    assert!(
        harness.raw()[raw_before_quit..]
            .windows(b"\x1b[J".len())
            .any(|bytes| bytes == b"\x1b[J"),
        "inline exit must clear the composer viewport before the shell resumes"
    );
    assert!(harness.wait_for_exit().success());
}

#[test]
fn run_activity_can_be_cancelled_without_waiting_for_wall_clock() {
    let mut harness = PtyHarness::spawn("inline-slow");
    harness.wait_for_screen("Ask Tea to do anything");
    harness.paste("wait for the timer");
    harness.send(b"\r");
    harness.wait_for_screen("Working (");
    assert!(harness.screen().contains("Working (0s"));
    harness.send(b"\x1b");
    harness.wait_for_screen("operation was cancelled");
    harness.paste("/quit");
    harness.send(b"\r");
    assert!(harness.wait_for_exit().success());
}

#[test]
fn inline_read_tool_flow_never_reports_an_event_sequence_gap() {
    let mut harness = PtyHarness::spawn("inline-read");
    harness.wait_for_raw(b"\x1b[?2004h");
    harness.send(b"\x1b[I");
    harness.paste("Read README.md and summarize it");
    harness.send(b"\r");
    harness.wait_for_screen("README summary complete");
    harness.wait_for_screen("Worked for");
    harness.settle(Duration::from_millis(150));

    assert_no_event_sequence_gap(&harness);
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
}

#[test]
#[ignore = "requires live OpenAI-compatible credentials from .env"]
fn live_inline_read_tool_flow_never_reports_an_event_sequence_gap() {
    let mut harness = PtyHarness::spawn_live("inline-live-read");
    harness.wait_for_raw(b"\x1b[?2004h");
    harness.send(b"\x1b[I");
    harness.paste(
        "Use the read tool to inspect README.md. Then reply only with the words TEA LIVE READ DONE joined by underscores.",
    );
    harness.send(b"\r");
    harness.wait_for_screen_timeout("TEA_LIVE_READ_DONE", Duration::from_mins(2));
    harness.wait_for_screen_timeout("Worked for", Duration::from_mins(2));
    harness.settle(Duration::from_secs(3));

    assert_no_event_sequence_gap(&harness);
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
}

#[test]
fn durable_quit_and_reopen_restores_the_transcript() {
    let mut harness = PtyHarness::spawn("reopen");
    submit_unicode_prompt(&mut harness);
    harness.wait_for_screen("PTY final");
    harness.send(&[0x04]);
    harness.wait_for_raw(b"TEA_PTY_REOPEN");
    harness.wait_for_raw_occurrences(b"\x1b[?2004h", 2);
    harness.wait_for_cursor_queries(2);
    harness.settle(Duration::from_millis(100));
    harness.wait_for_screen("PTY final");
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
    assert_eq!(harness.cursor_query_count(), 2);
    assert!(
        !harness
            .raw()
            .windows(b"\x1b[?1049h".len())
            .any(|bytes| bytes == b"\x1b[?1049h")
    );
    assert_no_event_sequence_gap(&harness);
}

#[test]
fn image_attachment_remove_submit_and_reopen_stay_textual_and_path_private() {
    let mut harness = PtyHarness::spawn("image-reopen");
    harness.wait_for_raw(b"\x1b[?2004h");

    harness.paste("/image private-source.png");
    harness.send(b"\r");
    harness.wait_for_screen("private-source.png");
    harness.wait_for_screen("image/png");

    harness.paste("/image remove 1");
    harness.send(b"\r");
    harness.settle(Duration::from_millis(100));
    assert!(!harness.screen().contains("private-source.png"));
    assert!(!harness.screen().contains("image/png"));

    harness.paste("/image private-source.png");
    harness.send(b"\r");
    harness.wait_for_screen("private-source.png");
    harness.paste("describe this image");
    harness.send(b"\r");
    harness.wait_for_screen("[image image/png · 21 B]");
    harness.wait_for_screen("PTY image response");

    harness.send(&[0x04]);
    harness.wait_for_raw(b"TEA_PTY_IMAGE_REOPEN");
    let reopened_at = harness.raw().len();
    harness.wait_for_raw_occurrences(b"\x1b[?2004h", 2);
    harness.wait_for_cursor_queries(2);
    harness.settle(Duration::from_millis(100));
    harness.wait_for_screen("[image image/png · 21 B]");
    harness.wait_for_screen("PTY image response");
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());

    let reopened_raw = &harness.raw()[reopened_at..];
    assert!(
        !reopened_raw
            .windows(b"private-source.png".len())
            .any(|bytes| bytes == b"private-source.png"),
        "the ephemeral source name leaked into reopened output"
    );
    assert!(
        !reopened_raw
            .windows(b"iVBOR".len())
            .any(|bytes| bytes == b"iVBOR"),
        "the inline payload leaked into reopened output"
    );
    for protocol in [KITTY_IMAGE_PREFIX, SIXEL_IMAGE_PREFIX, ITERM_IMAGE_PREFIX] {
        assert!(
            !harness
                .raw()
                .windows(protocol.len())
                .any(|bytes| bytes == protocol),
            "terminal image protocol was emitted: {protocol:?}"
        );
    }
    assert_no_event_sequence_gap(&harness);
}
