use ratatui::style::{Modifier, Style};

use super::super::components::{
    ActionGroup, Component, DetailRow, DetailTree, Element, Markdown, RenderBlock, RenderContext,
    Text, VStack, truncate_cells,
};
use super::super::presentation::{PlanCell, PlanStep, PlanStepStatus};
use super::super::render_output::{RenderedLine, RenderedSpan};
use super::host::{CellContext, CellPresenter};
use super::style::{CellRole, CellStyle, CellVisualState};

pub(crate) const MAX_VISIBLE_PLAN_BODY_ROWS: usize = 12;

impl CellPresenter for PlanCell {
    fn role(&self) -> CellRole {
        CellRole::Plan
    }

    fn visual_state(&self) -> CellVisualState {
        if self.source().is_some() && self.steps().is_empty() {
            CellVisualState::Pending
        } else {
            CellVisualState::Success
        }
    }

    fn content<'a>(&'a self, ctx: &'a CellContext<'a>, resolved_style: CellStyle) -> Element<'a> {
        let proposal = self.source().is_some() && self.steps().is_empty();
        let style = resolved_style.foreground.resolve(ctx.theme);
        Element::new(VStack::new(vec![
            Element::new(ActionGroup::new(
                if proposal { "○" } else { "•" },
                self.title(),
                None,
                style,
                ctx.theme.footer,
                ctx.theme.footer,
            )),
            Element::new(PlanBody {
                source: self.source(),
                steps: self.steps(),
                note: self.note(),
            }),
        ]))
    }
}

struct PlanBody<'a> {
    source: Option<&'a str>,
    steps: &'a [PlanStep],
    note: Option<&'a str>,
}

impl Component for PlanBody<'_> {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        let mut children = Vec::new();
        if let Some(source) = self.source {
            children.push(Element::new(PlanSource { source }));
        }
        children.extend(
            self.steps
                .iter()
                .map(|step| Element::new(PlanStepView { step })),
        );
        if let Some(note) = self.note {
            children.push(Element::new(DetailTree::new(
                vec![DetailRow::new(note, ctx.theme.footer)],
                ctx.theme.footer,
            )));
        }

        let mut lines = Element::new(VStack::new(children))
            .render(ctx, width)
            .into_lines();
        if lines.len() > MAX_VISIBLE_PLAN_BODY_ROWS {
            let retained = MAX_VISIBLE_PLAN_BODY_ROWS.saturating_sub(1);
            let omitted = lines.len().saturating_sub(retained);
            lines.truncate(retained);
            lines.push(RenderedLine::new(
                truncate_cells(&format!("    ... +{omitted} plan rows"), usize::from(width)),
                ctx.theme.footer,
            ));
        }
        RenderBlock::from_lines(lines)
    }
}

struct PlanSource<'a> {
    source: &'a str,
}

impl Component for PlanSource<'_> {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        let inset = width > 4;
        let body_width = if inset {
            width.saturating_sub(4)
        } else {
            width.max(1)
        };
        let body = Element::new(Markdown::new(self.source))
            .render(ctx, body_width)
            .into_lines();
        if !inset {
            return RenderBlock::from_lines(body);
        }

        RenderBlock::from_lines(
            body.into_iter()
                .enumerate()
                .map(|(index, line)| {
                    prefix_line(
                        line,
                        if index == 0 { "  └ " } else { "    " },
                        ctx.theme.footer,
                    )
                })
                .collect(),
        )
    }
}

struct PlanStepView<'a> {
    step: &'a PlanStep,
}

impl Component for PlanStepView<'_> {
    fn render(&self, ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
        let (marker, marker_style, text_style) = match self.step.status() {
            PlanStepStatus::Pending => ("[ ]", ctx.theme.footer, ctx.theme.footer),
            PlanStepStatus::InProgress => (
                "[>]",
                ctx.theme.tool.add_modifier(Modifier::BOLD),
                ctx.theme.tool.add_modifier(Modifier::BOLD),
            ),
            PlanStepStatus::Completed => (
                "[x]",
                ctx.theme.footer.add_modifier(Modifier::BOLD),
                ctx.theme
                    .footer
                    .add_modifier(Modifier::DIM | Modifier::CROSSED_OUT),
            ),
        };
        Element::new(
            Text::new(self.step.text(), text_style)
                .with_prefixes(format!("  {marker} "), "      ")
                .with_prefix_style(marker_style),
        )
        .render(ctx, width)
    }
}

fn prefix_line(line: RenderedLine, prefix: &str, prefix_style: Style) -> RenderedLine {
    let mut spans = Vec::with_capacity(line.spans.len().saturating_add(1));
    spans.push(RenderedSpan::new(prefix.to_owned(), prefix_style));
    if line.spans.is_empty() {
        spans.push(RenderedSpan::new(line.text, line.style));
    } else {
        spans.extend(line.spans);
    }
    RenderedLine::from_spans(line.style, spans)
}

#[cfg(test)]
mod tests {
    use super::super::host::{CellContext, CellHost, CellPresenter, CellSpec};
    use super::super::style::CellVisualState;
    use crate::tui::Theme;
    use crate::tui::presentation::{PlanCell, PlanStep, PlanStepStatus};

    #[test]
    fn plan_proposals_are_pending_while_progress_is_successful() {
        let proposal = PlanCell::new("Proposed plan", Some("one step"), Vec::new(), None);
        let progress = PlanCell::new(
            "Plan progress",
            None,
            vec![PlanStep::new(PlanStepStatus::InProgress, "working")],
            None,
        );

        assert_eq!(proposal.visual_state(), CellVisualState::Pending);
        assert_eq!(progress.visual_state(), CellVisualState::Success);
    }

    #[test]
    fn plan_progress_markers_and_body_bound_are_preserved() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let mut steps = vec![
            PlanStep::new(PlanStepStatus::Pending, "pending"),
            PlanStep::new(PlanStepStatus::InProgress, "active"),
            PlanStep::new(PlanStepStatus::Completed, "complete"),
        ];
        steps.extend(
            (0..11).map(|index| PlanStep::new(PlanStepStatus::Pending, &format!("step {index}"))),
        );
        let cell = PlanCell::new("Plan progress", None, steps, None);

        let block = CellHost::default().render(CellSpec::new(&cell), &context, 48);
        let text = block
            .lines()
            .iter()
            .map(|line| line.text().trim_end())
            .collect::<Vec<_>>();

        assert_eq!(text.len(), 13);
        assert_eq!(text[0], "• Plan progress");
        assert_eq!(text[1], "  [ ] pending");
        assert_eq!(text[2], "  [>] active");
        assert_eq!(text[3], "  [x] complete");
        assert_eq!(text[12], "    ... +3 plan rows");
    }

    #[test]
    fn markdown_plan_source_reflows_through_the_host() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let cell = PlanCell::new(
            "Proposed plan",
            Some("inspect **shared state** before editing"),
            Vec::new(),
            None,
        );

        let block = CellHost::default().render(CellSpec::new(&cell), &context, 18);

        assert!(block.lines().len() > 2);
        assert_eq!(block.lines()[0].text().trim_end(), "○ Proposed plan");
        assert!(block.lines()[1].text().starts_with("  └ "));
        assert!(block.lines().iter().skip(1).any(|line| {
            line.rendered_spans().iter().any(|span| {
                span.style()
                    .add_modifier
                    .contains(ratatui::style::Modifier::BOLD)
            })
        }));
    }
}
