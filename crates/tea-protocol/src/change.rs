use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use crate::WebFetchPresentation;

/// Maximum UTF-8 byte length of a workspace-relative changed-file path.
pub const MAX_CODE_CHANGE_PATH_BYTES: usize = 4_096;
/// Maximum hunks retained for one code-change presentation.
pub const MAX_CODE_CHANGE_HUNKS: usize = 32;
/// Maximum lines retained in one hunk.
pub const MAX_CODE_CHANGE_LINES_PER_HUNK: usize = 128;
/// Maximum lines retained across one code-change presentation.
pub const MAX_CODE_CHANGE_LINES: usize = 1_024;
/// Maximum UTF-8 byte length of one retained source line.
pub const MAX_CODE_CHANGE_LINE_BYTES: usize = 1_024;
/// Maximum UTF-8 byte length of an optional unified patch.
pub const MAX_CODE_CHANGE_PATCH_BYTES: usize = 64 * 1024;

/// UI-only presentation attached to a tool result or preview.
///
/// Successful results persist this data with the tool execution record. Preview
/// events remain ephemeral. Both forms stay separate from model-visible
/// tool-result content.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ToolPresentation {
    /// A bounded file change with structured diff hunks.
    CodeChange(CodeChange),
    /// A bounded normalized client web-fetch result.
    WebFetch(Box<WebFetchPresentation>),
}

impl ToolPresentation {
    /// Returns the structured code change when this is a code-change presentation.
    #[must_use]
    pub const fn code_change(&self) -> Option<&CodeChange> {
        match self {
            Self::CodeChange(change) => Some(change),
            Self::WebFetch(_) => None,
        }
    }

    /// Returns the normalized fetch result when this is a web-fetch presentation.
    #[must_use]
    pub fn web_fetch(&self) -> Option<&WebFetchPresentation> {
        match self {
            Self::CodeChange(_) => None,
            Self::WebFetch(fetch) => Some(fetch.as_ref()),
        }
    }
}

/// Kind of file change represented by a code-change presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeChangeKind {
    /// A file was created.
    Create,
    /// An existing file was updated.
    Update,
    /// A file was deleted.
    Delete,
}

/// Reason a code-change presentation was deterministically truncated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeChangeTruncation {
    /// The configured hunk bound was reached.
    Hunks,
    /// The configured line bound was reached.
    Lines,
    /// A retained source line exceeded its byte bound.
    LineBytes,
    /// The optional unified patch exceeded its byte bound.
    PatchBytes,
}

/// Kind of one line in a code-change hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeChangeLineKind {
    /// Unchanged contextual source.
    Context,
    /// A line introduced by the new file content.
    Addition,
    /// A line removed from the old file content.
    Deletion,
}

/// One bounded, line-numbered source line in a diff hunk.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeChangeLine {
    kind: CodeChangeLineKind,
    old_line: Option<u32>,
    new_line: Option<u32>,
    text: String,
}

impl CodeChangeLine {
    /// Creates a validated diff line.
    ///
    /// # Errors
    ///
    /// Returns an error when the line kind, numbers, or text exceed the
    /// stable presentation bounds.
    pub fn new(
        kind: CodeChangeLineKind,
        old_line: Option<u32>,
        new_line: Option<u32>,
        text: impl Into<String>,
    ) -> Result<Self, CodeChangeValidationError> {
        let line = Self {
            kind,
            old_line,
            new_line,
            text: text.into(),
        };
        line.validate()?;
        Ok(line)
    }

    /// Returns the line kind.
    #[must_use]
    pub const fn kind(&self) -> CodeChangeLineKind {
        self.kind
    }

    /// Returns the one-based old-file line number when applicable.
    #[must_use]
    pub const fn old_line(&self) -> Option<u32> {
        self.old_line
    }

    /// Returns the one-based new-file line number when applicable.
    #[must_use]
    pub const fn new_line(&self) -> Option<u32> {
        self.new_line
    }

    /// Returns source text without its line ending.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    fn validate(&self) -> Result<(), CodeChangeValidationError> {
        if self.text.len() > MAX_CODE_CHANGE_LINE_BYTES {
            return Err(CodeChangeValidationError::LineTooLong);
        }
        let positive = |line: Option<u32>| line.is_some_and(|line| line > 0);
        let valid_numbers = match self.kind {
            CodeChangeLineKind::Context => positive(self.old_line) && positive(self.new_line),
            CodeChangeLineKind::Addition => self.old_line.is_none() && positive(self.new_line),
            CodeChangeLineKind::Deletion => positive(self.old_line) && self.new_line.is_none(),
        };
        if valid_numbers {
            Ok(())
        } else {
            Err(CodeChangeValidationError::InvalidLineNumbers)
        }
    }
}

