use std::collections::VecDeque;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

/// Maximum UTF-8 bytes retained by the interactive editor.
pub const MAX_EDITOR_BYTES: usize = 256 * 1024;
/// Maximum submitted prompts retained for local history navigation.
pub const MAX_EDITOR_HISTORY: usize = 100;
const MAX_UNDO_STATES: usize = 100;
const WORD_SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

/// Rejected editor mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EditorError {
    /// The mutation would exceed the configured UTF-8 byte bound.
    #[error("editor input exceeds its size limit")]
    TooLarge,
    /// The configured byte bound is zero.
    #[error("editor size limit must be non-zero")]
    InvalidLimit,
}

#[derive(Debug, Clone)]
struct EditorSnapshot {
    text: String,
    cursor: usize,
}

/// Unicode grapheme-safe bounded multiline editor state.
#[derive(Debug, Clone)]
pub struct Editor {
    text: String,
    cursor: usize,
    max_bytes: usize,
    undo: VecDeque<EditorSnapshot>,
    kill_buffer: String,
    history: VecDeque<String>,
    history_index: Option<usize>,
    history_draft: Option<EditorSnapshot>,
}

impl Default for Editor {
    fn default() -> Self {
        Self::with_limit("", MAX_EDITOR_BYTES).expect("the default editor limit is valid")
    }
}

impl Editor {
    /// Creates an empty editor with the production byte bound.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an editor with an explicit byte bound.
    ///
    /// # Errors
    ///
    /// Rejects a zero limit or initial text larger than the limit.
    pub fn with_limit(text: impl Into<String>, max_bytes: usize) -> Result<Self, EditorError> {
        if max_bytes == 0 {
            return Err(EditorError::InvalidLimit);
        }
        let text = text.into();
        if text.len() > max_bytes {
            return Err(EditorError::TooLarge);
        }
        let cursor = text.len();
        Ok(Self {
            text,
            cursor,
            max_bytes,
            undo: VecDeque::new(),
            kill_buffer: String::new(),
            history: VecDeque::new(),
            history_index: None,
            history_draft: None,
        })
    }

    /// Returns the editor contents.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the cursor UTF-8 byte offset.
    #[must_use]
    pub const fn cursor_byte(&self) -> usize {
        self.cursor
    }

