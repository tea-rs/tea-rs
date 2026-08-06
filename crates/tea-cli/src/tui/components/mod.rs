mod block;
// These protocol primitives are test-covered and available for future first-party presenters.
#[cfg_attr(not(test), allow(dead_code))]
mod lines;
mod markdown;
#[cfg_attr(not(test), allow(dead_code))]
mod spacer;
mod stack;
mod status;
pub(crate) mod style;
mod surface;
mod text;

pub(crate) use block::RenderBlock;
pub(crate) use markdown::Markdown;
pub(crate) use stack::VStack;
pub(crate) use status::{ActionGroup, ActionHeader, DetailRow, DetailTree};
pub(crate) use surface::Surface;
pub(crate) use text::{Text, truncate_cells, truncate_cells_with_ellipsis};

use super::theme::Theme;

pub(crate) struct RenderContext<'a> {
    pub(crate) theme: &'a Theme,
}

pub(crate) trait Component {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock;
}

pub(crate) struct Element<'a> {
    inner: Box<dyn Component + 'a>,
}

impl<'a> Element<'a> {
    pub(crate) fn new(component: impl Component + 'a) -> Self {
        Self {
            inner: Box::new(component),
        }
    }

    pub(crate) fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        self.inner.render(ctx, width.max(1))
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Style;

    use super::{Component, Element, RenderBlock, RenderContext};
    use crate::tui::{RenderedLine, Theme};

    struct WidthEcho;

    impl Component for WidthEcho {
        fn render(&self, _ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
            RenderBlock::from_lines(vec![RenderedLine::new(width.to_string(), Style::default())])
        }
    }

    #[test]
    fn element_normalizes_width_at_the_component_boundary() {
        let theme = Theme::default();
        let ctx = RenderContext { theme: &theme };
        assert_eq!(ctx.theme.generation, theme.generation);
        let element = Element::new(WidthEcho);

        for (requested, expected) in [(0, "1"), (1, "1"), (80, "80")] {
            let rendered = element.render(&ctx, requested);
            assert_eq!(rendered.lines()[0].text(), expected);
        }
    }

    #[test]
    fn empty_render_block_contains_no_lines() {
        let block = RenderBlock::empty();

        assert!(block.is_empty());
        assert!(block.lines().is_empty());
        assert!(block.into_lines().is_empty());
    }
}
