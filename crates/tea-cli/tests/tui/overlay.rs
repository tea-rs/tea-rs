use tea_cli::tui::CommandCompletion;

#[test]
fn command_completion_is_bounded_and_accepts_only_an_explicit_selection() {
    let mut completion = CommandCompletion::new([
        "/compact".to_owned(),
        "/model".to_owned(),
        "/session".to_owned(),
    ]);

    assert_eq!(completion.selected(), Some("/compact"));
    completion.move_next();
    assert_eq!(completion.selected(), Some("/model"));
    completion.move_previous();
    assert_eq!(completion.selected(), Some("/compact"));
    assert_eq!(completion.options(), ["/compact", "/model", "/session"]);
}
