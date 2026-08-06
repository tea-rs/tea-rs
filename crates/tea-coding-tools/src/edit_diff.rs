use similar::{ChangeTag, TextDiff};
use tea_protocol::{
    CodeChange, CodeChangeHunk, CodeChangeKind, CodeChangeLine, CodeChangeLineKind,
    CodeChangeTruncation, MAX_CODE_CHANGE_HUNKS, MAX_CODE_CHANGE_LINE_BYTES, MAX_CODE_CHANGE_LINES,
    MAX_CODE_CHANGE_LINES_PER_HUNK, MAX_CODE_CHANGE_PATCH_BYTES,
};

use crate::{FileToolError, FileToolErrorCode};

const CONTEXT_LINES: usize = 3;

pub(crate) fn code_change(
    file_path: &str,
    old_text: &str,
    new_text: &str,
    kind: CodeChangeKind,
) -> Result<CodeChange, FileToolError> {
    let diff = TextDiff::from_lines(old_text, new_text);
    let mut truncation = None;
    let mut retained_lines = 0usize;
    let mut hunks = Vec::new();

    'groups: for group in diff.grouped_ops(CONTEXT_LINES) {
        if hunks.len() == MAX_CODE_CHANGE_HUNKS {
            set_truncation(&mut truncation, CodeChangeTruncation::Hunks);
            break;
        }
        let mut lines = Vec::new();
        for operation in group {
            for change in diff.iter_changes(&operation) {
                if retained_lines == MAX_CODE_CHANGE_LINES
                    || lines.len() == MAX_CODE_CHANGE_LINES_PER_HUNK
                {
                    set_truncation(&mut truncation, CodeChangeTruncation::Lines);
                    break 'groups;
                }
                let text = bounded_line(
                    change.value().trim_end_matches(['\r', '\n']),
                    &mut truncation,
                );
                let (kind, old_line, new_line) = match change.tag() {
                    ChangeTag::Equal => (
                        CodeChangeLineKind::Context,
                        one_based(change.old_index()),
                        one_based(change.new_index()),
                    ),
                    ChangeTag::Insert => (
                        CodeChangeLineKind::Addition,
                        None,
                        one_based(change.new_index()),
                    ),
                    ChangeTag::Delete => (
                        CodeChangeLineKind::Deletion,
                        one_based(change.old_index()),
                        None,
                    ),
                };
                lines.push(
                    CodeChangeLine::new(kind, old_line, new_line, text)
                        .map_err(|_| FileToolError::new(FileToolErrorCode::Internal))?,
                );
                retained_lines = retained_lines.saturating_add(1);
            }
        }
        if !lines.is_empty() {
            hunks.push(hunk(lines)?);
        }
    }

    let unified_patch = diff
        .unified_diff()
        .context_radius(CONTEXT_LINES)
        .header(file_path, file_path)
        .to_string();
    let unified_patch = if unified_patch.len() <= MAX_CODE_CHANGE_PATCH_BYTES {
        (!unified_patch.is_empty()).then_some(unified_patch)
    } else {
        set_truncation(&mut truncation, CodeChangeTruncation::PatchBytes);
        None
    };
    let first_changed_line = hunks
        .iter()
        .flat_map(CodeChangeHunk::lines)
        .find(|line| line.kind() != CodeChangeLineKind::Context)
        .and_then(|line| line.new_line().or(line.old_line()));

    CodeChange::new(
        file_path,
        kind,
        hunks,
        truncation.is_some(),
        truncation,
        unified_patch,
        first_changed_line,
    )
    .map_err(|_| FileToolError::new(FileToolErrorCode::Internal))
}

fn hunk(lines: Vec<CodeChangeLine>) -> Result<CodeChangeHunk, FileToolError> {
    let old_start = lines.iter().find_map(CodeChangeLine::old_line).unwrap_or(0);
    let new_start = lines.iter().find_map(CodeChangeLine::new_line).unwrap_or(0);
    let old_lines = lines
        .iter()
        .filter(|line| line.old_line().is_some())
        .count();
    let new_lines = lines
        .iter()
        .filter(|line| line.new_line().is_some())
        .count();
    CodeChangeHunk::new(
        old_start,
        u32::try_from(old_lines).unwrap_or(u32::MAX),
        new_start,
        u32::try_from(new_lines).unwrap_or(u32::MAX),
        lines,
    )
    .map_err(|_| FileToolError::new(FileToolErrorCode::Internal))
}

fn one_based(index: Option<usize>) -> Option<u32> {
    index.map(|index| u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX))
}

fn bounded_line(value: &str, truncation: &mut Option<CodeChangeTruncation>) -> String {
    if value.len() <= MAX_CODE_CHANGE_LINE_BYTES {
        return value.to_owned();
    }
    set_truncation(truncation, CodeChangeTruncation::LineBytes);
    let limit = MAX_CODE_CHANGE_LINE_BYTES.saturating_sub(3);
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}...", &value[..end])
}

fn set_truncation(truncation: &mut Option<CodeChangeTruncation>, candidate: CodeChangeTruncation) {
    if truncation.is_none() {
        *truncation = Some(candidate);
    }
}
