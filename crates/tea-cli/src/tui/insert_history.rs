//! Codex-style insertion of finalized rows into terminal scrollback.
//!
//! Adapted from `OpenAI` Codex `codex-rs/tui/src/insert_history.rs` under Apache-2.0.

use std::fmt;
use std::io;
use std::io::Write;

use crossterm::Command;
use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{
    Attribute, Color as CrosstermColor, Colors, Print, SetAttribute, SetBackgroundColor, SetColors,
    SetForegroundColor,
};
use crossterm::terminal::{Clear, ClearType};
use ratatui::backend::{Backend, IntoCrossterm as _};
use ratatui::style::{Color, Modifier, Style};

use super::custom_terminal::Terminal;
use super::hyperlink::{write_close, write_open};
use super::render_output::RenderedLine;

/// Inserts finalized, pre-wrapped rows immediately above the inline viewport.
pub(crate) fn insert_history_lines<B>(
    terminal: &mut Terminal<B>,
    lines: &[RenderedLine],
) -> io::Result<()>
where
    B: Backend<Error = io::Error> + Write,
{
    if lines.is_empty() || terminal.viewport_area.width == 0 {
        return Ok(());
    }

    let screen_size = terminal.backend().size()?;
    let mut area = terminal.viewport_area;
    let last_cursor_pos = terminal.last_known_cursor_pos;
    let inserted_rows = u16::try_from(lines.len()).unwrap_or(u16::MAX);

    let cursor_top = if area.bottom() < screen_size.height {
        let scroll_amount = inserted_rows.min(screen_size.height - area.bottom());
        let top_1based = area.top() + 1;
        let writer = terminal.backend_mut();
        queue!(writer, SetScrollRegion(top_1based..screen_size.height))?;
        queue!(writer, MoveTo(0, area.top()))?;
        for _ in 0..scroll_amount {
            queue!(writer, Print("\x1bM"))?;
        }
        queue!(writer, ResetScrollRegion)?;

        let cursor_top = area.top().saturating_sub(1);
        area.y += scroll_amount;
        cursor_top
    } else {
        area.top().saturating_sub(1)
    };

    let hyperlinks_enabled = terminal.hyperlinks_enabled();
    let writer = terminal.backend_mut();
    queue!(writer, SetScrollRegion(1..area.top()))?;
    queue!(writer, MoveTo(0, cursor_top))?;
    for line in lines {
        queue!(writer, Print("\r\n"), Clear(ClearType::UntilNewLine))?;
        write_line(writer, line, hyperlinks_enabled)?;
    }
    queue!(writer, ResetScrollRegion)?;
    queue!(writer, MoveTo(last_cursor_pos.x, last_cursor_pos.y))?;

    if area != terminal.viewport_area {
        terminal.set_viewport_area(area);
    }
    terminal.invalidate_viewport();
    Ok(())
}

fn write_line(
    writer: &mut impl Write,
    line: &RenderedLine,
    hyperlinks_enabled: bool,
) -> io::Result<()> {
    if line.rendered_spans().is_empty() {
        return write_spans(
            writer,
            [OutputSpan {
                text: line.text(),
                style: line.style(),
                link: None,
            }],
            hyperlinks_enabled,
        );
    }
    write_spans(
        writer,
        line.rendered_spans().iter().map(|span| OutputSpan {
            text: span.text(),
            style: span.style(),
            link: span.link(),
        }),
        hyperlinks_enabled,
    )
}

#[derive(Clone, Copy)]
struct OutputSpan<'a> {
    text: &'a str,
    style: Style,
    link: Option<&'a str>,
}

