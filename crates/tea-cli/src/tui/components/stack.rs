use ratatui::style::Style;

use super::super::render_output::RenderedLine;
use super::{Component, Element, RenderBlock, RenderContext};

pub(crate) struct VStack<'a> {
    children: Vec<Element<'a>>,
    gap: u16,
}

impl<'a> VStack<'a> {
    pub(crate) const fn new(children: Vec<Element<'a>>) -> Self {
        Self { children, gap: 0 }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) const fn with_gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }
}

impl Component for VStack<'_> {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        let mut lines = Vec::new();
        for child in &self.children {
            let mut child_lines = child.render(ctx, width).into_lines();
            if child_lines.is_empty() {
                continue;
            }
            if !lines.is_empty() {
                lines.extend(
                    (0..self.gap).map(|_| RenderedLine::new(String::new(), Style::default())),
                );
            }
            lines.append(&mut child_lines);
        }
        RenderBlock::from_lines(lines)
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Style;

    use super::VStack;
    use crate::tui::components::lines::Lines;
    use crate::tui::components::{Element, RenderContext};
    use crate::tui::{RenderedLine, Theme};

    fn text(value: &str) -> Element<'static> {
        Element::new(Lines::new(vec![RenderedLine::new(
            value.to_owned(),
            Style::default(),
        )]))
    }

    fn empty() -> Element<'static> {
        Element::new(Lines::new(Vec::new()))
    }

    #[test]
    fn vstack_adds_gap_only_between_non_empty_children() {
        let theme = Theme::default();
        let ctx = RenderContext { theme: &theme };
        let stack = VStack::new(vec![text("first"), empty(), text("second")]).with_gap(1);

        let rendered = Element::new(stack).render(&ctx, 20);
        let text = rendered
            .lines()
            .iter()
            .map(RenderedLine::text)
            .collect::<Vec<_>>();

        assert_eq!(text, vec!["first", "", "second"]);
    }

    #[test]
    fn vstack_never_adds_leading_or_trailing_gap() {
        let theme = Theme::default();
        let ctx = RenderContext { theme: &theme };
        let stack = VStack::new(vec![empty(), text("only"), empty()]).with_gap(2);

        let rendered = Element::new(stack).render(&ctx, 20);

        assert_eq!(rendered.lines().len(), 1);
        assert_eq!(rendered.lines()[0].text(), "only");
    }
}
