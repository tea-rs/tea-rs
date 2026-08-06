use super::{PtyHarness, occurrences};

const ALT_ENTER: &[u8] = b"\x1b[?1049h";
const ALT_LEAVE: &[u8] = b"\x1b[?1049l";
const PASTE_DISABLE: &[u8] = b"\x1b[?2004l";
const FOCUS_DISABLE: &[u8] = b"\x1b[?1004l";
const FOCUS_ENABLE: &[u8] = b"\x1b[?1004h";
const TITLE_SAVE: &[u8] = b"\x1b[22;2t";
const TITLE_SET: &[u8] = b"\x1b]2;Tea\x07";
const TITLE_RESTORE: &[u8] = b"\x1b[23;2t";

fn assert_reset_order(raw: &[u8]) {
    let focus = raw
        .windows(FOCUS_DISABLE.len())
        .rposition(|bytes| bytes == FOCUS_DISABLE)
        .unwrap();
    let paste = raw
        .windows(PASTE_DISABLE.len())
        .rposition(|bytes| bytes == PASTE_DISABLE)
        .unwrap();
    let alternate = raw
        .windows(ALT_LEAVE.len())
        .rposition(|bytes| bytes == ALT_LEAVE)
        .unwrap();
    let title = raw
        .windows(TITLE_RESTORE.len())
        .rposition(|bytes| bytes == TITLE_RESTORE)
        .unwrap();
    assert!(
        focus < paste && paste < alternate && alternate < title,
        "terminal reset order is unsafe"
    );
}

fn assert_title_cycles(raw: &[u8], expected: usize) {
    assert_eq!(occurrences(raw, TITLE_SAVE), expected);
    assert_eq!(occurrences(raw, TITLE_SET), expected);
    assert_eq!(occurrences(raw, TITLE_RESTORE), expected);
}

#[test]
fn normal_exit_restores_every_enabled_terminal_mode_in_reverse_order() {
    let mut harness = PtyHarness::spawn("fullscreen");
    harness.wait_for_raw(ALT_ENTER);
    assert_eq!(harness.query_responses(), 0, "unexpected capability query");
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
    assert_reset_order(harness.raw());
    assert_title_cycles(harness.raw(), 1);
}

#[test]
fn inline_exit_restores_the_saved_terminal_title_without_using_alt_screen() {
    let mut harness = PtyHarness::spawn("inline");
    harness.wait_for_raw(TITLE_SET);
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
    assert_title_cycles(harness.raw(), 1);
    assert_eq!(occurrences(harness.raw(), ALT_ENTER), 0);
    assert_eq!(occurrences(harness.raw(), ALT_LEAVE), 0);
}

#[test]
fn dumb_terminal_disables_optional_modes_without_affecting_exit_cleanup() {
    let mut harness = PtyHarness::spawn_with_term("fullscreen", "dumb");
    harness.wait_for_raw(ALT_ENTER);
    assert!(
        !harness
            .raw()
            .windows(FOCUS_ENABLE.len())
            .any(|bytes| bytes == FOCUS_ENABLE),
        "dumb terminals must not receive focus-change enablement"
    );
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
    assert!(
        !harness
            .raw()
            .windows(FOCUS_DISABLE.len())
            .any(|bytes| bytes == FOCUS_DISABLE),
        "disabled terminal modes must not be restored"
    );
    assert_title_cycles(harness.raw(), 0);
}

#[cfg(unix)]
#[test]
fn sigint_is_handled_inside_the_process_and_restores_terminal_modes() {
    let mut harness = PtyHarness::spawn("fullscreen");
    harness.wait_for_raw(ALT_ENTER);
    harness.interrupt();
    let status = harness.wait_for_exit();
    assert!(!status.success());
    assert_reset_order(harness.raw());
    assert_title_cycles(harness.raw(), 1);
}

#[test]
fn panic_hook_restores_the_real_terminal_before_unwinding() {
    let mut harness = PtyHarness::spawn("panic");
    harness.wait_for_raw(ALT_ENTER);
    let status = harness.wait_for_exit();
    assert!(!status.success());
    assert_reset_order(harness.raw());
    assert_title_cycles(harness.raw(), 1);
}

#[test]
fn foreground_child_handoff_restores_then_reenters_the_terminal() {
    let mut harness = PtyHarness::spawn("handoff");
    let status = harness.wait_for_exit();
    assert!(status.success());
    assert!(String::from_utf8_lossy(harness.raw()).contains("HANDOFF_CHILD"));
    assert_eq!(occurrences(harness.raw(), ALT_ENTER), 2);
    assert_eq!(occurrences(harness.raw(), ALT_LEAVE), 2);
    assert_title_cycles(harness.raw(), 2);
}
