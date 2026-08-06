// Adapted from OpenAI Codex's Apache-2.0 licensed TUI wrapping component at
// codex-rs/tui/src/wrapping.rs, commit 1836ae0612052137d0cabaff7807ff8314cee940.

use std::ops::Range;

use textwrap::Options;
use textwrap::WordSeparator;
use textwrap::core::Word;
use unicode_width::UnicodeWidthStr as _;

use super::{MarkdownSpan, push_span_with_metadata};

pub(super) fn wrap_spans(
    content: &[MarkdownSpan],
    initial_indent: &[MarkdownSpan],
    subsequent_indent: &[MarkdownSpan],
    width: usize,
) -> Vec<Vec<MarkdownSpan>> {
    let (flat, span_bounds) = flatten_spans(content);
    if flat.is_empty() {
        return vec![initial_indent.to_vec()];
    }

    let width = width.max(1);
    let initial_available = width.saturating_sub(spans_width(initial_indent)).max(1);
    let subsequent_available = width.saturating_sub(spans_width(subsequent_indent)).max(1);
    let (saw_url, saw_non_url) = token_kinds(&flat);
    if saw_url && saw_non_url {
        let ranges = mixed_url_wrap_ranges(&flat, initial_available, subsequent_available);
        if ranges.is_empty() {
            return vec![initial_indent.to_vec()];
        }
        return ranges
            .iter()
            .enumerate()
            .map(|(index, range)| {
                let prefix = if index == 0 {
                    initial_indent
                } else {
                    subsequent_indent
                };
                prefixed_slice(prefix, content, &span_bounds, range)
            })
            .collect();
    }

    let options = wrapping_options(initial_available, saw_url);
    let first_ranges = wrap_ranges_trim(&flat, &options);
    let Some(first_range) = first_ranges.first() else {
        return vec![initial_indent.to_vec()];
    };

    let mut output = Vec::new();
    output.push(prefixed_slice(
        initial_indent,
        content,
        &span_bounds,
        first_range,
    ));

    let base = skip_spaces(&flat, first_range.end);
    let subsequent_options = wrapping_options(subsequent_available, saw_url);
    for range in wrap_ranges_trim(&flat[base..], &subsequent_options) {
        if range.is_empty() {
            continue;
        }
        let source_range = (range.start + base)..(range.end + base);
        output.push(prefixed_slice(
            subsequent_indent,
            content,
            &span_bounds,
            &source_range,
        ));
    }

    output
}

fn wrapping_options(width: usize, preserve_urls: bool) -> Options<'static> {
    let options = Options::new(width).wrap_algorithm(textwrap::WrapAlgorithm::FirstFit);
    if preserve_urls {
        options
            .word_separator(WordSeparator::AsciiSpace)
            .word_splitter(textwrap::WordSplitter::NoHyphenation)
            .break_words(false)
    } else {
        options
    }
}

fn token_kinds(text: &str) -> (bool, bool) {
    let mut saw_url = false;
    let mut saw_non_url = false;
    for token in text.split_ascii_whitespace() {
        if is_url_like_token(token) {
            saw_url = true;
        } else if is_substantive_non_url_token(token) {
            saw_non_url = true;
        }
        if saw_url && saw_non_url {
            break;
        }
    }
    (saw_url, saw_non_url)
}

fn is_url_like_token(raw_token: &str) -> bool {
    let token = trim_url_token(raw_token);
    !token.is_empty() && (is_absolute_url_like(token) || is_bare_url_like(token))
}

fn is_substantive_non_url_token(raw_token: &str) -> bool {
    let token = trim_url_token(raw_token);
    if token.is_empty() || is_decorative_marker_token(raw_token, token) {
        return false;
    }
    token.chars().any(char::is_alphanumeric)
}

fn is_decorative_marker_token(raw_token: &str, token: &str) -> bool {
    let raw = raw_token.trim();
    matches!(
        raw,
        "-" | "*"
            | "+"
            | "•"
            | "◦"
            | "▪"
            | ">"
            | "|"
            | "│"
            | "┆"
            | "└"
            | "├"
            | "┌"
            | "┐"
            | "┘"
            | "┼"
    ) || is_ordered_list_marker(raw, token)
}

