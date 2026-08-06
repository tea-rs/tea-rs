use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::layout::{CursorState, Renderable, draw_lines};
use super::render_output::RenderedLine;

/// The bottom-aligned region that keeps composition available during activity.
pub(crate) struct BottomPane<'a> {
    lines: &'a [RenderedLine],
    cursor: Option<CursorState>,
}

impl<'a> BottomPane<'a> {
    #[must_use]
    pub(crate) const fn new(lines: &'a [RenderedLine], cursor: Option<CursorState>) -> Self {
        Self { lines, cursor }
    }

    fn start(&self, area: Rect) -> usize {
        self.lines.len().saturating_sub(usize::from(area.height))
    }
}

impl Renderable for BottomPane<'_> {
    fn desired_height(&self, _width: u16) -> u16 {
        u16::try_from(self.lines.len()).unwrap_or(u16::MAX)
    }

    fn render(&self, area: Rect, buffer: &mut Buffer) {
        draw_lines(&self.lines[self.start(area)..], area, buffer);
    }

    fn cursor(&self, area: Rect) -> Option<CursorState> {
        let cursor = self.cursor?;
        let start = self.start(area);
        let row = usize::from(cursor.y);
        if row < start || row >= start.saturating_add(usize::from(area.height)) {
            return None;
        }
        Some(CursorState {
            x: area
                .x
                .saturating_add(cursor.x.min(area.width.saturating_sub(1))),
            y: area
                .y
                .saturating_add(u16::try_from(row - start).unwrap_or(u16::MAX)),
        })
    }
}
