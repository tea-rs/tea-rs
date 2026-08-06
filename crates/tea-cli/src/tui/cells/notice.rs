use super::super::components::{DetailRow, DetailTree, Element, Text, VStack};
use super::super::presentation::{NoticeCell, NoticeSeverity};
use super::host::{CellContext, CellPresenter};
use super::style::{CellRole, CellStyle, CellVisualState};

impl CellPresenter for NoticeCell {
    fn role(&self) -> CellRole {
        CellRole::Notice
    }

    fn visual_state(&self) -> CellVisualState {
        match self.severity() {
            NoticeSeverity::Information => CellVisualState::Normal,
            NoticeSeverity::Warning => CellVisualState::Warning,
            NoticeSeverity::Error => CellVisualState::Error,
        }
    }

    fn content<'a>(&'a self, ctx: &'a CellContext<'a>, resolved_style: CellStyle) -> Element<'a> {
        let (marker, style) = (
            match self.severity() {
                NoticeSeverity::Information => "• ",
                NoticeSeverity::Warning => "! ",
                NoticeSeverity::Error => "■ ",
            },
            resolved_style.foreground.resolve(ctx.theme),
        );
        let mut children = vec![Element::new(
            Text::new(self.message(), style)
                .with_prefixes(marker, "  ")
                .with_prefix_style(style),
        )];
        if let Some(hint) = self.hint() {
            children.push(Element::new(DetailTree::new(
                vec![DetailRow::new(hint, ctx.theme.footer)],
                ctx.theme.footer,
            )));
        }
        Element::new(VStack::new(children))
    }
}

#[cfg(test)]
mod tests {
    use super::super::host::{CellContext, CellHost, CellPresenter, CellSpec};
    use super::super::style::CellVisualState;
    use crate::tui::Theme;
    use crate::tui::presentation::{NoticeCell, NoticeKind, NoticeSeverity};

    #[test]
    fn notice_severity_maps_to_visual_state_once() {
        for (severity, expected) in [
            (NoticeSeverity::Information, CellVisualState::Normal),
            (NoticeSeverity::Warning, CellVisualState::Warning),
            (NoticeSeverity::Error, CellVisualState::Error),
        ] {
            let cell = NoticeCell::new(NoticeKind::General, severity, "message", None);
            assert_eq!(cell.visual_state(), expected);
        }
    }

    #[test]
    fn notice_renders_marker_message_and_optional_hint_through_the_host() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let cell = NoticeCell::new(
            NoticeKind::General,
            NoticeSeverity::Warning,
            "check output",
            Some("retry once"),
        );

        let block = CellHost::default().render(CellSpec::new(&cell), &context, 24);
        let text = block
            .lines()
            .iter()
            .map(|line| line.text().trim_end())
            .collect::<Vec<_>>();

        assert_eq!(text, ["! check output", "  └ retry once"]);
    }
}
