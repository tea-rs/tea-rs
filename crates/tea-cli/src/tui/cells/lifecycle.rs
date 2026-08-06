use super::super::components::{ActionGroup, DetailRow, Element};
use super::super::presentation::{LifecycleCell, LifecycleStatus, TimelineDetailKind};
use super::host::{CellContext, CellPresenter};
use super::style::{CellRole, CellStyle, CellVisualState};

impl CellPresenter for LifecycleCell {
    fn role(&self) -> CellRole {
        CellRole::Tool
    }

    fn visual_state(&self) -> CellVisualState {
        match self.status() {
            LifecycleStatus::Proposed | LifecycleStatus::Requested | LifecycleStatus::Queued => {
                CellVisualState::Pending
            }
            LifecycleStatus::ApprovalPending | LifecycleStatus::Uncertain => {
                CellVisualState::Warning
            }
            LifecycleStatus::Running => CellVisualState::Running,
            LifecycleStatus::Succeeded => CellVisualState::Success,
            LifecycleStatus::Failed | LifecycleStatus::Interrupted => CellVisualState::Error,
        }
    }

    fn content<'a>(&'a self, ctx: &'a CellContext<'a>, resolved_style: CellStyle) -> Element<'a> {
        let marker = match self.status() {
            LifecycleStatus::Proposed | LifecycleStatus::Requested | LifecycleStatus::Queued => "○",
            LifecycleStatus::ApprovalPending | LifecycleStatus::Uncertain => "?",
            LifecycleStatus::Running => ctx.theme.status_marker(self.tick()),
            LifecycleStatus::Succeeded => "•",
            LifecycleStatus::Failed | LifecycleStatus::Interrupted => "■",
        };
        let details = self
            .details()
            .iter()
            .filter(|detail| self.expanded() || detail.kind() != TimelineDetailKind::Metadata)
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
                self.target(),
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
    use crate::tui::presentation::{
        LifecycleCell, LifecycleKind, LifecycleStatus, TimelineDetail, TimelineDetailKind,
    };
    use crate::tui::{TerminalCapabilities, Theme};

    fn cell(
        status: LifecycleStatus,
        details: Vec<TimelineDetail>,
        expanded: bool,
        tick: u64,
    ) -> LifecycleCell {
        LifecycleCell::new(
            LifecycleKind::ToolCall,
            "Running",
            Some("cargo test"),
            status,
            details,
            expanded,
            tick,
        )
    }

    #[test]
    fn lifecycle_status_maps_to_one_visual_state() {
        for (status, expected) in [
            (LifecycleStatus::Proposed, CellVisualState::Pending),
            (LifecycleStatus::Requested, CellVisualState::Pending),
            (LifecycleStatus::Queued, CellVisualState::Pending),
            (LifecycleStatus::ApprovalPending, CellVisualState::Warning),
            (LifecycleStatus::Uncertain, CellVisualState::Warning),
            (LifecycleStatus::Running, CellVisualState::Running),
            (LifecycleStatus::Succeeded, CellVisualState::Success),
            (LifecycleStatus::Failed, CellVisualState::Error),
            (LifecycleStatus::Interrupted, CellVisualState::Error),
        ] {
            assert_eq!(cell(status, Vec::new(), false, 0).visual_state(), expected);
        }
    }

    #[test]
    fn every_lifecycle_status_retains_its_marker() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let host = CellHost::default();
        for (status, marker) in [
            (LifecycleStatus::Proposed, "○"),
            (LifecycleStatus::Requested, "○"),
            (LifecycleStatus::Queued, "○"),
            (LifecycleStatus::ApprovalPending, "?"),
            (LifecycleStatus::Uncertain, "?"),
            (LifecycleStatus::Running, theme.status_marker(2)),
            (LifecycleStatus::Succeeded, "•"),
            (LifecycleStatus::Failed, "■"),
            (LifecycleStatus::Interrupted, "■"),
        ] {
            let rendered = host.render(
                CellSpec::new(&cell(status, Vec::new(), false, 2)),
                &context,
                40,
            );
            assert!(rendered.lines()[0].text().starts_with(marker));
        }
    }

    #[test]
    fn metadata_details_remain_hidden_until_expanded() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let host = CellHost::default();
        let details = vec![
            TimelineDetail::new(TimelineDetailKind::Metadata, Some("id"), "call-1"),
            TimelineDetail::new(TimelineDetailKind::Output, None, "visible output"),
        ];

        let collapsed = cell(LifecycleStatus::Running, details.clone(), false, 0);
        let expanded = cell(LifecycleStatus::Running, details, true, 0);
        let collapsed = host.render(CellSpec::new(&collapsed), &context, 48);
        let expanded = host.render(CellSpec::new(&expanded), &context, 48);

        assert_eq!(collapsed.lines().len(), 2);
        assert!(
            !collapsed
                .lines()
                .iter()
                .any(|line| line.text().contains("call-1"))
        );
        assert_eq!(expanded.lines().len(), 3);
        assert!(
            expanded
                .lines()
                .iter()
                .any(|line| line.text().contains("id call-1"))
        );
    }

    #[test]
    fn running_marker_honors_reduced_motion() {
        let theme = Theme::for_capabilities(TerminalCapabilities::from_environment(
            Some("xterm-truecolor"),
            Some("truecolor"),
            false,
            true,
        ));
        let context = CellContext { theme: &theme };
        let host = CellHost::default();

        for tick in [0, 1, 2, 3] {
            let running = cell(LifecycleStatus::Running, Vec::new(), false, tick);
            let rendered = host.render(CellSpec::new(&running), &context, 40);
            assert!(rendered.lines()[0].text().starts_with("* "));
        }
    }
}
