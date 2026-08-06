use std::io;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use tea_cli::tui::{
    TerminalDriver, TerminalGuard, TerminalMode, TerminalOptions, TerminalTitle, ViewportMode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    Enable(TerminalMode),
    Disable(TerminalMode),
    Drain,
    Child,
}

#[derive(Clone)]
struct MockDriver {
    operations: Arc<Mutex<Vec<Operation>>>,
    fail_on_enable: Option<TerminalMode>,
}

impl TerminalDriver for MockDriver {
    fn enable(&mut self, mode: TerminalMode) -> io::Result<()> {
        self.operations
            .lock()
            .unwrap()
            .push(Operation::Enable(mode));
        if self.fail_on_enable == Some(mode) {
            Err(io::Error::other("injected terminal failure"))
        } else {
            Ok(())
        }
    }

    fn disable(&mut self, mode: TerminalMode) -> io::Result<()> {
        self.operations
            .lock()
            .unwrap()
            .push(Operation::Disable(mode));
        Ok(())
    }

    fn drain(&mut self, deadline: Instant) -> io::Result<()> {
        assert!(deadline > Instant::now());
        self.operations.lock().unwrap().push(Operation::Drain);
        Ok(())
    }
}

fn terminal_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn options(viewport: ViewportMode) -> TerminalOptions {
    TerminalOptions {
        viewport,
        synchronized_output: true,
        focus_events: true,
        mouse_capture: false,
        keyboard_enhancement: false,
        cursor_visible: false,
        title: None,
        hyperlinks: false,
    }
}

fn titled_options(viewport: ViewportMode, title: &str) -> TerminalOptions {
    TerminalOptions {
        title: Some(TerminalTitle::new(title)),
        ..options(viewport)
    }
}

#[test]
fn terminal_title_is_bounded_sanitized_and_owned_in_both_viewport_modes() {
    let _serial = terminal_test_lock();
    let title = TerminalTitle::new(&format!(
        "  Tea\n\u{1b}]2;unsafe\u{7} {}  ",
        "界".repeat(64)
    ));
    assert!(title.as_str().len() <= 64);
    assert_eq!(title.as_str().trim(), title.as_str());
    assert!(!title.as_str().chars().any(char::is_control));

    for viewport in [ViewportMode::Inline, ViewportMode::Fullscreen] {
        let operations = Arc::new(Mutex::new(Vec::new()));
        {
            let _guard = TerminalGuard::enter(
                MockDriver {
                    operations: Arc::clone(&operations),
                    fail_on_enable: None,
                },
                TerminalOptions {
                    title: Some(title),
                    ..options(viewport)
                },
            )
            .unwrap();
        }
        let operations = operations.lock().unwrap();
        assert_eq!(
            operations.first(),
            Some(&Operation::Enable(TerminalMode::Title(title)))
        );
        assert_eq!(
            operations.last(),
            Some(&Operation::Disable(TerminalMode::Title(title)))
        );
    }
}

#[test]
fn failure_after_title_enablement_restores_the_saved_title() {
    let _serial = terminal_test_lock();
    let operations = Arc::new(Mutex::new(Vec::new()));
    let title = TerminalTitle::new("Tea");
    let error = TerminalGuard::enter(
        MockDriver {
            operations: Arc::clone(&operations),
            fail_on_enable: Some(TerminalMode::Raw),
        },
        TerminalOptions {
            title: Some(title),
            ..options(ViewportMode::Inline)
        },
    )
    .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(
        operations.lock().unwrap().as_slice(),
        &[
            Operation::Enable(TerminalMode::Title(title)),
            Operation::Enable(TerminalMode::Raw),
            Operation::Disable(TerminalMode::Title(title)),
        ]
    );
}

#[test]
fn ledger_tears_down_in_reverse_and_inline_never_leaves_alternate_screen() {
    let _serial = terminal_test_lock();
    let fullscreen_log = Arc::new(Mutex::new(Vec::new()));
    {
        let guard = TerminalGuard::enter(
            MockDriver {
                operations: Arc::clone(&fullscreen_log),
                fail_on_enable: None,
            },
            options(ViewportMode::Fullscreen),
        )
        .unwrap();
        guard.begin_frame().unwrap();
        guard.end_frame(Duration::from_secs(1)).unwrap();
    }
    let operations = fullscreen_log.lock().unwrap();
    assert!(operations.contains(&Operation::Enable(TerminalMode::AlternateScreen)));
    assert!(operations.contains(&Operation::Enable(TerminalMode::SynchronizedOutput)));
    assert!(operations.contains(&Operation::Disable(TerminalMode::SynchronizedOutput)));
    assert_eq!(
        &operations[operations.len() - 5..],
        &[
            Operation::Disable(TerminalMode::CursorHidden),
            Operation::Disable(TerminalMode::FocusEvents),
            Operation::Disable(TerminalMode::BracketedPaste),
            Operation::Disable(TerminalMode::AlternateScreen),
            Operation::Disable(TerminalMode::Raw),
        ]
    );
    drop(operations);

    let inline_log = Arc::new(Mutex::new(Vec::new()));
    {
        let _guard = TerminalGuard::enter(
            MockDriver {
                operations: Arc::clone(&inline_log),
                fail_on_enable: None,
            },
            options(ViewportMode::Inline),
        )
        .unwrap();
    }
    assert!(!inline_log.lock().unwrap().iter().any(|operation| matches!(
        operation,
        Operation::Enable(TerminalMode::AlternateScreen)
            | Operation::Disable(TerminalMode::AlternateScreen)
    )));
}

