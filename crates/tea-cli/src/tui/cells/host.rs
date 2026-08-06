use super::super::components::style::Insets;
use super::super::components::{Element, RenderContext, Surface};
use super::super::render_output::RenderedLine;
use super::super::theme::Theme;
use super::style::{CellRole, CellStyle, CellStylePatch, CellStyleSheet, CellVisualState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CellDescriptor {
    pub(crate) role: CellRole,
    pub(crate) state: CellVisualState,
    pub(crate) style_patch: CellStylePatch,
}

pub(crate) trait CellPresenter {
    fn role(&self) -> CellRole;

    fn visual_state(&self) -> CellVisualState {
        CellVisualState::Normal
    }

    fn style_patch(&self) -> CellStylePatch {
        CellStylePatch::default()
    }

    fn descriptor(&self) -> CellDescriptor {
        CellDescriptor {
            role: self.role(),
            state: self.visual_state(),
            style_patch: self.style_patch(),
        }
    }

    fn content<'a>(&'a self, ctx: &'a CellContext<'a>, resolved_style: CellStyle) -> Element<'a>;
}

#[derive(Clone, Copy)]
pub(crate) struct CellSpec<'a> {
    pub(crate) presenter: &'a dyn CellPresenter,
    pub(crate) descriptor: CellDescriptor,
}

impl<'a> CellSpec<'a> {
    pub(crate) fn new(presenter: &'a dyn CellPresenter) -> Self {
        Self {
            descriptor: presenter.descriptor(),
            presenter,
        }
    }
}

#[derive(Debug)]
pub(crate) struct CellContext<'a> {
    pub(crate) theme: &'a Theme,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CellBlock {
    lines: Vec<RenderedLine>,
    margin: Insets,
}

impl CellBlock {
    pub(crate) const fn empty() -> Self {
        Self {
            lines: Vec::new(),
            margin: Insets {
                top: 0,
                right: 0,
                bottom: 0,
                left: 0,
            },
        }
    }

    pub(crate) const fn new(lines: Vec<RenderedLine>, margin: Insets) -> Self {
        Self { lines, margin }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn lines(&self) -> &[RenderedLine] {
        &self.lines
    }

    #[cfg(test)]
    pub(crate) const fn margin(&self) -> Insets {
        self.margin
    }

    pub(super) fn into_parts(self) -> (Vec<RenderedLine>, Insets) {
        (self.lines, self.margin)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CellHost {
    stylesheet: CellStyleSheet,
}

impl CellHost {
    pub(crate) const fn new(stylesheet: CellStyleSheet) -> Self {
        Self { stylesheet }
    }

    pub(crate) const fn stylesheet_generation(&self) -> u64 {
        self.stylesheet.generation()
    }

    pub(crate) fn render<'a>(
        &self,
        spec: CellSpec<'a>,
        ctx: &'a CellContext<'a>,
        width: u16,
    ) -> CellBlock {
        let width = width.max(1);
        let resolved = self.stylesheet.resolve(
            spec.descriptor.role,
            spec.descriptor.state,
            spec.descriptor.style_patch,
        );
        let (left, right) =
            fit_horizontal_margins(width, resolved.margin.left, resolved.margin.right);
        let surface_width = width.saturating_sub(left).saturating_sub(right).max(1);
        let content = spec.presenter.content(ctx, resolved);
        let surface = Surface::new(content)
            .with_padding(resolved.padding)
            .with_background(resolved.background);
        let render_context = RenderContext { theme: ctx.theme };
        let rendered = Element::new(surface).render(&render_context, surface_width);
        if rendered.is_empty() {
            return CellBlock::empty();
        }

        CellBlock::new(
            rendered.into_lines(),
            Insets {
                right,
                left,
                ..resolved.margin
            },
        )
    }
}

impl Default for CellHost {
    fn default() -> Self {
        Self::new(CellStyleSheet::default())
    }
}

fn fit_horizontal_margins(width: u16, requested_left: u16, requested_right: u16) -> (u16, u16) {
    let available = u32::from(width.max(1).saturating_sub(1));
    let left = u32::from(requested_left);
    let right = u32::from(requested_right);
    let total = left.saturating_add(right);
    if total <= available {
        return (requested_left, requested_right);
    }
    if left == right {
        let fitted = left.min(available / 2);
        let fitted = u16::try_from(fitted).unwrap_or(u16::MAX);
        return (fitted, fitted);
    }
    if total == 0 {
        return (0, 0);
    }

    let mut fitted_left = available.saturating_mul(left) / total;
    let mut fitted_right = available.saturating_mul(right) / total;
    if fitted_left.saturating_add(fitted_right) < available {
        let left_remainder = available.saturating_mul(left) % total;
        let right_remainder = available.saturating_mul(right) % total;
        if left_remainder > right_remainder || (left_remainder == right_remainder && left > right) {
            fitted_left = fitted_left.saturating_add(1).min(left);
        } else {
            fitted_right = fitted_right.saturating_add(1).min(right);
        }
    }

    (
        u16::try_from(fitted_left).unwrap_or(u16::MAX),
        u16::try_from(fitted_right).unwrap_or(u16::MAX),
    )
}

#[cfg(test)]
mod tests {
    use ratatui::style::Style;

    use super::{
        CellBlock, CellContext, CellDescriptor, CellHost, CellPresenter, CellSpec,
        fit_horizontal_margins,
    };
    use crate::tui::Theme;
    use crate::tui::cells::style::{
        CellRole, CellStyle, CellStylePatch, CellStyleSheet, CellVisualState, ForegroundRole,
        InsetsPatch,
    };
    use crate::tui::components::style::Insets;
    use crate::tui::components::{Component, Element, RenderBlock, RenderContext};
    use crate::tui::render_output::RenderedLine;

    #[derive(Clone, Copy)]
    enum Echo {
        Width,
        Foreground,
        Empty,
    }

    struct EchoComponent {
        echo: Echo,
        foreground: ForegroundRole,
    }

    impl Component for EchoComponent {
        fn render(&self, _ctx: &RenderContext<'_>, width: u16) -> RenderBlock {
            let text = match self.echo {
                Echo::Width => width.to_string(),
                Echo::Foreground => format!("{:?}", self.foreground),
                Echo::Empty => return RenderBlock::empty(),
            };
            RenderBlock::from_lines(vec![RenderedLine::new(text, Style::default())])
        }
    }

    struct FakePresenter {
        descriptor: CellDescriptor,
        echo: Echo,
    }

    impl FakePresenter {
        fn new(role: CellRole, echo: Echo) -> Self {
            Self {
                descriptor: CellDescriptor {
                    role,
                    state: CellVisualState::Normal,
                    style_patch: CellStylePatch::default(),
                },
                echo,
            }
        }
    }

    impl CellPresenter for FakePresenter {
        fn role(&self) -> CellRole {
            self.descriptor.role
        }

        fn visual_state(&self) -> CellVisualState {
            self.descriptor.state
        }

        fn style_patch(&self) -> CellStylePatch {
            self.descriptor.style_patch
        }

        fn content<'a>(
            &'a self,
            _ctx: &'a CellContext<'a>,
            resolved_style: CellStyle,
        ) -> Element<'a> {
            Element::new(EchoComponent {
                echo: self.echo,
                foreground: resolved_style.foreground,
            })
        }
    }

