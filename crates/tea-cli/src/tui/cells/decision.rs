use super::super::components::{ActionGroup, DetailRow, Element};
use super::super::presentation::{DecisionCell, DecisionStatus, TimelineDetailKind};
use super::host::{CellContext, CellPresenter};
use super::style::{CellRole, CellStyle, CellVisualState};

impl CellPresenter for DecisionCell {
    fn role(&self) -> CellRole {
        CellRole::Decision
    }

    fn visual_state(&self) -> CellVisualState {
        match self.status() {
            DecisionStatus::Pending | DecisionStatus::Submitting => CellVisualState::Pending,
            DecisionStatus::Approved => CellVisualState::Success,
            DecisionStatus::Denied | DecisionStatus::TimedOut | DecisionStatus::Aborted => {
                CellVisualState::Error
            }
        }
    }

    fn content<'a>(&'a self, ctx: &'a CellContext<'a>, resolved_style: CellStyle) -> Element<'a> {
        let marker = match self.status() {
            DecisionStatus::Pending => "?",
            DecisionStatus::Approved => "•",
            DecisionStatus::Denied | DecisionStatus::TimedOut | DecisionStatus::Aborted => "■",
            DecisionStatus::Submitting => ctx.theme.status_marker(0),
        };
        let details = self
            .details()
            .iter()
            .map(|detail| {
                let text = detail.label().map_or_else(
                    || detail.text().to_owned(),
                    |label| format!("{label} {}", detail.text()),
                );
                let style = match detail.kind() {
                    TimelineDetailKind::Metadata | TimelineDetailKind::Progress => ctx.theme.footer,
                    TimelineDetailKind::Output => ctx.theme.normal,
                    TimelineDetailKind::Error => ctx.theme.error,
                };
                DetailRow::new(text, style)
            })
            .collect();
        Element::new(
            ActionGroup::new(
                marker,
                self.action(),
                (!self.subject().is_empty()).then_some(self.subject()),
                resolved_style.foreground.resolve(ctx.theme),
                ctx.theme.footer,
                ctx.theme.footer,
            )
            .with_details(details),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::host::{CellContext, CellHost, CellPresenter, CellSpec};
    use super::super::style::CellVisualState;
    use crate::tui::Theme;
    use crate::tui::presentation::{
        DecisionCell, DecisionStatus, TimelineDetail, TimelineDetailKind,
    };

    #[test]
    fn decision_status_maps_to_pending_success_or_error_visual_state() {
        for (status, expected) in [
            (DecisionStatus::Pending, CellVisualState::Pending),
            (DecisionStatus::Submitting, CellVisualState::Pending),
            (DecisionStatus::Approved, CellVisualState::Success),
            (DecisionStatus::Denied, CellVisualState::Error),
            (DecisionStatus::TimedOut, CellVisualState::Error),
            (DecisionStatus::Aborted, CellVisualState::Error),
        ] {
            let cell = DecisionCell::new("Decision", "subject", status, Vec::new());
            assert_eq!(cell.visual_state(), expected);
        }
    }

    #[test]
    fn decision_renders_action_subject_and_structured_details_through_the_host() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let cell = DecisionCell::new(
            "Approval required",
            "write_text_file",
            DecisionStatus::Pending,
            vec![TimelineDetail::new(
                TimelineDetailKind::Metadata,
                Some("scope"),
                "session",
            )],
        );

        let block = CellHost::default().render(CellSpec::new(&cell), &context, 48);
        let text = block
            .lines()
            .iter()
            .map(|line| line.text().trim_end())
            .collect::<Vec<_>>();

        assert_eq!(
            text,
            ["? Approval required write_text_file", "  └ scope session"]
        );
    }
}
