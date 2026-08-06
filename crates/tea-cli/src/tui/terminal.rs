use std::io::{self, Write};
use std::sync::{Arc, Mutex, Once, OnceLock, Weak};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    BeginSynchronizedUpdate, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{ExecutableCommand as _, QueueableCommand as _};

use super::hyperlink::write_close;

const MAX_TERMINAL_TITLE_BYTES: usize = 64;
const SAVE_WINDOW_TITLE: &[u8] = b"\x1b[22;2t";
const SET_WINDOW_TITLE_PREFIX: &[u8] = b"\x1b]2;";
const SET_WINDOW_TITLE_SUFFIX: &[u8] = b"\x07";
const RESTORE_WINDOW_TITLE: &[u8] = b"\x1b[23;2t";

/// Conservative color support inferred from standard terminal environment variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCapability {
    /// Do not emit color sequences.
    None,
    /// ANSI sixteen-color palette.
    Ansi16,
    /// ANSI 256-color palette.
    Ansi256,
    /// True-color palette.
    TrueColor,
}

/// Terminal display features that are safe to use without an active probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalCapabilities {
    color: ColorCapability,
    reduced_motion: bool,
    interactive: bool,
    hyperlinks: bool,
}

impl TerminalCapabilities {
    /// Detects conservative capabilities from the process environment.
    #[must_use]
    pub fn detect(reduced_motion: bool) -> Self {
        let terminal = std::env::var("TERM").ok();
        let color_terminal = std::env::var("COLORTERM").ok();
        Self::from_environment(
            terminal.as_deref(),
            color_terminal.as_deref(),
            std::env::var_os("NO_COLOR").is_some(),
            reduced_motion,
        )
    }

    /// Resolves capabilities from explicit environment values for deterministic callers and tests.
    #[must_use]
    pub fn from_environment(
        terminal: Option<&str>,
        color_terminal: Option<&str>,
        no_color: bool,
        reduced_motion: bool,
    ) -> Self {
        let dumb = terminal.is_some_and(|value| value.eq_ignore_ascii_case("dumb"));
        let color = if no_color || dumb {
            ColorCapability::None
        } else if color_terminal.is_some_and(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("truecolor") || value.contains("24bit")
        }) {
            ColorCapability::TrueColor
        } else if terminal.is_some_and(|value| value.contains("256color")) {
            ColorCapability::Ansi256
        } else {
            ColorCapability::Ansi16
        };
        let hyperlinks = terminal.is_some_and(supports_osc8_terminal);
        Self {
            color,
            reduced_motion: reduced_motion || dumb,
            interactive: !dumb,
            hyperlinks,
        }
    }

    /// Returns the selected safe color palette.
    #[must_use]
    pub const fn color(self) -> ColorCapability {
        self.color
    }

    /// Returns whether animated visual affordances should remain still.
    #[must_use]
    pub const fn reduced_motion(self) -> bool {
        self.reduced_motion
    }

    /// Returns whether synchronized output is safe for this terminal class.
    #[must_use]
    pub const fn supports_synchronized_output(self) -> bool {
        self.interactive
    }

    /// Returns whether focus notifications are safe for this terminal class.
    #[must_use]
    pub const fn supports_focus_events(self) -> bool {
        self.interactive
    }

    /// Returns whether guarded terminal-title ownership is safe for this terminal class.
    #[must_use]
    pub const fn supports_title(self) -> bool {
        self.interactive
    }

    /// Returns whether guarded OSC8 output is safe for this terminal class.
    #[must_use]
    pub const fn supports_hyperlinks(self) -> bool {
        self.hyperlinks
    }
}

fn supports_osc8_terminal(terminal: &str) -> bool {
    let terminal = terminal.to_ascii_lowercase();
    if terminal == "dumb" || terminal.starts_with("screen") || terminal.starts_with("tmux") {
        return false;
    }
    [
        "xterm",
        "rxvt",
        "kitty",
        "wezterm",
        "alacritty",
        "foot",
        "ghostty",
    ]
    .iter()
    .any(|family| terminal.contains(family))
}

/// Fullscreen compatibility or Codex-style native scrollback ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportMode {
    /// Own the alternate screen for the interaction.
    Fullscreen,
    /// Render a dynamic normal-screen viewport above terminal-native scrollback.
    Inline,
}

/// Bounded terminal title with protocol control bytes removed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TerminalTitle {
    bytes: [u8; MAX_TERMINAL_TITLE_BYTES],
    len: u8,
}

