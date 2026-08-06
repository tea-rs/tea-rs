use ratatui::style::Style;

use super::super::render_output::RenderedLine;
use super::{Component, RenderBlock, RenderContext};

/// Intentional cell-internal whitespace. Top-level spacing belongs to `CellList`.
pub(crate) struct Spacer {
    rows: u16,
}

impl Spacer {
    pub(crate) const fn new(rows: u16) -> Self {
        Self { rows }
    }
}

impl Component for Spacer {
    fn render(&self, _ctx: &RenderContext<'_>, _width: u16) -> RenderBlock {
        RenderBlock::from_lines(
            (0..self.rows)
                .map(|_| RenderedLine::new(String::new(), Style::default()))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Spacer;
    use crate::tui::Theme;
    use crate::tui::components::{Element, RenderContext};

    #[test]
    fn spacer_renders_exactly_the_requested_internal_rows() {
        let theme = Theme::default();
        let ctx = RenderContext { theme: &theme };

        let rendered = Element::new(Spacer::new(3)).render(&ctx, 20);

        assert_eq!(rendered.lines().len(), 3);
        assert!(rendered.lines().iter().all(|line| line.text().is_empty()));
    }
}
