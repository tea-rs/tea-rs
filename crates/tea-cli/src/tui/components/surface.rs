use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr as _;

use super::super::render_output::{RenderedLine, RenderedSpan};
use super::style::{BackgroundRole, Insets};
use super::{Component, Element, RenderBlock, RenderContext};

pub(crate) struct Surface<'a> {
    child: Element<'a>,
    padding: Insets,
    background: BackgroundRole,
}

impl<'a> Surface<'a> {
    pub(crate) fn new(child: Element<'a>) -> Self {
        Self {
            child,
            padding: Insets::default(),
            background: BackgroundRole::Transparent,
        }
    }

    pub(crate) const fn with_padding(mut self, padding: Insets) -> Self {
        self.padding = padding;
        self
    }

    pub(crate) const fn with_background(mut self, background: BackgroundRole) -> Self {
        self.background = background;
        self
    }
}

impl Component for Surface<'_> {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        let (left, child_width) = fit_horizontal_padding(width, self.padding);
        let child = self.child.render(ctx, child_width);
        if child.is_empty() {
            return RenderBlock::empty();
        }
        if self.background == BackgroundRole::Transparent && self.padding == Insets::default() {
            return child;
        }

        let background = background_patch(self.background, ctx);
        let blank_style = fill_style(self.background, ctx, background);
        let blank = || RenderedLine::new(" ".repeat(usize::from(width)), blank_style);
        let row_count = usize::from(self.padding.top)
            .saturating_add(child.lines().len())
            .saturating_add(usize::from(self.padding.bottom));
        let mut lines = Vec::with_capacity(row_count);
        lines.extend((0..self.padding.top).map(|_| blank()));
        lines.extend(
            child
                .into_lines()
                .into_iter()
                .map(|line| render_content_row(line, width, left, background, blank_style)),
        );
        lines.extend((0..self.padding.bottom).map(|_| blank()));
        RenderBlock::from_lines(lines)
    }
}

fn fit_horizontal_padding(width: u16, padding: Insets) -> (u16, u16) {
    let available = width.saturating_sub(1);
    let left = padding.left.min(available);
    let remaining = available.saturating_sub(left);
    let right = padding.right.min(remaining);
    let child_width = width.saturating_sub(left).saturating_sub(right).max(1);
    (left, child_width)
}

fn render_content_row(
    line: RenderedLine,
    width: u16,
    left: u16,
    background: Option<Style>,
    fill_style: Style,
) -> RenderedLine {
    let line_style = patch_style(line.style, background);
    let content_width = line.text.width();
    let left = usize::from(left);
    let trailing = usize::from(width).saturating_sub(left.saturating_add(content_width));
    if line.spans.is_empty() && left == 0 && line_style == fill_style {
        return RenderedLine::new(format!("{}{}", line.text, " ".repeat(trailing)), line_style);
    }
    let mut spans = Vec::with_capacity(line.spans.len().saturating_add(2));

    if left > 0 {
        spans.push(RenderedSpan::new(" ".repeat(left), fill_style));
    }
    if line.spans.is_empty() {
        if !line.text.is_empty() {
            spans.push(RenderedSpan::new(line.text, line_style));
        }
    } else {
        spans.extend(line.spans.into_iter().map(|mut span| {
            span.style = patch_style(span.style, background);
            span
        }));
    }
    if trailing > 0 {
        spans.push(RenderedSpan::new(" ".repeat(trailing), fill_style));
    }

    RenderedLine::from_spans(line_style, spans)
}

fn patch_style(style: Style, background: Option<Style>) -> Style {
    background.map_or(style, |background| style.patch(background))
}

fn fill_style(role: BackgroundRole, ctx: &RenderContext<'_>, background: Option<Style>) -> Style {
    if role == BackgroundRole::Composer {
        ctx.theme.composer
    } else {
        patch_style(Style::default(), background)
    }
}

