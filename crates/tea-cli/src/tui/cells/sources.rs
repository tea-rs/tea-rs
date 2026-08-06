use super::super::components::{
    ActionHeader, Component, Element, RenderBlock, RenderContext, Text, VStack, truncate_cells,
};
use super::super::presentation::{SourcesCell, TimelineSource};
use super::super::render_output::{RenderedLine, RenderedSpan};
use super::host::{CellContext, CellPresenter};
use super::style::{CellRole, CellStyle};

impl CellPresenter for SourcesCell {
    fn role(&self) -> CellRole {
        CellRole::Sources
    }

    fn content<'a>(&'a self, ctx: &'a CellContext<'a>, resolved_style: CellStyle) -> Element<'a> {
        Element::new(VStack::new(vec![
            Element::new(ActionHeader::new(
                "•",
                "Sources",
                None,
                resolved_style.foreground.resolve(ctx.theme),
                ctx.theme.footer,
            )),
            Element::new(SourceList {
                sources: self.sources(),
            }),
        ]))
    }
}

struct SourceList<'a> {
    sources: &'a [TimelineSource],
}

impl Component for SourceList<'_> {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        let children = self
            .sources
            .iter()
            .enumerate()
            .map(|(index, source)| {
                Element::new(LinkedSource {
                    source,
                    index,
                    total: self.sources.len(),
                })
            })
            .collect();
        Element::new(VStack::new(children)).render(ctx, width)
    }
}

struct LinkedSource<'a> {
    source: &'a TimelineSource,
    index: usize,
    total: usize,
}

impl Component for LinkedSource<'_> {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        let last = self.index.saturating_add(1) == self.total;
        let first_prefix = if last { "  └ " } else { "  ├ " };
        let wrapped = Element::new(
            Text::new(self.source.label(), ctx.theme.footer).with_prefixes(first_prefix, "    "),
        )
        .render(ctx, width)
        .into_lines();
        let lines = wrapped
            .into_iter()
            .enumerate()
            .map(|(line_index, line)| {
                let prefix = if line_index == 0 {
                    first_prefix
                } else {
                    "    "
                };
                let prefix = truncate_cells(prefix, usize::from(width));
                let Some(content) = line.text.strip_prefix(&prefix) else {
                    return line;
                };
                let content = match self.source.destination() {
                    Some(destination) => RenderedSpan::with_link(
                        content.to_owned(),
                        ctx.theme.footer,
                        destination.to_owned(),
                    ),
                    None => RenderedSpan::new(content.to_owned(), ctx.theme.footer),
                };
                RenderedLine::from_spans(
                    ctx.theme.footer,
                    vec![RenderedSpan::new(prefix, ctx.theme.footer), content],
                )
            })
            .collect();
        RenderBlock::from_lines(lines)
    }
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr as _;

    use super::super::host::{CellContext, CellHost, CellSpec};
    use crate::tui::Theme;
    use crate::tui::presentation::{SourcesCell, TimelineSource};

    #[test]
    fn source_labels_wrap_and_preserve_validated_hyperlinks() {
        let source = tea_protocol::ExternalSource::new(
            "https://Example.COM:443/a/../docs?section=hosted-search",
        )
        .unwrap()
        .with_title("Hosted search architecture reference")
        .unwrap();
        let cell = SourcesCell::new(vec![TimelineSource::from_external(&source)]);
        let theme = Theme::default();
        let context = CellContext { theme: &theme };

        let rendered = CellHost::default().render(CellSpec::new(&cell), &context, 20);
        let links = rendered
            .lines()
            .iter()
            .flat_map(crate::tui::render_output::RenderedLine::rendered_spans)
            .filter_map(|span| span.link())
            .collect::<Vec<_>>();

        assert_eq!(rendered.lines()[0].text().trim_end(), "• Sources");
        assert!(
            rendered
                .lines()
                .iter()
                .all(|line| line.text().width() <= 18)
        );
        assert!(!links.is_empty());
        assert!(
            links
                .iter()
                .all(|link| *link == "https://example.com/docs?section=hosted-search")
        );
    }
}
