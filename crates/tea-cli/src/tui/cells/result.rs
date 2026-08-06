use ratatui::style::Style;

use super::super::components::{
    ActionHeader, Component, Element, Markdown, RenderBlock, RenderContext, Text,
    truncate_cells_with_ellipsis,
};
use super::super::presentation::{OutputFormat, ResultCell};
use super::super::render_output::{RenderedLine, RenderedSpan};
use super::host::{CellContext, CellPresenter};
use super::style::{CellRole, CellStyle, CellVisualState};

const TERMINAL_PREVIEW_LINES_PER_END: usize = 5;

impl CellPresenter for ResultCell {
    fn role(&self) -> CellRole {
        CellRole::Result
    }

    fn visual_state(&self) -> CellVisualState {
        if self.is_error() {
            CellVisualState::Error
        } else {
            CellVisualState::Success
        }
    }

    fn content<'a>(&'a self, ctx: &'a CellContext<'a>, resolved_style: CellStyle) -> Element<'a> {
        Element::new(ResultView {
            action: self.action(),
            source_name: self.source_name(),
            content: self.content(),
            format: self.format(),
            is_error: self.is_error(),
            primary_style: resolved_style.foreground.resolve(ctx.theme),
        })
    }
}

struct ResultView<'a> {
    action: &'a str,
    source_name: Option<&'a str>,
    content: &'a str,
    format: OutputFormat,
    is_error: bool,
    primary_style: Style,
}

impl Component for ResultView<'_> {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        let marker = if self.is_error { "■" } else { "•" };
        let target_is_inline = ActionHeader::new(
            marker,
            self.action,
            self.source_name,
            self.primary_style,
            ctx.theme.footer,
        )
        .target_is_inline(width);
        let header = ActionHeader::new(
            marker,
            self.action,
            target_is_inline.then_some(self.source_name).flatten(),
            self.primary_style,
            ctx.theme.footer,
        );
        let mut lines = Element::new(header).render(ctx, width).into_lines();
        let overflow_source = (!target_is_inline).then_some(self.source_name).flatten();
        let body_index = usize::from(overflow_source.is_some());
        let total = body_index.saturating_add(1);
        if let Some(source_name) = overflow_source {
            lines.extend(
                render_tree_text(
                    source_name,
                    0,
                    total,
                    ctx.theme.footer,
                    ctx.theme.footer,
                    ctx,
                    width,
                )
                .into_lines(),
            );
        }
        lines.extend(
            ResultBody {
                content: self.content,
                format: self.format,
                index: body_index,
                total,
                style: if self.is_error {
                    ctx.theme.error
                } else {
                    ctx.theme.footer
                },
            }
            .render(ctx, width)
            .into_lines(),
        );
        RenderBlock::from_lines(lines)
    }
}

struct ResultBody<'a> {
    content: &'a str,
    format: OutputFormat,
    index: usize,
    total: usize,
    style: Style,
}

impl Component for ResultBody<'_> {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        let content = if self.content.is_empty() {
            "(no output)"
        } else {
            self.content
        };
        match self.format {
            OutputFormat::Terminal => {
                render_terminal_preview(content, self.index, self.total, self.style, ctx, width)
            }
            OutputFormat::Markdown => render_markdown_result(content, ctx, width),
            OutputFormat::Plain => render_tree_text(
                content,
                self.index,
                self.total,
                self.style,
                ctx.theme.footer,
                ctx,
                width,
            ),
        }
    }
}

fn render_markdown_result(content: &str, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
    let body_width = width.saturating_sub(4).max(1);
    let body = Element::new(Markdown::new(content))
        .render(ctx, body_width)
        .into_lines();
    if width <= 4 {
        return RenderBlock::from_lines(body);
    }
    RenderBlock::from_lines(
        body.into_iter()
            .enumerate()
            .map(|(index, line)| {
                prefix_line(
                    line,
                    if index == 0 { "  └ " } else { "    " },
                    ctx.theme.footer,
                )
            })
            .collect(),
    )
}

fn render_terminal_preview(
    content: &str,
    index: usize,
    total: usize,
    style: Style,
    ctx: &RenderContext<'_>,
    width: u16,
) -> RenderBlock {
    let mut section_style = style;
    let mut lines = Vec::new();
    let source_lines = content.lines().collect::<Vec<_>>();
    for (line_index, line) in source_lines.iter().copied().enumerate() {
        let text = if let Some(rest) = line.strip_prefix("stdout:") {
            section_style = ctx.theme.normal;
            rest.trim_start()
        } else if let Some(rest) = line.strip_prefix("stderr:") {
            section_style = ctx.theme.error;
            rest.trim_start()
        } else {
            line
        };
        let generated_empty_stream = text == "(empty)"
            && line_index
                .checked_sub(1)
                .and_then(|index| source_lines.get(index))
                .is_some_and(|line| matches!(*line, "stdout:" | "stderr:"))
            && source_lines
                .get(line_index.saturating_add(1))
                .is_none_or(|line| matches!(*line, "stdout:" | "stderr:"));
        if !text.is_empty() && !generated_empty_stream {
            lines.push((text, section_style));
        }
    }
    if lines.is_empty() {
        return render_tree_text(content, index, total, style, ctx.theme.footer, ctx, width);
    }

    let head_len = lines.len().min(TERMINAL_PREVIEW_LINES_PER_END);
    let tail_len = lines
        .len()
        .saturating_sub(head_len)
        .min(TERMINAL_PREVIEW_LINES_PER_END);
    let omitted = lines.len().saturating_sub(head_len + tail_len);
    let mut rendered = Vec::new();
    let mut first = true;
    for &(text, line_style) in &lines[..head_len] {
        append_terminal_preview_line(
            &mut rendered,
            text,
            line_style,
            first,
            index,
            total,
            ctx,
            width,
        );
        first = false;
    }
    if omitted > 0 {
        append_terminal_preview_line(
            &mut rendered,
            &format!("... +{omitted} lines"),
            ctx.theme.footer,
            first,
            index,
            total,
            ctx,
            width,
        );
        first = false;
    }
    for &(text, line_style) in &lines[lines.len().saturating_sub(tail_len)..] {
        append_terminal_preview_line(
            &mut rendered,
            text,
            line_style,
            first,
            index,
            total,
            ctx,
            width,
        );
        first = false;
    }
    RenderBlock::from_lines(rendered)
}