/// One grouped hunk of a code-change presentation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeChangeHunk {
    old_start: u32,
    old_lines: u32,
    new_start: u32,
    new_lines: u32,
    lines: Vec<CodeChangeLine>,
}

impl CodeChangeHunk {
    /// Creates a validated diff hunk.
    ///
    /// # Errors
    ///
    /// Returns an error when line counts or contained lines violate the
    /// durable presentation contract.
    pub fn new(
        old_start: u32,
        old_lines: u32,
        new_start: u32,
        new_lines: u32,
        lines: Vec<CodeChangeLine>,
    ) -> Result<Self, CodeChangeValidationError> {
        let hunk = Self {
            old_start,
            old_lines,
            new_start,
            new_lines,
            lines,
        };
        hunk.validate()?;
        Ok(hunk)
    }

    /// Returns the old-file hunk start, with zero permitted for an empty file.
    #[must_use]
    pub const fn old_start(&self) -> u32 {
        self.old_start
    }

    /// Returns the number of old-file lines covered by this hunk.
    #[must_use]
    pub const fn old_lines(&self) -> u32 {
        self.old_lines
    }

    /// Returns the new-file hunk start, with zero permitted for an empty file.
    #[must_use]
    pub const fn new_start(&self) -> u32 {
        self.new_start
    }

    /// Returns the number of new-file lines covered by this hunk.
    #[must_use]
    pub const fn new_lines(&self) -> u32 {
        self.new_lines
    }

    /// Returns ordered, line-numbered hunk lines.
    #[must_use]
    pub fn lines(&self) -> &[CodeChangeLine] {
        &self.lines
    }

    fn validate(&self) -> Result<(), CodeChangeValidationError> {
        if self.lines.is_empty() || self.lines.len() > MAX_CODE_CHANGE_LINES_PER_HUNK {
            return Err(CodeChangeValidationError::InvalidHunkLines);
        }
        for line in &self.lines {
            line.validate()?;
        }
        let old_start = self
            .lines
            .iter()
            .find_map(CodeChangeLine::old_line)
            .unwrap_or(0);
        let new_start = self
            .lines
            .iter()
            .find_map(CodeChangeLine::new_line)
            .unwrap_or(0);
        let old_lines = u32::try_from(
            self.lines
                .iter()
                .filter(|line| line.old_line().is_some())
                .count(),
        )
        .unwrap_or(u32::MAX);
        let new_lines = u32::try_from(
            self.lines
                .iter()
                .filter(|line| line.new_line().is_some())
                .count(),
        )
        .unwrap_or(u32::MAX);
        if (
            self.old_start,
            self.old_lines,
            self.new_start,
            self.new_lines,
        ) != (old_start, old_lines, new_start, new_lines)
        {
            return Err(CodeChangeValidationError::InconsistentHunkRange);
        }
        Ok(())
    }
}

/// Bounded, structured presentation of one changed file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CodeChange {
    path: String,
    kind: CodeChangeKind,
    hunks: Vec<CodeChangeHunk>,
    truncated: bool,
    truncation: Option<CodeChangeTruncation>,
    patch: Option<String>,
    first_changed_line: Option<u32>,
}

