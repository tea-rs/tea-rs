use pulldown_cmark::Alignment;
use unicode_width::UnicodeWidthStr;

use super::wrapping;
use super::{
    MAX_MARKDOWN_LINES, MarkdownKind, MarkdownLine, MarkdownSpan, MarkdownSpanStyle,
    MarkdownSyntaxStyle, append_spans, push_span_with_metadata, span,
};

const MAX_TABLE_COLUMNS: usize = 32;
const MAX_TABLE_ROWS: usize = MAX_MARKDOWN_LINES;
const MAX_TABLE_BYTES: usize = 512 * 1024;
const TABLE_COLUMN_GAP: usize = 2;
const TABLE_CELL_PADDING: usize = 1;
const MIN_COLUMN_WIDTH: usize = 3;
const FIELD_LEADING_PADDING: usize = 1;
const FIELD_GAP: usize = 2;
const MIN_VALUE_WIDTH: usize = 3;
const MIN_ALIGNED_COMPACT_VALUE_WIDTH: usize = 12;
const MIN_ALIGNED_EXPANSIVE_VALUE_WIDTH: usize = 24;
const MIN_SCANNABLE_NARRATIVE_WIDTH: usize = 12;
const MIN_SCANNABLE_TOKEN_HEAVY_WIDTH: usize = 12;
const CRAMPED_EXPANSIVE_CELL_LINES: usize = 4;
const CATASTROPHIC_NARRATIVE_CELL_LINES: usize = 7;
const STACKED_VALUE_INDENT: usize = 2;

#[derive(Clone, Debug, Default)]
struct TableCell {
    lines: Vec<Vec<MarkdownSpan>>,
}

impl TableCell {
    fn push_span(
        &mut self,
        text: &str,
        style: MarkdownSpanStyle,
        link: Option<&str>,
        syntax: Option<MarkdownSyntaxStyle>,
    ) {
        if self.lines.is_empty() {
            self.lines.push(Vec::new());
        }
        if let Some(line) = self.lines.last_mut() {
            push_span_with_metadata(line, text, style, link, syntax);
        }
    }

    fn hard_break(&mut self) {
        self.lines.push(Vec::new());
    }

    fn plain_text(&self) -> String {
        self.lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug)]
struct TableBodyRow {
    cells: Vec<TableCell>,
    has_table_pipe_syntax: bool,
}

#[derive(Debug)]
pub(super) struct TableState {
    alignments: Vec<Alignment>,
    header: Option<Vec<TableCell>>,
    rows: Vec<TableBodyRow>,
    current_row: Option<Vec<TableCell>>,
    current_row_has_table_pipe_syntax: bool,
    current_cell: Option<TableCell>,
    in_header: bool,
    retained_bytes: usize,
}

impl TableState {
    pub(super) fn new(mut alignments: Vec<Alignment>) -> Self {
        alignments.truncate(MAX_TABLE_COLUMNS);
        Self {
            alignments,
            header: None,
            rows: Vec::new(),
            current_row: None,
            current_row_has_table_pipe_syntax: false,
            current_cell: None,
            in_header: false,
            retained_bytes: 0,
        }
    }

    pub(super) fn start_head(&mut self) {
        self.in_header = true;
        self.current_row = Some(Vec::new());
    }

    pub(super) fn end_head(&mut self) {
        self.finish_cell();
        if let Some(row) = self.current_row.take() {
            self.header = Some(row);
        }
        self.in_header = false;
    }

    pub(super) fn start_row(&mut self, has_table_pipe_syntax: bool) {
        self.current_row = Some(Vec::new());
        self.current_row_has_table_pipe_syntax = has_table_pipe_syntax;
    }

    pub(super) fn end_row(&mut self) {
        self.finish_cell();
        let Some(row) = self.current_row.take() else {
            return;
        };
        if self.in_header {
            self.header = Some(row);
        } else if self.rows.len() < MAX_TABLE_ROWS {
            self.rows.push(TableBodyRow {
                cells: row,
                has_table_pipe_syntax: self.current_row_has_table_pipe_syntax,
            });
        }
        self.current_row_has_table_pipe_syntax = false;
    }

