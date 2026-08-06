use super::super::components::{ActionGroup, Element};
use super::super::presentation::{QueuedInputCell, QueuedInputKind};
use super::host::{CellContext, CellPresenter};
use super::style::{CellRole, CellStyle};

impl CellPresenter for QueuedInputCell {
    fn role(&self) -> CellRole {
        CellRole::QueuedInput
    }

    fn content<'a>(&'a self, ctx: &'a CellContext<'a>, resolved_style: CellStyle) -> Element<'a> {
        let action = match self.kind() {
            QueuedInputKind::Steering => "Queued steering",
            QueuedInputKind::FollowUp => "Queued follow-up",
        };
        Element::new(ActionGroup::new(
            "○",
            action,
            (!self.preview().is_empty()).then_some(self.preview()),
            resolved_style.foreground.resolve(ctx.theme),
            ctx.theme.footer,
            ctx.theme.footer,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::super::host::{CellContext, CellHost, CellPresenter, CellSpec};
    use super::super::style::CellRole;
    use crate::tui::Theme;
    use crate::tui::presentation::{CellId, QueuedInputCell, QueuedInputKind};

    #[test]
    fn queue_variants_share_layout_but_retain_typed_semantics() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let host = CellHost::default();
        let steering = QueuedInputCell::new(QueuedInputKind::Steering, "review this");
        let follow_up = QueuedInputCell::new(QueuedInputKind::FollowUp, "review this");

        assert_eq!(steering.role(), CellRole::QueuedInput);
        assert_eq!(follow_up.role(), CellRole::QueuedInput);
        assert_eq!(steering.kind(), QueuedInputKind::Steering);
        assert_eq!(follow_up.kind(), QueuedInputKind::FollowUp);
        assert_ne!(
            CellId::Queue {
                kind: steering.kind(),
                index: 0,
            },
            CellId::Queue {
                kind: follow_up.kind(),
                index: 0,
            }
        );

        let steering = host.render(CellSpec::new(&steering), &context, 40);
        let follow_up = host.render(CellSpec::new(&follow_up), &context, 40);
        assert_eq!(
            steering.lines()[0].text().trim_end(),
            "○ Queued steering review this"
        );
        assert_eq!(
            follow_up.lines()[0].text().trim_end(),
            "○ Queued follow-up review this"
        );
        assert_eq!(steering.margin(), follow_up.margin());
    }
}
