use super::super::components::{Element, Markdown, Text, VStack};
use super::super::presentation::{MessageAuthor, MessageCell, OutputFormat};
use super::host::{CellContext, CellPresenter};
use super::style::{CellRole, CellStyle};

impl CellPresenter for MessageCell {
    fn role(&self) -> CellRole {
        match self.author() {
            MessageAuthor::User => CellRole::UserMessage,
            MessageAuthor::Assistant => CellRole::AssistantMessage,
        }
    }

    fn content<'a>(&'a self, ctx: &'a CellContext<'a>, resolved_style: CellStyle) -> Element<'a> {
        if self.source().is_empty() {
            return Element::new(VStack::new(Vec::new()));
        }

        let style = resolved_style.foreground.resolve(ctx.theme);
        if self.author() == MessageAuthor::User {
            return Element::new(Text::new(self.source(), style).with_prefixes("› ", "  "));
        }

        match self.format() {
            OutputFormat::Markdown => Element::new(Markdown::new(self.source())),
            OutputFormat::Plain | OutputFormat::Terminal => {
                Element::new(Text::new(self.source(), style))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr as _;

    use super::super::host::{CellContext, CellHost, CellSpec};
    use crate::tui::Theme;
    use crate::tui::components::style::Insets;
    use crate::tui::presentation::{MessageAuthor, MessageCell, OutputFormat};

    #[test]
    fn user_message_is_full_bleed_with_composer_surface_and_bottom_margin() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let cell = MessageCell::new(MessageAuthor::User, "ship it", OutputFormat::Plain);

        let block = CellHost::default().render(CellSpec::new(&cell), &context, 16);

        assert_eq!(block.margin().left, 0);
        assert_eq!(block.margin().right, 0);
        assert_eq!(block.margin().bottom, 1);
        assert_eq!(block.lines().len(), 3);
        assert_eq!(block.lines()[1].text().trim_end(), "› ship it");
        assert!(block.lines().iter().all(|line| line.text().width() == 16));
        assert!(block.lines().iter().all(|line| {
            line.style().bg == theme.composer.bg
                && line
                    .rendered_spans()
                    .iter()
                    .all(|span| span.style().bg == theme.composer.bg)
        }));
    }

    #[test]
    fn assistant_message_is_transparent_with_default_inset_and_bottom_margin() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let cell = MessageCell::new(MessageAuthor::Assistant, "**done**", OutputFormat::Markdown);

        let block = CellHost::default().render(CellSpec::new(&cell), &context, 16);

        assert_eq!(block.margin().left, 1);
        assert_eq!(block.margin().right, 1);
        assert_eq!(block.margin().bottom, 1);
        assert_eq!(block.lines().len(), 1);
        assert_eq!(block.lines()[0].text().trim_end(), "done");
        assert!(block.lines().iter().all(|line| {
            line.style().bg.is_none()
                && line
                    .rendered_spans()
                    .iter()
                    .all(|span| span.style().bg.is_none())
        }));
    }

    #[test]
    fn empty_message_has_no_surface_or_external_spacing() {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        let cell = MessageCell::new(MessageAuthor::Assistant, "", OutputFormat::Plain);

        let block = CellHost::default().render(CellSpec::new(&cell), &context, 16);

        assert!(block.is_empty());
        assert_eq!(block.margin(), Insets::default());
    }
}