    pub(super) fn start_cell(&mut self) {
        self.current_cell = Some(TableCell::default());
    }

    pub(super) fn end_cell(&mut self) {
        self.finish_cell();
    }

    fn finish_cell(&mut self) {
        let Some(cell) = self.current_cell.take() else {
            return;
        };
        let row = self.current_row.get_or_insert_with(Vec::new);
        if row.len() < self.alignments.len() {
            row.push(cell);
        }
    }

    pub(super) const fn in_cell(&self) -> bool {
        self.current_cell.is_some()
    }

    pub(super) fn push_span(
        &mut self,
        text: &str,
        style: MarkdownSpanStyle,
        link: Option<&str>,
        syntax: Option<MarkdownSyntaxStyle>,
    ) {
        let remaining = MAX_TABLE_BYTES.saturating_sub(self.retained_bytes);
        if remaining == 0 {
            return;
        }
        let text = truncate_to_bytes(text, remaining);
        self.retained_bytes = self.retained_bytes.saturating_add(text.len());
        if let Some(cell) = self.current_cell.as_mut() {
            cell.push_span(text, style, link, syntax);
        }
    }

    pub(super) fn hard_break(&mut self) {
        if let Some(cell) = self.current_cell.as_mut() {
            cell.hard_break();
        }
    }
}

