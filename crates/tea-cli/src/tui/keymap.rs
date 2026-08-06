use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tea_coding::config::TuiSettings;

/// Semantic action selected by one configured key chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingAction {
    /// Submit a prompt while idle.
    Submit,
    /// Insert a newline without submitting.
    Newline,
    /// Abort the active operation.
    Abort,
    /// Clear the editor.
    Clear,
    /// Exit the interactive application.
    Exit,
    /// Open the model selector.
    SelectModel,
    /// Toggle reasoning visibility.
    ToggleThinking,
    /// Toggle tool detail visibility.
    ToggleTools,
    /// Copy the last assistant response.
    Copy,
    /// Submit steering while a run is active.
    Steer,
    /// Queue a follow-up while a run is active.
    FollowUp,
    /// Restore queued messages to the editor.
    RetrieveQueued,
}

/// Built-in text editor action shared across terminal platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorAction {
    InsertNewline,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveWordLeft,
    MoveWordRight,
    MoveLineStart { cross_line: bool },
    MoveLineEnd { cross_line: bool },
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
    KillLineStart,
    KillLineEnd,
    Yank,
    Undo,
}

/// Resolves Codex-style built-in composer editing shortcuts.
pub(crate) fn resolve_editor_action(event: KeyEvent) -> Option<EditorAction> {
    let code = event.code;
    let modifiers = event.modifiers;
    let none = KeyModifiers::NONE;
    let shift = KeyModifiers::SHIFT;
    let control = KeyModifiers::CONTROL;
    let alt = KeyModifiers::ALT;
    let control_shift = control | shift;
    let control_alt = control | alt;

    let action = match code {
        KeyCode::Enter if matches!(modifiers, value if value == none || value == shift || value == alt) => {
            EditorAction::InsertNewline
        }
        KeyCode::Char('j' | 'm') if modifiers == control => EditorAction::InsertNewline,
        KeyCode::Left if modifiers == none => EditorAction::MoveLeft,
        KeyCode::Char('b') if modifiers == control => EditorAction::MoveLeft,
        KeyCode::Right if modifiers == none => EditorAction::MoveRight,
        KeyCode::Char('f') if modifiers == control => EditorAction::MoveRight,
        KeyCode::Up if modifiers == none => EditorAction::MoveUp,
        KeyCode::Char('p') if modifiers == control => EditorAction::MoveUp,
        KeyCode::Down if modifiers == none => EditorAction::MoveDown,
        KeyCode::Char('n') if modifiers == control => EditorAction::MoveDown,
        KeyCode::Left if modifiers == alt || modifiers == control => EditorAction::MoveWordLeft,
        KeyCode::Char('b') if modifiers == alt => EditorAction::MoveWordLeft,
        KeyCode::Right if modifiers == alt || modifiers == control => EditorAction::MoveWordRight,
        KeyCode::Char('f') if modifiers == alt => EditorAction::MoveWordRight,
        KeyCode::Home if modifiers == none => EditorAction::MoveLineStart { cross_line: false },
        KeyCode::Char('a') if modifiers == control => {
            EditorAction::MoveLineStart { cross_line: true }
        }
        KeyCode::End if modifiers == none => EditorAction::MoveLineEnd { cross_line: false },
        KeyCode::Char('e') if modifiers == control => {
            EditorAction::MoveLineEnd { cross_line: true }
        }
        KeyCode::Backspace
            if modifiers == alt || modifiers == control || modifiers == control_shift =>
        {
            EditorAction::DeleteWordBackward
        }
        KeyCode::Char('w') if modifiers == control => EditorAction::DeleteWordBackward,
        KeyCode::Char('h') if modifiers == control_alt => EditorAction::DeleteWordBackward,
        KeyCode::Backspace if modifiers == none || modifiers == shift => {
            EditorAction::DeleteBackward
        }
        KeyCode::Delete
            if modifiers == alt || modifiers == control || modifiers == control_shift =>
        {
            EditorAction::DeleteWordForward
        }
        KeyCode::Char('d') if modifiers == alt => EditorAction::DeleteWordForward,
        KeyCode::Delete if modifiers == none || modifiers == shift => EditorAction::DeleteForward,
        KeyCode::Char('h') if modifiers == control => EditorAction::DeleteBackward,
        KeyCode::Char('d') if modifiers == control => EditorAction::DeleteForward,
        KeyCode::Char('u') if modifiers == control => EditorAction::KillLineStart,
        KeyCode::Char('k') if modifiers == control => EditorAction::KillLineEnd,
        KeyCode::Char('y') if modifiers == control => EditorAction::Yank,
        KeyCode::Char('z') if modifiers == control => EditorAction::Undo,
        _ => return None,
    };
    Some(action)
}