impl TerminalTitle {
    /// Sanitizes and bounds a title, falling back to `Tea` when no text remains.
    #[must_use]
    pub fn new(value: &str) -> Self {
        let mut bytes = [0; MAX_TERMINAL_TITLE_BYTES];
        let mut len = 0usize;
        let mut pending_space = false;
        for character in value.chars() {
            if character.is_control() {
                continue;
            }
            if character.is_whitespace() {
                pending_space = len > 0;
                continue;
            }
            if pending_space && len < MAX_TERMINAL_TITLE_BYTES {
                bytes[len] = b' ';
                len += 1;
            }
            pending_space = false;
            let mut encoded = [0; 4];
            let encoded = character.encode_utf8(&mut encoded).as_bytes();
            if len.saturating_add(encoded.len()) > MAX_TERMINAL_TITLE_BYTES {
                break;
            }
            bytes[len..len + encoded.len()].copy_from_slice(encoded);
            len += encoded.len();
        }
        if len > 0 && bytes[len - 1] == b' ' {
            len -= 1;
        }
        if len == 0 {
            bytes[..3].copy_from_slice(b"Tea");
            len = 3;
        }
        Self {
            bytes,
            len: u8::try_from(len).unwrap_or(64),
        }
    }

    /// Returns the sanitized title text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.bytes
            .get(..usize::from(self.len))
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or("Tea")
    }
}

impl Default for TerminalTitle {
    fn default() -> Self {
        Self::new("Tea")
    }
}

impl std::fmt::Debug for TerminalTitle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("TerminalTitle")
            .field(&self.as_str())
            .finish()
    }
}

/// Every terminal feature managed by the canonical mode ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalMode {
    /// Saved prior title plus one bounded Tea-owned window title.
    Title(TerminalTitle),
    /// Crossterm raw input mode.
    Raw,
    /// Alternate-screen fullscreen viewport.
    AlternateScreen,
    /// Bracketed paste reporting.
    BracketedPaste,
    /// Focus-change reporting.
    FocusEvents,
    /// Mouse capture.
    MouseCapture,
    /// Kitty keyboard enhancement stack entry.
    KeyboardEnhancement,
    /// Hidden terminal cursor owned by the renderer.
    CursorHidden,
    /// One in-progress synchronized-output frame.
    SynchronizedOutput,
    /// Guarded OSC8 output channel that must be closed during recovery.
    HyperlinkOutput,
}

/// Requested terminal features. Unsupported keyboard enhancement is ignored during entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // Independent terminal capabilities are boolean.
pub struct TerminalOptions {
    /// Bounded title to own for the duration of the guarded TUI session.
    pub title: Option<TerminalTitle>,
    /// Selected viewport behavior.
    pub viewport: ViewportMode,
    /// Enable synchronized frame boundaries.
    pub synchronized_output: bool,
    /// Enable focus-change events.
    pub focus_events: bool,
    /// Enable mouse capture.
    pub mouse_capture: bool,
    /// Enable keyboard protocol enhancement.
    pub keyboard_enhancement: bool,
    /// Leave the cursor visible for an editor-owned placement.
    pub cursor_visible: bool,
    /// Enable guarded OSC8 hyperlink output.
    pub hyperlinks: bool,
}

impl Default for TerminalOptions {
    fn default() -> Self {
        Self {
            title: None,
            viewport: ViewportMode::Fullscreen,
            synchronized_output: true,
            focus_events: true,
            mouse_capture: false,
            keyboard_enhancement: false,
            cursor_visible: false,
            hyperlinks: false,
        }
    }
}

/// Replaceable terminal lifecycle driver used by the guard and tests.
#[allow(clippy::missing_errors_doc)] // Each operation reports the underlying terminal I/O failure.
pub trait TerminalDriver: Send {
    /// Enables one feature.
    fn enable(&mut self, mode: TerminalMode) -> io::Result<()>;
    /// Disables one previously enabled feature.
    fn disable(&mut self, mode: TerminalMode) -> io::Result<()>;
    /// Drains queued terminal output within the caller's deadline contract.
    fn drain(&mut self, deadline: Instant) -> io::Result<()>;
}

/// Crossterm lifecycle driver over one exclusive terminal writer.
#[derive(Debug)]
pub struct CrosstermDriver<W> {
    writer: W,
}

impl<W> CrosstermDriver<W> {
    /// Creates a driver around the only writer allowed to own the TTY.
    #[must_use]
    pub const fn new(writer: W) -> Self {
        Self { writer }
    }
}

