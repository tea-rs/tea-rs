use ratatui::style::{Color, Style};
use tea_protocol::{CodeChange, CodeChangeHunk, CodeChangeLineKind, CodeChangeTruncation};
use unicode_width::UnicodeWidthStr as _;

use super::super::components::{
    ActionHeader, Component, Element, RenderBlock, RenderContext, Text, truncate_cells,
};
use super::super::presentation::{DiffCell, terminal_safe_text};
use super::super::render_output::{RenderedLine, RenderedSpan};
use super::super::terminal::ColorCapability;
use super::host::{CellContext, CellPresenter};
use super::style::{CellRole, CellStyle, CellVisualState};

impl CellPresenter for DiffCell {
    fn role(&self) -> CellRole {
        CellRole::Diff
    }

    fn visual_state(&self) -> CellVisualState {
        CellVisualState::Success
    }

    fn content<'a>(&'a self, ctx: &'a CellContext<'a>, resolved_style: CellStyle) -> Element<'a> {
        Element::new(DiffView {
            action: self.action(),
            change: self.change(),
            primary_style: resolved_style.foreground.resolve(ctx.theme),
        })
    }
}

struct DiffView<'a> {
    action: &'a str,
    change: &'a CodeChange,
    primary_style: Style,
}

impl DiffView<'_> {
    fn render_header(&self, ctx: &RenderContext<'_>, width: u16) -> Vec<RenderedLine> {
        let action = terminal_safe_text(self.action);
        let path = terminal_safe_text(self.change.path());
        let header_margin = 1_u16.min(width.saturating_sub(1) / 2);
        let header_width = width.saturating_sub(header_margin.saturating_mul(2)).max(1);
        let target_is_inline = ActionHeader::new(
            "•",
            &action,
            Some(&path),
            self.primary_style,
            ctx.theme.footer,
        )
        .target_is_inline(header_width);
        let header = ActionHeader::new(
            "•",
            &action,
            target_is_inline.then_some(path.as_str()),
            self.primary_style,
            ctx.theme.footer,
        );
        let mut lines = Element::new(header).render(ctx, header_width).into_lines();
        if !target_is_inline {
            lines.extend(
                Element::new(Text::single_line(&path, ctx.theme.footer))
                    .render(ctx, header_width)
                    .into_lines(),
            );
        }
        lines
            .into_iter()
            .map(|line| line.with_left_columns(usize::from(header_margin)))
            .collect()
    }

    fn line_number_width(&self) -> usize {
        self.change
            .hunks()
            .iter()
            .flat_map(CodeChangeHunk::lines)
            .filter_map(|line| match line.kind() {
                CodeChangeLineKind::Deletion => line.old_line(),
                CodeChangeLineKind::Addition | CodeChangeLineKind::Context => {
                    line.new_line().or(line.old_line())
                }
            })
            .max()
            .map_or(1, |line| line.to_string().len())
    }
}

impl Component for DiffView<'_> {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        let mut lines = self.render_header(ctx, width);
        let line_number_width = self.line_number_width();
        let mut rendered_details = false;
        for hunk in self.change.hunks() {
            rendered_details = true;
            lines.extend(
                Element::new(Text::single_line(
                    format!(
                        "@@ -{},{} +{},{} @@",
                        hunk.old_start(),
                        hunk.old_lines(),
                        hunk.new_start(),
                        hunk.new_lines()
                    ),
                    ctx.theme.footer,
                ))
                .render(ctx, width)
                .into_lines(),
            );
            for line in hunk.lines() {
                let line_number = match line.kind() {
                    CodeChangeLineKind::Deletion => line.old_line(),
                    CodeChangeLineKind::Addition | CodeChangeLineKind::Context => {
                        line.new_line().or(line.old_line())
                    }
                };
                lines.extend(
                    render_diff_line(
                        line_number.unwrap_or(0),
                        line_number_width,
                        line.kind(),
                        line.text(),
                        ctx,
                        width,
                    )
                    .into_lines(),
                );
            }
        }
        if self.change.truncated() {
            rendered_details = true;
            lines.extend(
                Element::new(Text::single_line(
                    format!(
                        "diff truncated ({})",
                        diff_truncation_text(self.change.truncation())
                    ),
                    ctx.theme.warning,
                ))
                .render(ctx, width)
                .into_lines(),
            );
        }
        if !rendered_details {
            lines.extend(
                Element::new(Text::single_line("(no textual change)", ctx.theme.footer))
                    .render(ctx, width)
                    .into_lines(),
            );
        }
        RenderBlock::from_lines(lines)
    }
}

