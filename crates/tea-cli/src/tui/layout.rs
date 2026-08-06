use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

use super::hyperlink::{Hyperlink, HyperlinkBuffer};
use super::render_output::RenderedLine;

/// Display-cell cursor geometry returned by a measured renderable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CursorState {
    pub(crate) x: u16,
    pub(crate) y: u16,
}

/// A terminal region that can measure, paint, and optionally own the cursor.
pub(crate) trait Renderable {
    fn desired_height(&self, width: u16) -> u16;

    fn render(&self, area: Rect, buffer: &mut Buffer);

    fn cursor(&self, _area: Rect) -> Option<CursorState> {
        None
    }
}

/// Rectangles for a transcript above a bottom-aligned interactive pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VerticalLayout {
    pub(crate) transcript: Rect,
    pub(crate) bottom_pane: Rect,
}

impl VerticalLayout {
    #[must_use]
    pub(crate) fn with_bottom_pane(area: Rect, desired_bottom_height: u16) -> Self {
        let bottom_height = desired_bottom_height.min(area.height);
        let transcript_height = area.height.saturating_sub(bottom_height);
        Self {
            transcript: Rect::new(area.x, area.y, area.width, transcript_height),
            bottom_pane: Rect::new(
                area.x,
                area.y.saturating_add(transcript_height),
                area.width,
                bottom_height,
            ),
        }
    }
}

pub(crate) fn draw_lines(lines: &[RenderedLine], area: Rect, buffer: &mut Buffer) {
    draw_lines_inner(lines, area, buffer, None);
}

pub(crate) fn draw_lines_with_hyperlinks(
    lines: &[RenderedLine],
    area: Rect,
    buffer: &mut Buffer,
    hyperlinks: &mut HyperlinkBuffer,
) {
    draw_lines_inner(lines, area, buffer, Some(hyperlinks));
}

fn draw_lines_inner(
    lines: &[RenderedLine],
    area: Rect,
    buffer: &mut Buffer,
    mut hyperlinks: Option<&mut HyperlinkBuffer>,
) {
    for (offset, line) in lines.iter().take(usize::from(area.height)).enumerate() {
        let Ok(offset) = u16::try_from(offset) else {
            break;
        };
        let y = area.y.saturating_add(offset);
        let rendered = line.as_ratatui_line();
        buffer.set_line(area.x, y, &rendered, area.width);
        if let Some(hyperlinks) = hyperlinks.as_deref_mut() {
            mark_line_hyperlinks(line, area.x, y, area.width, hyperlinks);
        }
    }
}

fn mark_line_hyperlinks(
    line: &RenderedLine,
    start_x: u16,
    y: u16,
    max_width: u16,
    hyperlinks: &mut HyperlinkBuffer,
) {
    let right = start_x.saturating_add(max_width);
    let mut x = start_x;
    for span in line.rendered_spans() {
        let destination = span.link().map(Hyperlink::from);
        for grapheme in span.text().graphemes(true) {
            let width = grapheme.width();
            if width == 0 {
                continue;
            }
            let Ok(width_u16) = u16::try_from(width) else {
                return;
            };
            if x.saturating_add(width_u16) > right {
                return;
            }
            hyperlinks.set_range(Position::new(x, y), width, destination.as_ref());
            x = x.saturating_add(width_u16);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::VerticalLayout;

    #[test]
    fn bottom_pane_is_measured_from_the_bottom_and_clipped_to_the_frame() {
        let layout = VerticalLayout::with_bottom_pane(Rect::new(3, 4, 80, 24), 6);
        assert_eq!(layout.transcript, Rect::new(3, 4, 80, 18));
        assert_eq!(layout.bottom_pane, Rect::new(3, 22, 80, 6));

        let clipped = VerticalLayout::with_bottom_pane(Rect::new(0, 0, 40, 3), 6);
        assert_eq!(clipped.transcript, Rect::new(0, 0, 40, 0));
        assert_eq!(clipped.bottom_pane, Rect::new(0, 0, 40, 3));
    }
}