fn is_ordered_list_marker(raw_token: &str, token: &str) -> bool {
    token.chars().all(|character| character.is_ascii_digit())
        && (raw_token.ends_with('.') || raw_token.ends_with(')'))
}

fn trim_url_token(token: &str) -> &str {
    token.trim_matches(|character: char| {
        matches!(
            character,
            '(' | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | ','
                | '.'
                | ';'
                | ':'
                | '!'
                | '\''
                | '"'
        )
    })
}

fn is_absolute_url_like(token: &str) -> bool {
    if !token.contains("://") {
        return false;
    }
    if let Ok(url) = url::Url::parse(token) {
        let scheme = url.scheme().to_ascii_lowercase();
        if matches!(
            scheme.as_str(),
            "http" | "https" | "ftp" | "ftps" | "ws" | "wss"
        ) {
            return url.host_str().is_some();
        }
        return true;
    }
    has_valid_scheme_prefix(token)
}

fn has_valid_scheme_prefix(token: &str) -> bool {
    let Some((scheme, rest)) = token.split_once("://") else {
        return false;
    };
    if scheme.is_empty() || rest.is_empty() {
        return false;
    }
    let mut characters = scheme.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

fn is_bare_url_like(token: &str) -> bool {
    let (host_port, has_trailer) = split_host_port_and_trailer(token);
    if host_port.is_empty() {
        return false;
    }
    if !has_trailer && !host_port.to_ascii_lowercase().starts_with("www.") {
        return false;
    }
    let (host, port) = split_host_and_port(host_port);
    if host.is_empty() || port.is_some_and(|port| !is_valid_port(port)) {
        return false;
    }
    host.eq_ignore_ascii_case("localhost") || is_ipv4(host) || is_domain_name(host)
}

fn split_host_port_and_trailer(token: &str) -> (&str, bool) {
    token
        .find(['/', '?', '#'])
        .map_or((token, false), |index| (&token[..index], true))
}

fn split_host_and_port(host_port: &str) -> (&str, Option<&str>) {
    if host_port.starts_with('[') {
        return (host_port, None);
    }
    if let Some((host, port)) = host_port.rsplit_once(':')
        && !host.is_empty()
        && !port.is_empty()
        && port.chars().all(|character| character.is_ascii_digit())
    {
        return (host, Some(port));
    }
    (host_port, None)
}

fn is_valid_port(port: &str) -> bool {
    !port.is_empty()
        && port.len() <= 5
        && port.chars().all(|character| character.is_ascii_digit())
        && port.parse::<u16>().is_ok()
}

fn is_ipv4(host: &str) -> bool {
    let parts = host.split('.').collect::<Vec<_>>();
    parts.len() == 4
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.parse::<u8>().is_ok())
}

fn is_domain_name(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if !host.contains('.') {
        return false;
    }
    let mut labels = host.split('.');
    let Some(tld) = labels.next_back() else {
        return false;
    };
    is_tld(tld) && labels.all(is_domain_label)
}

fn is_tld(label: &str) -> bool {
    (2..=63).contains(&label.len())
        && label
            .chars()
            .all(|character| character.is_ascii_alphabetic())
}