fn render_diff_line(
    line_number: u32,
    line_number_width: usize,
    kind: CodeChangeLineKind,
    text: &str,
    ctx: &RenderContext<'_>,
    width: u16,
) -> RenderBlock {
    let (marker, content_style, background) = match kind {
        CodeChangeLineKind::Context => (' ', ctx.theme.normal, None),
        CodeChangeLineKind::Addition => (
            '+',
            ctx.theme.success,
            diff_background(kind, ctx.theme.color_capability()),
        ),
        CodeChangeLineKind::Deletion => (
            '-',
            ctx.theme.error,
            diff_background(kind, ctx.theme.color_capability()),
        ),
    };
    let line_style = background.map_or(content_style, |color| content_style.bg(color));
    let gutter_style = background.map_or(ctx.theme.footer, |color| ctx.theme.footer.bg(color));
    let gutter = format!("{line_number:>line_number_width$} ");
    let prefix = format!("{gutter}{marker}");
    let width = usize::from(width);

    if width <= prefix.width() {
        let compact = truncate_cells(&format!("{prefix}{}", terminal_safe_text(text)), width);
        let padding = " ".repeat(width.saturating_sub(compact.width()));
        return RenderBlock::from_lines(vec![RenderedLine::new(
            format!("{compact}{padding}"),
            line_style,
        )]);
    }

    let wrapped = Element::new(
        Text::single_line(terminal_safe_text(text), content_style).with_prefix(&prefix),
    )
    .render(ctx, u16::try_from(width).unwrap_or(u16::MAX))
    .into_lines();
    let continuation = " ".repeat(prefix.width());
    let lines = wrapped
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let row_prefix = if index == 0 { &prefix } else { &continuation };
            let content = line
                .text
                .strip_prefix(row_prefix)
                .unwrap_or(line.text.as_str());
            let mut spans = Vec::with_capacity(4);
            if index == 0 {
                spans.push(RenderedSpan::new(gutter.clone(), gutter_style));
                spans.push(RenderedSpan::new(marker.to_string(), line_style));
            } else {
                spans.push(RenderedSpan::new(continuation.clone(), gutter_style));
            }
            spans.push(RenderedSpan::new(content.to_owned(), line_style));
            let used = row_prefix.width().saturating_add(content.width());
            spans.push(RenderedSpan::new(
                " ".repeat(width.saturating_sub(used)),
                line_style,
            ));
            RenderedLine::from_spans(line_style, spans)
        })
        .collect();
    RenderBlock::from_lines(lines)
}

fn diff_background(kind: CodeChangeLineKind, capability: ColorCapability) -> Option<Color> {
    match (kind, capability) {
        (CodeChangeLineKind::Addition, ColorCapability::TrueColor) => Some(Color::Rgb(33, 58, 43)),
        (CodeChangeLineKind::Deletion, ColorCapability::TrueColor) => Some(Color::Rgb(74, 34, 29)),
        (CodeChangeLineKind::Addition, ColorCapability::Ansi256) => Some(Color::Indexed(22)),
        (CodeChangeLineKind::Deletion, ColorCapability::Ansi256) => Some(Color::Indexed(52)),
        _ => None,
    }
}

const fn diff_truncation_text(truncation: Option<CodeChangeTruncation>) -> &'static str {
    match truncation {
        Some(CodeChangeTruncation::Hunks) => "hunks",
        Some(CodeChangeTruncation::Lines) => "lines",
        Some(CodeChangeTruncation::LineBytes) => "line bytes",
        Some(CodeChangeTruncation::PatchBytes) => "patch bytes",
        None => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;
    use tea_protocol::{
        CodeChange, CodeChangeHunk, CodeChangeKind, CodeChangeLine, CodeChangeLineKind,
        CodeChangeTruncation,
    };
    use unicode_width::UnicodeWidthStr as _;

    use super::super::host::{CellContext, CellHost, CellSpec};
    use crate::tui::Theme;
    use crate::tui::presentation::DiffCell;

    fn change(truncated: bool) -> CodeChange {
        let lines = vec![
            CodeChangeLine::new(
                CodeChangeLineKind::Context,
                Some(1),
                Some(1),
                "pub fn answer() {",
            )
            .unwrap(),
            CodeChangeLine::new(
                CodeChangeLineKind::Deletion,
                Some(2),
                None,
                "    old_value();",
            )
            .unwrap(),
            CodeChangeLine::new(
                CodeChangeLineKind::Addition,
                None,
                Some(2),
                format!("    {}new_value();", "界".repeat(8)),
            )
            .unwrap(),
        ];
        let hunk = CodeChangeHunk::new(1, 2, 1, 2, lines).unwrap();
        CodeChange::new(
            "src/lib.rs",
            CodeChangeKind::Update,
            vec![hunk],
            truncated,
            truncated.then_some(CodeChangeTruncation::Lines),
            None,
            Some(2),
        )
        .unwrap()
    }

    #[test]
    fn diff_is_full_bleed_with_line_numbers_and_per_line_backgrounds() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let cell = DiffCell::new("Updated", change(false));

        let rendered = CellHost::default().render(CellSpec::new(&cell), &context, 32);
        let deletion = rendered
            .lines()
            .iter()
            .find(|line| line.text().starts_with("2 -"))
            .unwrap();
        let addition = rendered
            .lines()
            .iter()
            .find(|line| line.text().starts_with("2 +"))
            .unwrap();

        assert_eq!(rendered.margin().left, 0);
        assert_eq!(rendered.margin().right, 0);
        assert!(rendered.lines()[0].text().starts_with(" • Updated"));
        assert!(rendered.lines()[1].text().starts_with("@@ -1,2 +1,2 @@"));
        assert_eq!(deletion.text().width(), 32);
        assert_eq!(addition.text().width(), 32);
        assert_eq!(deletion.style().bg, Some(Color::Rgb(74, 34, 29)));
        assert_eq!(addition.style().bg, Some(Color::Rgb(33, 58, 43)));
    }

    #[test]
    fn wide_unicode_and_truncation_remain_width_safe() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let cell = DiffCell::new("Updated", change(true));

        let rendered = CellHost::default().render(CellSpec::new(&cell), &context, 12);

        assert!(
            rendered
                .lines()
                .iter()
                .all(|line| line.text().width() <= 12)
        );
        assert!(
            rendered
                .lines()
                .iter()
                .any(|line| line.text().contains("diff truncat"))
        );
        let addition = rendered
            .lines()
            .iter()
            .position(|line| line.text().starts_with("2 +"))
            .unwrap();
        let addition_rows = rendered.lines()[addition..]
            .iter()
            .take_while(|line| line.text().starts_with("2 +") || line.text().starts_with("   "))
            .collect::<Vec<_>>();
        assert!(addition_rows.len() > 1);
        assert!(
            addition_rows
                .iter()
                .all(|line| line.style().bg == Some(Color::Rgb(33, 58, 43)))
        );
    }
}
