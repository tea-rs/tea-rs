//! Width-aware Markdown projection for the TUI transcript.
//!
//! The Writer and indentation model are adapted from `OpenAI` Codex's
//! Apache-2.0 licensed `codex-rs/tui/src/markdown_render.rs` at commit
//! `1836ae0612052137d0cabaff7807ff8314cee940`. Tea keeps renderer-neutral
//! semantic styles and its own terminal sanitization/theme boundary.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use unicode_width::UnicodeWidthStr as _;

mod highlight;
mod table;
mod wrapping;

const MAX_MARKDOWN_LINES: usize = 512;
#[cfg(test)]
const MAX_HYPERLINK_DESTINATION_BYTES: usize = super::hyperlink::MAX_OSC8_DESTINATION_BYTES;
const MAX_CODE_LANGUAGE_BYTES: usize = 64;

/// Terminal-visible structure of one projected Markdown line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MarkdownKind {
    Paragraph,
    Heading(u8),
    Quote(u8),
    Code,
    Rule,
    Blank,
}

/// Inline semantics carried to the terminal renderer without source markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct MarkdownSpanStyle(u8);

impl MarkdownSpanStyle {
    const STRONG: u8 = 1 << 0;
    const EMPHASIS: u8 = 1 << 1;
    const STRIKETHROUGH: u8 = 1 << 2;
    const CODE: u8 = 1 << 3;
    const LINK: u8 = 1 << 4;
    const LIST_MARKER: u8 = 1 << 5;
    const BLOCK_CODE: u8 = 1 << 6;

    pub(super) const fn strong(self) -> bool {
        self.0 & Self::STRONG != 0
    }

    pub(super) const fn emphasis(self) -> bool {
        self.0 & Self::EMPHASIS != 0
    }

    pub(super) const fn strikethrough(self) -> bool {
        self.0 & Self::STRIKETHROUGH != 0
    }

    pub(super) const fn code(self) -> bool {
        self.0 & Self::CODE != 0
    }

    pub(super) const fn link(self) -> bool {
        self.0 & Self::LINK != 0
    }

    pub(super) const fn list_marker(self) -> bool {
        self.0 & Self::LIST_MARKER != 0
    }

    pub(super) const fn block_code(self) -> bool {
        self.0 & Self::BLOCK_CODE != 0
    }

    const fn merge(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    const fn with_code(self) -> Self {
        self.merge(Self(Self::CODE))
    }

    const fn with_link(self) -> Self {
        self.merge(Self(Self::LINK))
    }

    const fn with_block_code(self) -> Self {
        self.merge(Self(Self::BLOCK_CODE))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MarkdownSyntaxColor {
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MarkdownSyntaxStyle {
    pub(super) foreground: Option<MarkdownSyntaxColor>,
    pub(super) bold: bool,
}

/// One owned content span with renderer-neutral Markdown semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MarkdownSpan {
    text: String,
    style: MarkdownSpanStyle,
    link: Option<String>,
    syntax: Option<MarkdownSyntaxStyle>,
}

impl MarkdownSpan {
    pub(super) fn text(&self) -> &str {
        &self.text
    }

    pub(super) const fn style(&self) -> MarkdownSpanStyle {
        self.style
    }

    pub(super) fn link(&self) -> Option<&str> {
        self.link.as_deref()
    }

    pub(super) const fn syntax_style(&self) -> Option<MarkdownSyntaxStyle> {
        self.syntax
    }
}

/// One renderer-neutral Markdown line after indentation and optional wrapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MarkdownLine {
    kind: MarkdownKind,
    spans: Vec<MarkdownSpan>,
}

impl MarkdownLine {
    pub(super) const fn kind(&self) -> MarkdownKind {
        self.kind
    }

    #[cfg(test)]
    fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }

    pub(super) fn spans(&self) -> &[MarkdownSpan] {
        &self.spans
    }
}

#[derive(Clone, Debug)]
struct IndentContext {
    prefix: Vec<MarkdownSpan>,
    marker: Option<Vec<MarkdownSpan>>,
    is_list: bool,
}

impl IndentContext {
    fn new(prefix: Vec<MarkdownSpan>, marker: Option<Vec<MarkdownSpan>>, is_list: bool) -> Self {
        Self {
            prefix,
            marker,
            is_list,
        }
    }
}

#[derive(Debug)]
struct CurrentLine {
    kind: MarkdownKind,
    spans: Vec<MarkdownSpan>,
    preserve_width: bool,
}

#[derive(Debug)]
struct LinkDestination {
    display: String,
    metadata: Option<String>,
}

#[derive(Debug)]
struct Writer<'input> {
    input: &'input str,
    output: Vec<MarkdownLine>,
    inline_styles: Vec<MarkdownSpanStyle>,
    indent_stack: Vec<IndentContext>,
    list_indices: Vec<Option<u64>>,
    list_needs_blank_before_next_item: Vec<bool>,
    list_item_start_line_counts: Vec<usize>,
    link_destination: Option<LinkDestination>,
    needs_newline: bool,
    pending_marker_line: bool,
    in_code_block: bool,
    code_block_language: Option<String>,
    code_block_buffer: String,
    heading_level: Option<u8>,
    quote_depth: usize,
    wrap_width: Option<usize>,
    current_line: Option<CurrentLine>,
    current_initial_indent: Vec<MarkdownSpan>,
    current_subsequent_indent: Vec<MarkdownSpan>,
    table_state: Option<table::TableState>,
}