#[test]
fn hyperlink_output_is_the_last_persistent_mode_enabled_and_the_first_restored() {
    let _serial = terminal_test_lock();
    let operations = Arc::new(Mutex::new(Vec::new()));
    {
        let _guard = TerminalGuard::enter(
            MockDriver {
                operations: Arc::clone(&operations),
                fail_on_enable: None,
            },
            TerminalOptions {
                hyperlinks: true,
                ..options(ViewportMode::Fullscreen)
            },
        )
        .unwrap();
    }

    let operations = operations.lock().unwrap();
    let enabled = operations
        .iter()
        .rposition(|operation| matches!(operation, Operation::Enable(_)))
        .unwrap();
    let disabled = operations
        .iter()
        .position(|operation| matches!(operation, Operation::Disable(_)))
        .unwrap();
    assert_eq!(
        operations[enabled],
        Operation::Enable(TerminalMode::HyperlinkOutput)
    );
    assert_eq!(
        operations[disabled],
        Operation::Disable(TerminalMode::HyperlinkOutput)
    );
}

#[test]
fn partial_enter_failure_restores_only_enabled_modes() {
    let _serial = terminal_test_lock();
    let operations = Arc::new(Mutex::new(Vec::new()));
    let error = TerminalGuard::enter(
        MockDriver {
            operations: Arc::clone(&operations),
            fail_on_enable: Some(TerminalMode::FocusEvents),
        },
        options(ViewportMode::Fullscreen),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(
        operations.lock().unwrap().as_slice(),
        &[
            Operation::Enable(TerminalMode::Raw),
            Operation::Enable(TerminalMode::AlternateScreen),
            Operation::Enable(TerminalMode::BracketedPaste),
            Operation::Enable(TerminalMode::FocusEvents),
            Operation::Disable(TerminalMode::BracketedPaste),
            Operation::Disable(TerminalMode::AlternateScreen),
            Operation::Disable(TerminalMode::Raw),
        ]
    );
}

#[test]
fn unsupported_keyboard_enhancement_does_not_block_terminal_entry() {
    let _serial = terminal_test_lock();
    let operations = Arc::new(Mutex::new(Vec::new()));
    let mut terminal_options = options(ViewportMode::Inline);
    terminal_options.keyboard_enhancement = true;
    let guard = TerminalGuard::enter(
        MockDriver {
            operations: Arc::clone(&operations),
            fail_on_enable: Some(TerminalMode::KeyboardEnhancement),
        },
        terminal_options,
    )
    .unwrap();
    drop(guard);

    let operations = operations.lock().unwrap();
    assert!(operations.contains(&Operation::Enable(TerminalMode::KeyboardEnhancement)));
    assert!(!operations.contains(&Operation::Disable(TerminalMode::KeyboardEnhancement)));
}

#[test]
fn child_handoff_parks_drains_restores_reenters_and_redraw_can_follow() {
    let _serial = terminal_test_lock();
    let operations = Arc::new(Mutex::new(Vec::new()));
    let guard = TerminalGuard::enter(
        MockDriver {
            operations: Arc::clone(&operations),
            fail_on_enable: None,
        },
        titled_options(ViewportMode::Fullscreen, "Tea child"),
    )
    .unwrap();
    guard
        .handoff(Duration::from_secs(1), || {
            assert!(guard.input_is_parked().unwrap());
            operations.lock().unwrap().push(Operation::Child);
            Ok(())
        })
        .unwrap();
    assert!(!guard.input_is_parked().unwrap());
    let operations = operations.lock().unwrap();
    let child = operations
        .iter()
        .position(|operation| *operation == Operation::Child)
        .unwrap();
    assert!(operations[..child].contains(&Operation::Drain));
    assert!(operations[..child].contains(&Operation::Disable(TerminalMode::Raw)));
    assert!(operations[child + 1..].contains(&Operation::Enable(TerminalMode::Raw)));
    assert!(
        operations[..child]
            .iter()
            .any(|operation| matches!(operation, Operation::Disable(TerminalMode::Title(_))))
    );
    assert!(
        operations[child + 1..]
            .iter()
            .any(|operation| matches!(operation, Operation::Enable(TerminalMode::Title(_))))
    );
}

#[test]
fn panic_hook_restores_every_enabled_mode() {
    let _serial = terminal_test_lock();
    let operations = Arc::new(Mutex::new(Vec::new()));
    let panic_log = Arc::clone(&operations);
    let result = std::panic::catch_unwind(move || {
        let _guard = TerminalGuard::enter(
            MockDriver {
                operations: panic_log,
                fail_on_enable: None,
            },
            titled_options(ViewportMode::Fullscreen, "Tea panic"),
        )
        .unwrap();
        panic!("injected panic after terminal initialization");
    });
    assert!(result.is_err());
    let operations = operations.lock().unwrap();
    assert!(operations.contains(&Operation::Disable(TerminalMode::CursorHidden)));
    assert!(operations.contains(&Operation::Disable(TerminalMode::AlternateScreen)));
    assert!(operations.contains(&Operation::Disable(TerminalMode::Raw)));
    assert!(
        operations
            .iter()
            .any(|operation| matches!(operation, Operation::Disable(TerminalMode::Title(_))))
    );
}
