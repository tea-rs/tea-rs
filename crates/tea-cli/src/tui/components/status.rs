use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr as _;

use super::super::render_output::{RenderedLine, RenderedSpan};
use super::stack::VStack;
use super::{Component, Element, RenderBlock, RenderContext, Text};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DetailRow {
    text: String,
    style: Style,
}

impl DetailRow {
    pub(crate) fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

pub(crate) struct DetailTree {
    rows: Vec<DetailRow>,
    prefix_style: Style,
}

impl DetailTree {
    pub(crate) const fn new(rows: Vec<DetailRow>, prefix_style: Style) -> Self {
        Self { rows, prefix_style }
    }
}

impl Component for DetailTree {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        let total = self.rows.len();
        let children = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let last = index.saturating_add(1) == total;
                let (first, continuation) = if last {
                    ("  └ ", "    ")
                } else {
                    ("  ├ ", "  │ ")
                };
                Element::new(
                    Text::new(row.text.clone(), row.style)
                        .with_prefixes(first, continuation)
                        .with_prefix_style(self.prefix_style),
                )
            })
            .collect();
        Element::new(VStack::new(children)).render(ctx, width)
    }
}

pub(crate) struct ActionGroup<'a> {
    marker: &'a str,
    action: &'a str,
    target: Option<&'a str>,
    primary_style: Style,
    secondary_style: Style,
    prefix_style: Style,
    details: Vec<DetailRow>,
}

impl<'a> ActionGroup<'a> {
    pub(crate) const fn new(
        marker: &'a str,
        action: &'a str,
        target: Option<&'a str>,
        primary_style: Style,
        secondary_style: Style,
        prefix_style: Style,
    ) -> Self {
        Self {
            marker,
            action,
            target,
            primary_style,
            secondary_style,
            prefix_style,
            details: Vec::new(),
        }
    }

    pub(crate) fn with_details(mut self, details: Vec<DetailRow>) -> Self {
        self.details = details;
        self
    }
}

impl Component for ActionGroup<'_> {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        let target_is_inline = ActionHeader::new(
            self.marker,
            self.action,
            self.target,
            self.primary_style,
            self.secondary_style,
        )
        .target_is_inline(width);
        let header = ActionHeader::new(
            self.marker,
            self.action,
            target_is_inline.then_some(self.target).flatten(),
            self.primary_style,
            self.secondary_style,
        );
        let mut details = Vec::with_capacity(
            self.details
                .len()
                .saturating_add(usize::from(self.target.is_some() && !target_is_inline)),
        );
        if !target_is_inline && let Some(target) = self.target {
            details.push(DetailRow::new(target, self.secondary_style));
        }
        details.extend(self.details.iter().cloned());

        Element::new(VStack::new(vec![
            Element::new(header),
            Element::new(DetailTree::new(details, self.prefix_style)),
        ]))
        .render(ctx, width)
    }
}

pub(crate) struct ActionHeader<'a> {
    marker: &'a str,
    action: &'a str,
    target: Option<&'a str>,
    primary_style: Style,
    secondary_style: Style,
}

impl<'a> ActionHeader<'a> {
    pub(crate) const fn new(
        marker: &'a str,
        action: &'a str,
        target: Option<&'a str>,
        primary_style: Style,
        secondary_style: Style,
    ) -> Self {
        Self {
            marker,
            action,
            target,
            primary_style,
            secondary_style,
        }
    }

    pub(crate) fn target_is_inline(&self, width: u16) -> bool {
        let fixed_width = self
            .marker
            .width()
            .saturating_add(1)
            .saturating_add(self.action.width());
        self.target.is_none_or(|target| {
            fixed_width.saturating_add(1).saturating_add(target.width()) <= usize::from(width)
        })
    }
}

impl Component for ActionHeader<'_> {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        let fixed_width = self
            .marker
            .width()
            .saturating_add(1)
            .saturating_add(self.action.width());
        if fixed_width > usize::from(width) {
            return Element::new(
                Text::new(self.action, self.primary_style.add_modifier(Modifier::BOLD))
                    .with_prefixes(format!("{} ", self.marker), "  ")
                    .with_prefix_style(self.primary_style),
            )
            .render(ctx, width);
        }

        let mut spans = vec![
            RenderedSpan::new(self.marker.to_owned(), self.primary_style),
            RenderedSpan::new(" ".to_owned(), self.primary_style),
            RenderedSpan::new(
                self.action.to_owned(),
                self.primary_style.add_modifier(Modifier::BOLD),
            ),
        ];
        if let Some(target) = self.target {
            spans.push(RenderedSpan::new(" ".to_owned(), self.secondary_style));
            spans.push(RenderedSpan::new(target.to_owned(), self.secondary_style));
        }
        RenderBlock::from_lines(vec![RenderedLine::from_spans(self.primary_style, spans)])
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Style;

    use super::ActionGroup;
    use crate::tui::Theme;
    use crate::tui::components::{Element, RenderContext};

    #[test]
    fn action_group_moves_only_an_overflowing_target_into_the_tree() {
        let theme = Theme::default();
        let context = RenderContext { theme: &theme };
        let group = ActionGroup::new(
            "○",
            "Queued",
            Some("long target"),
            Style::default(),
            Style::default(),
            Style::default(),
        );

        let wide = Element::new(group).render(&context, 24);
        assert_eq!(wide.lines().len(), 1);
        assert_eq!(wide.lines()[0].text(), "○ Queued long target");

        let narrow = Element::new(ActionGroup::new(
            "○",
            "Queued",
            Some("long target"),
            Style::default(),
            Style::default(),
            Style::default(),
        ))
        .render(&context, 12);
        assert_eq!(
            narrow
                .lines()
                .iter()
                .map(RenderedLine::text)
                .collect::<Vec<_>>(),
            ["○ Queued", "  └ long tar", "    get"]
        );
    }

    use crate::tui::RenderedLine;
}