impl<'input> Writer<'input> {
    fn new(input: &'input str, wrap_width: Option<usize>) -> Self {
        Self {
            input,
            output: Vec::new(),
            inline_styles: Vec::new(),
            indent_stack: Vec::new(),
            list_indices: Vec::new(),
            list_needs_blank_before_next_item: Vec::new(),
            list_item_start_line_counts: Vec::new(),
            link_destination: None,
            needs_newline: false,
            pending_marker_line: false,
            in_code_block: false,
            code_block_language: None,
            code_block_buffer: String::new(),
            heading_level: None,
            quote_depth: 0,
            wrap_width,
            current_line: None,
            current_initial_indent: Vec::new(),
            current_subsequent_indent: Vec::new(),
            table_state: None,
        }
    }

    fn run<'event>(
        mut self,
        events: impl Iterator<Item = (Event<'event>, Range<usize>)>,
    ) -> Vec<MarkdownLine> {
        for (event, range) in events {
            self.handle_event(event, range);
        }
        self.flush_current_line();
        while self
            .output
            .last()
            .is_some_and(|line| line.kind == MarkdownKind::Blank)
        {
            self.output.pop();
        }
        self.output.truncate(MAX_MARKDOWN_LINES);
        self.output
    }

    #[allow(clippy::too_many_lines)]
    fn handle_event(&mut self, event: Event<'_>, range: Range<usize>) {
        match event {
            Event::Start(tag) => self.start_tag(tag, range),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(code) => self.push_span_text(&code, self.inline_style().with_code()),
            Event::SoftBreak if self.in_table_cell() => {
                self.push_span_text(" ", self.inline_style());
            }
            Event::HardBreak if self.in_table_cell() => self.push_table_cell_hard_break(),
            Event::SoftBreak | Event::HardBreak => self.push_line(Vec::new()),
            Event::Rule => {
                self.flush_current_line();
                if !self.output.is_empty() {
                    self.push_blank_line();
                }
                self.push_line_with_kind(
                    vec![span("---", MarkdownSpanStyle::default())],
                    MarkdownKind::Rule,
                );
                self.needs_newline = true;
            }
            Event::TaskListMarker(checked) => self.push_span_text(
                if checked { "[x] " } else { "[ ] " },
                self.inline_style().merge(list_marker_style()),
            ),
            // Raw HTML and unsupported extensions never enter the terminal output channel.
            _ => {}
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>, range: Range<usize>) {
        match tag {
            Tag::Paragraph => self.start_paragraph(),
            Tag::Heading { level, .. } => self.start_heading(level),
            Tag::BlockQuote(_) => self.start_blockquote(),
            Tag::CodeBlock(kind) => self.start_codeblock(&kind),
            Tag::List(start) => self.start_list(start),
            Tag::Item => self.start_item(),
            Tag::Emphasis => self.push_inline_style(MarkdownSpanStyle(MarkdownSpanStyle::EMPHASIS)),
            Tag::Strong => {
                self.push_inline_style(MarkdownSpanStyle(MarkdownSpanStyle::STRONG));
            }
            Tag::Strikethrough => {
                self.push_inline_style(MarkdownSpanStyle(MarkdownSpanStyle::STRIKETHROUGH));
            }
            Tag::Link { dest_url, .. } => {
                let raw = dest_url.into_string();
                let metadata = validate_hyperlink_destination(&raw);
                self.link_destination = Some(LinkDestination {
                    display: metadata.clone().unwrap_or(raw),
                    metadata,
                });
                self.push_inline_style(MarkdownSpanStyle(MarkdownSpanStyle::LINK));
            }
            Tag::Table(alignments) => self.start_table(alignments),
            Tag::TableHead => self.start_table_head(),
            Tag::TableRow => self.start_table_row(range),
            Tag::TableCell => self.start_table_cell(),
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.end_paragraph(),
            TagEnd::Heading(_) => self.end_heading(),
            TagEnd::BlockQuote(_) => self.end_blockquote(),
            TagEnd::CodeBlock => self.end_codeblock(),
            TagEnd::List(_) => self.end_list(),
            TagEnd::Item => self.end_item(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.pop_inline_style();
            }
            TagEnd::Link => self.end_link(),
            TagEnd::Table => self.end_table(),
            TagEnd::TableHead => self.end_table_head(),
            TagEnd::TableRow => self.end_table_row(),
            TagEnd::TableCell => self.end_table_cell(),
            _ => {}
        }
    }

    fn start_paragraph(&mut self) {
        if self.in_table_cell() {
            return;
        }
        if self.needs_newline {
            self.push_blank_line();
        }
        self.push_line(Vec::new());
        self.needs_newline = false;
    }

    fn end_paragraph(&mut self) {
        if self.in_table_cell() {
            return;
        }
        self.needs_newline = true;
        self.pending_marker_line = false;
    }

    fn start_heading(&mut self, level: HeadingLevel) {
        if self.needs_newline {
            self.push_blank_line();
            self.needs_newline = false;
        }
        let level = heading_number(level);
        self.heading_level = Some(level);
        let heading_style = self
            .inline_style()
            .merge(MarkdownSpanStyle(MarkdownSpanStyle::STRONG));
        self.push_line(vec![span(
            &format!("{} ", "#".repeat(usize::from(level))),
            heading_style,
        )]);
        self.push_inline_style(MarkdownSpanStyle(MarkdownSpanStyle::STRONG));
        self.needs_newline = false;
    }

    fn end_heading(&mut self) {
        self.needs_newline = true;
        self.pop_inline_style();
        self.heading_level = None;
    }

    fn start_blockquote(&mut self) {
        if self.needs_newline {
            self.push_blank_line();
            self.needs_newline = false;
        }
        self.quote_depth = self.quote_depth.saturating_add(1);
        self.indent_stack.push(IndentContext::new(
            vec![span("> ", MarkdownSpanStyle::default())],
            None,
            false,
        ));
    }

    fn end_blockquote(&mut self) {
        self.indent_stack.pop();
        self.quote_depth = self.quote_depth.saturating_sub(1);
        self.needs_newline = true;
    }

    fn start_list(&mut self, start: Option<u64>) {
        if self.list_indices.is_empty() && self.needs_newline {
            self.push_line(Vec::new());
        }
        self.list_indices.push(start);
        self.list_needs_blank_before_next_item.push(false);
    }

    fn end_list(&mut self) {
        self.list_indices.pop();
        self.list_needs_blank_before_next_item.pop();
        self.needs_newline = true;
    }

    fn start_item(&mut self) {
        if self
            .list_needs_blank_before_next_item
            .last_mut()
            .is_some_and(std::mem::take)
        {
            self.push_blank_line();
        }
        self.flush_current_line();
        self.list_item_start_line_counts.push(self.output.len());
        self.pending_marker_line = true;

        let depth = self.list_indices.len();
        let is_ordered = self.list_indices.last().is_some_and(Option::is_some);
        let marker_width = depth.saturating_mul(4).saturating_sub(3).max(1);
        let marker = self.list_indices.last_mut().map(|index| match index {
            None => vec![span(
                &(" ".repeat(marker_width.saturating_sub(1)) + "- "),
                list_marker_style(),
            )],
            Some(index) => {
                let current = *index;
                *index = index.saturating_add(1);
                vec![span(
                    &format!("{current:marker_width$}. "),
                    list_marker_style(),
                )]
            }
        });
        let indent_len = if is_ordered {
            marker_width.saturating_add(2)
        } else {
            marker_width.saturating_add(1)
        };
        self.indent_stack.push(IndentContext::new(
            vec![span(&" ".repeat(indent_len), MarkdownSpanStyle::default())],
            marker,
            true,
        ));
        self.needs_newline = false;
    }

    fn end_item(&mut self) {
        self.flush_current_line();
        let start = self.list_item_start_line_counts.pop().unwrap_or_default();
        if self.output.len().saturating_sub(start) > 1
            && let Some(needs_blank) = self.list_needs_blank_before_next_item.last_mut()
        {
            *needs_blank = true;
        }
        self.indent_stack.pop();
        self.pending_marker_line = false;
    }

    fn start_codeblock(&mut self, kind: &CodeBlockKind<'_>) {
        self.flush_current_line();
        if !self.output.is_empty() {
            self.push_blank_line();
        }
        self.in_code_block = true;
        let (indent, language) = match kind {
            CodeBlockKind::Fenced(info) => (
                String::new(),
                info.split([',', ' ', '\t'])
                    .next()
                    .filter(|value| !value.is_empty() && value.len() <= MAX_CODE_LANGUAGE_BYTES)
                    .map(str::to_owned),
            ),
            CodeBlockKind::Indented => (" ".repeat(4), None),
        };
        self.code_block_language = language;
        self.code_block_buffer.clear();
        self.indent_stack.push(IndentContext::new(
            vec![span(&indent, MarkdownSpanStyle::default())],
            None,
            false,
        ));
        self.needs_newline = true;
    }

    fn end_codeblock(&mut self) {
        if let Some(language) = self.code_block_language.take() {
            let code = std::mem::take(&mut self.code_block_buffer);
            if let Some(lines) = highlight::highlight_code(&code, &language) {
                let style = self.inline_style().with_block_code();
                for line in lines {
                    self.push_line(Vec::new());
                    for span in line {
                        self.push_span_text_with_metadata(
                            &span.text,
                            style,
                            None,
                            Some(span.style),
                        );
                    }
                }
            } else {
                self.text(&code);
            }
        }
        self.needs_newline = true;
        self.in_code_block = false;
        self.indent_stack.pop();
    }

    fn end_link(&mut self) {
        self.pop_inline_style();
        if let Some(LinkDestination { display, metadata }) = self.link_destination.take()
            && !display.is_empty()
        {
            let style = self.inline_style().with_link();
            self.push_span_text_with_link(" (", style, metadata.as_deref());
            self.push_span_text_with_link(&display, style, metadata.as_deref());
            self.push_span_text_with_link(")", style, metadata.as_deref());
        }
    }

    fn text(&mut self, text: &str) {
        if self.in_table_cell() {
            for (index, line) in text.lines().enumerate() {
                if index > 0 {
                    self.push_table_cell_hard_break();
                }
                self.push_span_text(line, self.inline_style());
            }
            return;
        }
        if self.pending_marker_line {
            self.push_line(Vec::new());
        }
        self.pending_marker_line = false;

        if self.in_code_block && self.code_block_language.is_some() {
            self.code_block_buffer.push_str(text);
            return;
        }

        if self.in_code_block && !self.needs_newline && self.current_line_has_content() {
            self.push_line(Vec::new());
        }
        for (index, line) in text.lines().enumerate() {
            if self.needs_newline {
                self.push_line(Vec::new());
                self.needs_newline = false;
            }
            if index > 0 {
                self.push_line(Vec::new());
            }
            let style = if self.in_code_block {
                self.inline_style().with_block_code()
            } else {
                self.inline_style()
            };
            self.push_span_text(line, style);
        }
        self.needs_newline = false;
    }

    fn current_line_has_content(&self) -> bool {
        self.current_line
            .as_ref()
            .is_some_and(|line| !line.spans.is_empty())
            || self
                .output
                .last()
                .is_some_and(|line| !line.spans.is_empty())
    }

    fn push_inline_style(&mut self, style: MarkdownSpanStyle) {
        self.inline_styles.push(self.inline_style().merge(style));
    }

    fn pop_inline_style(&mut self) {
        self.inline_styles.pop();
    }

    fn inline_style(&self) -> MarkdownSpanStyle {
        self.inline_styles.last().copied().unwrap_or_default()
    }

    fn current_kind(&self) -> MarkdownKind {
        if self.quote_depth > 0 {
            MarkdownKind::Quote(quote_level(self.quote_depth))
        } else if self.in_code_block {
            MarkdownKind::Code
        } else if let Some(level) = self.heading_level {
            MarkdownKind::Heading(level)
        } else {
            MarkdownKind::Paragraph
        }
    }

    fn push_line(&mut self, spans: Vec<MarkdownSpan>) {
        let kind = self.current_kind();
        self.push_line_with_kind(spans, kind);
    }

    fn push_line_with_kind(&mut self, spans: Vec<MarkdownSpan>, kind: MarkdownKind) {
        self.flush_current_line();
        let was_pending = self.pending_marker_line;
        self.current_initial_indent = self.prefix_spans(was_pending);
        self.current_subsequent_indent = self.prefix_spans(false);
        self.current_line = Some(CurrentLine {
            kind,
            spans,
            preserve_width: self.in_code_block,
        });
        self.pending_marker_line = false;
    }

    fn push_span_text(&mut self, text: &str, style: MarkdownSpanStyle) {
        let link = if style.link() {
            self.link_destination
                .as_ref()
                .and_then(|destination| destination.metadata.clone())
        } else {
            None
        };
        self.push_span_text_with_link(text, style, link.as_deref());
    }

    fn push_span_text_with_link(
        &mut self,
        text: &str,
        style: MarkdownSpanStyle,
        link: Option<&str>,
    ) {
        self.push_span_text_with_metadata(text, style, link, None);
    }

    fn push_span_text_with_metadata(
        &mut self,
        text: &str,
        style: MarkdownSpanStyle,
        link: Option<&str>,
        syntax: Option<MarkdownSyntaxStyle>,
    ) {
        if let Some(table_state) = self.table_state.as_mut()
            && table_state.in_cell()
        {
            table_state.push_span(text, style, link, syntax);
            return;
        }
        if self.current_line.is_none() {
            self.push_line(Vec::new());
        }
        if let Some(line) = self.current_line.as_mut() {
            push_span_with_metadata(&mut line.spans, text, style, link, syntax);
        }
    }

    fn flush_current_line(&mut self) {
        let Some(line) = self.current_line.take() else {
            return;
        };
        let wrapped = if line.preserve_width {
            let mut spans = self.current_initial_indent.clone();
            append_spans(&mut spans, line.spans);
            vec![spans]
        } else if let Some(width) = self.wrap_width {
            wrapping::wrap_spans(
                &line.spans,
                &self.current_initial_indent,
                &self.current_subsequent_indent,
                width,
            )
        } else {
            let mut spans = self.current_initial_indent.clone();
            append_spans(&mut spans, line.spans);
            vec![spans]
        };
        for spans in wrapped {
            self.push_output_line(MarkdownLine {
                kind: line.kind,
                spans,
            });
        }
        self.current_initial_indent.clear();
        self.current_subsequent_indent.clear();
    }

    fn push_blank_line(&mut self) {
        self.flush_current_line();
        if self.indent_stack.iter().all(|context| context.is_list) {
            self.push_output_line(MarkdownLine {
                kind: MarkdownKind::Blank,
                spans: Vec::new(),
            });
        } else {
            self.push_line(Vec::new());
            self.flush_current_line();
        }
    }

    fn push_output_line(&mut self, line: MarkdownLine) {
        if self.output.len() < MAX_MARKDOWN_LINES {
            self.output.push(line);
        }
    }

    fn prefix_spans(&self, pending_marker_line: bool) -> Vec<MarkdownSpan> {
        let mut prefix = Vec::new();
        let last_marker_index = if pending_marker_line {
            self.indent_stack
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, context)| context.marker.as_ref().map(|_| index))
        } else {
            None
        };
        let last_list_index = self
            .indent_stack
            .iter()
            .rposition(|context| context.is_list);

