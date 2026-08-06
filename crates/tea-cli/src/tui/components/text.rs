use ratatui::style::Style;
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

use super::super::render_output::{RenderedLine, RenderedSpan};
use super::{Component, RenderBlock, RenderContext};

const MAX_RENDER_TEXT_BYTES: usize = 32 * 1024;

enum TextLayout {
    LogicalLines,
    SingleLine,
    Truncated,
}

enum Prefix {
    None,
    Label(String),
    Explicit {
        first: String,
        continuation: Option<String>,
    },
}

pub(crate) struct Text {
    source: String,
    style: Style,
    prefix_style: Option<Style>,
    prefix: Prefix,
    layout: TextLayout,
}

impl Text {
    pub(crate) fn new(source: impl Into<String>, style: Style) -> Self {
        Self {
            source: source.into(),
            style,
            prefix_style: None,
            prefix: Prefix::None,
            layout: TextLayout::LogicalLines,
        }
    }

    pub(crate) fn single_line(source: impl Into<String>, style: Style) -> Self {
        Self {
            layout: TextLayout::SingleLine,
            ..Self::new(source, style)
        }
    }

    pub(crate) fn truncated(source: impl Into<String>, style: Style) -> Self {
        Self {
            layout: TextLayout::Truncated,
            ..Self::new(source, style)
        }
    }

    pub(crate) fn with_label(mut self, label: impl Into<String>) -> Self {
        self.prefix = Prefix::Label(label.into());
        self
    }

    pub(crate) fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Prefix::Explicit {
            first: prefix.into(),
            continuation: None,
        };
        self
    }

    pub(crate) fn with_prefixes(
        mut self,
        first: impl Into<String>,
        continuation: impl Into<String>,
    ) -> Self {
        self.prefix = Prefix::Explicit {
            first: first.into(),
            continuation: Some(continuation.into()),
        };
        self
    }

    pub(crate) const fn with_prefix_style(mut self, style: Style) -> Self {
        self.prefix_style = Some(style);
        self
    }

    fn prefixes(&self, width: usize) -> (String, String) {
        match &self.prefix {
            Prefix::None => (String::new(), String::new()),
            Prefix::Label(label) => {
                let label = sanitize(label);
                let first = if label.is_empty() {
                    String::new()
                } else {
                    format!("{label}: ")
                };
                let continuation = " ".repeat(first.width().min(width.saturating_sub(1)));
                (first, continuation)
            }
            Prefix::Explicit {
                first,
                continuation,
            } => {
                let continuation = continuation.clone().unwrap_or_else(|| {
                    " ".repeat(
                        truncate_cells(first, width)
                            .width()
                            .min(width.saturating_sub(1)),
                    )
                });
                (first.clone(), continuation)
            }
        }
    }
}

impl Component for Text {
    fn render(&self, _ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        let width = usize::from(width);
        let source = sanitize(&self.source);
        let (first_prefix, continuation) = self.prefixes(width);
        let mut lines = match self.layout {
            TextLayout::Truncated => vec![RenderedLine::new(
                truncate_cells(&format!("{first_prefix}{source}"), width),
                self.style,
            )],
            TextLayout::SingleLine => {
                let mut lines = Vec::new();
                wrap_one(&mut lines, &first_prefix, &source, width, self.style);
                lines
            }
            TextLayout::LogicalLines => {
                let mut lines = Vec::new();
                for (line_index, logical_line) in source.split('\n').enumerate() {
                    let prefix = if line_index == 0 {
                        first_prefix.as_str()
                    } else {
                        continuation.as_str()
                    };
                    wrap_one(&mut lines, prefix, logical_line, width, self.style);
                }
                lines
            }
        };

        if let Some(prefix_style) = self.prefix_style {
            lines = style_prefixes(
                lines,
                &first_prefix,
                &continuation,
                prefix_style,
                self.style,
            );
        }
        RenderBlock::from_lines(lines)
    }
}

fn style_prefixes(
    lines: Vec<RenderedLine>,
    first_prefix: &str,
    continuation: &str,
    prefix_style: Style,
    content_style: Style,
) -> Vec<RenderedLine> {
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let prefix = if index == 0 {
                first_prefix
            } else {
                continuation
            };
            let Some(content) = line.text.strip_prefix(prefix) else {
                return line;
            };
            RenderedLine::from_spans(
                content_style,
                vec![
                    RenderedSpan::new(prefix.to_owned(), prefix_style),
                    RenderedSpan::new(content.to_owned(), content_style),
                ],
            )
        })
        .collect()
}

