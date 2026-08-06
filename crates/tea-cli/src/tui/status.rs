use std::fmt::Write as _;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use tea_protocol::Usage;
use unicode_width::UnicodeWidthChar as _;

use super::components::{Element, RenderContext, Text};
use super::layout::{Renderable, draw_lines};
use super::render_output::RenderedLine;
use super::state::{ModelRetryView, ToolProgressView, ToolView, TuiState};
use super::theme::Theme;

/// Bounded, observable task state placed immediately above the composer.
pub(crate) struct StatusIndicator {
    lines: Vec<RenderedLine>,
}

impl StatusIndicator {
    #[must_use]
    pub(crate) fn from_state(state: &TuiState, width: usize, theme: &Theme) -> Option<Self> {
        let view = status_view(state, theme.status_marker(state.run_elapsed_seconds))?;
        let mut lines = Vec::with_capacity(4);
        lines.push(blank_line());
        match view {
            StatusView::Activity { primary, detail } => {
                append_text(&mut lines, &primary, width, theme.thinking, theme);
                if let Some(detail) = detail {
                    append_text(
                        &mut lines,
                        &format!("  └ {detail}"),
                        width,
                        theme.footer,
                        theme,
                    );
                }
            }
            StatusView::Completed {
                elapsed_seconds,
                usage,
            } => {
                lines.push(worked_for_line(elapsed_seconds, width, theme.footer));
                if let Some(usage) = usage {
                    append_text(
                        &mut lines,
                        &format!("  └ {}", format_usage(usage)),
                        width,
                        theme.footer,
                        theme,
                    );
                }
            }
        }
        lines.push(blank_line());
        Some(Self { lines })
    }

    #[must_use]
    pub(crate) fn lines(&self) -> &[RenderedLine] {
        &self.lines
    }

    #[must_use]
    pub(crate) const fn needs_elapsed_tick(state: &TuiState) -> bool {
        state.running
    }
}

impl Renderable for StatusIndicator {
    fn desired_height(&self, _width: u16) -> u16 {
        u16::try_from(self.lines.len()).unwrap_or(u16::MAX)
    }

    fn render(&self, area: Rect, buffer: &mut Buffer) {
        draw_lines(&self.lines, area, buffer);
    }
}

enum StatusView<'a> {
    Activity {
        primary: String,
        detail: Option<String>,
    },
    Completed {
        elapsed_seconds: u64,
        usage: Option<&'a Usage>,
    },
}

fn status_view<'a>(state: &'a TuiState, marker: &str) -> Option<StatusView<'a>> {
    if state.resyncing {
        return Some(StatusView::Activity {
            primary: format!("{marker} Syncing session"),
            detail: None,
        });
    }
    if let Some(approval) = state.approval.as_ref() {
        return Some(StatusView::Activity {
            primary: format!(
                "{marker} Waiting for approval ({})",
                fmt_elapsed_compact(state.run_elapsed_seconds)
            ),
            detail: Some(approval.tool_name.clone()),
        });
    }
    if let Some(retry) = state.model_retry.as_ref() {
        return Some(StatusView::Activity {
            primary: format!("{marker} {}", format_retry(retry)),
            detail: None,
        });
    }
    if state.running {
        if let Some(tool) = running_tool(state) {
            return Some(StatusView::Activity {
                primary: format!(
                    "{marker} Running {} ({}, esc to interrupt)",
                    tool.tool_name,
                    fmt_elapsed_compact(state.run_elapsed_seconds),
                ),
                detail: tool.progress.as_ref().map(progress_text),
            });
        }
        return Some(StatusView::Activity {
            primary: format!(
                "{marker} Working ({}, esc to interrupt)",
                fmt_elapsed_compact(state.run_elapsed_seconds),
            ),
            detail: None,
        });
    }
    if matches!(state.run_status, Some(tea_protocol::RunStatus::Completed)) {
        return Some(StatusView::Completed {
            elapsed_seconds: state.run_elapsed_seconds,
            usage: state.usage.as_ref(),
        });
    }
    None
}

fn blank_line() -> RenderedLine {
    RenderedLine::new(String::new(), Style::default())
}

fn append_text(
    output: &mut Vec<RenderedLine>,
    text: &str,
    width: usize,
    style: Style,
    theme: &Theme,
) {
    let context = RenderContext { theme };
    output.extend(
        Element::new(Text::new(text, style))
            .render(&context, u16::try_from(width).unwrap_or(u16::MAX).max(1))
            .into_lines(),
    );
}