#[allow(clippy::too_many_arguments)]
fn append_terminal_preview_line(
    output: &mut Vec<RenderedLine>,
    text: &str,
    style: Style,
    first: bool,
    index: usize,
    total: usize,
    ctx: &RenderContext<'_>,
    width: u16,
) {
    let body_width = usize::from(width.saturating_sub(4));
    let text = if body_width == 0 {
        String::new()
    } else {
        truncate_cells_with_ellipsis(text, body_width)
    };
    if first {
        output.extend(
            render_tree_text(&text, index, total, style, ctx.theme.footer, ctx, width).into_lines(),
        );
    } else {
        output.extend(
            Element::new(
                Text::new(text, style)
                    .with_prefixes("    ", "    ")
                    .with_prefix_style(ctx.theme.footer),
            )
            .render(ctx, width)
            .into_lines(),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn render_tree_text(
    text: &str,
    index: usize,
    total: usize,
    style: Style,
    prefix_style: Style,
    ctx: &RenderContext<'_>,
    width: u16,
) -> RenderBlock {
    let last = index.saturating_add(1) == total;
    let (first, continuation) = if last {
        ("  └ ", "    ")
    } else {
        ("  ├ ", "  │ ")
    };
    Element::new(
        Text::new(text, style)
            .with_prefixes(first, continuation)
            .with_prefix_style(prefix_style),
    )
    .render(ctx, width)
}

fn prefix_line(line: RenderedLine, prefix: &str, prefix_style: Style) -> RenderedLine {
    let mut spans = Vec::with_capacity(line.spans.len().saturating_add(1));
    spans.push(RenderedSpan::new(prefix.to_owned(), prefix_style));
    if line.spans.is_empty() {
        spans.push(RenderedSpan::new(line.text, line.style));
    } else {
        spans.extend(line.spans);
    }
    RenderedLine::from_spans(line.style, spans)
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr as _;

    use super::super::host::{CellContext, CellHost, CellPresenter, CellSpec};
    use super::super::style::CellVisualState;
    use crate::tui::Theme;
    use crate::tui::presentation::{OutputFormat, ResultCell};

    #[test]
    fn result_state_and_empty_output_fallback_are_preserved() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let success = ResultCell::new("Returned", None, "", OutputFormat::Plain, false);
        let failure = ResultCell::new("Failed", None, "boom", OutputFormat::Plain, true);

        assert_eq!(success.visual_state(), CellVisualState::Success);
        assert_eq!(failure.visual_state(), CellVisualState::Error);

        let rendered = CellHost::default().render(CellSpec::new(&success), &context, 32);
        assert_eq!(
            rendered
                .lines()
                .iter()
                .map(|line| line.text().trim_end())
                .collect::<Vec<_>>(),
            ["• Returned", "  └ (no output)"]
        );
    }

    #[test]
    fn plain_markdown_and_terminal_results_retain_their_formats() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let host = CellHost::default();
        let plain = ResultCell::new("Returned", None, "plain text", OutputFormat::Plain, false);
        let markdown = ResultCell::new(
            "Returned",
            None,
            "**bold text**",
            OutputFormat::Markdown,
            false,
        );
        let terminal = ResultCell::new(
            "Returned",
            Some("cargo test"),
            "exit 0\nstdout:\n39 passed\nstderr:\nwarning",
            OutputFormat::Terminal,
            false,
        );

        let plain = host.render(CellSpec::new(&plain), &context, 48);
        let markdown = host.render(CellSpec::new(&markdown), &context, 48);
        let terminal = host.render(CellSpec::new(&terminal), &context, 48);

        assert_eq!(plain.lines()[1].text().trim_end(), "  └ plain text");
        assert!(markdown.lines()[1].rendered_spans().iter().any(|span| {
            span.style()
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        }));
        assert_eq!(terminal.lines()[1].style(), theme.footer);
        assert_eq!(terminal.lines()[2].style(), theme.normal);
        assert_eq!(terminal.lines()[3].style(), theme.error);
    }

    #[test]
    fn terminal_preview_keeps_bounded_head_and_tail_at_narrow_widths() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let terminal_output = (1..=14)
            .map(|line| format!("line {line} {}", "wide-token-界".repeat(3)))
            .collect::<Vec<_>>()
            .join("\n");
        let cell = ResultCell::new(
            "Returned",
            Some("process"),
            &terminal_output,
            OutputFormat::Terminal,
            false,
        );

        let rendered = CellHost::default().render(CellSpec::new(&cell), &context, 20);
        let text = rendered
            .lines()
            .iter()
            .map(crate::tui::render_output::RenderedLine::text)
            .collect::<Vec<_>>();

        assert_eq!(text.len(), 12);
        assert!(text[1].starts_with("  └ line 1"));
        assert_eq!(text[6], "    ... +4 lines");
        assert!(text[11].starts_with("    line 14"));
        assert!(text.iter().all(|line| line.width() <= 18));
    }
}