fn wrap_one(
    output: &mut Vec<RenderedLine>,
    first_prefix: &str,
    text: &str,
    width: usize,
    style: Style,
) {
    let mut current = truncate_cells(first_prefix, width);
    let indent_width = current.width().min(width.saturating_sub(1));
    let indent = " ".repeat(indent_width);
    for grapheme in text.graphemes(true) {
        let grapheme_width = grapheme.width();
        if grapheme_width == 0 {
            if current.width() < width {
                current.push_str(grapheme);
            }
            continue;
        }
        if current.width().saturating_add(grapheme_width) > width {
            output.push(RenderedLine::new(std::mem::take(&mut current), style));
            current.push_str(&indent);
        }
        if grapheme_width <= width.saturating_sub(current.width()) {
            current.push_str(grapheme);
        } else {
            current.push('�');
        }
    }
    output.push(RenderedLine::new(current, style));
}

pub(crate) fn truncate_cells(text: &str, width: usize) -> String {
    let mut result = String::new();
    for grapheme in text.graphemes(true) {
        if result.width().saturating_add(grapheme.width()) > width {
            break;
        }
        result.push_str(grapheme);
    }
    result
}

pub(crate) fn truncate_cells_with_ellipsis(text: &str, width: usize) -> String {
    let text = sanitize(text);
    if text.width() <= width {
        return text;
    }
    let marker = if width >= 3 { "..." } else { "." };
    let prefix = truncate_cells(&text, width.saturating_sub(marker.width()));
    format!("{prefix}{marker}")
}

pub(crate) fn sanitize(text: &str) -> String {
    let end = if text.len() <= MAX_RENDER_TEXT_BYTES {
        text.len()
    } else {
        let mut end = MAX_RENDER_TEXT_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        end
    };
    text[..end]
        .chars()
        .map(|character| match character {
            '\n' => '\n',
            '\t' => ' ',
            '\u{1b}' => '␛',
            value if value.is_control() => '�',
            value => value,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Style};
    use unicode_width::UnicodeWidthStr as _;

    use super::Text;
    use crate::tui::components::{Element, RenderContext};
    use crate::tui::{RenderedLine, Theme};

    fn render(text: Text, width: u16) -> Vec<RenderedLine> {
        let theme = Theme::default();
        let context = RenderContext { theme: &theme };
        Element::new(text).render(&context, width).into_lines()
    }

    #[test]
    fn plain_text_wraps_at_display_cell_width() {
        let lines = render(Text::new("alpha界beta", Style::default()), 5);

        assert_eq!(
            lines.iter().map(RenderedLine::text).collect::<Vec<_>>(),
            vec!["alpha", "界bet", "a"]
        );
        assert!(lines.iter().all(|line| line.text().width() <= 5));
    }

    #[test]
    fn prefixes_and_continuation_prefixes_are_width_safe() {
        let lines = render(
            Text::new("abcdef\nxy", Style::default())
                .with_prefixes("• ", "  ")
                .with_prefix_style(Style::default().fg(Color::Yellow)),
            4,
        );

        assert_eq!(
            lines.iter().map(RenderedLine::text).collect::<Vec<_>>(),
            vec!["• ab", "  cd", "  ef", "  xy"]
        );
        assert!(lines.iter().all(|line| line.text().width() <= 4));
    }

    #[test]
    fn width_one_replaces_unrepresentable_wide_graphemes_without_overflow() {
        let lines = render(Text::new("界A", Style::default()), 1);

        assert!(lines.iter().all(|line| line.text().width() <= 1));
        assert!(lines.iter().any(|line| line.text().contains('�')));
        assert_eq!(lines.last().map(RenderedLine::text), Some("A"));
    }

    #[test]
    fn rendering_does_not_modify_raw_source() {
        let source = String::from("line one\nline two\t界");
        let original = source.clone();

        let _ = render(Text::new(source.as_str(), Style::default()), 6);

        assert_eq!(source, original);
    }
}