fn is_domain_label(label: &str) -> bool {
    if label.is_empty() || label.len() > 63 {
        return false;
    }
    let mut characters = label.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    let Some(last) = label.chars().next_back() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && label
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

#[derive(Clone, Debug)]
struct MixedUrlWord {
    range: Range<usize>,
    is_url: bool,
}

impl MixedUrlWord {
    fn width(&self, text: &str) -> usize {
        text[self.range.clone()].width()
    }
}

fn mixed_url_wrap_ranges(
    text: &str,
    initial_width: usize,
    subsequent_width: usize,
) -> Vec<Range<usize>> {
    let leading_space_width = text
        .chars()
        .take_while(|character| *character == ' ')
        .count();
    let mut words = Vec::new();
    let mut cursor = 0;
    for word in WordSeparator::AsciiSpace.find_words(text) {
        let word_start = cursor;
        let word_end = word_start + word.word.len();
        let trailing_space_end = word_end + word.whitespace.len();
        if !word.word.is_empty() {
            words.push(MixedUrlWord {
                range: word_start..word_end,
                is_url: is_url_like_token(word.word),
            });
        }
        cursor = trailing_space_end;
    }

    let mut lines = Vec::new();
    let mut line_start = None;
    let mut line_end = 0;
    let mut line_width = 0;
    let mut line_limit = initial_width.max(1);

    for word in words {
        let mut pending = split_mixed_url_word(text, word, line_limit);
        let mut pending_index = 0;
        while let Some(piece) = pending.get(pending_index).cloned() {
            let empty_line_prefix_width = if line_start.is_none() && lines.is_empty() {
                leading_space_width
            } else {
                0
            };
            let empty_line_piece_limit = line_limit.saturating_sub(empty_line_prefix_width).max(1);
            if line_start.is_none() && !piece.is_url && piece.width(text) > empty_line_piece_limit {
                pending.splice(
                    pending_index..=pending_index,
                    split_mixed_url_word(text, piece, empty_line_piece_limit),
                );
                continue;
            }

            let piece_width = piece.width(text);
            let inter_word_space =
                line_start.map_or(0, |_| text[line_end..piece.range.start].len());
            let fits = if line_start.is_none() {
                piece.is_url
                    || empty_line_prefix_width + piece_width <= line_limit
                    || empty_line_prefix_width >= line_limit
            } else {
                line_width + inter_word_space + piece_width <= line_limit
            };

            if fits {
                if line_start.is_none() {
                    let first_output_line = lines.is_empty();
                    line_start = Some(if first_output_line {
                        0
                    } else {
                        piece.range.start
                    });
                    line_width = if first_output_line {
                        leading_space_width + piece_width
                    } else {
                        piece_width
                    };
                } else {
                    line_width += inter_word_space + piece_width;
                }
                line_end = piece.range.end;
                pending_index += 1;
                continue;
            }

            if let Some(start) = line_start.take() {
                lines.push(start..line_end);
            }
            line_end = 0;
            line_width = 0;
            line_limit = subsequent_width.max(1);
        }
    }

    if let Some(start) = line_start {
        lines.push(start..line_end);
    }
    lines
}

fn split_mixed_url_word(text: &str, word: MixedUrlWord, line_limit: usize) -> Vec<MixedUrlWord> {
    if word.is_url || word.width(text) <= line_limit {
        return vec![word];
    }
    let source = Word::from(&text[word.range.clone()]);
    let mut offset = word.range.start;
    source
        .break_apart(line_limit.max(1))
        .map(|piece| {
            let end = offset + piece.word.len();
            let piece = MixedUrlWord {
                range: offset..end,
                is_url: false,
            };
            offset = end;
            piece
        })
        .collect()
}

fn spans_width(spans: &[MarkdownSpan]) -> usize {
    spans.iter().map(|span| span.text.width()).sum()
}

fn skip_spaces(text: &str, start: usize) -> usize {
    start
        + text[start..]
            .chars()
            .take_while(|character| *character == ' ')
            .map(char::len_utf8)
            .sum::<usize>()
}

fn prefixed_slice(
    prefix: &[MarkdownSpan],
    content: &[MarkdownSpan],
    span_bounds: &[Range<usize>],
    range: &Range<usize>,
) -> Vec<MarkdownSpan> {
    let mut output = prefix.to_vec();
    for (span, bounds) in content.iter().zip(span_bounds) {
        let start = range.start.max(bounds.start);
        let end = range.end.min(bounds.end);
        if start >= end {
            continue;
        }
        let local_start = start - bounds.start;
        let local_end = end - bounds.start;
        push_span_with_metadata(
            &mut output,
            &span.text[local_start..local_end],
            span.style,
            span.link.as_deref(),
            span.syntax,
        );
    }
    output
}

fn flatten_spans(spans: &[MarkdownSpan]) -> (String, Vec<Range<usize>>) {
    let mut flat = String::new();
    let mut bounds = Vec::with_capacity(spans.len());
    for span in spans {
        let start = flat.len();
        flat.push_str(&span.text);
        bounds.push(start..flat.len());
    }
    (flat, bounds)
}

fn wrap_ranges_trim(text: &str, options: &Options<'_>) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut cursor = 0;
    for (line_index, line) in textwrap::wrap(text, options).iter().enumerate() {
        let range = match line {
            std::borrow::Cow::Borrowed(slice) => {
                borrowed_slice_range(text, slice).unwrap_or_else(|| {
                    map_owned_line(text, cursor, slice, indent(options, line_index))
                })
            }
            std::borrow::Cow::Owned(slice) => {
                map_owned_line(text, cursor, slice, indent(options, line_index))
            }
        };
        cursor = range.end;
        ranges.push(range);
    }
    ranges
}

fn indent<'a>(options: &'a Options<'a>, line_index: usize) -> &'a str {
    if line_index == 0 {
        options.initial_indent
    } else {
        options.subsequent_indent
    }
}

fn borrowed_slice_range(text: &str, slice: &str) -> Option<Range<usize>> {
    let text_start = text.as_ptr() as usize;
    let text_end = text_start.checked_add(text.len())?;
    let slice_start = slice.as_ptr() as usize;
    let slice_end = slice_start.checked_add(slice.len())?;
    if slice_start < text_start || slice_end > text_end {
        return None;
    }
    Some((slice_start - text_start)..(slice_end - text_start))
}

fn map_owned_line(
    text: &str,
    cursor: usize,
    wrapped: &str,
    synthetic_prefix: &str,
) -> Range<usize> {
    let wrapped = wrapped.strip_prefix(synthetic_prefix).unwrap_or(wrapped);
    let mut start = cursor;
    while start < text.len() && !wrapped.starts_with(' ') {
        let Some(character) = text[start..].chars().next() else {
            break;
        };
        if character != ' ' {
            break;
        }
        start += character.len_utf8();
    }

    let mut end = start;
    let mut saw_source_character = false;
    let mut characters = wrapped.chars().peekable();
    while let Some(character) = characters.next() {
        if let Some(source) = text[end..].chars().next()
            && character == source
        {
            end += source.len_utf8();
            saw_source_character = true;
            continue;
        }
        if character == '-' && characters.peek().is_none() {
            continue;
        }
        if saw_source_character {
            break;
        }
    }
    start..end
}

#[cfg(test)]
mod tests {
    use super::{is_url_like_token, wrap_spans};
    use crate::tui::markdown::{MarkdownSpan, MarkdownSpanStyle};
    use unicode_width::UnicodeWidthStr as _;

    fn span(text: &str, style: MarkdownSpanStyle) -> MarkdownSpan {
        MarkdownSpan {
            text: text.to_owned(),
            style,
            link: None,
            syntax: None,
        }
    }

    fn linked_span(text: &str, destination: &str) -> MarkdownSpan {
        MarkdownSpan {
            text: text.to_owned(),
            style: MarkdownSpanStyle(MarkdownSpanStyle::LINK),
            link: Some(destination.to_owned()),
            syntax: None,
        }
    }

    fn line_text(lines: &[Vec<MarkdownSpan>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn wraps_at_words_and_preserves_prefixes_and_styles() {
        let strong = MarkdownSpanStyle(MarkdownSpanStyle::STRONG);
        let lines = wrap_spans(
            &[
                span("This item ", MarkdownSpanStyle::default()),
                span("wraps onto another visible line", strong),
            ],
            &[span("1. ", MarkdownSpanStyle::default())],
            &[span("   ", MarkdownSpanStyle::default())],
            24,
        );
        let text = line_text(&lines);

        assert_eq!(text, ["1. This item wraps onto", "   another visible line"]);
        assert!(lines[1].iter().any(|span| span.style.strong()));
    }

    #[test]
    fn keeps_a_long_url_like_token_intact() {
        let url = "example.test/a-very-long-path-with-many-segments-and-query?x=1&y=2";
        let lines = wrap_spans(&[span(url, MarkdownSpanStyle::default())], &[], &[], 20);

        assert_eq!(line_text(&lines), [url]);
    }

    #[test]
    fn mixed_url_line_keeps_regular_words_intact() {
        let lines = wrap_spans(
            &[span(
                "see https://example.com/path and keep strikethrough intact while wrapping prose",
                MarkdownSpanStyle::default(),
            )],
            &[],
            &[],
            36,
        );

        assert_eq!(
            line_text(&lines),
            [
                "see https://example.com/path and",
                "keep strikethrough intact while",
                "wrapping prose",
            ]
        );
    }

    #[test]
    fn keeps_a_url_split_across_styled_spans_intact() {
        let strong = MarkdownSpanStyle(MarkdownSpanStyle::STRONG);
        let link = MarkdownSpanStyle(MarkdownSpanStyle::LINK);
        let lines = wrap_spans(
            &[
                span("see ", MarkdownSpanStyle::default()),
                span("https://exa", strong),
                span("mple.com/path", link),
                span(" now", MarkdownSpanStyle::default()),
            ],
            &[],
            &[],
            10,
        );

        assert_eq!(
            line_text(&lines),
            ["see", "https://example.com/path", "now"]
        );
        assert!(lines[1].iter().any(|span| span.style.strong()));
        assert!(lines[1].iter().any(|span| span.style.link()));
    }

    #[test]
    fn wrapping_and_slicing_preserve_hyperlink_metadata() {
        let destination = "https://example.com/docs";
        let lines = wrap_spans(
            &[linked_span("linked words wrap naturally", destination)],
            &[],
            &[],
            8,
        );

        assert_eq!(
            line_text(&lines),
            ["linked", "words", "wrap", "naturall", "y"]
        );
        assert!(
            lines
                .iter()
                .flatten()
                .all(|span| { span.link.as_deref() == Some(destination) && span.style.link() })
        );
    }

    #[test]
    fn mixed_url_line_still_splits_an_overlong_non_url_token() {
        let long_non_url = "a_very_long_token_without_spaces_to_force_wrapping";
        let lines = wrap_spans(
            &[span(
                &format!("see https://ex.com {long_non_url}"),
                MarkdownSpanStyle::default(),
            )],
            &[],
            &[],
            24,
        );
        let text = line_text(&lines);

        assert!(text.iter().any(|line| line.contains("https://ex.com")));
        assert!(!text.iter().any(|line| line.contains(long_non_url)));
    }

    #[test]
    fn url_detection_accepts_supported_hosts_and_rejects_paths_and_invalid_ports() {
        for token in [
            "https://example.com/path",
            "myapp://open/some/path",
            "example.com/path",
            "www.example.com",
            "localhost:3000/api",
            "192.168.1.1:8080/health",
        ] {
            assert!(is_url_like_token(token), "expected URL token: {token}");
        }
        for token in [
            "src/main.rs",
            "foo/bar",
            "localhost:99999/path",
            "example.com:abc/path",
        ] {
            assert!(
                !is_url_like_token(token),
                "expected prose/path token: {token}"
            );
        }
    }

    #[test]
    fn mixed_url_wrapping_respects_distinct_initial_and_continuation_prefixes() {
        let lines = wrap_spans(
            &[span(
                "see https://example.com/path now",
                MarkdownSpanStyle::default(),
            )],
            &[span("1. ", MarkdownSpanStyle::default())],
            &[span("   ", MarkdownSpanStyle::default())],
            10,
        );

        assert_eq!(
            line_text(&lines),
            ["1. see", "   https://example.com/path", "   now"]
        );
    }

    #[test]
    fn keeps_default_wrapping_for_a_long_non_url_token() {
        let token = "a_very_long_token_without_spaces_to_force_wrapping";
        let lines = wrap_spans(&[span(token, MarkdownSpanStyle::default())], &[], &[], 20);

        assert!(lines.len() > 1);
        assert!(line_text(&lines).iter().all(|line| line.width() <= 20));
    }
}