impl<W: Write + Send> TerminalDriver for CrosstermDriver<W> {
    fn enable(&mut self, mode: TerminalMode) -> io::Result<()> {
        match mode {
            TerminalMode::Title(title) => self.enable_title(title),
            TerminalMode::Raw => crossterm::terminal::enable_raw_mode(),
            TerminalMode::AlternateScreen => self.writer.execute(EnterAlternateScreen).map(drop),
            TerminalMode::BracketedPaste => self.writer.execute(EnableBracketedPaste).map(drop),
            TerminalMode::FocusEvents => self.writer.execute(EnableFocusChange).map(drop),
            TerminalMode::MouseCapture => self.writer.execute(EnableMouseCapture).map(drop),
            TerminalMode::KeyboardEnhancement => self
                .writer
                .execute(PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
                ))
                .map(drop),
            TerminalMode::CursorHidden => self.writer.execute(Hide).map(drop),
            TerminalMode::SynchronizedOutput => {
                self.writer.queue(BeginSynchronizedUpdate).map(drop)
            }
            TerminalMode::HyperlinkOutput => Ok(()),
        }
    }

    fn disable(&mut self, mode: TerminalMode) -> io::Result<()> {
        match mode {
            TerminalMode::Title(_) => self.restore_title(),
            TerminalMode::Raw => crossterm::terminal::disable_raw_mode(),
            TerminalMode::AlternateScreen => self.writer.execute(LeaveAlternateScreen).map(drop),
            TerminalMode::BracketedPaste => self.writer.execute(DisableBracketedPaste).map(drop),
            TerminalMode::FocusEvents => self.writer.execute(DisableFocusChange).map(drop),
            TerminalMode::MouseCapture => self.writer.execute(DisableMouseCapture).map(drop),
            TerminalMode::KeyboardEnhancement => {
                self.writer.execute(PopKeyboardEnhancementFlags).map(drop)
            }
            TerminalMode::CursorHidden => self.writer.execute(Show).map(drop),
            TerminalMode::SynchronizedOutput => {
                self.writer.execute(EndSynchronizedUpdate).map(drop)
            }
            TerminalMode::HyperlinkOutput => {
                write_close(&mut self.writer)?;
                self.writer.flush()
            }
        }
    }

    fn drain(&mut self, deadline: Instant) -> io::Result<()> {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "terminal frame drain deadline expired",
            ));
        }
        self.writer.flush()?;
        if Instant::now() > deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "terminal frame drain exceeded its deadline",
            ));
        }
        Ok(())
    }
}

impl<W: Write> CrosstermDriver<W> {
    fn enable_title(&mut self, title: TerminalTitle) -> io::Result<()> {
        let result = (|| {
            self.writer.write_all(SAVE_WINDOW_TITLE)?;
            self.writer.write_all(SET_WINDOW_TITLE_PREFIX)?;
            self.writer.write_all(title.as_str().as_bytes())?;
            self.writer.write_all(SET_WINDOW_TITLE_SUFFIX)?;
            self.writer.flush()
        })();
        if let Err(error) = result {
            let _ = self.restore_title();
            return Err(error);
        }
        Ok(())
    }

    fn restore_title(&mut self) -> io::Result<()> {
        self.writer.write_all(RESTORE_WINDOW_TITLE)?;
        self.writer.flush()
    }
}

struct GuardState {
    driver: Box<dyn TerminalDriver>,
    enabled: Vec<TerminalMode>,
    options: TerminalOptions,
    input_parked: bool,
}

impl std::fmt::Debug for GuardState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuardState")
            .field("enabled", &self.enabled)
            .field("options", &self.options)
            .field("input_parked", &self.input_parked)
            .finish_non_exhaustive()
    }
}

impl GuardState {
    fn enter(&mut self) -> io::Result<()> {
        if let Some(title) = self.options.title {
            self.enable(TerminalMode::Title(title))?;
        }
        self.enable(TerminalMode::Raw)?;
        if self.options.viewport == ViewportMode::Fullscreen {
            self.enable(TerminalMode::AlternateScreen)?;
        }
        self.enable(TerminalMode::BracketedPaste)?;
        if self.options.focus_events {
            self.enable(TerminalMode::FocusEvents)?;
        }
        if self.options.mouse_capture {
            self.enable(TerminalMode::MouseCapture)?;
        }
        if self.options.keyboard_enhancement {
            // Legacy Windows consoles and terminals without the Kitty protocol
            // cannot distinguish Shift+Enter. Keep the TUI usable when the
            // optional enhancement is unavailable.
            let _ = self.enable(TerminalMode::KeyboardEnhancement);
        }
        if !self.options.cursor_visible {
            self.enable(TerminalMode::CursorHidden)?;
        }
        if self.options.hyperlinks {
            self.enable(TerminalMode::HyperlinkOutput)?;
        }
        Ok(())
    }

