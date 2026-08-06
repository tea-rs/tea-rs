use ratatui::style::{Color, Modifier, Style};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

use super::super::markdown as projection;
use super::super::render_output::{RenderedLine, RenderedSpan, push_rendered_span};
use super::super::terminal::ColorCapability;
use super::text::sanitize;
use super::{Component, RenderBlock, RenderContext};

pub(crate) struct Markdown {
    source: String,
}

impl Markdown {
    pub(crate) fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
        }
    }
}

impl Component for Markdown {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        let requested_width = usize::from(width);
        let projection_width = if requested_width == 1 {
            4
        } else {
            requested_width
        };
        let lines = projection::project_with_width(&self.source, Some(projection_width))
            .iter()
            .map(|line| render_markdown_line(line, ctx))
            .collect::<Vec<_>>();
        if projection_width == requested_width {
            RenderBlock::from_lines(lines)
        } else {
            RenderBlock::from_lines(fit_lines_to_width(lines, requested_width))
        }
    }
}

fn fit_lines_to_width(lines: Vec<RenderedLine>, width: usize) -> Vec<RenderedLine> {
    lines
        .into_iter()
        .flat_map(|line| fit_line_to_width(line, width))
        .collect()
}

fn fit_line_to_width(line: RenderedLine, width: usize) -> Vec<RenderedLine> {
    let line_style = line.style;
    let source_spans = if line.spans.is_empty() {
        vec![RenderedSpan::new(line.text, line_style)]
    } else {
        line.spans
    };
    let mut output = Vec::new();
    let mut current = Vec::new();
    let mut current_width = 0;

    for span in source_spans {
        for grapheme in span.text.graphemes(true) {
            let grapheme_width = grapheme.width();
            if grapheme_width == 0 {
                if current_width < width {
                    push_rendered_span(&mut current, span_fragment(&span, grapheme));
                }
                continue;
            }
            let (text, rendered_width) = if grapheme_width > width {
                ("�", 1)
            } else {
                (grapheme, grapheme_width)
            };
            if current_width.saturating_add(rendered_width) > width && !current.is_empty() {
                output.push(RenderedLine::from_spans(
                    line_style,
                    std::mem::take(&mut current),
                ));
                current_width = 0;
            }
            if current_width.saturating_add(rendered_width) <= width {
                push_rendered_span(&mut current, span_fragment(&span, text));
                current_width = current_width.saturating_add(rendered_width);
            }
        }
    }
    if !current.is_empty() || output.is_empty() {
        output.push(RenderedLine::from_spans(line_style, current));
    }
    output
}

fn span_fragment(span: &RenderedSpan, text: &str) -> RenderedSpan {
    match span.link() {
        Some(destination) => {
            RenderedSpan::with_link(text.to_owned(), span.style(), destination.to_owned())
        }
        None => RenderedSpan::new(text.to_owned(), span.style()),
    }
}

fn render_markdown_line(line: &projection::MarkdownLine, ctx: &RenderContext<'_>) -> RenderedLine {
    let theme = ctx.theme;
    let style = match line.kind() {
        projection::MarkdownKind::Paragraph => theme.assistant,
        projection::MarkdownKind::Heading(_) => theme.markdown_heading,
        projection::MarkdownKind::Quote(_) => theme.markdown_quote,
        projection::MarkdownKind::Code => theme.markdown_code,
        projection::MarkdownKind::Rule => theme.footer,
        projection::MarkdownKind::Blank => {
            return RenderedLine::new(String::new(), theme.assistant);
        }
    };
    let mut spans = Vec::with_capacity(line.spans().len());
    for span in line.spans() {
        let text = sanitize(span.text());
        let mut span_style = markdown_span_style(style, span.style(), ctx);
        if let Some(syntax) = span.syntax_style() {
            span_style = markdown_syntax_style(span_style, syntax, ctx);
        }
        let rendered = match span.link() {
            Some(destination) => RenderedSpan::with_link(text, span_style, destination.to_owned()),
            None => RenderedSpan::new(text, span_style),
        };
        push_rendered_span(&mut spans, rendered);
    }
    RenderedLine::from_spans(style, spans)
}

fn markdown_span_style(
    base: Style,
    markdown: projection::MarkdownSpanStyle,
    ctx: &RenderContext<'_>,
) -> Style {
    let theme = ctx.theme;
    let mut style = base;
    if markdown.link() {
        style = style.patch(theme.markdown_link);
    }
    if markdown.code() {
        style = style.patch(theme.markdown_inline_code);
    }
    if markdown.block_code() {
        style = style.patch(theme.markdown_code);
    }
    if markdown.list_marker() {
        style = style.patch(theme.markdown_list_marker);
    }
    if markdown.strong() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if markdown.emphasis() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if markdown.strikethrough() {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }
    style
}

fn markdown_syntax_style(
    base: Style,
    syntax: projection::MarkdownSyntaxStyle,
    ctx: &RenderContext<'_>,
) -> Style {
    let mut style = base;
    let color_capability = ctx.theme.color_capability();
    if color_capability == ColorCapability::None {
        return style;
    }
    if let Some(projection::MarkdownSyntaxColor::Rgb(red, green, blue)) = syntax.foreground {
        let foreground = match color_capability {
            ColorCapability::None => return style,
            ColorCapability::Ansi16 => nearest_ansi16(red, green, blue),
            ColorCapability::Ansi256 => Color::Indexed(nearest_ansi256(red, green, blue)),
            ColorCapability::TrueColor => Color::Rgb(red, green, blue),
        };
        style = style.fg(foreground);
    }
    if syntax.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    style
}

