use super::super::render_output::RenderedLine;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct RenderBlock {
    lines: Vec<RenderedLine>,
}

impl RenderBlock {
    pub(crate) const fn empty() -> Self {
        Self { lines: Vec::new() }
    }

    pub(crate) const fn from_lines(lines: Vec<RenderedLine>) -> Self {
        Self { lines }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub(crate) fn lines(&self) -> &[RenderedLine] {
        &self.lines
    }

    pub(crate) fn into_lines(self) -> Vec<RenderedLine> {
        self.lines
    }
}
