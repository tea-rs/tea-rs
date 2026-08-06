use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::hyperlink::HyperlinkBuffer;
use super::layout::{Renderable, draw_lines, draw_lines_with_hyperlinks};
use super::render_output::RenderedLine;
use super::state::TranscriptViewport;
use super::theme::Theme;

/// The pre-viewport transcript region. P2 replaces its tail selection with
/// user-controlled scroll/follow-tail state without changing its layout API.
pub(crate) struct Transcript<'a> {
    lines: &'a [RenderedLine],
    viewport: &'a TranscriptViewport,
    marker: Option<RenderedLine>,
}

impl<'a> Transcript<'a> {
    #[must_use]
    pub(crate) fn new(
        lines: &'a [RenderedLine],
        viewport: &'a TranscriptViewport,
        theme: &Theme,
    ) -> Self {
        let marker = (!viewport.follows_tail() && viewport.unread_items() > 0).then(|| {
            RenderedLine::new(
                format!(
                    "new output ({}) · ctrl+end to follow",
                    viewport.unread_items()
                ),
                theme.footer,
            )
        });
        Self {
            lines,
            viewport,
            marker,
        }
    }

    pub(crate) fn render_with_hyperlinks(
        &self,
        area: Rect,
        buffer: &mut Buffer,
        hyperlinks: &mut HyperlinkBuffer,
    ) {
        self.render_inner(area, buffer, Some(hyperlinks));
    }

    fn render_inner(
        &self,
        area: Rect,
        buffer: &mut Buffer,
        mut hyperlinks: Option<&mut HyperlinkBuffer>,
    ) {
        let marker_height = u16::from(self.marker.is_some() && area.height > 0);
        let content = Rect::new(
            area.x,
            area.y,
            area.width,
            area.height.saturating_sub(marker_height),
        );
        let visible = visible_frame(self.lines, usize::from(content.height), self.viewport);
        let vertical_offset = if self.viewport.follows_tail() {
            usize::from(content.height).saturating_sub(visible.len())
        } else {
            0
        };
        let transcript_area = Rect::new(
            content.x,
            content
                .y
                .saturating_add(u16::try_from(vertical_offset).unwrap_or(u16::MAX)),
            content.width,
            content
                .height
                .saturating_sub(u16::try_from(vertical_offset).unwrap_or(u16::MAX)),
        );
        if let Some(hyperlinks) = hyperlinks.as_mut() {
            draw_lines_with_hyperlinks(&visible, transcript_area, buffer, hyperlinks);
        } else {
            draw_lines(&visible, transcript_area, buffer);
        }
        if let Some(marker) = &self.marker {
            let marker_area = Rect::new(
                area.x,
                area.y.saturating_add(content.height),
                area.width,
                marker_height,
            );
            if let Some(hyperlinks) = hyperlinks.as_mut() {
                draw_lines_with_hyperlinks(
                    std::slice::from_ref(marker),
                    marker_area,
                    buffer,
                    hyperlinks,
                );
            } else {
                draw_lines(std::slice::from_ref(marker), marker_area, buffer);
            }
        }
    }
}

impl Renderable for Transcript<'_> {
    fn desired_height(&self, _width: u16) -> u16 {
        u16::try_from(self.lines.len()).unwrap_or(u16::MAX)
    }

    fn render(&self, area: Rect, buffer: &mut Buffer) {
        self.render_inner(area, buffer, None);
    }
}

fn visible_frame(
    lines: &[RenderedLine],
    height: usize,
    viewport: &TranscriptViewport,
) -> Vec<RenderedLine> {
    if height == 0 {
        return Vec::new();
    }
    if lines.len() <= height {
        return lines.to_vec();
    }
    if viewport.follows_tail() {
        return lines[lines.len().saturating_sub(height)..].to_vec();
    }
    let max_offset = lines.len().saturating_sub(height);
    let offset = viewport.offset_from_tail_rows().min(max_offset);
    let end = lines.len().saturating_sub(offset);
    let start = end.saturating_sub(height);
    lines[start..end].to_vec()
}
