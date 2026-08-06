use super::super::render_output::RenderedLine;
use super::{Component, RenderBlock, RenderContext};

pub(crate) struct Lines {
    lines: Vec<RenderedLine>,
}

impl Lines {
    pub(crate) const fn new(lines: Vec<RenderedLine>) -> Self {
        Self { lines }
    }
}

impl Component for Lines {
    fn render(&self, _ctx: &RenderContext<'_>, _width: u16) -> RenderBlock {
        RenderBlock::from_lines(self.lines.clone())
    }
}