pub(super) struct RenderedTable {
    pub(super) table_lines: Vec<MarkdownLine>,
    pub(super) table_lines_prewrapped: bool,
    pub(super) spillover_lines: Vec<MarkdownLine>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TableColumnKind {
    Narrative,
    TokenHeavy,
    Compact,
}

#[derive(Clone, Debug)]
struct TableColumnMetrics {
    max_width: usize,
    header_token_width: usize,
    body_token_width: usize,
    kind: TableColumnKind,
}

#[allow(clippy::too_many_lines)]
pub(super) fn render(
    mut state: TableState,
    wrap_width: Option<usize>,
    prefix_width: usize,
    row_kind: MarkdownKind,
) -> RenderedTable {
    let column_count = state.alignments.len();
    if column_count == 0 {
        return RenderedTable {
            table_lines: Vec::new(),
            table_lines_prewrapped: true,
            spillover_lines: Vec::new(),
        };
    }

    let mut spillover = Vec::new();
    let mut rows = Vec::with_capacity(state.rows.len());
    for row in state.rows {
        if column_count > 1 && is_spillover_row(&row) {
            if let Some(cell) = row.cells.into_iter().next() {
                spillover.push(cell);
            }
        } else {
            rows.push(row.cells);
        }
    }

    let mut header = state
        .header
        .take()
        .unwrap_or_else(|| vec![TableCell::default(); column_count]);
    normalize_row(&mut header, column_count);
    for row in &mut rows {
        normalize_row(row, column_count);
    }

    let metrics = collect_column_metrics(&header, &rows, column_count);
    let available_width = available_table_width(wrap_width, prefix_width, column_count);
    let widths = compute_column_widths(&metrics, available_width);
    let spillover_lines = spillover
        .into_iter()
        .flat_map(|cell| cell.lines)
        .take(MAX_MARKDOWN_LINES)
        .map(|spans| MarkdownLine {
            kind: row_kind,
            spans,
        })
        .collect();

    let Some(column_widths) = widths else {
        if !rows.is_empty() {
            return RenderedTable {
                table_lines: render_records(
                    &header,
                    &rows,
                    &metrics,
                    available_record_width(wrap_width, prefix_width),
                    row_kind,
                ),
                table_lines_prewrapped: true,
                spillover_lines,
            };
        }
        return RenderedTable {
            table_lines: render_pipe_fallback(&header, &rows, &state.alignments, row_kind),
            table_lines_prewrapped: false,
            spillover_lines,
        };
    };

    if should_render_records(&rows, &column_widths, &metrics) {
        return RenderedTable {
            table_lines: render_records(
                &header,
                &rows,
                &metrics,
                available_record_width(wrap_width, prefix_width),
                row_kind,
            ),
            table_lines_prewrapped: true,
            spillover_lines,
        };
    }

    let mut out = render_table_row(
        &header,
        &column_widths,
        &state.alignments,
        MarkdownSpanStyle(MarkdownSpanStyle::STRONG),
        row_kind,
    );
    push_bounded(
        &mut out,
        MarkdownLine {
            kind: MarkdownKind::Rule,
            spans: vec![span(
                &render_separator(&column_widths, '━'),
                MarkdownSpanStyle::default(),
            )],
        },
    );
    for (index, row) in rows.iter().enumerate() {
        for line in render_table_row(
            row,
            &column_widths,
            &state.alignments,
            MarkdownSpanStyle::default(),
            row_kind,
        ) {
            push_bounded(&mut out, line);
        }
        if index + 1 < rows.len() {
            push_bounded(
                &mut out,
                MarkdownLine {
                    kind: MarkdownKind::Rule,
                    spans: vec![span(
                        &render_separator(&column_widths, '─'),
                        MarkdownSpanStyle::default(),
                    )],
                },
            );
        }
        if out.len() >= MAX_MARKDOWN_LINES {
            break;
        }
    }

    RenderedTable {
        table_lines: out,
        table_lines_prewrapped: true,
        spillover_lines,
    }
}

fn normalize_row(row: &mut Vec<TableCell>, column_count: usize) {
    row.truncate(column_count);
    row.resize(column_count, TableCell::default());
}

fn available_table_width(
    wrap_width: Option<usize>,
    prefix_width: usize,
    column_count: usize,
) -> Option<usize> {
    wrap_width.map(|width| {
        let reserved = prefix_width
            + column_count.saturating_sub(1) * TABLE_COLUMN_GAP
            + column_count * TABLE_CELL_PADDING * 2;
        width.saturating_sub(reserved)
    })
}

fn available_record_width(wrap_width: Option<usize>, prefix_width: usize) -> Option<usize> {
    wrap_width.map(|width| width.saturating_sub(prefix_width).max(1))
}

fn compute_column_widths(
    metrics: &[TableColumnMetrics],
    available_width: Option<usize>,
) -> Option<Vec<usize>> {
    let mut widths = metrics
        .iter()
        .map(|column| column.max_width.max(MIN_COLUMN_WIDTH))
        .collect::<Vec<_>>();
    let Some(max_width) = available_width else {
        return Some(widths);
    };
    let minimum_total = metrics.len() * MIN_COLUMN_WIDTH;
    if max_width < minimum_total {
        return None;
    }

    let mut floors = metrics
        .iter()
        .map(preferred_column_floor)
        .collect::<Vec<_>>();
    let floor_total = floors.iter().sum::<usize>();
    if floor_total > max_width {
        let minimums = vec![MIN_COLUMN_WIDTH; floors.len()];
        shrink_columns(&mut floors, &minimums, metrics, floor_total - max_width);
    }

    let total_width = widths.iter().sum::<usize>();
    if total_width > max_width
        && shrink_columns(&mut widths, &floors, metrics, total_width - max_width) > 0
    {
        return None;
    }
    Some(widths)
}

fn collect_column_metrics(
    header: &[TableCell],
    rows: &[Vec<TableCell>],
    column_count: usize,
) -> Vec<TableColumnMetrics> {
    (0..column_count)
        .map(|column| {
            let header_cell = &header[column];
            let header_plain = header_cell.plain_text();
            let header_token_width = longest_token_width(&header_plain);
            let mut max_width = cell_width(header_cell);
            let mut body_token_width = 0usize;
            let mut body_token_count = 0usize;
            let mut long_body_token_count = 0usize;
            let mut total_words = 0usize;
            let mut total_cells = 0usize;
            let mut total_cell_width = 0usize;

            for row in rows {
                let cell = &row[column];
                max_width = max_width.max(cell_width(cell));
                let plain = cell.plain_text();
                let mut word_count = 0;
                for token in plain.split_whitespace() {
                    let token_width = token.width();
                    body_token_width = body_token_width.max(token_width);
                    long_body_token_count += usize::from(token_width >= 20);
                    word_count += 1;
                }
                if word_count > 0 {
                    body_token_count += word_count;
                    total_words += word_count;
                    total_cells += 1;
                    total_cell_width += plain.width();
                }
            }

            let narrative = if total_cells == 0 {
                header_plain.split_whitespace().count() >= 4 || header_plain.width() >= 28
            } else {
                total_words >= total_cells.saturating_mul(4)
                    || total_cell_width >= total_cells.saturating_mul(28)
            };
            let kind = if long_body_token_count > 0
                && long_body_token_count >= body_token_count.saturating_sub(long_body_token_count)
            {
                TableColumnKind::TokenHeavy
            } else if narrative {
                TableColumnKind::Narrative
            } else {
                TableColumnKind::Compact
            };
            TableColumnMetrics {
                max_width,
                header_token_width,
                body_token_width,
                kind,
            }
        })
        .collect()
}

fn preferred_column_floor(metrics: &TableColumnMetrics) -> usize {
    let target = match metrics.kind {
        TableColumnKind::Narrative | TableColumnKind::TokenHeavy => 16,
        TableColumnKind::Compact => metrics
            .header_token_width
            .max(metrics.body_token_width.min(16)),
    };
    target
        .max(MIN_COLUMN_WIDTH)
        .min(metrics.max_width.max(MIN_COLUMN_WIDTH))
}

fn shrink_columns(
    widths: &mut [usize],
    floors: &[usize],
    metrics: &[TableColumnMetrics],
    mut amount: usize,
) -> usize {
    for kind in [
        TableColumnKind::TokenHeavy,
        TableColumnKind::Narrative,
        TableColumnKind::Compact,
    ] {
        let slack_total = widths
            .iter()
            .enumerate()
            .filter(|(index, _)| metrics[*index].kind == kind)
            .map(|(index, width)| width.saturating_sub(floors[index]))
            .sum::<usize>();
        let to_remove = amount.min(slack_total);
        if to_remove == 0 {
            continue;
        }

        let mut low = 0;
        let mut high = widths
            .iter()
            .enumerate()
            .filter(|(index, _)| metrics[*index].kind == kind)
            .map(|(index, width)| width.saturating_sub(floors[index]))
            .max()
            .unwrap_or(0);
        while low < high {
            let cap = low + (high - low) / 2;
            let removed = widths
                .iter()
                .enumerate()
                .filter(|(index, _)| metrics[*index].kind == kind)
                .map(|(index, width)| width.saturating_sub(floors[index]).saturating_sub(cap))
                .sum::<usize>();
            if removed > to_remove {
                low = cap + 1;
            } else {
                high = cap;
            }
        }

        let cap = low;
        let mut removed = 0;
        for (index, width) in widths.iter_mut().enumerate() {
            if metrics[index].kind == kind {
                let reduction = width.saturating_sub(floors[index]).saturating_sub(cap);
                *width -= reduction;
                removed += reduction;
            }
        }
        let mut remainder = to_remove - removed;
        for (index, width) in widths.iter_mut().enumerate() {
            if remainder == 0 {
                break;
            }
            if metrics[index].kind == kind && width.saturating_sub(floors[index]) == cap {
                *width -= 1;
                remainder -= 1;
            }
        }
        amount -= to_remove;
        if amount == 0 {
            break;
        }
    }
    amount
}

fn render_table_row(
    row: &[TableCell],
    widths: &[usize],
    alignments: &[Alignment],
    row_style: MarkdownSpanStyle,
    kind: MarkdownKind,
) -> Vec<MarkdownLine> {
    let wrapped_cells = row
        .iter()
        .zip(widths)
        .map(|(cell, width)| wrap_cell(cell, *width))
        .collect::<Vec<_>>();
    let row_height = wrapped_cells.iter().map(Vec::len).max().unwrap_or(1);
    let mut output = Vec::with_capacity(row_height);

    for row_line in 0..row_height {
        let Some(last_visible_column) = wrapped_cells.iter().rposition(|lines| {
            lines
                .get(row_line)
                .is_some_and(|line| spans_width(line) > 0)
        }) else {
            output.push(MarkdownLine {
                kind,
                spans: Vec::new(),
            });
            continue;
        };
        let mut spans = Vec::new();
        for (column, width) in widths.iter().enumerate().take(last_visible_column + 1) {
            append_spaces(&mut spans, TABLE_CELL_PADDING);
            let line = wrapped_cells[column]
                .get(row_line)
                .cloned()
                .unwrap_or_default();
            let remaining = width.saturating_sub(spans_width(&line));
            let (left, right) = alignment_padding(alignments[column], remaining);
            append_spaces(&mut spans, left);
            append_styled_spans(&mut spans, line, row_style);
            if column != last_visible_column {
                append_spaces(&mut spans, right + TABLE_CELL_PADDING + TABLE_COLUMN_GAP);
            }
        }
        output.push(MarkdownLine { kind, spans });
    }
    output
}

fn render_separator(widths: &[usize], character: char) -> String {
    widths
        .iter()
        .map(|width| character.to_string().repeat(width + TABLE_CELL_PADDING * 2))
        .collect::<Vec<_>>()
        .join(&" ".repeat(TABLE_COLUMN_GAP))
}

fn should_render_records(
    rows: &[Vec<TableCell>],
    widths: &[usize],
    metrics: &[TableColumnMetrics],
) -> bool {
    if rows.is_empty() {
        return false;
    }
    let affected = rows
        .iter()
        .filter(|row| {
            let fragmented = row
                .iter()
                .zip(widths)
                .zip(metrics)
                .any(|((cell, width), metrics)| {
                    let has_long_token = cell
                        .plain_text()
                        .split_whitespace()
                        .any(|token| token.width() > *width);
                    match metrics.kind {
                        TableColumnKind::Compact => has_long_token,
                        TableColumnKind::TokenHeavy => {
                            *width < MIN_SCANNABLE_TOKEN_HEAVY_WIDTH && has_long_token
                        }
                        TableColumnKind::Narrative => false,
                    }
                });
            fragmented || expansive_cells_are_starved(row, widths, metrics)
        })
        .count();
    let threshold = if rows.len() == 1 {
        1
    } else {
        2.max(rows.len().div_ceil(3))
    };
    affected >= threshold
}

fn expansive_cells_are_starved(
    row: &[TableCell],
    widths: &[usize],
    metrics: &[TableColumnMetrics],
) -> bool {
    let expansive = row
        .iter()
        .zip(widths)
        .zip(metrics)
        .filter(|(_, metrics)| metrics.kind != TableColumnKind::Compact)
        .map(|((cell, width), metrics)| (metrics.kind, *width, wrap_cell(cell, *width).len()))
        .collect::<Vec<_>>();
    expansive
        .iter()
        .filter(|(_, _, height)| *height >= CRAMPED_EXPANSIVE_CELL_LINES)
        .count()
        >= 2
        || expansive.iter().any(|(kind, width, height)| {
            *kind == TableColumnKind::Narrative
                && *width < MIN_SCANNABLE_NARRATIVE_WIDTH
                && *height >= CATASTROPHIC_NARRATIVE_CELL_LINES
        })
}

fn render_records(
    headers: &[TableCell],
    rows: &[Vec<TableCell>],
    metrics: &[TableColumnMetrics],
    available_width: Option<usize>,
    kind: MarkdownKind,
) -> Vec<MarkdownLine> {
    let label_width = headers
        .iter()
        .map(|header| header.plain_text().width())
        .max()
        .unwrap_or(0);
    let minimum_value_width = if metrics
        .iter()
        .any(|metrics| metrics.kind != TableColumnKind::Compact)
    {
        MIN_ALIGNED_EXPANSIVE_VALUE_WIDTH
    } else {
        MIN_ALIGNED_COMPACT_VALUE_WIDTH
    };
    let aligned = available_width.is_none_or(|width| {
        FIELD_LEADING_PADDING + label_width + FIELD_GAP + minimum_value_width <= width
    });
    let mut output = Vec::new();

    for (row_index, row) in rows.iter().enumerate() {
        for (header, value) in headers.iter().zip(row) {
            if aligned {
                render_aligned_field(
                    &mut output,
                    header,
                    value,
                    label_width,
                    available_width,
                    kind,
                );
            } else {
                render_stacked_field(&mut output, header, value, available_width, kind);
            }
        }
        if row_index + 1 < rows.len() {
            let width = available_width.unwrap_or_else(|| widest_line_width(&output));
            push_bounded(
                &mut output,
                MarkdownLine {
                    kind: MarkdownKind::Rule,
                    spans: vec![span(&"─".repeat(width), MarkdownSpanStyle::default())],
                },
            );
        }
        if output.len() >= MAX_MARKDOWN_LINES {
            break;
        }
    }
    output
}

fn render_aligned_field(
    output: &mut Vec<MarkdownLine>,
    header: &TableCell,
    value: &TableCell,
    label_width: usize,
    available_width: Option<usize>,
    kind: MarkdownKind,
) {
    let value_indent = FIELD_LEADING_PADDING + label_width + FIELD_GAP;
    let value_width = available_width.map_or_else(
        || cell_width(value).max(MIN_VALUE_WIDTH),
        |width| width.saturating_sub(value_indent).max(MIN_VALUE_WIDTH),
    );
    for (index, value_line) in wrap_cell(value, value_width).into_iter().enumerate() {
        let mut spans = Vec::new();
        if index == 0 {
            append_spaces(&mut spans, FIELD_LEADING_PADDING);
            push_span_with_metadata(
                &mut spans,
                &header.plain_text(),
                MarkdownSpanStyle(MarkdownSpanStyle::STRONG),
                None,
                None,
            );
            append_spaces(
                &mut spans,
                label_width.saturating_sub(header.plain_text().width()) + FIELD_GAP,
            );
        } else {
            append_spaces(&mut spans, value_indent);
        }
        append_spans(&mut spans, value_line);
        push_bounded(output, MarkdownLine { kind, spans });
    }
}

fn render_stacked_field(
    output: &mut Vec<MarkdownLine>,
    header: &TableCell,
    value: &TableCell,
    available_width: Option<usize>,
    kind: MarkdownKind,
) {
    let label_indent = available_width.map_or(FIELD_LEADING_PADDING, |width| {
        FIELD_LEADING_PADDING.min(width.saturating_sub(1))
    });
    let label_width = available_width.map_or_else(
        || header.plain_text().width().max(1),
        |width| width.saturating_sub(label_indent).max(1),
    );
    let labels = wrapping::wrap_spans(
        &[span(
            &header.plain_text(),
            MarkdownSpanStyle(MarkdownSpanStyle::STRONG),
        )],
        &[],
        &[],
        label_width,
    );
    for label in labels {
        let mut spans = Vec::new();
        append_spaces(&mut spans, label_indent);
        append_spans(&mut spans, label);
        push_bounded(output, MarkdownLine { kind, spans });
    }

    let value_indent = available_width.map_or(STACKED_VALUE_INDENT, |width| {
        STACKED_VALUE_INDENT.min(width.saturating_sub(1))
    });
    let value_width = available_width.map_or_else(
        || cell_width(value).max(1),
        |width| width.saturating_sub(value_indent).max(1),
    );
    for value_line in wrap_cell(value, value_width) {
        let mut spans = Vec::new();
        append_spaces(&mut spans, value_indent);
        append_spans(&mut spans, value_line);
        push_bounded(output, MarkdownLine { kind, spans });
    }
}

fn render_pipe_fallback(
    header: &[TableCell],
    rows: &[Vec<TableCell>],
    alignments: &[Alignment],
    kind: MarkdownKind,
) -> Vec<MarkdownLine> {
    let mut output = vec![MarkdownLine {
        kind,
        spans: row_to_pipe_spans(header),
    }];
    output.push(MarkdownLine {
        kind,
        spans: vec![span(
            &alignments_to_pipe_delimiter(alignments),
            MarkdownSpanStyle::default(),
        )],
    });
    output.extend(
        rows.iter()
            .take(MAX_MARKDOWN_LINES - 2)
            .map(|row| MarkdownLine {
                kind,
                spans: row_to_pipe_spans(row),
            }),
    );
    output
}

fn row_to_pipe_spans(row: &[TableCell]) -> Vec<MarkdownSpan> {
    let mut output = vec![span("|", MarkdownSpanStyle::default())];
    for cell in row {
        append_spaces(&mut output, 1);
        for (line_index, line) in cell.lines.iter().enumerate() {
            if line_index > 0 {
                append_spaces(&mut output, 1);
            }
            for source in line {
                let escaped = source.text.replace('|', "\\|");
                push_span_with_metadata(
                    &mut output,
                    &escaped,
                    source.style,
                    source.link.as_deref(),
                    source.syntax,
                );
            }
        }
        push_span_with_metadata(&mut output, " |", MarkdownSpanStyle::default(), None, None);
    }
    output
}

fn alignments_to_pipe_delimiter(alignments: &[Alignment]) -> String {
    let mut output = String::from("|");
    for alignment in alignments {
        output.push_str(match alignment {
            Alignment::Left => ":---",
            Alignment::Center => ":---:",
            Alignment::Right => "---:",
            Alignment::None => "---",
        });
        output.push('|');
    }
    output
}

fn wrap_cell(cell: &TableCell, width: usize) -> Vec<Vec<MarkdownSpan>> {
    if cell.lines.is_empty() {
        return vec![Vec::new()];
    }
    let mut output = Vec::new();
    for line in &cell.lines {
        let wrapped = wrapping::wrap_spans(line, &[], &[], width.max(1));
        if wrapped.is_empty() {
            output.push(Vec::new());
        } else {
            output.extend(wrapped);
        }
    }
    if output.is_empty() {
        output.push(Vec::new());
    }
    output
}

fn is_spillover_row(row: &TableBodyRow) -> bool {
    !row.has_table_pipe_syntax
        && row
            .cells
            .first()
            .is_some_and(|cell| !cell.plain_text().trim().is_empty())
        && row
            .cells
            .iter()
            .skip(1)
            .all(|cell| cell.plain_text().trim().is_empty())
}

fn alignment_padding(alignment: Alignment, remaining: usize) -> (usize, usize) {
    match alignment {
        Alignment::Left | Alignment::None => (0, remaining),
        Alignment::Center => (remaining / 2, remaining - remaining / 2),
        Alignment::Right => (remaining, 0),
    }
}

fn append_styled_spans(
    output: &mut Vec<MarkdownSpan>,
    spans: Vec<MarkdownSpan>,
    overlay: MarkdownSpanStyle,
) {
    for source in spans {
        push_span_with_metadata(
            output,
            &source.text,
            source.style.merge(overlay),
            source.link.as_deref(),
            source.syntax,
        );
    }
}

fn append_spaces(output: &mut Vec<MarkdownSpan>, count: usize) {
    if count > 0 {
        push_span_with_metadata(
            output,
            &" ".repeat(count),
            MarkdownSpanStyle::default(),
            None,
            None,
        );
    }
}

fn spans_width(spans: &[MarkdownSpan]) -> usize {
    spans.iter().map(|span| span.text.width()).sum()
}

fn cell_width(cell: &TableCell) -> usize {
    cell.lines
        .iter()
        .map(|line| spans_width(line))
        .max()
        .unwrap_or(0)
}

fn longest_token_width(text: &str) -> usize {
    text.split_whitespace()
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0)
}

fn widest_line_width(lines: &[MarkdownLine]) -> usize {
    lines
        .iter()
        .map(|line| spans_width(&line.spans))
        .max()
        .unwrap_or(0)
}

fn push_bounded(output: &mut Vec<MarkdownLine>, line: MarkdownLine) {
    if output.len() < MAX_MARKDOWN_LINES {
        output.push(line);
    }
}

fn truncate_to_bytes(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
