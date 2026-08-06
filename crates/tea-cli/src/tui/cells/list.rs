use ratatui::style::Style;

use super::super::components::RenderBlock;
use super::super::render_output::RenderedLine;
use super::host::CellBlock;

pub(crate) struct CellList {
    blocks: Vec<CellBlock>,
}

impl CellList {
    pub(crate) const fn new(blocks: Vec<CellBlock>) -> Self {
        Self { blocks }
    }

    pub(crate) fn render(self) -> RenderBlock {
        let mut blocks = self.blocks.into_iter().filter(|block| !block.is_empty());
        let Some(first) = blocks.next() else {
            return RenderBlock::empty();
        };
        let (first_lines, first_margin) = first.into_parts();
        let mut lines = Vec::new();
        append_blank_rows(&mut lines, first_margin.top);
        append_block_lines(&mut lines, first_lines, first_margin.left);
        let mut previous_bottom = first_margin.bottom;

        for block in blocks {
            let (block_lines, margin) = block.into_parts();
            append_blank_rows(&mut lines, previous_bottom.max(margin.top));
            append_block_lines(&mut lines, block_lines, margin.left);
            previous_bottom = margin.bottom;
        }
        append_blank_rows(&mut lines, previous_bottom);
        RenderBlock::from_lines(lines)
    }
}

fn append_blank_rows(lines: &mut Vec<RenderedLine>, rows: u16) {
    lines.extend((0..rows).map(|_| RenderedLine::new(String::new(), Style::default())));
}

fn append_block_lines(lines: &mut Vec<RenderedLine>, block: Vec<RenderedLine>, left: u16) {
    lines.extend(
        block
            .into_iter()
            .map(|line| line.with_left_columns(usize::from(left))),
    );
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Style};

    use super::CellList;
    use crate::tui::Theme;
    use crate::tui::cells::host::{CellBlock, CellContext, CellHost, CellPresenter, CellSpec};
    use crate::tui::cells::style::{
        CellRole, CellStyle, CellStylePatch, CellStyleSheet, InsetsPatch,
    };
    use crate::tui::components::style::Insets;
    use crate::tui::components::{Component, Element, RenderBlock, RenderContext};
    use crate::tui::render_output::{RenderedLine, RenderedSpan};

    fn block(text: &str, margin: Insets) -> CellBlock {
        CellBlock::new(
            vec![RenderedLine::new(text.to_owned(), Style::default())],
            margin,
        )
    }

    #[test]
    fn list_collapses_adjacent_vertical_margins_with_max() {
        let rendered = CellList::new(vec![
            block(
                "first",
                Insets {
                    top: 1,
                    right: 0,
                    bottom: 2,
                    left: 0,
                },
            ),
            block(
                "second",
                Insets {
                    top: 3,
                    right: 0,
                    bottom: 4,
                    left: 0,
                },
            ),
        ])
        .render();
        let text = rendered
            .lines()
            .iter()
            .map(RenderedLine::text)
            .collect::<Vec<_>>();

        assert_eq!(
            text,
            vec!["", "first", "", "", "", "second", "", "", "", ""]
        );
    }

    #[test]
    fn list_filters_empty_cells_before_resolving_spacing() {
        let rendered = CellList::new(vec![
            block(
                "first",
                Insets {
                    bottom: 1,
                    ..Insets::default()
                },
            ),
            CellBlock::new(
                Vec::new(),
                Insets {
                    top: 9,
                    bottom: 9,
                    ..Insets::default()
                },
            ),
            block(
                "second",
                Insets {
                    top: 2,
                    ..Insets::default()
                },
            ),
        ])
        .render();
        let text = rendered
            .lines()
            .iter()
            .map(RenderedLine::text)
            .collect::<Vec<_>>();

        assert_eq!(text, vec!["first", "", "", "second"]);
    }

    #[test]
    fn list_applies_horizontal_margin_without_losing_links() {
        let destination = "https://example.com/docs";
        let style = Style::default().fg(Color::Blue);
        let linked = RenderedLine::from_spans(
            style,
            vec![RenderedSpan::with_link(
                "docs".to_owned(),
                style,
                destination.to_owned(),
            )],
        );
        let rendered = CellList::new(vec![CellBlock::new(
            vec![linked],
            Insets {
                left: 2,
                ..Insets::default()
            },
        )])
        .render();

        assert_eq!(rendered.lines()[0].text(), "  docs");
        assert_eq!(rendered.lines()[0].rendered_spans()[0].link(), None);
        assert_eq!(
            rendered.lines()[0].rendered_spans()[1].link(),
            Some(destination)
        );
    }

    struct OneLine(&'static str);

    impl Component for OneLine {
        fn render(&self, _ctx: &RenderContext<'_>, _width: u16) -> RenderBlock {
            RenderBlock::from_lines(vec![RenderedLine::new(self.0.to_owned(), Style::default())])
        }
    }

    struct SimplePresenter {
        role: CellRole,
        text: &'static str,
    }

    impl CellPresenter for SimplePresenter {
        fn role(&self) -> CellRole {
            self.role
        }

        fn content<'a>(
            &'a self,
            _ctx: &'a CellContext<'a>,
            _resolved_style: CellStyle,
        ) -> Element<'a> {
            Element::new(OneLine(self.text))
        }
    }

    fn render_mixed_list(host: &CellHost) -> RenderBlock {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let first = SimplePresenter {
            role: CellRole::Diff,
            text: "first",
        };
        let second = SimplePresenter {
            role: CellRole::Notice,
            text: "second",
        };
        CellList::new(vec![
            host.render(CellSpec::new(&first), &context, 20),
            host.render(CellSpec::new(&second), &context, 20),
        ])
        .render()
    }

    #[test]
    fn changing_global_vertical_defaults_reflows_a_mixed_list_without_presenter_edits() {
        let compact = render_mixed_list(&CellHost::default());
        let spaced_host =
            CellHost::new(CellStyleSheet::default().with_global_patch(CellStylePatch {
                margin: InsetsPatch {
                    top: Some(1),
                    bottom: Some(1),
                    ..InsetsPatch::default()
                },
                ..CellStylePatch::default()
            }));
        let spaced = render_mixed_list(&spaced_host);

        assert_eq!(compact.lines().len(), 2);
        assert_eq!(spaced.lines().len(), 5);
        assert_eq!(
            spaced
                .lines()
                .iter()
                .map(|line| line.text().trim())
                .collect::<Vec<_>>(),
            vec!["", "first", "", "second", ""]
        );
    }
}