        for (index, context) in self.indent_stack.iter().enumerate() {
            if pending_marker_line {
                if Some(index) == last_marker_index
                    && let Some(marker) = &context.marker
                {
                    append_spans(&mut prefix, marker.clone());
                    continue;
                }
                if context.is_list && last_marker_index.is_some_and(|marker| marker > index) {
                    continue;
                }
            } else if context.is_list && Some(index) != last_list_index {
                continue;
            }
            append_spans(&mut prefix, context.prefix.clone());
        }
        prefix
    }

    fn start_table(&mut self, alignments: Vec<pulldown_cmark::Alignment>) {
        self.flush_current_line();
        if self.needs_newline {
            self.push_blank_line();
            self.needs_newline = false;
        }
        self.table_state = Some(table::TableState::new(alignments));
    }

    fn end_table(&mut self) {
        let Some(table_state) = self.table_state.take() else {
            return;
        };
        let pending_marker_line = self.pending_marker_line;
        let prefix_width = self
            .prefix_spans(pending_marker_line)
            .iter()
            .map(|span| span.text.width())
            .sum();
        let rendered = table::render(
            table_state,
            self.wrap_width,
            prefix_width,
            self.current_kind(),
        );
        for (index, line) in rendered.table_lines.into_iter().enumerate() {
            if rendered.table_lines_prewrapped {
                self.push_prewrapped_line(line, pending_marker_line && index == 0);
            } else {
                self.push_line_with_kind(line.spans, line.kind);
                self.flush_current_line();
            }
        }
        self.pending_marker_line = false;
        for line in rendered.spillover_lines {
            self.push_line_with_kind(line.spans, line.kind);
            self.flush_current_line();
        }
        self.needs_newline = true;
    }

    fn start_table_head(&mut self) {
        if let Some(table_state) = self.table_state.as_mut() {
            table_state.start_head();
        }
    }

    fn end_table_head(&mut self) {
        if let Some(table_state) = self.table_state.as_mut() {
            table_state.end_head();
        }
    }

    fn start_table_row(&mut self, source_range: Range<usize>) {
        let source = self.input.get(source_range).unwrap_or_default().trim();
        let has_table_pipe_syntax = source.starts_with('|') || source.ends_with('|');
        if let Some(table_state) = self.table_state.as_mut() {
            table_state.start_row(has_table_pipe_syntax);
        }
    }

    fn end_table_row(&mut self) {
        if let Some(table_state) = self.table_state.as_mut() {
            table_state.end_row();
        }
    }

    fn start_table_cell(&mut self) {
        if let Some(table_state) = self.table_state.as_mut() {
            table_state.start_cell();
        }
    }

    fn end_table_cell(&mut self) {
        if let Some(table_state) = self.table_state.as_mut() {
            table_state.end_cell();
        }
    }

    fn in_table_cell(&self) -> bool {
        self.table_state
            .as_ref()
            .is_some_and(table::TableState::in_cell)
    }

    fn push_table_cell_hard_break(&mut self) {
        if let Some(table_state) = self.table_state.as_mut() {
            table_state.hard_break();
        }
    }

    fn push_prewrapped_line(&mut self, mut line: MarkdownLine, pending_marker_line: bool) {
        let mut spans = self.prefix_spans(pending_marker_line);
        append_spans(&mut spans, line.spans);
        line.spans = spans;
        self.push_output_line(line);
    }
}