    fn render(host: &CellHost, presenter: &FakePresenter, width: u16) -> CellBlock {
        let theme = Theme::default();
        let context = CellContext { theme: &theme };
        host.render(CellSpec::new(presenter), &context, width)
    }

    #[test]
    fn host_applies_defaults_when_presenter_returns_no_patch() {
        let host = CellHost::default();
        let block = render(
            &host,
            &FakePresenter::new(CellRole::Notice, Echo::Foreground),
            20,
        );

        assert_eq!(host.stylesheet_generation(), 1);
        assert_eq!(
            block.margin(),
            Insets {
                top: 0,
                right: 1,
                bottom: 0,
                left: 1,
            }
        );
        assert_eq!(block.lines()[0].text().trim_end(), "Normal");
    }

    #[test]
    fn host_exposes_each_distinct_stylesheet_generation() {
        let default = CellHost::default();
        let patched = CellHost::new(CellStyleSheet::default().with_global_patch(CellStylePatch {
            margin: InsetsPatch {
                top: Some(1),
                ..InsetsPatch::default()
            },
            ..CellStylePatch::default()
        }));

        assert_eq!(default.stylesheet_generation(), 1);
        assert_eq!(patched.stylesheet_generation(), 2);
    }

    #[test]
    fn host_applies_instance_patch_last() {
        let mut presenter = FakePresenter::new(CellRole::Notice, Echo::Foreground);
        presenter.descriptor.state = CellVisualState::Error;
        presenter.descriptor.style_patch = CellStylePatch {
            margin: InsetsPatch {
                left: Some(2),
                ..InsetsPatch::default()
            },
            foreground: Some(ForegroundRole::Approval),
            ..CellStylePatch::default()
        };

        let block = render(&CellHost::default(), &presenter, 20);

        assert_eq!(block.margin().left, 2);
        assert_eq!(block.margin().right, 1);
        assert_eq!(block.lines()[0].text().trim_end(), "Approval");
    }

    #[test]
    fn host_returns_no_padding_or_margins_for_empty_content() {
        let block = render(
            &CellHost::default(),
            &FakePresenter::new(CellRole::UserMessage, Echo::Empty),
            20,
        );

        assert!(block.is_empty());
        assert_eq!(block.margin(), Insets::default());
    }

    #[test]
    fn narrow_symmetric_margins_leave_at_least_one_content_column() {
        let host = CellHost::default();
        let presenter = FakePresenter::new(CellRole::Notice, Echo::Width);

        for (width, expected_margin, expected_content_width) in [
            (0, (0, 0), "1"),
            (1, (0, 0), "1"),
            (2, (0, 0), "2"),
            (3, (1, 1), "1"),
        ] {
            let block = render(&host, &presenter, width);
            assert_eq!((block.margin().left, block.margin().right), expected_margin);
            assert_eq!(block.lines()[0].text().trim_end(), expected_content_width);
        }
    }

    #[test]
    fn horizontal_margin_fitting_is_symmetric_and_proportional() {
        assert_eq!(fit_horizontal_margins(1, 1, 1), (0, 0));
        assert_eq!(fit_horizontal_margins(2, 1, 1), (0, 0));
        assert_eq!(fit_horizontal_margins(3, 1, 1), (1, 1));
        assert_eq!(fit_horizontal_margins(5, 3, 3), (2, 2));

        assert_eq!(fit_horizontal_margins(4, 3, 1), (2, 1));
        assert_eq!(fit_horizontal_margins(3, 5, 1), (2, 0));
        assert_eq!(fit_horizontal_margins(3, 1, 5), (0, 2));
    }
}