fn worked_for_line(elapsed_seconds: u64, width: usize, style: Style) -> RenderedLine {
    let label = format!("─ Worked for {} ─", fmt_elapsed_compact(elapsed_seconds));
    let mut text = String::with_capacity(width);
    let mut used: usize = 0;
    for character in label.chars() {
        let character_width = character.width().unwrap_or(0);
        if used.saturating_add(character_width) > width {
            break;
        }
        text.push(character);
        used = used.saturating_add(character_width);
    }
    text.push_str(&"─".repeat(width.saturating_sub(used)));
    RenderedLine::new(text, style)
}

fn format_usage(usage: &Usage) -> String {
    let cached = usage
        .cache_read_tokens()
        .map_or(0, tea_protocol::TokenCount::get);
    let billable = usage
        .input_tokens()
        .get()
        .saturating_sub(cached)
        .saturating_add(usage.output_tokens().get());
    let mut text = format!("{} tokens", format_number(billable));
    if cached > 0 {
        write!(text, " (+ {} cached)", format_number(cached))
            .expect("writing to a String cannot fail");
    }
    text
}

fn format_number(value: u64) -> String {
    let mut formatted = value.to_string();
    let mut index = formatted.len();
    while index > 3 {
        index -= 3;
        formatted.insert(index, ',');
    }
    formatted
}

fn format_retry(retry: &ModelRetryView) -> String {
    format!(
        "Retrying ({}/{}) in {}s (esc to interrupt)",
        retry.attempt,
        retry.max_retries,
        retry.remaining_seconds()
    )
}

fn fmt_elapsed_compact(elapsed_seconds: u64) -> String {
    if elapsed_seconds < 60 {
        return format!("{elapsed_seconds}s");
    }
    if elapsed_seconds < 3600 {
        let minutes = elapsed_seconds / 60;
        let seconds = elapsed_seconds % 60;
        return format!("{minutes}m {seconds:02}s");
    }
    let hours = elapsed_seconds / 3600;
    let minutes = (elapsed_seconds % 3600) / 60;
    let seconds = elapsed_seconds % 60;
    format!("{hours}h {minutes:02}m {seconds:02}s")
}

fn running_tool(state: &TuiState) -> Option<&ToolView> {
    state
        .tool_order
        .iter()
        .rev()
        .filter_map(|tool_call_id| state.tools.get(tool_call_id))
        .find(|tool| tool.status == "running")
        .or_else(|| state.tools.values().find(|tool| tool.status == "running"))
}

fn progress_text(progress: &ToolProgressView) -> String {
    let units = progress.total_units.map_or_else(
        || progress.completed_units.to_string(),
        |total| format!("{}/{}", progress.completed_units, total),
    );
    format!("{units} {}", progress.message)
}

#[cfg(test)]
mod tests {
    use tea_protocol::{TokenCount, Usage};

    use super::{fmt_elapsed_compact, format_retry, format_usage};
    use crate::tui::ModelRetryView;

    #[test]
    fn elapsed_time_matches_codex_compact_format() {
        assert_eq!(fmt_elapsed_compact(0), "0s");
        assert_eq!(fmt_elapsed_compact(59), "59s");
        assert_eq!(fmt_elapsed_compact(60), "1m 00s");
        assert_eq!(fmt_elapsed_compact(125), "2m 05s");
        assert_eq!(fmt_elapsed_compact(3661), "1h 01m 01s");
    }

    #[test]
    fn usage_reports_non_cached_total_and_cached_breakdown() {
        let usage = Usage::new(
            TokenCount::new(12_300).unwrap(),
            TokenCount::new(456).unwrap(),
        )
        .with_cache_read(TokenCount::new(2_300).unwrap());

        assert_eq!(format_usage(&usage), "10,456 tokens (+ 2,300 cached)");
    }

    #[test]
    fn retry_status_shows_bounded_countdown_and_cancel_hint() {
        let mut retry = ModelRetryView::new(
            "0195a0b1-7e00-7000-8000-000000000001".parse().unwrap(),
            1,
            3,
            2_000,
        );
        assert_eq!(
            format_retry(&retry),
            "Retrying (1/3) in 2s (esc to interrupt)"
        );
        retry.advance(3);
        assert_eq!(
            format_retry(&retry),
            "Retrying (1/3) in 0s (esc to interrupt)"
        );
    }
}