impl CodeChange {
    /// Creates a validated structured file change.
    ///
    /// # Errors
    ///
    /// Returns an error when presentation data exceeds its stable storage
    /// bounds or its truncation marker is inconsistent.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        path: impl Into<String>,
        kind: CodeChangeKind,
        hunks: Vec<CodeChangeHunk>,
        truncated: bool,
        truncation: Option<CodeChangeTruncation>,
        unified_patch: Option<String>,
        first_changed_line: Option<u32>,
    ) -> Result<Self, CodeChangeValidationError> {
        let change = Self {
            path: path.into(),
            kind,
            hunks,
            truncated,
            truncation,
            patch: unified_patch,
            first_changed_line,
        };
        change.validate()?;
        Ok(change)
    }

    /// Returns the workspace-relative path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the file-change kind.
    #[must_use]
    pub const fn kind(&self) -> CodeChangeKind {
        self.kind
    }

    /// Returns ordered structured hunks.
    #[must_use]
    pub fn hunks(&self) -> &[CodeChangeHunk] {
        &self.hunks
    }

    /// Returns whether any presentation bound caused truncation.
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    /// Returns the deterministic truncation reason when the change was truncated.
    #[must_use]
    pub const fn truncation(&self) -> Option<CodeChangeTruncation> {
        self.truncation
    }

    /// Returns the optional bounded unified patch.
    #[must_use]
    pub fn patch(&self) -> Option<&str> {
        self.patch.as_deref()
    }

    /// Returns the first changed one-based new-file line when available.
    #[must_use]
    pub const fn first_changed_line(&self) -> Option<u32> {
        self.first_changed_line
    }

    fn validate(&self) -> Result<(), CodeChangeValidationError> {
        if self.path.is_empty()
            || self.path.len() > MAX_CODE_CHANGE_PATH_BYTES
            || self.path.contains('\0')
        {
            return Err(CodeChangeValidationError::InvalidPath);
        }
        if self.hunks.len() > MAX_CODE_CHANGE_HUNKS {
            return Err(CodeChangeValidationError::TooManyHunks);
        }
        let mut total_lines = 0usize;
        for hunk in &self.hunks {
            hunk.validate()?;
            total_lines = total_lines.saturating_add(hunk.lines.len());
        }
        if total_lines > MAX_CODE_CHANGE_LINES {
            return Err(CodeChangeValidationError::TooManyLines);
        }
        if self
            .patch
            .as_ref()
            .is_some_and(|patch| patch.len() > MAX_CODE_CHANGE_PATCH_BYTES)
        {
            return Err(CodeChangeValidationError::PatchTooLong);
        }
        if self.first_changed_line.is_some_and(|line| line == 0) {
            return Err(CodeChangeValidationError::InvalidFirstChangedLine);
        }
        if self.truncated == self.truncation.is_none() {
            return Err(CodeChangeValidationError::InconsistentTruncation);
        }
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeChangeDef<'a> {
    path: &'a str,
    kind: CodeChangeKind,
    hunks: &'a [CodeChangeHunk],
    truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncation: Option<CodeChangeTruncation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    patch: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_changed_line: Option<u32>,
}

impl Serialize for CodeChange {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(serde::ser::Error::custom)?;
        CodeChangeDef {
            path: &self.path,
            kind: self.kind,
            hunks: &self.hunks,
            truncated: self.truncated,
            truncation: self.truncation,
            patch: self.patch.as_deref(),
            first_changed_line: self.first_changed_line,
        }
        .serialize(serializer)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawCodeChange {
    path: String,
    kind: CodeChangeKind,
    hunks: Vec<CodeChangeHunk>,
    truncated: bool,
    #[serde(default)]
    truncation: Option<CodeChangeTruncation>,
    #[serde(default)]
    patch: Option<String>,
    #[serde(default)]
    first_changed_line: Option<u32>,
}

impl<'de> Deserialize<'de> for CodeChange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawCodeChange::deserialize(deserializer)?;
        Self::new(
            raw.path,
            raw.kind,
            raw.hunks,
            raw.truncated,
            raw.truncation,
            raw.patch,
            raw.first_changed_line,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Validation failure for a structured code-change presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CodeChangeValidationError {
    /// File path is empty, contains a NUL, or exceeds its byte bound.
    #[error("code-change path is invalid")]
    InvalidPath,
    /// The hunk count exceeds its bound.
    #[error("code-change has too many hunks")]
    TooManyHunks,
    /// The total line count exceeds its bound.
    #[error("code-change has too many lines")]
    TooManyLines,
    /// One hunk has no lines or exceeds its line bound.
    #[error("code-change hunk line count is invalid")]
    InvalidHunkLines,
    /// One line exceeds its byte bound.
    #[error("code-change line is too long")]
    LineTooLong,
    /// One line's kind and old/new line numbers are inconsistent.
    #[error("code-change line numbers are invalid")]
    InvalidLineNumbers,
    /// Hunk start/count fields do not match their retained lines.
    #[error("code-change hunk range is inconsistent")]
    InconsistentHunkRange,
    /// The optional unified patch exceeds its byte bound.
    #[error("code-change patch is too long")]
    PatchTooLong,
    /// The optional first changed line must be one-based.
    #[error("code-change first changed line is invalid")]
    InvalidFirstChangedLine,
    /// Truncation must have exactly one reason when it is set.
    #[error("code-change truncation state is inconsistent")]
    InconsistentTruncation,
}
