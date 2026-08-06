use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// One width-bounded terminal row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedLine {
    pub(crate) text: String,
    pub(crate) style: Style,
    pub(crate) spans: Vec<RenderedSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderedSpan {
    pub(crate) text: String,
    pub(crate) style: Style,
    pub(crate) link: Option<String>,
}

impl RenderedSpan {
    pub(crate) fn new(text: String, style: Style) -> Self {
        Self {
            text,
            style,
            link: None,
        }
    }

    pub(crate) fn with_link(text: String, style: Style, link: String) -> Self {
        Self {
            text,
            style,
            link: Some(link),
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) const fn style(&self) -> Style {
        self.style
    }

    pub(crate) fn link(&self) -> Option<&str> {
        self.link.as_deref()
    }
}

pub(crate) fn push_rendered_span(output: &mut Vec<RenderedSpan>, span: RenderedSpan) {
    if let Some(last) = output.last_mut()
        && last.style == span.style
        && last.link == span.link
    {
        last.text.push_str(&span.text);
    } else {
        output.push(span);
    }
}

impl RenderedLine {
    /// Creates one renderer-owned terminal row.
    #[must_use]
    pub(crate) fn new(text: String, style: Style) -> Self {
        Self {
            text,
            style,
            spans: Vec::new(),
        }
    }

    pub(crate) fn from_spans(style: Style, spans: Vec<RenderedSpan>) -> Self {
        let text = spans.iter().map(|span| span.text.as_str()).collect();
        Self { text, style, spans }
    }

    /// Returns row text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns semantic row style.
    #[must_use]
    pub const fn style(&self) -> Style {
        self.style
    }

    pub(crate) fn as_ratatui_line(&self) -> Line<'_> {
        if self.spans.is_empty() {
            return Line::styled(self.text.as_str(), self.style);
        }
        Line::from(
            self.spans
                .iter()
                .map(|span| Span::styled(span.text.as_str(), span.style))
                .collect::<Vec<_>>(),
        )
    }

    pub(crate) fn rendered_spans(&self) -> &[RenderedSpan] {
        &self.spans
    }

    pub(crate) fn with_left_columns(self, columns: usize) -> Self {
        if columns == 0 {
            return self;
        }
        let mut spans = Vec::with_capacity(self.spans.len().saturating_add(2));
        spans.push(RenderedSpan::new(" ".repeat(columns), Style::default()));
        if self.spans.is_empty() {
            spans.push(RenderedSpan::new(self.text, self.style));
        } else {
            spans.extend(self.spans);
        }
        Self::from_spans(Style::default(), spans)
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Style};

    use super::{RenderedLine, RenderedSpan, push_rendered_span};

    #[test]
    fn adjacent_spans_merge_only_when_style_and_link_match() {
        let style = Style::default().fg(Color::Green);
        let other_style = Style::default().fg(Color::Red);
        let mut spans = Vec::new();

        push_rendered_span(
            &mut spans,
            RenderedSpan::with_link("a".to_owned(), style, "https://example.com/a".to_owned()),
        );
        push_rendered_span(
            &mut spans,
            RenderedSpan::with_link("b".to_owned(), style, "https://example.com/a".to_owned()),
        );
        push_rendered_span(
            &mut spans,
            RenderedSpan::with_link("c".to_owned(), style, "https://example.com/b".to_owned()),
        );
        push_rendered_span(&mut spans, RenderedSpan::new("d".to_owned(), other_style));

        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].text(), "ab");
        assert_eq!(spans[0].link(), Some("https://example.com/a"));
        assert_eq!(spans[1].text(), "c");
        assert_eq!(spans[1].link(), Some("https://example.com/b"));
        assert_eq!(spans[2].text(), "d");
        assert_eq!(spans[2].style(), other_style);
    }

    #[test]
    fn prepending_columns_preserves_link_metadata() {
        let destination = "https://example.com/docs";
        let style = Style::default().fg(Color::Blue);
        let line = RenderedLine::from_spans(
            style,
            vec![RenderedSpan::with_link(
                "docs".to_owned(),
                style,
                destination.to_owned(),
            )],
        )
        .with_left_columns(2);

        assert_eq!(line.text(), "  docs");
        assert_eq!(line.rendered_spans().len(), 2);
        assert_eq!(line.rendered_spans()[0].text(), "  ");
        assert_eq!(line.rendered_spans()[0].link(), None);
        assert_eq!(line.rendered_spans()[1].text(), "docs");
        assert_eq!(line.rendered_spans()[1].link(), Some(destination));
    }
}