fn nearest_ansi16(red: u8, green: u8, blue: u8) -> Color {
    const PALETTE: [((u8, u8, u8), Color); 16] = [
        ((0, 0, 0), Color::Black),
        ((205, 49, 49), Color::Red),
        ((13, 188, 121), Color::Green),
        ((229, 229, 16), Color::Yellow),
        ((36, 114, 200), Color::Blue),
        ((188, 63, 188), Color::Magenta),
        ((17, 168, 205), Color::Cyan),
        ((229, 229, 229), Color::Gray),
        ((102, 102, 102), Color::DarkGray),
        ((241, 76, 76), Color::LightRed),
        ((35, 209, 139), Color::LightGreen),
        ((245, 245, 67), Color::LightYellow),
        ((59, 142, 234), Color::LightBlue),
        ((214, 112, 214), Color::LightMagenta),
        ((41, 184, 219), Color::LightCyan),
        ((255, 255, 255), Color::White),
    ];
    PALETTE
        .iter()
        .min_by_key(|(candidate, _)| color_distance((red, green, blue), *candidate))
        .map_or(Color::Gray, |(_, color)| *color)
}

fn nearest_ansi256(red: u8, green: u8, blue: u8) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let nearest = |value: u8| {
        LEVELS
            .iter()
            .enumerate()
            .min_by_key(|(_, candidate)| value.abs_diff(**candidate))
            .map_or((0, 0), |(index, level)| (index, *level))
    };
    let (red_index, cube_red) = nearest(red);
    let (green_index, cube_green) = nearest(green);
    let (blue_index, cube_blue) = nearest(blue);
    let cube = 16 + 36 * red_index + 6 * green_index + blue_index;

    let average = (u16::from(red) + u16::from(green) + u16::from(blue)) / 3;
    let gray_index = usize::from(average.saturating_sub(8).saturating_add(5) / 10).min(23);
    let gray_level = 8_u8.saturating_add(u8::try_from(gray_index).unwrap_or(23) * 10);
    let gray = 232 + gray_index;
    let rgb = (red, green, blue);
    let selected = if color_distance(rgb, (gray_level, gray_level, gray_level))
        < color_distance(rgb, (cube_red, cube_green, cube_blue))
    {
        gray
    } else {
        cube
    };
    u8::try_from(selected).unwrap_or(231)
}

fn color_distance(left: (u8, u8, u8), right: (u8, u8, u8)) -> u32 {
    [
        left.0.abs_diff(right.0),
        left.1.abs_diff(right.1),
        left.2.abs_diff(right.2),
    ]
    .into_iter()
    .map(|difference| u32::from(difference).pow(2))
    .sum()
}

#[cfg(test)]
mod tests {
    use ratatui::style::Modifier;
    use unicode_width::UnicodeWidthStr as _;

    use super::Markdown;
    use crate::tui::components::{Element, RenderContext};
    use crate::tui::{RenderedLine, Theme};

    fn render(source: &str, width: u16, theme: &Theme) -> Vec<RenderedLine> {
        let context = RenderContext { theme };
        Element::new(Markdown::new(source))
            .render(&context, width)
            .into_lines()
    }

    #[test]
    fn headings_lists_code_and_links_preserve_semantic_styles() {
        let theme = Theme::default();
        let lines = render(
            "# Heading\n\n- **bold** and `code`\n\n[docs](https://example.com/docs)",
            80,
            &theme,
        );

        assert_eq!(lines[0].style(), theme.markdown_heading);
        assert!(lines[0].style().add_modifier.contains(Modifier::BOLD));
        assert!(lines.iter().flat_map(RenderedLine::rendered_spans).any(
            |span| span.text().contains('-') && span.style() == theme.markdown_list_marker
        ));
        assert!(
            lines
                .iter()
                .flat_map(RenderedLine::rendered_spans)
                .any(|span| span.text() == "code"
                    && span.style().fg == theme.markdown_inline_code.fg)
        );
        assert!(
            lines
                .iter()
                .flat_map(RenderedLine::rendered_spans)
                .any(|span| span.text().contains("docs")
                    && span.link() == Some("https://example.com/docs"))
        );
    }

    #[test]
    fn validated_hyperlink_metadata_survives_markdown_rendering() {
        let theme = Theme::default();
        let lines = render(
            "Open [Tea](HTTPS://Example.COM:443/a/../docs) now.",
            24,
            &theme,
        );
        let destinations = lines
            .iter()
            .flat_map(RenderedLine::rendered_spans)
            .filter_map(|span| span.link())
            .collect::<Vec<_>>();

        assert!(!destinations.is_empty());
        assert!(
            destinations
                .iter()
                .all(|link| *link == "https://example.com/docs")
        );
        assert!(lines.iter().all(|line| !line.text().contains('\u{1b}')));
    }

    #[test]
    fn width_one_and_wide_unicode_never_overflow() {
        let theme = Theme::default();
        let lines = render("[界A](https://example.com)", 1, &theme);

        assert!(lines.iter().all(|line| line.text().width() <= 1));
        assert!(
            lines
                .iter()
                .flat_map(RenderedLine::rendered_spans)
                .any(|span| span.link() == Some("https://example.com/"))
        );
    }

    #[test]
    fn rendering_does_not_modify_raw_markdown_source() {
        let theme = Theme::default();
        let source = String::from("## Heading\n\n`界` and **bold**");
        let original = source.clone();

        let _ = render(&source, 8, &theme);

        assert_eq!(source, original);
    }
}
