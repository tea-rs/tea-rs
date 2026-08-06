//! Bounded fenced-code syntax highlighting.
//!
//! Adapted from `OpenAI` Codex's Apache-2.0 licensed
//! `codex-rs/tui/src/render/highlight.rs` at commit
//! `1836ae0612052137d0cabaff7807ff8314cee940`.

use std::sync::OnceLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

use super::{MarkdownSyntaxColor, MarkdownSyntaxStyle};

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static SYNTAX_THEME: OnceLock<Theme> = OnceLock::new();

/// Skip highlighting for inputs larger than 512 KiB.
pub(super) const MAX_HIGHLIGHT_BYTES: usize = 512 * 1_024;
/// Skip highlighting for inputs with more than 10,000 lines.
pub(super) const MAX_HIGHLIGHT_LINES: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HighlightedSpan {
    pub(super) text: String,
    pub(super) style: MarkdownSyntaxStyle,
}

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn syntax_theme() -> &'static Theme {
    SYNTAX_THEME.get_or_init(|| {
        let mut themes = ThemeSet::load_defaults().themes;
        themes
            .remove("base16-ocean.dark")
            .unwrap_or_else(|| themes.into_values().next().unwrap_or_default())
    })
}

fn find_syntax(language: &str) -> Option<&'static SyntaxReference> {
    let syntaxes = syntax_set();
    let normalized = language.to_ascii_lowercase();
    let language = match normalized.as_str() {
        "csharp" | "c-sharp" => "c#",
        "cppm" | "cxxm" | "ixx" => "cpp",
        "golang" => "go",
        "python3" => "python",
        "shell" => "bash",
        _ => language,
    };
    syntaxes
        .find_syntax_by_token(language)
        .or_else(|| syntaxes.find_syntax_by_name(language))
        .or_else(|| {
            syntaxes
                .syntaxes()
                .iter()
                .find(|syntax| syntax.name.eq_ignore_ascii_case(language))
        })
        .or_else(|| syntaxes.find_syntax_by_extension(language))
}

fn syntax_style(style: Style) -> MarkdownSyntaxStyle {
    MarkdownSyntaxStyle {
        foreground: Some(MarkdownSyntaxColor::Rgb(
            style.foreground.r,
            style.foreground.g,
            style.foreground.b,
        )),
        bold: style.font_style.contains(FontStyle::BOLD),
    }
}

/// Returns highlighted source lines, or `None` for unknown and bounded-out input.
pub(super) fn highlight_code(code: &str, language: &str) -> Option<Vec<Vec<HighlightedSpan>>> {
    if code.is_empty()
        || code.len() > MAX_HIGHLIGHT_BYTES
        || code.lines().count() > MAX_HIGHLIGHT_LINES
    {
        return None;
    }
    let syntax = find_syntax(language)?;
    let mut highlighter = HighlightLines::new(syntax, syntax_theme());
    let mut lines = Vec::new();
    for line in LinesWithEndings::from(code) {
        let ranges = highlighter.highlight_line(line, syntax_set()).ok()?;
        let mut spans = Vec::new();
        for (style, text) in ranges {
            let text = text.trim_end_matches(['\n', '\r']);
            if !text.is_empty() {
                spans.push(HighlightedSpan {
                    text: text.to_owned(),
                    style: syntax_style(style),
                });
            }
        }
        lines.push(spans);
    }
    Some(lines)
}

#[cfg(test)]
mod tests {
    use super::{MAX_HIGHLIGHT_LINES, find_syntax, highlight_code};

    #[test]
    fn resolves_common_languages_extensions_and_aliases() {
        for language in [
            "rust",
            "rs",
            "python",
            "python3",
            "javascript",
            "js",
            "bash",
            "shell",
            "csharp",
        ] {
            assert!(
                find_syntax(language).is_some(),
                "missing syntax for {language}"
            );
        }
    }

    #[test]
    fn highlighting_preserves_multiline_and_crlf_content() {
        let code = "fn main() {\r\n    println!(\"hi\");\r\n}\r\n";
        let lines = highlight_code(code, "rust").unwrap();
        let reconstructed = lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(reconstructed, "fn main() {\n    println!(\"hi\");\n}");
        assert!(lines.iter().flatten().all(|span| !span.text.contains('\r')));
    }

    #[test]
    fn line_limit_counts_a_final_line_without_a_newline() {
        let mut code = "x\n".repeat(MAX_HIGHLIGHT_LINES);
        code.push('x');
        assert_eq!(highlight_code(&code, "rust"), None);
    }
}