/// Invalid or ambiguous key binding configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyMapError {
    /// A key chord has invalid syntax.
    #[error("key binding syntax is invalid")]
    Invalid,
    /// Two actions active in the same context use the same key chord.
    #[error("key binding configuration is ambiguous")]
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyChord {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyChord {
    fn matches(self, event: KeyEvent) -> bool {
        self.code == event.code && self.modifiers == event.modifiers
    }
}

/// Validated context-aware interactive key map.
#[derive(Debug, Clone)]
pub struct KeyMap {
    submit: KeyChord,
    newline: KeyChord,
    abort: KeyChord,
    clear: KeyChord,
    exit: KeyChord,
    select_model: KeyChord,
    toggle_thinking: KeyChord,
    toggle_tools: KeyChord,
    copy: KeyChord,
    steer: KeyChord,
    follow_up: KeyChord,
    retrieve_queued: KeyChord,
}

impl KeyMap {
    /// Parses and validates resolved TUI settings.
    ///
    /// # Errors
    ///
    /// Rejects invalid chords and same-context ambiguity.
    pub fn from_settings(settings: &TuiSettings) -> Result<Self, KeyMapError> {
        let map = Self {
            submit: parse_chord(&settings.submit_key)?,
            newline: parse_chord(&settings.newline_key)?,
            abort: parse_chord(&settings.abort_key)?,
            clear: parse_chord(&settings.clear_key)?,
            exit: parse_chord(&settings.exit_key)?,
            select_model: parse_chord(&settings.model_key)?,
            toggle_thinking: parse_chord(&settings.toggle_thinking_key)?,
            toggle_tools: parse_chord(&settings.toggle_tools_key)?,
            copy: parse_chord(&settings.copy_key)?,
            steer: parse_chord(&settings.steering_key)?,
            follow_up: parse_chord(&settings.follow_up_key)?,
            retrieve_queued: parse_chord(&settings.retrieve_queued_key)?,
        };
        map.validate_ambiguity()?;
        Ok(map)
    }

    /// Resolves a key according to idle/running context.
    #[must_use]
    pub fn resolve(&self, event: KeyEvent, running: bool) -> Option<BindingAction> {
        let contextual = if running {
            [
                (self.abort, BindingAction::Abort),
                (self.follow_up, BindingAction::FollowUp),
                (self.steer, BindingAction::Steer),
            ]
        } else {
            [
                (self.submit, BindingAction::Submit),
                (self.newline, BindingAction::Newline),
                (self.abort, BindingAction::Abort),
            ]
        };
        contextual
            .into_iter()
            .chain([
                (self.clear, BindingAction::Clear),
                (self.exit, BindingAction::Exit),
                (self.select_model, BindingAction::SelectModel),
                (self.toggle_thinking, BindingAction::ToggleThinking),
                (self.toggle_tools, BindingAction::ToggleTools),
                (self.copy, BindingAction::Copy),
                (self.retrieve_queued, BindingAction::RetrieveQueued),
            ])
            .find_map(|(chord, action)| chord.matches(event).then_some(action))
    }

    fn validate_ambiguity(&self) -> Result<(), KeyMapError> {
        let globals = [
            self.newline,
            self.clear,
            self.exit,
            self.select_model,
            self.toggle_thinking,
            self.toggle_tools,
            self.copy,
            self.retrieve_queued,
        ];
        if has_duplicate(&globals)
            || globals.contains(&self.submit)
            || globals.contains(&self.steer)
            || globals.contains(&self.follow_up)
            || globals.contains(&self.abort)
            || self.submit == self.abort
            || self.steer == self.follow_up
            || self.abort == self.steer
            || self.abort == self.follow_up
        {
            return Err(KeyMapError::Ambiguous);
        }
        Ok(())
    }
}

fn has_duplicate(values: &[KeyChord]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[index + 1..].contains(value))
}