    fn enable(&mut self, mode: TerminalMode) -> io::Result<()> {
        if self.enabled.contains(&mode) {
            return Ok(());
        }
        self.driver.enable(mode)?;
        self.enabled.push(mode);
        Ok(())
    }

    fn disable(&mut self, mode: TerminalMode) -> io::Result<()> {
        if let Some(index) = self.enabled.iter().position(|enabled| *enabled == mode) {
            self.driver.disable(mode)?;
            self.enabled.remove(index);
        }
        Ok(())
    }

    fn restore(&mut self) -> io::Result<()> {
        let mut first_error = None;
        while let Some(mode) = self.enabled.pop() {
            if let Err(error) = self.driver.disable(mode) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

static PANIC_HOOK_ONCE: Once = Once::new();
static ACTIVE_TERMINAL: OnceLock<Mutex<Option<Weak<Mutex<GuardState>>>>> = OnceLock::new();

fn install_panic_hook() {
    PANIC_HOOK_ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |information| {
            let active = ACTIVE_TERMINAL.get().and_then(|slot| {
                slot.try_lock()
                    .map_or_else(
                        |error| match error {
                            std::sync::TryLockError::Poisoned(error) => Some(error.into_inner()),
                            std::sync::TryLockError::WouldBlock => None,
                        },
                        Some,
                    )
                    .and_then(|slot| slot.as_ref().and_then(Weak::upgrade))
            });
            if let Some(active) = active {
                match active.try_lock() {
                    Ok(mut state) => {
                        let _ = state.restore();
                    }
                    Err(std::sync::TryLockError::Poisoned(error)) => {
                        let _ = error.into_inner().restore();
                    }
                    Err(std::sync::TryLockError::WouldBlock) => {}
                }
            }
            previous(information);
        }));
    });
}

/// RAII terminal owner with reverse-order teardown and foreground-child handoff.
#[derive(Debug)]
pub struct TerminalGuard {
    state: Arc<Mutex<GuardState>>,
}