/// Projects Markdown without width-driven wrapping.
#[cfg(test)]
pub(super) fn project(text: &str) -> Vec<MarkdownLine> {
    project_with_width(text, None)
}

/// Projects Markdown into lines already laid out for the terminal width.
pub(super) fn project_with_width(text: &str, width: Option<usize>) -> Vec<MarkdownLine> {
    let parser = Parser::new_ext(
        text,
        Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES,
    );
    Writer::new(text, width.map(|value| value.max(1))).run(parser.into_offset_iter())
}

pub(super) fn push_span_with_metadata(
    current: &mut Vec<MarkdownSpan>,
    text: &str,
    style: MarkdownSpanStyle,
    link: Option<&str>,
    syntax: Option<MarkdownSyntaxStyle>,
) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = current.last_mut()
        && last.style == style
        && last.link.as_deref() == link
        && last.syntax == syntax
    {
        last.text.push_str(text);
        return;
    }
    current.push(MarkdownSpan {
        text: text.to_owned(),
        style,
        link: link.map(str::to_owned),
        syntax,
    });
}

fn append_spans(output: &mut Vec<MarkdownSpan>, spans: Vec<MarkdownSpan>) {
    for span in spans {
        push_span_with_metadata(
            output,
            &span.text,
            span.style,
            span.link.as_deref(),
            span.syntax,
        );
    }
}