fn parse_chord(value: &str) -> Result<KeyChord, KeyMapError> {
    let mut modifiers = KeyModifiers::NONE;
    let mut code = None;
    for token in value.split('+') {
        let token = token.trim().to_ascii_lowercase();
        match token.as_str() {
            "ctrl" | "control" => modifiers.insert(KeyModifiers::CONTROL),
            "alt" | "option" => modifiers.insert(KeyModifiers::ALT),
            "shift" => modifiers.insert(KeyModifiers::SHIFT),
            "super" | "cmd" | "command" => modifiers.insert(KeyModifiers::SUPER),
            _ if code.is_none() => code = parse_code(&token),
            _ => return Err(KeyMapError::Invalid),
        }
    }
    Ok(KeyChord {
        code: code.ok_or(KeyMapError::Invalid)?,
        modifiers,
    })
}

fn parse_code(value: &str) -> Option<KeyCode> {
    match value {
        "enter" | "return" => Some(KeyCode::Enter),
        "esc" | "escape" => Some(KeyCode::Esc),
        "backspace" => Some(KeyCode::Backspace),
        "delete" => Some(KeyCode::Delete),
        "tab" => Some(KeyCode::Tab),
        "left" => Some(KeyCode::Left),
        "right" => Some(KeyCode::Right),
        "up" => Some(KeyCode::Up),
        "down" => Some(KeyCode::Down),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        "space" => Some(KeyCode::Char(' ')),
        value => {
            let mut characters = value.chars();
            let character = characters.next()?;
            characters
                .next()
                .is_none()
                .then_some(KeyCode::Char(character))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EditorAction, resolve_editor_action};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn codex_editor_shortcut_matrix_resolves_to_text_actions() {
        let ctrl = KeyModifiers::CONTROL;
        let alt = KeyModifiers::ALT;
        let shift = KeyModifiers::SHIFT;
        let ctrl_shift = ctrl | shift;
        let cases = [
            (
                KeyCode::Char('a'),
                ctrl,
                EditorAction::MoveLineStart { cross_line: true },
            ),
            (
                KeyCode::Char('e'),
                ctrl,
                EditorAction::MoveLineEnd { cross_line: true },
            ),
            (KeyCode::Char('b'), ctrl, EditorAction::MoveLeft),
            (KeyCode::Char('f'), ctrl, EditorAction::MoveRight),
            (KeyCode::Char('p'), ctrl, EditorAction::MoveUp),
            (KeyCode::Char('n'), ctrl, EditorAction::MoveDown),
            (KeyCode::Char('b'), alt, EditorAction::MoveWordLeft),
            (KeyCode::Char('f'), alt, EditorAction::MoveWordRight),
            (KeyCode::Left, ctrl, EditorAction::MoveWordLeft),
            (KeyCode::Right, alt, EditorAction::MoveWordRight),
            (KeyCode::Char('h'), ctrl, EditorAction::DeleteBackward),
            (KeyCode::Char('d'), ctrl, EditorAction::DeleteForward),
            (KeyCode::Char('w'), ctrl, EditorAction::DeleteWordBackward),
            (
                KeyCode::Backspace,
                ctrl_shift,
                EditorAction::DeleteWordBackward,
            ),
            (KeyCode::Char('d'), alt, EditorAction::DeleteWordForward),
            (KeyCode::Delete, ctrl_shift, EditorAction::DeleteWordForward),
            (KeyCode::Char('u'), ctrl, EditorAction::KillLineStart),
            (KeyCode::Char('k'), ctrl, EditorAction::KillLineEnd),
            (KeyCode::Char('y'), ctrl, EditorAction::Yank),
            (KeyCode::Char('j'), ctrl, EditorAction::InsertNewline),
            (KeyCode::Char('m'), ctrl, EditorAction::InsertNewline),
            (KeyCode::Enter, alt, EditorAction::InsertNewline),
        ];

        for (code, modifiers, expected) in cases {
            assert_eq!(
                resolve_editor_action(KeyEvent::new(code, modifiers)),
                Some(expected)
            );
        }
    }
}