#[allow(clippy::missing_errors_doc)] // Public lifecycle methods all expose terminal I/O failures.
impl TerminalGuard {
    /// Enables the requested modes and registers panic restoration.
    ///
    /// # Errors
    ///
    /// On partial initialization failure, every successfully enabled mode is
    /// disabled in reverse order before returning.
    pub fn enter(
        driver: impl TerminalDriver + 'static,
        options: TerminalOptions,
    ) -> io::Result<Self> {
        install_panic_hook();
        let state = Arc::new(Mutex::new(GuardState {
            driver: Box::new(driver),
            enabled: Vec::new(),
            options,
            input_parked: false,
        }));
        let guard = Self { state };
        let slot = ACTIVE_TERMINAL.get_or_init(|| Mutex::new(None));
        {
            let mut active = slot
                .lock()
                .map_err(|_| io::Error::other("active terminal registry is poisoned"))?;
            if active.as_ref().and_then(Weak::upgrade).is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "another terminal guard is already active",
                ));
            }
            // Register before terminal I/O so the panic hook can recover a
            // driver panic during partial initialization.
            *active = Some(Arc::downgrade(&guard.state));
        }
        {
            let mut locked = guard
                .state
                .lock()
                .map_err(|_| io::Error::other("terminal mode ledger is poisoned"))?;
            if let Err(error) = locked.enter() {
                let _ = locked.restore();
                return Err(error);
            }
        }
        Ok(guard)
    }

    /// Starts a synchronized frame when configured.
    pub fn begin_frame(&self) -> io::Result<()> {
        let mut state = self.lock()?;
        if state.options.synchronized_output {
            state.enable(TerminalMode::SynchronizedOutput)?;
        }
        Ok(())
    }

    /// Ends and drains a synchronized frame within a deadline.
    pub fn end_frame(&self, timeout: Duration) -> io::Result<()> {
        let mut state = self.lock()?;
        let end_result = state.disable(TerminalMode::SynchronizedOutput);
        let drain_result = state.driver.drain(Instant::now() + timeout);
        end_result.and(drain_result)
    }

    /// Restores all modes before suspend or foreground-child ownership.
    pub fn suspend(&self, timeout: Duration) -> io::Result<()> {
        let mut state = self.lock()?;
        state.input_parked = true;
        let sync_result = state.disable(TerminalMode::SynchronizedOutput);
        let drain_result = state.driver.drain(Instant::now() + timeout);
        let restore_result = state.restore();
        sync_result.and(drain_result).and(restore_result)
    }

    /// Re-enables modes and allows terminal input after reprobe.
    pub fn resume(&self) -> io::Result<()> {
        let mut state = self.lock()?;
        if let Err(error) = state.enter() {
            let _ = state.restore();
            return Err(error);
        }
        state.input_parked = false;
        Ok(())
    }

    /// Parks input, drains output, restores modes, runs a foreground child, and
    /// always attempts terminal reinitialization before returning.
    pub fn handoff<T>(
        &self,
        timeout: Duration,
        child: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<T> {
        self.suspend(timeout)?;
        let child_result = child();
        let resume_result = self.resume();
        match (child_result, resume_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) | (Ok(_), Err(error)) => Err(error),
        }
    }

    /// Returns the enabled-mode ledger for diagnostics and tests.
    pub fn enabled_modes(&self) -> io::Result<Vec<TerminalMode>> {
        Ok(self.lock()?.enabled.clone())
    }

    /// Returns whether input is parked for a foreground owner.
    pub fn input_is_parked(&self) -> io::Result<bool> {
        Ok(self.lock()?.input_parked)
    }

    fn lock(&self) -> io::Result<std::sync::MutexGuard<'_, GuardState>> {
        self.state
            .lock()
            .map_err(|_| io::Error::other("terminal mode ledger is poisoned"))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        match self.state.lock() {
            Ok(mut state) => {
                let _ = state.restore();
            }
            Err(error) => {
                let _ = error.into_inner().restore();
            }
        }
        if let Some(slot) = ACTIVE_TERMINAL.get()
            && let Ok(mut active) = slot.lock()
            && active
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some_and(|value| Arc::ptr_eq(&value, &self.state))
        {
            *active = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ColorCapability, CrosstermDriver, TerminalCapabilities, TerminalDriver, TerminalMode,
        TerminalTitle,
    };

    #[test]
    fn environment_capabilities_are_conservative_for_dumb_and_no_color_terminals() {
        let dumb = TerminalCapabilities::from_environment(Some("dumb"), None, false, false);
        assert_eq!(dumb.color(), ColorCapability::None);
        assert!(dumb.reduced_motion());
        assert!(!dumb.supports_synchronized_output());
        assert!(!dumb.supports_focus_events());
        assert!(!dumb.supports_title());
        assert!(!dumb.supports_hyperlinks());

        let no_color =
            TerminalCapabilities::from_environment(Some("xterm-256color"), None, true, false);
        assert_eq!(no_color.color(), ColorCapability::None);
        assert!(no_color.supports_synchronized_output());
        assert!(no_color.supports_title());
        assert!(no_color.supports_hyperlinks());

        for terminal in [None, Some("screen-256color"), Some("tmux-256color")] {
            assert!(
                !TerminalCapabilities::from_environment(terminal, None, false, false)
                    .supports_hyperlinks()
            );
        }

        let true_color = TerminalCapabilities::from_environment(
            Some("xterm-256color"),
            Some("truecolor"),
            false,
            true,
        );
        assert_eq!(true_color.color(), ColorCapability::TrueColor);
        assert!(true_color.reduced_motion());
    }

    #[test]
    fn crossterm_driver_saves_sets_and_restores_the_exact_window_title() {
        let title = TerminalTitle::default();
        let mode = TerminalMode::Title(title);
        let mut driver = CrosstermDriver::new(Vec::new());

        driver.enable(mode).unwrap();
        driver.disable(mode).unwrap();

        assert_eq!(driver.writer, b"\x1b[22;2t\x1b]2;Tea\x07\x1b[23;2t");
    }

    #[test]
    fn crossterm_driver_closes_guarded_hyperlink_output() {
        let mut driver = CrosstermDriver::new(Vec::new());

        driver.enable(TerminalMode::HyperlinkOutput).unwrap();
        driver.disable(TerminalMode::HyperlinkOutput).unwrap();

        assert_eq!(driver.writer, b"\x1b]8;;\x1b\\");
    }
}