fn span(text: &str, style: MarkdownSpanStyle) -> MarkdownSpan {
    MarkdownSpan {
        text: text.to_owned(),
        style,
        link: None,
        syntax: None,
    }
}

fn validate_hyperlink_destination(destination: &str) -> Option<String> {
    super::hyperlink::validate_destination(destination)
}

const fn list_marker_style() -> MarkdownSpanStyle {
    MarkdownSpanStyle(MarkdownSpanStyle::LIST_MARKER)
}

const fn heading_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn quote_level(depth: usize) -> u8 {
    u8::try_from(depth).unwrap_or(u8::MAX).max(1)
}

#[cfg(test)]
mod tests {
    use super::highlight::{MAX_HIGHLIGHT_BYTES, MAX_HIGHLIGHT_LINES, highlight_code};
    use super::{
        MAX_CODE_LANGUAGE_BYTES, MAX_HYPERLINK_DESTINATION_BYTES, MarkdownKind, MarkdownLine,
        project, project_with_width, validate_hyperlink_destination,
    };
    use unicode_width::UnicodeWidthStr as _;

    fn plain_lines(markdown: &str) -> Vec<String> {
        project(markdown).iter().map(MarkdownLine::text).collect()
    }

    #[test]
    fn projects_supported_markdown_and_ignores_html() {
        let lines = project(
            "# Plan\n\n- first\n- second\n\n> quoted\n\n```rust\nlet x = 1;\n```\n\n<script>alert(1)</script>",
        );
        let rendered = lines
            .iter()
            .map(MarkdownLine::text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            lines
                .iter()
                .any(|line| line.kind() == MarkdownKind::Heading(1))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.kind() == MarkdownKind::Quote(1))
        );
        assert!(lines.iter().any(|line| line.kind() == MarkdownKind::Code));
        assert!(rendered.contains("# Plan"));
        assert!(rendered.contains("- first"));
        assert!(rendered.contains("> quoted"));
        assert!(rendered.contains("let x = 1;"));
        assert!(!rendered.contains("<script>"));
    }

    #[test]
    fn incomplete_markdown_has_a_safe_text_fallback() {
        let lines = project("before\n```rust\nfn unfinished(");
        let rendered = lines
            .iter()
            .map(MarkdownLine::text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("before"));
        assert!(rendered.contains("fn unfinished("));
        assert!(lines.iter().any(|line| line.kind() == MarkdownKind::Code));
    }

    #[test]
    fn ordered_lists_increment_independently_at_each_depth() {
        assert_eq!(
            plain_lines("3. first\n4. second\n   1. nested\n   2. next\n5. third"),
            [
                "3. first",
                "4. second",
                "    1. nested",
                "    2. next",
                "",
                "5. third"
            ]
        );
    }

    #[test]
    fn unordered_list_markers_are_visible_and_nested_at_four_columns() {
        assert_eq!(
            plain_lines("- List item 1\n  - Nested list item 1\n"),
            ["- List item 1", "    - Nested list item 1"]
        );
    }

    #[test]
    fn adjacent_unordered_items_remain_compact() {
        assert_eq!(
            plain_lines("- List item 1\n- List item 2\n"),
            ["- List item 1", "- List item 2"]
        );
    }

    #[test]
    fn mixed_nested_lists_keep_codex_alignment_and_sibling_spacing() {
        assert_eq!(
            plain_lines("1. Outer\n    - Inner A\n    - Inner B\n2. Next\n"),
            ["1. Outer", "    - Inner A", "    - Inner B", "", "2. Next"]
        );
    }

    #[test]
    fn loose_list_paragraphs_keep_blank_lines_and_item_indentation() {
        assert_eq!(
            plain_lines("1. First paragraph\n\n   Second paragraph of same item\n\n2. Next item\n"),
            [
                "1. First paragraph",
                "",
                "   Second paragraph of same item",
                "",
                "2. Next item",
            ]
        );
    }

    #[test]
    fn tight_soft_break_uses_the_item_continuation_prefix() {
        assert_eq!(
            plain_lines("- item line1\n  item line2\n"),
            ["- item line1", "  item line2"]
        );
    }

    #[test]
    fn deeply_nested_mixed_lists_use_four_column_levels() {
        assert_eq!(
            plain_lines("1. A\n    - B\n        1. C\n2. D\n"),
            ["1. A", "    - B", "        1. C", "", "2. D"]
        );
    }

    #[test]
    fn continuation_paragraphs_align_under_the_item_body() {
        assert_eq!(
            plain_lines("- Intro\n\n  Continuation paragraph\n"),
            ["- Intro", "", "  Continuation paragraph"]
        );
        assert_eq!(
            plain_lines("1. Intro\n\n   More details\n"),
            ["1. Intro", "", "   More details"]
        );
    }

    #[test]
    fn fenced_code_inside_list_item_keeps_item_indentation() {
        assert_eq!(
            plain_lines("- Item\n\n  ```\n  first\n  second\n  ```\n"),
            ["- Item", "", "  first", "  second"]
        );
    }

    #[test]
    fn fenced_code_preserves_copyable_width() {
        let lines = project_with_width(
            "```text\nthis code line is deliberately wider than twelve columns\n```\n",
            Some(12),
        );
        assert_eq!(
            lines.iter().map(MarkdownLine::text).collect::<Vec<_>>(),
            ["this code line is deliberately wider than twelve columns"]
        );
    }

    #[test]
    fn known_fenced_language_adds_syntax_metadata_without_changing_text() {
        let lines = project("```rust,no_run title=demo\nfn main() { println!(\"hi\"); }\n```\n");

        assert_eq!(
            lines.iter().map(MarkdownLine::text).collect::<Vec<_>>(),
            ["fn main() { println!(\"hi\"); }"]
        );
        assert!(
            lines
                .iter()
                .flat_map(MarkdownLine::spans)
                .any(|span| span.syntax_style().is_some())
        );

        let nested = project("- Item\n\n  ```rust\n  fn main() {}\n  ```\n");
        let code = nested
            .iter()
            .find(|line| line.kind() == MarkdownKind::Code)
            .unwrap();
        assert_eq!(code.text(), "  fn main() {}");
        assert!(
            code.spans()
                .iter()
                .any(|span| span.syntax_style().is_some())
        );
    }

    #[test]
    fn unknown_or_absent_fenced_language_stays_plain() {
        for (markdown, expected) in [
            ("```xyzlang\nplain code\n```\n", "plain code"),
            ("```\nplain code\n```\n", "plain code"),
            ("    plain code\n", "    plain code"),
        ] {
            let lines = project(markdown);
            assert_eq!(
                lines
                    .iter()
                    .map(MarkdownLine::text)
                    .filter(|line| !line.is_empty())
                    .collect::<Vec<_>>(),
                [expected]
            );
            assert!(
                lines
                    .iter()
                    .flat_map(MarkdownLine::spans)
                    .all(|span| span.syntax_style().is_none())
            );
        }

        let oversized_language = "x".repeat(MAX_CODE_LANGUAGE_BYTES + 1);
        let markdown = format!("```{oversized_language}\nplain code\n```\n");
        let lines = project(&markdown);
        assert_eq!(
            lines
                .iter()
                .map(MarkdownLine::text)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>(),
            ["plain code"]
        );
        assert!(
            lines
                .iter()
                .flat_map(MarkdownLine::spans)
                .all(|span| span.syntax_style().is_none())
        );
    }

    #[test]
    fn syntax_highlighting_rejects_oversized_or_excessive_line_inputs() {
        let oversized = "x".repeat(MAX_HIGHLIGHT_BYTES + 1);
        assert_eq!(highlight_code(&oversized, "rust"), None);

        let many_lines = "let x = 1;\n".repeat(MAX_HIGHLIGHT_LINES + 1);
        assert_eq!(highlight_code(&many_lines, "rust"), None);
    }

    #[test]
    fn blockquote_inside_nested_list_keeps_all_prefixes() {
        assert_eq!(
            plain_lines("1. A\n    - B\n      > inner\n2. C\n"),
            ["1. A", "    - B", "      > inner", "", "2. C"]
        );
    }

    #[test]
    fn wrapped_list_item_uses_continuation_indent_and_separates_sibling() {
        let lines = project_with_width(
            "1. This item wraps onto another visible rendered line\n2. Next item\n",
            Some(24),
        );
        assert_eq!(
            lines.iter().map(MarkdownLine::text).collect::<Vec<_>>(),
            [
                "1. This item wraps onto",
                "   another visible",
                "   rendered line",
                "",
                "2. Next item",
            ]
        );
    }

    #[test]
    fn table_renders_aligned_rows_and_respects_column_alignment() {
        let lines =
            project("| Left | Center | Right |\n|:-----|:------:|------:|\n| a | b | c |\n");
        let text = lines.iter().map(MarkdownLine::text).collect::<Vec<_>>();

        assert_eq!(text[0], " Left    Center    Right");
        assert_eq!(text[1], "━━━━━━  ━━━━━━━━  ━━━━━━━");
        assert_eq!(text[2], " a         b           c");
        assert!(
            lines[0]
                .spans()
                .iter()
                .filter(|span| !span.text().trim().is_empty())
                .all(|span| span.style().strong())
        );
    }

    #[test]
    fn narrow_table_transposes_body_rows_into_stacked_records() {
        let lines = project_with_width(
            "| Key | Value | Extra | More |\n|---|---|---|---|\n| item | alpha | beta | gamma |\n",
            Some(16),
        );
        let text = lines.iter().map(MarkdownLine::text).collect::<Vec<_>>();

        assert_eq!(
            text,
            [
                " Key", "  item", " Value", "  alpha", " Extra", "  beta", " More", "  gamma"
            ]
        );
        assert!(text.iter().all(|line| line.width() <= 16));

        let one_column = project_with_width(
            "| Key | Value | Extra | More |\n|---|---|---|---|\n| item | alpha | beta | gamma |\n",
            Some(1),
        );
        assert!(one_column.iter().all(|line| line.text().width() <= 1));
    }

    #[test]
    fn table_cells_preserve_inline_styles_and_safe_link_metadata() {
        let lines = project(
            "| Key | Content | More |\n|---|---|---|\n| **item** | [docs](https://example.test/tea) | `code` |\n",
        );
        let spans = lines
            .iter()
            .flat_map(MarkdownLine::spans)
            .collect::<Vec<_>>();

        assert!(
            spans
                .iter()
                .any(|span| span.text() == "item" && span.style().strong())
        );
        assert!(
            spans
                .iter()
                .any(|span| span.text() == "code" && span.style().code())
        );
        assert!(spans.iter().any(|span| {
            span.text().contains("docs")
                && span.style().link()
                && span.link() == Some("https://example.test/tea")
        }));
    }

    #[test]
    fn malformed_table_row_without_boundary_pipes_spills_back_to_prose() {
        let lines = project("| A | B |\n|---|---|\n| 1 | 2 |\nThis paragraph must spill out.\n");
        let text = lines.iter().map(MarkdownLine::text).collect::<Vec<_>>();

        assert_eq!(text[0], " A      B");
        assert_eq!(text[2], " 1      2");
        assert_eq!(text[3], "This paragraph must spill out.");
    }

    #[test]
    fn table_projection_remains_bounded_by_the_markdown_line_limit() {
        let mut markdown = String::from("| Key | Value |\n|---|---|\n");
        for index in 0..(super::MAX_MARKDOWN_LINES + 64) {
            use std::fmt::Write as _;
            writeln!(markdown, "| {index} | row {index} |").unwrap();
        }

        assert!(project_with_width(&markdown, Some(80)).len() <= super::MAX_MARKDOWN_LINES);
    }

    #[test]
    fn links_keep_visible_text_and_a_plain_text_target() {
        let lines = project("Read [Tea docs](https://example.test/tea) first.");
        let rendered = lines
            .iter()
            .map(MarkdownLine::text)
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(rendered, "Read Tea docs (https://example.test/tea) first.");
    }

    #[test]
    fn valid_links_carry_one_normalized_bounded_destination() {
        let lines =
            project("Read [Tea **docs**](HTTPS://Example.COM:443/a/../tea?q=rust#install) first.");
        let linked = lines[0]
            .spans()
            .iter()
            .filter(|span| span.style().link())
            .collect::<Vec<_>>();

        assert_eq!(
            lines[0].text(),
            "Read Tea docs (https://example.com/tea?q=rust#install) first."
        );
        assert!(!linked.is_empty());
        assert!(
            linked
                .iter()
                .all(|span| span.link() == Some("https://example.com/tea?q=rust#install"))
        );
        assert!(linked.iter().any(|span| span.style().strong()));
    }

    #[test]
    fn unsafe_or_oversized_links_never_become_metadata() {
        for destination in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "ftp://example.com/archive",
            "https://user:secret@example.com/private",
            "https://example.com/\u{1b}]8;;https://evil.test\u{7}",
            "https://",
        ] {
            assert_eq!(
                validate_hyperlink_destination(destination),
                None,
                "unsafe destination was accepted: {destination:?}"
            );
        }
        let oversized = format!(
            "https://example.com/{}",
            "a".repeat(MAX_HYPERLINK_DESTINATION_BYTES)
        );
        assert_eq!(validate_hyperlink_destination(&oversized), None);

        let lines = project("Open [unsafe](javascript:alert(1)) carefully.");
        assert_eq!(
            lines[0].text(),
            "Open unsafe (javascript:alert(1)) carefully."
        );
        assert!(lines[0].spans().iter().all(|span| span.link().is_none()));
    }

    #[test]
    fn separate_paragraphs_keep_one_compact_blank_line() {
        let lines = project("First paragraph.\n\nSecond paragraph.");
        assert_eq!(
            lines
                .iter()
                .map(|line| (line.kind(), line.text()))
                .collect::<Vec<_>>(),
            [
                (MarkdownKind::Paragraph, "First paragraph.".to_owned()),
                (MarkdownKind::Blank, String::new()),
                (MarkdownKind::Paragraph, "Second paragraph.".to_owned()),
            ]
        );
    }

    #[test]
    fn nested_quotes_keep_their_visible_depth() {
        let lines = project("> outer\n>\n> > inner");
        let quotes = lines
            .iter()
            .filter_map(|line| match line.kind() {
                MarkdownKind::Quote(depth) => Some((depth, line.text())),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            quotes,
            [
                (1, "> outer".to_owned()),
                (1, "> ".to_owned()),
                (2, "> > inner".to_owned()),
            ]
        );
    }

    #[test]
    fn inline_code_is_content_not_visible_markdown_source() {
        let lines = project("Use `tea-cli` for **strong output**.");
        let rendered = lines
            .iter()
            .map(MarkdownLine::text)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(rendered, "Use tea-cli for strong output.");
    }

    #[test]
    fn inline_semantics_survive_nested_markdown_without_delimiters() {
        let lines =
            project("**strong *both***, ~~removed~~, `tea-cli`, and [docs](https://example.test).");
        let spans = lines[0].spans();

        let strong = spans.iter().find(|span| span.text() == "strong ").unwrap();
        assert!(strong.style().strong());
        assert!(!strong.style().emphasis());
        let both = spans.iter().find(|span| span.text() == "both").unwrap();
        assert!(both.style().strong());
        assert!(both.style().emphasis());
        assert!(
            spans
                .iter()
                .find(|span| span.text() == "removed")
                .unwrap()
                .style()
                .strikethrough()
        );
        assert!(
            spans
                .iter()
                .find(|span| span.text() == "tea-cli")
                .unwrap()
                .style()
                .code()
        );
        assert!(
            spans
                .iter()
                .find(|span| span.text().starts_with("docs "))
                .unwrap()
                .style()
                .link()
        );
        assert_eq!(
            lines[0].text(),
            "strong both, removed, tea-cli, and docs (https://example.test/)."
        );
    }
}
