// This is derived from `codex-rs/tui/src/custom_terminal.rs` and
// `ratatui::Terminal`. The Ratatui-derived portions retain Ratatui's MIT license:
//
// Copyright (c) 2016-2022 Florian Dehau
// Copyright (c) 2023-2025 The Ratatui Developers
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use std::io;
use std::io::Write;

use crossterm::cursor::{MoveTo, SetCursorStyle};
use crossterm::queue;
use crossterm::style::{
    Attribute, Colors, Print, SetAttribute, SetBackgroundColor, SetColors, SetForegroundColor,
};
use crossterm::terminal::{Clear, ClearType as CrosstermClearType};
use ratatui::backend::{Backend, ClearType, IntoCrossterm as _};
use ratatui::buffer::{Buffer, Cell, CellDiffOption};
use ratatui::layout::{Position, Rect, Size};
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr as _;

use super::hyperlink::{Hyperlink, HyperlinkBuffer, write_close, write_open};

/// One dynamically positioned inline frame.
pub(crate) struct Frame<'a> {
    cursor_position: Option<Position>,
    viewport_area: Rect,
    buffer: &'a mut Buffer,
    hyperlinks: Option<&'a mut HyperlinkBuffer>,
}

impl Frame<'_> {
    pub(crate) const fn area(&self) -> Rect {
        self.viewport_area
    }

    pub(crate) fn set_cursor_position<P: Into<Position>>(&mut self, position: P) {
        self.cursor_position = Some(position.into());
    }

    pub(crate) fn buffers_mut(&mut self) -> (&mut Buffer, Option<&mut HyperlinkBuffer>) {
        (self.buffer, self.hyperlinks.as_deref_mut())
    }
}

/// Codex-style terminal whose Ratatui viewport can move independently of the screen.
pub(crate) struct Terminal<B>
where
    B: Backend<Error = io::Error> + Write,
{
    backend: B,
    buffers: [Buffer; 2],
    hyperlinks: [HyperlinkBuffer; 2],
    hyperlinks_enabled: bool,
    current: usize,
    hidden_cursor: bool,
    pub(crate) viewport_area: Rect,
    pub(crate) last_known_screen_size: Size,
    pub(crate) last_known_cursor_pos: Position,
}

