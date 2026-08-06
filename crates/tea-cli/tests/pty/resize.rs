use std::time::Duration;

use super::PtyHarness;

#[test]
fn resize_storm_and_large_code_block_remain_bounded_and_width_safe() {
    let mut harness = PtyHarness::spawn("inline");
    harness.wait_for_raw(b"\x1b[?2004h");
    let resize_frame = harness.frame_count() + 1;
    for step in 0..16 {
        harness.resize(20 + step % 5, 72 + step % 9);
    }
    harness.wait_for_frames(resize_frame);
    harness.paste("render a large code block");
    harness.wait_for_screen("render a large code block");
    harness.send(b"\r");
    harness.wait_for_screen("wide response");
    harness.settle(Duration::from_millis(50));
    let resize_frame = harness.frame_count() + 1;
    harness.resize(14, 64);
    harness.wait_for_frames(resize_frame);
    let edit_frame = harness.frame_count() + 1;
    harness.send(b"x");
    harness.wait_for_frames(edit_frame);
    let erase_frame = harness.frame_count() + 1;
    harness.send(b"\x7f");
    harness.wait_for_frames(erase_frame);
    harness.wait_for_screen("wide response");
    assert!(!harness.screen().contains('\u{fffd}'));
    assert!(
        !harness
            .raw()
            .windows(b"\x1b[?1049h".len())
            .any(|bytes| bytes == b"\x1b[?1049h")
    );
    assert!(
        harness.frame_count() <= 32,
        "resize storm: {} frames",
        harness.frame_count()
    );
    harness.send(&[0x04]);
    assert!(harness.wait_for_exit().success());
}
