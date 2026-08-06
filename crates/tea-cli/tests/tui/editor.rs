use tea_cli::tui::{Editor, EditorError};

#[test]
fn unicode_multiline_movement_and_deletion_use_grapheme_boundaries() {
    let mut editor = Editor::with_limit("a中e\u{301}\n🙂z", 128).unwrap();
    assert_eq!(editor.cursor_byte(), editor.text().len());

    editor.move_left();
    editor.move_left();
    assert_eq!(editor.cursor_byte(), "a中e\u{301}\n".len());
    editor.move_up();
    assert_eq!(editor.cursor_byte(), 0);
    editor.move_end();
    assert_eq!(editor.cursor_byte(), "a中e\u{301}".len());
    editor.delete_backward();
    assert_eq!(editor.text(), "a中\n🙂z");
    editor.delete_word_backward();
    assert_eq!(editor.text(), "a\n🙂z");
}

#[test]
fn paste_is_atomic_undoable_and_bounded() {
    let mut editor = Editor::with_limit("ok", 6).unwrap();
    assert_eq!(editor.insert_paste("🙂"), Ok(()));
    assert_eq!(editor.text(), "ok🙂");
    assert_eq!(editor.insert_paste("x"), Err(EditorError::TooLarge));
    assert_eq!(editor.text(), "ok🙂");
    assert!(editor.undo());
    assert_eq!(editor.text(), "ok");
}

#[test]
fn submitted_history_is_bounded_and_restores_drafts() {
    let mut editor = Editor::with_limit("first", 128).unwrap();
    assert_eq!(editor.submit(), Some("first".to_owned()));
    editor.insert_paste("draft").unwrap();
    assert!(editor.previous_history());
    assert_eq!(editor.text(), "first");
    assert!(editor.next_history());
    assert_eq!(editor.text(), "draft");
}

#[test]
fn codex_style_word_movement_and_forward_deletion_are_unicode_safe() {
    let mut editor = Editor::with_limit("alpha beta", 128).unwrap();
    editor.move_word_left();
    assert_eq!(editor.cursor_byte(), "alpha ".len());
    editor.move_word_left();
    assert_eq!(editor.cursor_byte(), 0);
    editor.move_word_right();
    assert_eq!(editor.cursor_byte(), "alpha".len());
    assert!(editor.delete_word_forward());
    assert_eq!(editor.text(), "alpha");
    assert!(editor.yank().unwrap());
    assert_eq!(editor.text(), "alpha beta");

    let mut editor = Editor::with_limit("a🙂e\u{301}", 128).unwrap();
    editor.move_home();
    assert!(editor.delete_forward());
    assert_eq!(editor.text(), "🙂e\u{301}");
    assert!(editor.delete_forward());
    assert_eq!(editor.text(), "e\u{301}");

    let mut editor = Editor::with_limit(">alpha", 128).unwrap();
    editor.move_home();
    editor.move_word_right();
    assert_eq!(editor.cursor_byte(), 1);
}

#[test]
fn line_kills_yank_and_undo_preserve_multiline_text() {
    let mut editor = Editor::with_limit("one\ntwo three", 128).unwrap();
    editor.move_home();
    for _ in 0..3 {
        editor.move_right();
    }
    assert!(editor.kill_to_line_end());
    assert_eq!(editor.text(), "one\ntwo");
    assert!(editor.has_kill_buffer());
    assert!(editor.yank().unwrap());
    assert_eq!(editor.text(), "one\ntwo three");

    editor.move_end();
    assert!(editor.kill_to_line_start());
    assert_eq!(editor.text(), "one\n");
    assert!(editor.undo());
    assert_eq!(editor.text(), "one\ntwo three");
}

#[test]
fn yank_respects_the_editor_byte_limit_without_losing_the_kill_buffer() {
    let mut editor = Editor::with_limit("abc", 3).unwrap();
    assert!(editor.kill_to_line_start());
    editor.insert_paste("xyz").unwrap();
    assert_eq!(editor.yank(), Err(EditorError::TooLarge));
    assert_eq!(editor.text(), "xyz");
    assert!(editor.has_kill_buffer());
}