fn write_spans<'a>(
    writer: &mut impl Write,
    content: impl IntoIterator<Item = OutputSpan<'a>>,
    hyperlinks_enabled: bool,
) -> io::Result<()> {
    let mut foreground = Color::Reset;
    let mut background = Color::Reset;
    let mut modifiers = Modifier::empty();
    let mut active_link: Option<String> = None;
    for span in content {
        let next_link = hyperlinks_enabled.then_some(span.link).flatten();
        if active_link.as_deref() != next_link {
            if active_link.take().is_some() {
                write_close(writer)?;
            }
            if let Some(next_link) = next_link
                && write_open(writer, next_link)?
            {
                active_link = Some(next_link.to_owned());
            }
        }
        let mut next_modifiers = Modifier::empty();
        next_modifiers.insert(span.style.add_modifier);
        next_modifiers.remove(span.style.sub_modifier);
        if next_modifiers != modifiers {
            ModifierDiff {
                from: modifiers,
                to: next_modifiers,
            }
            .queue(writer)?;
            modifiers = next_modifiers;
        }

        let next_foreground = span.style.fg.unwrap_or(Color::Reset);
        let next_background = span.style.bg.unwrap_or(Color::Reset);
        if next_foreground != foreground || next_background != background {
            queue!(
                writer,
                SetColors(Colors::new(
                    next_foreground.into_crossterm(),
                    next_background.into_crossterm()
                ))
            )?;
            foreground = next_foreground;
            background = next_background;
        }
        queue!(writer, Print(span.text))?;
    }

    if active_link.is_some() {
        write_close(writer)?;
    }
    queue!(
        writer,
        SetForegroundColor(CrosstermColor::Reset),
        SetBackgroundColor(CrosstermColor::Reset),
        SetAttribute(Attribute::Reset),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SetScrollRegion(std::ops::Range<u16>);

impl Command for SetScrollRegion {
    fn write_ansi(&self, writer: &mut impl fmt::Write) -> fmt::Result {
        write!(writer, "\x1b[{};{}r", self.0.start, self.0.end)
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        panic!("scroll regions require ANSI terminal processing")
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResetScrollRegion;

impl Command for ResetScrollRegion {
    fn write_ansi(&self, writer: &mut impl fmt::Write) -> fmt::Result {
        write!(writer, "\x1b[r")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        panic!("scroll regions require ANSI terminal processing")
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

struct ModifierDiff {
    from: Modifier,
    to: Modifier,
}

impl ModifierDiff {
    fn queue(self, writer: &mut impl Write) -> io::Result<()> {
        let removed = self.from - self.to;
        for (modifier, attribute) in [
            (Modifier::REVERSED, Attribute::NoReverse),
            (Modifier::ITALIC, Attribute::NoItalic),
            (Modifier::UNDERLINED, Attribute::NoUnderline),
            (Modifier::CROSSED_OUT, Attribute::NotCrossedOut),
        ] {
            if removed.contains(modifier) {
                queue!(writer, SetAttribute(attribute))?;
            }
        }
        if removed.intersects(Modifier::BOLD | Modifier::DIM) {
            queue!(writer, SetAttribute(Attribute::NormalIntensity))?;
            if self.to.contains(Modifier::DIM) {
                queue!(writer, SetAttribute(Attribute::Dim))?;
            }
        }
        if removed.intersects(Modifier::SLOW_BLINK | Modifier::RAPID_BLINK) {
            queue!(writer, SetAttribute(Attribute::NoBlink))?;
        }

        let added = self.to - self.from;
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
            if added.contains(modifier) {
                queue!(writer, SetAttribute(attribute))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Style;

    use super::*;

    #[test]
    fn styled_spans_reset_terminal_attributes() {
        let mut bytes = Vec::new();
        write_spans(
            &mut bytes,
            [
                OutputSpan {
                    text: "A",
                    style: Style::new().bold(),
                    link: None,
                },
                OutputSpan {
                    text: "B",
                    style: Style::default(),
                    link: None,
                },
            ],
            false,
        )
        .unwrap();
        let output = String::from_utf8(bytes).unwrap();
        assert!(output.contains('A'));
        assert!(output.contains('B'));
        assert!(output.ends_with("\x1b[0m"));
    }

    #[test]
    fn history_links_are_balanced_before_style_reset() {
        let mut bytes = Vec::new();
        write_spans(
            &mut bytes,
            [OutputSpan {
                text: "Tea docs",
                style: Style::new().underlined(),
                link: Some("https://example.test/"),
            }],
            true,
        )
        .unwrap();

        let output = String::from_utf8(bytes).unwrap();
        assert!(output.contains("\x1b]8;;https://example.test/\x1b\\"));
        assert!(output.contains("Tea docs\x1b]8;;\x1b\\"));
        assert!(output.ends_with("\x1b[0m"));
    }
}
