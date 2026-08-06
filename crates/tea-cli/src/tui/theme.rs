use ratatui::style::{Color, Modifier, Style};

use super::terminal::{ColorCapability, TerminalCapabilities};

/// Small semantic terminal palette independent of durable state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Monotonic identifier included in render-cache keys.
    pub generation: u64,
    /// Normal transcript text.
    pub normal: Style,
    /// User-authored text.
    pub user: Style,
    /// Assistant-authored text.
    pub assistant: Style,
    /// Markdown heading text.
    pub markdown_heading: Style,
    /// Markdown quote text.
    pub markdown_quote: Style,
    /// Markdown fenced or indented code.
    pub markdown_code: Style,
    /// Markdown inline code.
    pub markdown_inline_code: Style,
    /// Markdown link labels and visible fallback targets.
    pub markdown_link: Style,
    /// Markdown ordered and unordered list markers.
    pub markdown_list_marker: Style,
    /// Reasoning text.
    pub thinking: Style,
    /// Tool activity.
    pub tool: Style,
    /// Successful lifecycle or result output.
    pub success: Style,
    /// Warning lifecycle or notice output.
    pub warning: Style,
    /// Failed lifecycle or result output.
    pub error: Style,
    /// Low-salience informational output.
    pub information: Style,
    /// Approval warning.
    pub approval: Style,
    /// Error/notification text.
    pub notification: Style,
    /// Editor input.
    pub editor: Style,
    /// Full-width low-contrast editor surface.
    pub composer: Style,
    /// Footer statistics.
    pub footer: Style,
    reduced_motion: bool,
    color_capability: ColorCapability,
}

impl Theme {
    /// Builds a palette that avoids unsupported terminal output sequences.
    #[must_use]
    pub(crate) fn for_capabilities(capabilities: TerminalCapabilities) -> Self {
        let reduced_motion = capabilities.reduced_motion();
        match capabilities.color() {
            ColorCapability::None => Self {
                generation: 1 + u64::from(reduced_motion),
                normal: Style::default(),
                user: Style::default(),
                assistant: Style::default(),
                markdown_heading: Style::default().add_modifier(Modifier::BOLD),
                markdown_quote: Style::default().add_modifier(Modifier::ITALIC),
                markdown_code: Style::default().add_modifier(Modifier::DIM),
                markdown_inline_code: Style::default().add_modifier(Modifier::REVERSED),
                markdown_link: Style::default().add_modifier(Modifier::UNDERLINED),
                markdown_list_marker: Style::default().add_modifier(Modifier::BOLD),
                thinking: Style::default().add_modifier(Modifier::DIM),
                tool: Style::default().add_modifier(Modifier::BOLD),
                success: Style::default().add_modifier(Modifier::BOLD),
                warning: Style::default().add_modifier(Modifier::BOLD),
                error: Style::default().add_modifier(Modifier::BOLD),
                information: Style::default(),
                approval: Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
                notification: Style::default().add_modifier(Modifier::BOLD),
                editor: Style::default(),
                composer: Style::default().add_modifier(Modifier::REVERSED),
                footer: Style::default().add_modifier(Modifier::DIM),
                reduced_motion,
                color_capability: ColorCapability::None,
            },
            ColorCapability::Ansi16 => Self::with_palette(
                3 + u64::from(reduced_motion),
                Color::DarkGray,
                reduced_motion,
                ColorCapability::Ansi16,
            ),
            ColorCapability::Ansi256 => Self::with_palette(
                5 + u64::from(reduced_motion),
                Color::Indexed(236),
                reduced_motion,
                ColorCapability::Ansi256,
            ),
            ColorCapability::TrueColor => Self::with_palette(
                7 + u64::from(reduced_motion),
                Color::Rgb(36, 40, 45),
                reduced_motion,
                ColorCapability::TrueColor,
            ),
        }
    }

    fn with_palette(
        generation: u64,
        composer_background: Color,
        reduced_motion: bool,
        color_capability: ColorCapability,
    ) -> Self {
        Self {
            generation,
            normal: Style::default().fg(Color::Reset),
            user: Style::default().fg(Color::Cyan),
            assistant: Style::default().fg(Color::White),
            markdown_heading: Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            markdown_quote: Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::ITALIC),
            markdown_code: Style::default().fg(Color::Gray),
            markdown_inline_code: Style::default().fg(Color::LightCyan),
            markdown_link: Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::UNDERLINED),
            markdown_list_marker: Style::default().fg(Color::LightBlue),
            thinking: Style::default().fg(Color::DarkGray),
            tool: Style::default().fg(Color::Yellow),
            success: Style::default().fg(Color::Green),
            warning: Style::default().fg(Color::Yellow),
            error: Style::default().fg(Color::LightRed),
            information: Style::default().fg(Color::Reset),
            approval: Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            notification: Style::default().fg(Color::LightRed),
            editor: Style::default().fg(Color::LightCyan),
            composer: Style::default().fg(Color::White).bg(composer_background),
            footer: Style::default().fg(Color::DarkGray),
            reduced_motion,
            color_capability,
        }
    }

    pub(crate) const fn color_capability(&self) -> ColorCapability {
        self.color_capability
    }

    /// Returns the status marker for the current elapsed-second tick.
    #[must_use]
    pub(crate) const fn status_marker(&self, elapsed_seconds: u64) -> &'static str {
        if self.reduced_motion {
            "*"
        } else {
            match elapsed_seconds % 4 {
                0 => "*",
                1 | 3 => "+",
                _ => "x",
            }
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::for_capabilities(TerminalCapabilities::from_environment(
            Some("xterm-truecolor"),
            Some("truecolor"),
            false,
            false,
        ))
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Modifier;

    use super::{TerminalCapabilities, Theme};

    #[test]
    fn no_color_theme_keeps_the_composer_visible_and_motion_still() {
        let theme = Theme::for_capabilities(TerminalCapabilities::from_environment(
            Some("dumb"),
            None,
            false,
            false,
        ));

        assert!(theme.composer.add_modifier.contains(Modifier::REVERSED));
        assert_eq!(theme.status_marker(0), "*");
        assert_eq!(theme.status_marker(2), "*");
    }
}