fn background_patch(role: BackgroundRole, ctx: &RenderContext<'_>) -> Option<Style> {
    let source = match role {
        BackgroundRole::Transparent => return None,
        BackgroundRole::Composer => ctx.theme.composer,
        BackgroundRole::Success => ctx.theme.success,
        BackgroundRole::Warning => ctx.theme.warning,
        BackgroundRole::Error => ctx.theme.error,
    };
    let mut patch = source
        .bg
        .map_or_else(Style::default, |color| Style::default().bg(color));
    if source.bg.is_none() && source.add_modifier.contains(Modifier::REVERSED) {
        patch = patch.add_modifier(Modifier::REVERSED);
    }
    Some(patch)
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Style};
    use unicode_width::UnicodeWidthStr as _;

    use super::Surface;
    use crate::tui::Theme;
    use crate::tui::components::lines::Lines;
    use crate::tui::components::style::{BackgroundRole, Insets};
    use crate::tui::components::{Component, Element, RenderBlock, RenderContext};
    use crate::tui::render_output::{RenderedLine, RenderedSpan};

    struct WidthEcho;

    impl Component for WidthEcho {
        fn render(&self, _ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
            RenderBlock::from_lines(vec![RenderedLine::new(width.to_string(), Style::default())])
        }
    }

    #[test]
    fn transparent_surface_preserves_existing_diff_backgrounds() {
        let theme = Theme::default();
        let ctx = RenderContext { theme: &theme };
        let destination = "https://example.com/change";
        let diff_style = Style::default().fg(Color::Green).bg(Color::Rgb(33, 58, 43));
        let child = Lines::new(vec![RenderedLine::from_spans(
            diff_style,
            vec![RenderedSpan::with_link(
                "+change".to_owned(),
                diff_style,
                destination.to_owned(),
            )],
        )]);

        let rendered = Element::new(Surface::new(Element::new(child))).render(&ctx, 8);
        let content = &rendered.lines()[0].rendered_spans()[0];

        assert_eq!(content.style().bg, diff_style.bg);
        assert_eq!(content.link(), Some(destination));
    }

    #[test]
    fn surface_background_covers_padding_and_trailing_columns() {
        let theme = Theme::default();
        let ctx = RenderContext { theme: &theme };
        let destination = "https://example.com/docs";
        let child = Lines::new(vec![RenderedLine::from_spans(
            Style::default(),
            vec![RenderedSpan::with_link(
                "x".to_owned(),
                Style::default().fg(Color::Cyan),
                destination.to_owned(),
            )],
        )]);
        let surface = Surface::new(Element::new(child))
            .with_padding(Insets {
                top: 1,
                right: 1,
                bottom: 1,
                left: 1,
            })
            .with_background(BackgroundRole::Composer);

        let rendered = Element::new(surface).render(&ctx, 5);

        assert_eq!(rendered.lines().len(), 3);
        assert!(rendered.lines().iter().all(|line| line.text().width() == 5));
        assert!(
            rendered
                .lines()
                .iter()
                .all(|line| line.style().bg == theme.composer.bg)
        );
        assert!(
            rendered.lines()[1]
                .rendered_spans()
                .iter()
                .all(|span| span.style().bg == theme.composer.bg)
        );
        assert_eq!(
            rendered.lines()[1].rendered_spans()[1].link(),
            Some(destination)
        );
    }

    #[test]
    fn narrow_surface_saturates_without_panicking() {
        let theme = Theme::default();
        let ctx = RenderContext { theme: &theme };

        for requested in [0, 1, 2] {
            let surface = Surface::new(Element::new(WidthEcho)).with_padding(Insets {
                top: 0,
                right: 8,
                bottom: 0,
                left: 8,
            });
            let rendered = Element::new(surface).render(&ctx, requested);
            let normalized = usize::from(requested.max(1));

            assert_eq!(rendered.lines().len(), 1);
            assert_eq!(rendered.lines()[0].text().width(), normalized);
        }
    }

    #[test]
    fn empty_surface_does_not_create_padding_only_rows() {
        let theme = Theme::default();
        let ctx = RenderContext { theme: &theme };
        let surface = Surface::new(Element::new(Lines::new(Vec::new()))).with_padding(Insets {
            top: 3,
            right: 2,
            bottom: 3,
            left: 2,
        });

        let rendered = Element::new(surface).render(&ctx, 20);

        assert!(rendered.is_empty());
    }
}
