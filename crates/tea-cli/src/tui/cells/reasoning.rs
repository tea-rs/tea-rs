use ratatui::style::{Modifier, Style};

use super::super::components::{Component, Element, Markdown, RenderBlock, RenderContext, Text};
use super::super::presentation::ReasoningCell;
use super::super::render_output::{RenderedLine, RenderedSpan};
use super::host::{CellContext, CellPresenter};
use super::style::{CellRole, CellStyle};

impl CellPresenter for ReasoningCell {
    fn role(&self) -> CellRole {
        CellRole::Reasoning
    }

    fn content<'a>(&'a self, ctx: &'a CellContext<'a>, resolved_style: CellStyle) -> Element<'a> {
        Element::new(ReasoningContent {
            source: self.source(),
            collapsed: self.collapsed(),
            style: resolved_style
                .foreground
                .resolve(ctx.theme)
                .add_modifier(Modifier::DIM | Modifier::ITALIC),
        })
    }
}

struct ReasoningContent<'a> {
    source: &'a str,
    collapsed: bool,
    style: Style,
}

impl Component for ReasoningContent<'_> {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        if self.collapsed {
            return Element::new(
                Text::new("Reasoning collapsed", self.style)
                    .with_prefixes("• ", "  ")
                    .with_prefix_style(self.style),
            )
            .render(ctx, width);
        }

        let body_width = width.saturating_sub(2).max(1);
        let mut body = Element::new(Markdown::new(self.source))
            .render(ctx, body_width)
            .into_lines();
        if body.is_empty() {
            body.push(RenderedLine::new("(empty)".to_owned(), self.style));
        }
        RenderBlock::from_lines(
            body.into_iter()
                .enumerate()
                .map(|(index, line)| {
                    prefix_line(
                        patch_line(line, self.style),
                        if index == 0 { "• " } else { "  " },
                        self.style,
                    )
                })
                .collect(),
        )
    }
}

fn patch_line(mut line: RenderedLine, patch: Style) -> RenderedLine {
    line.style = line.style.patch(patch);
    for span in &mut line.spans {
        span.style = span.style.patch(patch);
    }
    line
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
    use super::super::host::{CellContext, CellHost, CellSpec};
    use crate::tui::Theme;
    use crate::tui::presentation::ReasoningCell;

    #[test]
    fn collapsed_and_expanded_reasoning_preserve_the_existing_output_grammar() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let collapsed = ReasoningCell::new("hidden source", true);
        let expanded = ReasoningCell::new("inspect **state**", false);
        let host = CellHost::default();

        let collapsed = host.render(CellSpec::new(&collapsed), &context, 24);
        let expanded = host.render(CellSpec::new(&expanded), &context, 24);

        assert_eq!(collapsed.lines().len(), 1);
        assert_eq!(
            collapsed.lines()[0].text().trim_end(),
            "• Reasoning collapsed"
        );
        assert_eq!(expanded.lines().len(), 1);
        assert_eq!(expanded.lines()[0].text().trim_end(), "• inspect state");
        assert!(expanded.lines()[0].rendered_spans().iter().any(|span| {
            span.style()
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        }));
    }
}