    /// Returns the display-cell cursor column within the current logical line.
    #[must_use]
    pub fn cursor_display_column(&self) -> usize {
        let start = self.text[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        self.text[start..self.cursor].width()
    }

    /// Inserts one character at the cursor.
    ///
    /// # Errors
    ///
    /// Rejects input that would exceed the byte bound.
    pub fn insert_char(&mut self, character: char) -> Result<(), EditorError> {
        let mut encoded = [0_u8; 4];
        self.insert_paste(character.encode_utf8(&mut encoded))
    }

    /// Inserts one bounded paste atomically at the cursor.
    ///
    /// # Errors
    ///
    /// Rejects the entire paste without mutation when it would exceed the bound.
    pub fn insert_paste(&mut self, text: &str) -> Result<(), EditorError> {
        if self.text.len().saturating_add(text.len()) > self.max_bytes {
            return Err(EditorError::TooLarge);
        }
        if text.is_empty() {
            return Ok(());
        }
        self.record_undo();
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.reset_history_navigation();
        Ok(())
    }

    /// Inserts one logical newline.
    ///
    /// # Errors
    ///
    /// Rejects insertion at the byte bound.
    pub fn insert_newline(&mut self) -> Result<(), EditorError> {
        self.insert_char('\n')
    }

    /// Moves left by one extended grapheme cluster.
    pub fn move_left(&mut self) {
        self.cursor = previous_grapheme_boundary(&self.text, self.cursor);
    }

    /// Moves right by one extended grapheme cluster.
    pub fn move_right(&mut self) {
        self.cursor = next_grapheme_boundary(&self.text, self.cursor);
    }

    /// Moves to the beginning of the preceding Unicode word.
    pub fn move_word_left(&mut self) {
        self.cursor = self.previous_word_start();
    }

    /// Moves to the end of the following Unicode word.
    pub fn move_word_right(&mut self) {
        self.cursor = self.next_word_end();
    }

    /// Moves to the start of the current logical line.
    pub fn move_home(&mut self) {
        self.cursor = self.current_line_start();
    }

    /// Moves to the current line start, or the preceding line start when already there.
    pub fn move_home_or_previous_line(&mut self) {
        let start = self.current_line_start();
        self.cursor = if self.cursor == start && start > 0 {
            self.text[..start - 1]
                .rfind('\n')
                .map_or(0, |index| index + 1)
        } else {
            start
        };
    }

    /// Moves to the end of the current logical line.
    pub fn move_end(&mut self) {
        self.cursor = self.current_line_end();
    }

    /// Moves to the current line end, or the following line end when already there.
    pub fn move_end_or_next_line(&mut self) {
        let end = self.current_line_end();
        self.cursor = if self.cursor == end && end < self.text.len() {
            self.text[end + 1..]
                .find('\n')
                .map_or(self.text.len(), |index| end + 1 + index)
        } else {
            end
        };
    }

    /// Moves one logical line upward while preserving display column when possible.
    pub fn move_up(&mut self) {
        let current_start = self.text[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        if current_start == 0 {
            return;
        }
        let previous_end = current_start - 1;
        let previous_start = self.text[..previous_end]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        self.cursor = byte_at_display_column(
            &self.text,
            previous_start,
            previous_end,
            self.cursor_display_column(),
        );
    }

    /// Moves one logical line downward while preserving display column when possible.
    pub fn move_down(&mut self) {
        let column = self.cursor_display_column();
        let current_end = self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |index| self.cursor + index);
        if current_end == self.text.len() {
            return;
        }
        let next_start = current_end + 1;
        let next_end = self.text[next_start..]
            .find('\n')
            .map_or(self.text.len(), |index| next_start + index);
        self.cursor = byte_at_display_column(&self.text, next_start, next_end, column);
    }

    /// Deletes one extended grapheme cluster before the cursor.
    pub fn delete_backward(&mut self) -> bool {
        let start = previous_grapheme_boundary(&self.text, self.cursor);
        if start == self.cursor {
            return false;
        }
        self.record_undo();
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.reset_history_navigation();
        true
    }

    /// Deletes one extended grapheme cluster after the cursor.
    pub fn delete_forward(&mut self) -> bool {
        let end = next_grapheme_boundary(&self.text, self.cursor);
        if end == self.cursor {
            return false;
        }
        self.record_undo();
        self.text.replace_range(self.cursor..end, "");
        self.reset_history_navigation();
        true
    }

    /// Deletes the preceding Unicode word and intervening whitespace.
    pub fn delete_word_backward(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.kill_range(self.previous_word_start()..self.cursor)
    }

    /// Deletes the following Unicode word and leading separators.
    pub fn delete_word_forward(&mut self) -> bool {
        let end = self.next_word_end();
        self.kill_range(self.cursor..end)
    }

    /// Deletes from the cursor to the logical line start and stores it for yanking.
    pub fn kill_to_line_start(&mut self) -> bool {
        let start = self.current_line_start();
        let start = if self.cursor == start && start > 0 {
            start - 1
        } else {
            start
        };
        self.kill_range(start..self.cursor)
    }

    /// Deletes from the cursor to the logical line end and stores it for yanking.
    pub fn kill_to_line_end(&mut self) -> bool {
        let end = self.current_line_end();
        let end = if self.cursor == end && end < self.text.len() {
            end + 1
        } else {
            end
        };
        self.kill_range(self.cursor..end)
    }

    /// Returns whether a previous word or line kill can be yanked.
    #[must_use]
    pub fn has_kill_buffer(&self) -> bool {
        !self.kill_buffer.is_empty()
    }

    /// Inserts the most recently killed text at the cursor.
    ///
    /// # Errors
    ///
    /// Rejects insertion when it would exceed the configured byte bound.
    pub fn yank(&mut self) -> Result<bool, EditorError> {
        if self.kill_buffer.is_empty() {
            return Ok(false);
        }
        let killed = self.kill_buffer.clone();
        self.insert_paste(&killed)?;
        Ok(true)
    }

    /// Clears the editor as one undoable mutation.
    pub fn clear(&mut self) -> bool {
        if self.text.is_empty() {
            return false;
        }
        self.record_undo();
        self.text.clear();
        self.cursor = 0;
        self.reset_history_navigation();
        true
    }

    /// Restores the most recent mutation baseline.
    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop_back() else {
            return false;
        };
        self.text = snapshot.text;
        self.cursor = snapshot.cursor;
        self.reset_history_navigation();
        true
    }

    /// Submits non-whitespace contents, records history, and clears the editor.
    pub fn submit(&mut self) -> Option<String> {
        if self.text.trim().is_empty() {
            return None;
        }
        let submitted = std::mem::take(&mut self.text);
        self.cursor = 0;
        if self.history.back() != Some(&submitted) {
            if self.history.len() == MAX_EDITOR_HISTORY {
                self.history.pop_front();
            }
            self.history.push_back(submitted.clone());
        }
        self.undo.clear();
        self.reset_history_navigation();
        Some(submitted)
    }

    /// Replaces the current draft with the previous submitted history item.
    pub fn previous_history(&mut self) -> bool {
        if self.history.is_empty() {
            return false;
        }
        let index = self
            .history_index
            .map_or(self.history.len() - 1, |index| index.saturating_sub(1));
        if self.history_index.is_none() {
            self.history_draft = Some(EditorSnapshot {
                text: self.text.clone(),
                cursor: self.cursor,
            });
        }
        self.history_index = Some(index);
        self.text.clone_from(&self.history[index]);
        self.cursor = self.text.len();
        true
    }