impl<B> Terminal<B>
where
    B: Backend<Error = io::Error> + Write,
{
    pub(crate) fn with_cursor_position(
        backend: B,
        screen_size: Size,
        cursor_pos: Position,
        hyperlinks_enabled: bool,
    ) -> Self {
        Self {
            backend,
            buffers: [Buffer::empty(Rect::ZERO), Buffer::empty(Rect::ZERO)],
            hyperlinks: [
                HyperlinkBuffer::empty(Rect::ZERO),
                HyperlinkBuffer::empty(Rect::ZERO),
            ],
            hyperlinks_enabled,
            current: 0,
            hidden_cursor: false,
            viewport_area: Rect::new(0, cursor_pos.y, 0, 0),
            last_known_screen_size: screen_size,
            last_known_cursor_pos: cursor_pos,
        }
    }

    pub(crate) const fn backend(&self) -> &B {
        &self.backend
    }

    pub(crate) fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub(crate) const fn hyperlinks_enabled(&self) -> bool {
        self.hyperlinks_enabled
    }

    pub(crate) fn size(&self) -> io::Result<Size> {
        self.backend.size()
    }

    pub(crate) fn set_viewport_area(&mut self, area: Rect) {
        self.buffers[self.current].resize(area);
        self.buffers[1 - self.current].resize(area);
        self.hyperlinks[self.current].resize(area);
        self.hyperlinks[1 - self.current].resize(area);
        self.viewport_area = area;
    }

    pub(crate) fn invalidate_viewport(&mut self) {
        self.buffers[1 - self.current].reset();
        self.hyperlinks[1 - self.current].reset();
    }

    pub(crate) fn clear(&mut self) -> io::Result<()> {
        if self.viewport_area.is_empty() {
            return Ok(());
        }
        self.clear_after_position(self.viewport_area.as_position())
    }

    pub(crate) fn clear_after_position(&mut self, position: Position) -> io::Result<()> {
        self.backend.set_cursor_position(position)?;
        self.backend.clear_region(ClearType::AfterCursor)?;
        self.invalidate_viewport();
        Ok(())
    }

    /// Clears the visible screen without purging terminal-owned scrollback.
    pub(crate) fn clear_visible_screen(&mut self) -> io::Result<()> {
        let home = Position::ORIGIN;
        self.backend.set_cursor_position(home)?;
        self.backend.clear_region(ClearType::All)?;
        self.backend.set_cursor_position(home)?;
        Write::flush(&mut self.backend)?;
        self.last_known_cursor_pos = home;
        self.invalidate_viewport();
        Ok(())
    }

    pub(crate) fn scroll_region_up(
        &mut self,
        region: std::ops::Range<u16>,
        amount: u16,
    ) -> io::Result<()> {
        if amount == 0 || region.is_empty() {
            return Ok(());
        }
        write!(
            self.backend,
            "\x1b[{};{}r\x1b[{}S\x1b[r",
            region.start.saturating_add(1),
            region.end,
            amount
        )?;
        Write::flush(&mut self.backend)
    }

    pub(crate) fn clear_scrollback_and_visible_screen(&mut self) -> io::Result<()> {
        write!(self.backend, "\x1b[r\x1b[0m\x1b[H\x1b[2J\x1b[3J\x1b[H")?;
        Write::flush(&mut self.backend)?;
        self.last_known_cursor_pos = Position::ORIGIN;
        self.invalidate_viewport();
        Ok(())
    }

    pub(crate) fn draw(&mut self, render: impl FnOnce(&mut Frame<'_>)) -> io::Result<()> {
        let screen_size = self.size()?;
        self.last_known_screen_size = screen_size;

        let current = self.current;
        let cursor_position = {
            let mut frame = Frame {
                cursor_position: None,
                viewport_area: self.viewport_area,
                buffer: &mut self.buffers[current],
                hyperlinks: self
                    .hyperlinks_enabled
                    .then_some(&mut self.hyperlinks[current]),
            };
            render(&mut frame);
            frame.cursor_position
        };

        let previous = 1 - self.current;
        let commands = diff_buffers(
            &self.buffers[previous],
            &self.buffers[self.current],
            &self.hyperlinks[previous],
            &self.hyperlinks[self.current],
        );
        draw(&mut self.backend, commands.into_iter())?;

        if let Some(position) = cursor_position {
            queue!(self.backend, SetCursorStyle::DefaultUserShape)?;
            self.backend.show_cursor()?;
            self.hidden_cursor = false;
            self.backend.set_cursor_position(position)?;
            self.last_known_cursor_pos = position;
        } else {
            self.backend.hide_cursor()?;
            self.hidden_cursor = true;
        }

        self.buffers[previous].reset();
        self.hyperlinks[previous].reset();
        self.current = previous;
        Backend::flush(&mut self.backend)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DrawCommand {
    Put {
        x: u16,
        y: u16,
        cell: Cell,
        link: Option<Hyperlink>,
    },
    ClearToEnd {
        x: u16,
        y: u16,
        bg: Color,
    },
}

// This is the same row-tail strategy used by Codex's custom terminal: clear the
// blank suffix explicitly, then emit only cells that still carry visible state.
fn diff_buffers(
    previous: &Buffer,
    next: &Buffer,
    previous_links: &HyperlinkBuffer,
    next_links: &HyperlinkBuffer,
) -> Vec<DrawCommand> {
    let mut updates = Vec::new();
    let mut last_nonblank_columns = vec![0; usize::from(next.area.height)];

    for y in 0..next.area.height {
        let row_start = usize::from(y) * usize::from(next.area.width);
        let row_end = row_start + usize::from(next.area.width);
        let row = &next.content[row_start..row_end];
        let bg = row.last().map_or(Color::Reset, |cell| cell.bg);
        let mut last_nonblank_column = 0;
        let mut column = 0;
        while column < row.len() {
            let cell = &row[column];
            let width = cell.symbol().width();
            if cell.symbol() != " " || cell.bg != bg || cell.modifier != Modifier::empty() {
                last_nonblank_column = column + width.saturating_sub(1);
            }
            column += width.max(1);
        }

        if last_nonblank_column + 1 < row.len() {
            let (x, y) = next.pos_of(row_start + last_nonblank_column + 1);
            updates.push(DrawCommand::ClearToEnd { x, y, bg });
        }
        last_nonblank_columns[usize::from(y)] =
            u16::try_from(last_nonblank_column).unwrap_or(u16::MAX);
    }

    let mut invalidated = 0;
    let mut to_skip = 0;
    for (index, (current, prior)) in next.content.iter().zip(previous.content.iter()).enumerate() {
        let (x, y) = next.pos_of(index);
        let position = Position::new(x, y);
        let current_link = next_links.get(position);
        let prior_link = previous_links.get(position);
        let changed = current != prior
            || current_link != prior_link
            || current.diff_option == CellDiffOption::AlwaysUpdate;
        if current.diff_option != CellDiffOption::Skip
            && (changed || invalidated > 0)
            && to_skip == 0
        {
            let relative_x = x.saturating_sub(next.area.x);
            let relative_y = usize::from(y.saturating_sub(next.area.y));
            if relative_x <= last_nonblank_columns[relative_y] {
                updates.push(DrawCommand::Put {
                    x,
                    y,
                    cell: current.clone(),
                    link: current_link.cloned(),
                });
            }
        }

        to_skip = current.symbol().width().saturating_sub(1);
        let affected_width = current.symbol().width().max(prior.symbol().width());
        invalidated = affected_width.max(invalidated).saturating_sub(1);
    }
    updates
}

fn draw(writer: &mut impl Write, commands: impl Iterator<Item = DrawCommand>) -> io::Result<()> {
    let mut foreground = Color::Reset;
    let mut background = Color::Reset;
    let mut modifiers = Modifier::empty();
    let mut last_position: Option<Position> = None;
    let mut active_link: Option<Hyperlink> = None;

    for command in commands {
        let (x, y) = match &command {
            DrawCommand::Put { x, y, .. } | DrawCommand::ClearToEnd { x, y, .. } => (*x, *y),
        };
        if !matches!(last_position, Some(position) if x == position.x.saturating_add(1) && y == position.y)
        {
            close_active_link(writer, &mut active_link)?;
            queue!(writer, MoveTo(x, y))?;
        }
        last_position = Some(Position::new(x, y));

        match command {
            DrawCommand::Put { cell, link, .. } => {
                switch_active_link(writer, &mut active_link, link)?;
                if cell.modifier != modifiers {
                    queue!(writer, SetAttribute(Attribute::Reset))?;
                    foreground = Color::Reset;
                    background = Color::Reset;
                    for (modifier, attribute) in [
                        (Modifier::REVERSED, Attribute::Reverse),
                        (Modifier::BOLD, Attribute::Bold),
                        (Modifier::ITALIC, Attribute::Italic),
                        (Modifier::UNDERLINED, Attribute::Underlined),
                        (Modifier::DIM, Attribute::Dim),
                        (Modifier::CROSSED_OUT, Attribute::CrossedOut),
                        (Modifier::SLOW_BLINK, Attribute::SlowBlink),
                        (Modifier::RAPID_BLINK, Attribute::RapidBlink),
                    ] {
                        if cell.modifier.contains(modifier) {
                            queue!(writer, SetAttribute(attribute))?;
                        }
                    }
                    modifiers = cell.modifier;
                }
                if cell.fg != foreground || cell.bg != background {
                    queue!(
                        writer,
                        SetColors(Colors::new(
                            cell.fg.into_crossterm(),
                            cell.bg.into_crossterm()
                        ))
                    )?;
                    foreground = cell.fg;
                    background = cell.bg;
                }
                queue!(writer, Print(cell.symbol()))?;
            }
            DrawCommand::ClearToEnd { bg, .. } => {
                close_active_link(writer, &mut active_link)?;
                queue!(writer, SetAttribute(Attribute::Reset))?;
                modifiers = Modifier::empty();
                foreground = Color::Reset;
                queue!(writer, SetBackgroundColor(bg.into_crossterm()))?;
                background = bg;
                queue!(writer, Clear(CrosstermClearType::UntilNewLine))?;
            }
        }
    }

    close_active_link(writer, &mut active_link)?;
    queue!(
        writer,
        SetForegroundColor(crossterm::style::Color::Reset),
        SetBackgroundColor(crossterm::style::Color::Reset),
        SetAttribute(Attribute::Reset),
    )
}

fn switch_active_link(
    writer: &mut impl Write,
    active: &mut Option<Hyperlink>,
    next: Option<Hyperlink>,
) -> io::Result<()> {
    if active.as_deref() == next.as_deref() {
        return Ok(());
    }
    close_active_link(writer, active)?;
    if let Some(next) = next
        && write_open(writer, &next)?
    {
        *active = Some(next);
    }
    Ok(())
}

fn close_active_link(writer: &mut impl Write, active: &mut Option<Hyperlink>) -> io::Result<()> {
    if active.take().is_some() {
        write_close(writer)?;
    }
    Ok(())
}

impl<B> Drop for Terminal<B>
where
    B: Backend<Error = io::Error> + Write,
{
    fn drop(&mut self) {
        if self.hyperlinks_enabled {
            let _ = write_close(&mut self.backend);
        }
        let _ = queue!(self.backend, SetCursorStyle::DefaultUserShape);
        if self.hidden_cursor {
            let _ = self.backend.show_cursor();
        }
        let _ = Write::flush(&mut self.backend);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ratatui::style::Style;

    use super::*;

    #[test]
    fn shorter_row_clears_the_tail_left_by_wide_text() {
        let area = Rect::new(0, 0, 40, 1);
        let mut previous = Buffer::empty(area);
        let mut next = Buffer::empty(area);
        previous.set_string(
            0,
            0,
            "tea-rs: 一个用 Rust 实现的无界面 Agent 运行时",
            Style::default(),
        );
        next.set_string(0, 0, "• event sequence gap detected", Style::default());

        let previous_links = HyperlinkBuffer::empty(area);
        let next_links = HyperlinkBuffer::empty(area);
        let commands = diff_buffers(&previous, &next, &previous_links, &next_links);
        let clear_x = u16::try_from("• event sequence gap detected".width()).unwrap();
        assert!(commands.iter().any(
            |command| matches!(command, DrawCommand::ClearToEnd { x, y: 0, .. } if *x == clear_x)
        ));
    }

    #[test]
    fn clear_to_end_uses_the_codex_terminal_sequence() {
        let mut output = Vec::new();
        draw(
            &mut output,
            [DrawCommand::ClearToEnd {
                x: 3,
                y: 2,
                bg: Color::Reset,
            }]
            .into_iter(),
        )
        .unwrap();

        assert!(
            output
                .windows(b"\x1b[K".len())
                .any(|bytes| bytes == b"\x1b[K")
        );
    }

    #[test]
    fn link_only_changes_redraw_the_visible_cell() {
        let area = Rect::new(0, 0, 2, 1);
        let mut previous = Buffer::empty(area);
        let mut next = Buffer::empty(area);
        previous.set_string(0, 0, "A", Style::default());
        next.set_string(0, 0, "A", Style::default());
        let mut previous_links = HyperlinkBuffer::empty(area);
        previous_links.set(Position::new(0, 0), Some(Arc::from("https://old.test/")));
        let mut next_links = HyperlinkBuffer::empty(area);
        next_links.set(Position::new(0, 0), Some(Arc::from("https://new.test/")));

        let commands = diff_buffers(&previous, &next, &previous_links, &next_links);

        assert!(commands.iter().any(|command| matches!(
            command,
            DrawCommand::Put { link: Some(link), .. } if &**link == "https://new.test/"
        )));
    }

    #[test]
    fn draw_switches_and_closes_osc8_links_before_cursor_jumps_and_frame_end() {
        let linked = Arc::<str>::from("https://example.test/");
        let mut first = Cell::default();
        first.set_symbol("A");
        let mut second = Cell::default();
        second.set_symbol("B");
        let mut output = Vec::new();

        draw(
            &mut output,
            [
                DrawCommand::Put {
                    x: 0,
                    y: 0,
                    cell: first,
                    link: Some(Arc::clone(&linked)),
                },
                DrawCommand::Put {
                    x: 1,
                    y: 0,
                    cell: second,
                    link: Some(linked),
                },
                DrawCommand::ClearToEnd {
                    x: 4,
                    y: 0,
                    bg: Color::Reset,
                },
            ]
            .into_iter(),
        )
        .unwrap();

        assert_eq!(
            output
                .windows(b"\x1b]8;;https://example.test/\x1b\\".len())
                .filter(|bytes| *bytes == b"\x1b]8;;https://example.test/\x1b\\")
                .count(),
            1
        );
        assert!(
            output
                .windows(b"AB\x1b]8;;\x1b\\".len())
                .any(|bytes| bytes == b"AB\x1b]8;;\x1b\\")
        );
    }
}