    /// Moves toward the current draft in submitted history navigation.
    pub fn next_history(&mut self) -> bool {
        let Some(index) = self.history_index else {
            return false;
        };
        if index + 1 < self.history.len() {
            self.history_index = Some(index + 1);
            self.text.clone_from(&self.history[index + 1]);
            self.cursor = self.text.len();
        } else {
            let draft = self.history_draft.take().unwrap_or(EditorSnapshot {
                text: String::new(),
                cursor: 0,
            });
            self.text = draft.text;
            self.cursor = draft.cursor;
            self.history_index = None;
        }
        true
    }

    fn record_undo(&mut self) {
        if self.undo.len() == MAX_UNDO_STATES {
            self.undo.pop_front();
        }
        self.undo.push_back(EditorSnapshot {
            text: self.text.clone(),
            cursor: self.cursor,
        });
    }

    fn reset_history_navigation(&mut self) {
        self.history_index = None;
        self.history_draft = None;
    }

    fn current_line_start(&self) -> usize {
        self.text[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1)
    }

    fn current_line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |index| self.cursor + index)
    }

    fn previous_word_start(&self) -> usize {
        let prefix = &self.text[..self.cursor];
        let Some((last_index, character)) = prefix
            .char_indices()
            .rev()
            .find(|(_, character)| !character.is_whitespace())
        else {
            return 0;
        };
        let run_start = prefix[..last_index]
            .char_indices()
            .rev()
            .find(|(_, character)| character.is_whitespace())
            .map_or(0, |(index, character)| index + character.len_utf8());
        let run_end = last_index + character.len_utf8();
        let mut pieces = split_word_pieces(&prefix[run_start..run_end])
            .into_iter()
            .rev()
            .peekable();
        let Some((piece_start, piece)) = pieces.next() else {
            return run_start;
        };
        let mut start = run_start + piece_start;
        if piece.chars().all(is_word_separator) {
            while let Some((index, piece)) = pieces.peek() {
                if !piece.chars().all(is_word_separator) {
                    break;
                }
                start = run_start + *index;
                pieces.next();
            }
        }
        start
    }

    fn next_word_end(&self) -> usize {
        let suffix = &self.text[self.cursor..];
        let Some(first_non_whitespace) = suffix.find(|character: char| !character.is_whitespace())
        else {
            return self.text.len();
        };
        let run = &suffix[first_non_whitespace..];
        let run = &run[..run.find(char::is_whitespace).unwrap_or(run.len())];
        let mut pieces = split_word_pieces(run).into_iter().peekable();
        let Some((start, piece)) = pieces.next() else {
            return self.cursor + first_non_whitespace;
        };
        let word_start = self.cursor + first_non_whitespace + start;
        let mut end = word_start + piece.len();
        if piece.chars().all(is_word_separator) {
            while let Some((index, piece)) = pieces.peek() {
                if !piece.chars().all(is_word_separator) {
                    break;
                }
                end = self.cursor + first_non_whitespace + *index + piece.len();
                pieces.next();
            }
        }
        end
    }

    fn kill_range(&mut self, range: Range<usize>) -> bool {
        if range.is_empty() {
            return false;
        }
        self.record_undo();
        self.kill_buffer = self.text[range.clone()].to_owned();
        self.text.replace_range(range.clone(), "");
        self.cursor = range.start;
        self.reset_history_navigation();
        true
    }
}

fn is_word_separator(character: char) -> bool {
    WORD_SEPARATORS.contains(character)
}

fn split_word_pieces(run: &str) -> Vec<(usize, &str)> {
    let mut pieces = Vec::new();
    for (segment_start, segment) in run.split_word_bound_indices() {
        let mut piece_start = 0;
        let mut characters = segment.char_indices();
        let Some((_, first_character)) = characters.next() else {
            continue;
        };
        let mut in_separator = is_word_separator(first_character);
        for (index, character) in characters {
            let separator = is_word_separator(character);
            if separator == in_separator {
                continue;
            }
            pieces.push((segment_start + piece_start, &segment[piece_start..index]));
            piece_start = index;
            in_separator = separator;
        }
        pieces.push((segment_start + piece_start, &segment[piece_start..]));
    }
    pieces
}

fn previous_grapheme_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor]
        .grapheme_indices(true)
        .next_back()
        .map_or(cursor, |(index, _)| index)
}

fn next_grapheme_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .graphemes(true)
        .next()
        .map_or(cursor, |grapheme| cursor + grapheme.len())
}

fn byte_at_display_column(text: &str, start: usize, end: usize, target: usize) -> usize {
    let mut width = 0_usize;
    let mut cursor = start;
    for grapheme in text[start..end].graphemes(true) {
        let next = width.saturating_add(grapheme.width());
        if next > target {
            break;
        }
        width = next;
        cursor += grapheme.len();
    }
    cursor
}
