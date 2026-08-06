use ratatui::style::Style;

use super::super::components::style::{BackgroundRole, Insets};
use super::super::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct InsetsPatch {
    pub(crate) top: Option<u16>,
    pub(crate) right: Option<u16>,
    pub(crate) bottom: Option<u16>,
    pub(crate) left: Option<u16>,
}

impl InsetsPatch {
    fn merge_into(self, insets: &mut Insets) {
        if let Some(top) = self.top {
            insets.top = top;
        }
        if let Some(right) = self.right {
            insets.right = right;
        }
        if let Some(bottom) = self.bottom {
            insets.bottom = bottom;
        }
        if let Some(left) = self.left {
            insets.left = left;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CellRole {
    UserMessage,
    AssistantMessage,
    Reasoning,
    Plan,
    Tool,
    Result,
    Sources,
    Diff,
    QueuedInput,
    Notice,
    Decision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CellVisualState {
    Normal,
    Pending,
    Running,
    Success,
    Warning,
    Error,
}

// Instance patches may select these roles even when built-in presets currently do not.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForegroundRole {
    Normal,
    Composer,
    User,
    Assistant,
    Thinking,
    Tool,
    Success,
    Warning,
    Error,
    Information,
    Approval,
    Muted,
}

impl ForegroundRole {
    pub(crate) const fn resolve(self, theme: &Theme) -> Style {
        match self {
            Self::Normal => theme.normal,
            Self::Composer => theme.composer,
            Self::User => theme.user,
            Self::Assistant => theme.assistant,
            Self::Thinking => theme.thinking,
            Self::Tool => theme.tool,
            Self::Success => theme.success,
            Self::Warning => theme.warning,
            Self::Error => theme.error,
            Self::Information => theme.information,
            Self::Approval => theme.approval,
            Self::Muted => theme.footer,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CellStyle {
    pub(crate) margin: Insets,
    pub(crate) padding: Insets,
    pub(crate) background: BackgroundRole,
    pub(crate) foreground: ForegroundRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct CellStylePatch {
    pub(crate) margin: InsetsPatch,
    pub(crate) padding: InsetsPatch,
    pub(crate) background: Option<BackgroundRole>,
    pub(crate) foreground: Option<ForegroundRole>,
}

impl CellStylePatch {
    fn apply_to(self, style: &mut CellStyle) {
        self.margin.merge_into(&mut style.margin);
        self.padding.merge_into(&mut style.padding);
        if let Some(background) = self.background {
            style.background = background;
        }
        if let Some(foreground) = self.foreground {
            style.foreground = foreground;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CellStyleSheet {
    generation: u64,
    global: CellStyle,
}

impl CellStyleSheet {
    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(test)]
    pub(crate) fn with_global_patch(mut self, patch: CellStylePatch) -> Self {
        patch.apply_to(&mut self.global);
        self.generation = self.generation.saturating_add(1);
        self
    }

    pub(crate) fn resolve(
        &self,
        role: CellRole,
        state: CellVisualState,
        instance: CellStylePatch,
    ) -> CellStyle {
        let mut resolved = self.global;
        role_patch(role).apply_to(&mut resolved);
        state_patch(role, state).apply_to(&mut resolved);
        instance.apply_to(&mut resolved);
        resolved
    }
}

impl Default for CellStyleSheet {
    fn default() -> Self {
        Self {
            generation: 1,
            global: CellStyle {
                margin: Insets {
                    top: 0,
                    right: 1,
                    bottom: 0,
                    left: 1,
                },
                padding: Insets::default(),
                background: BackgroundRole::Transparent,
                foreground: ForegroundRole::Normal,
            },
        }
    }
}

fn role_patch(role: CellRole) -> CellStylePatch {
    match role {
        CellRole::UserMessage => CellStylePatch {
            margin: InsetsPatch {
                right: Some(0),
                bottom: Some(1),
                left: Some(0),
                ..InsetsPatch::default()
            },
            padding: InsetsPatch {
                top: Some(1),
                bottom: Some(1),
                ..InsetsPatch::default()
            },
            background: Some(BackgroundRole::Composer),
            foreground: Some(ForegroundRole::Composer),
        },
        CellRole::AssistantMessage => CellStylePatch {
            margin: InsetsPatch {
                bottom: Some(1),
                ..InsetsPatch::default()
            },
            foreground: Some(ForegroundRole::Assistant),
            ..CellStylePatch::default()
        },
        CellRole::Reasoning => CellStylePatch {
            foreground: Some(ForegroundRole::Thinking),
            ..CellStylePatch::default()
        },
        CellRole::Plan | CellRole::Tool => CellStylePatch {
            foreground: Some(ForegroundRole::Tool),
            ..CellStylePatch::default()
        },
        CellRole::Sources => CellStylePatch {
            foreground: Some(ForegroundRole::Success),
            ..CellStylePatch::default()
        },
        CellRole::Diff => CellStylePatch {
            margin: InsetsPatch {
                right: Some(0),
                left: Some(0),
                ..InsetsPatch::default()
            },
            background: Some(BackgroundRole::Transparent),
            ..CellStylePatch::default()
        },
        CellRole::QueuedInput => CellStylePatch {
            foreground: Some(ForegroundRole::Muted),
            ..CellStylePatch::default()
        },
        CellRole::Result | CellRole::Notice | CellRole::Decision => CellStylePatch::default(),
    }
}

fn state_patch(role: CellRole, state: CellVisualState) -> CellStylePatch {
    let foreground = match (role, state) {
        (CellRole::Decision, CellVisualState::Pending) => ForegroundRole::Approval,
        (CellRole::Plan, CellVisualState::Pending) | (_, CellVisualState::Running) => {
            ForegroundRole::Tool
        }
        (_, CellVisualState::Normal) => return CellStylePatch::default(),
        (_, CellVisualState::Pending) => ForegroundRole::Muted,
        (_, CellVisualState::Success) => ForegroundRole::Success,
        (_, CellVisualState::Warning) => ForegroundRole::Warning,
        (_, CellVisualState::Error) => ForegroundRole::Error,
    };
    CellStylePatch {
        foreground: Some(foreground),
        ..CellStylePatch::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CellRole, CellStylePatch, CellStyleSheet, CellVisualState, ForegroundRole, InsetsPatch,
    };
    use crate::tui::components::style::BackgroundRole;

    #[test]
    fn style_resolution_is_global_then_role_then_state_then_instance() {
        let sheet = CellStyleSheet::default();

        let role = sheet.resolve(
            CellRole::UserMessage,
            CellVisualState::Normal,
            CellStylePatch::default(),
        );
        assert_eq!(role.margin.left, 0);
        assert_eq!(role.foreground, ForegroundRole::Composer);
        assert_eq!(role.background, BackgroundRole::Composer);

        let state = sheet.resolve(
            CellRole::UserMessage,
            CellVisualState::Error,
            CellStylePatch::default(),
        );
        assert_eq!(state.foreground, ForegroundRole::Error);

        let instance = sheet.resolve(
            CellRole::UserMessage,
            CellVisualState::Error,
            CellStylePatch {
                margin: InsetsPatch {
                    top: Some(4),
                    ..InsetsPatch::default()
                },
                background: Some(BackgroundRole::Warning),
                foreground: Some(ForegroundRole::Approval),
                ..CellStylePatch::default()
            },
        );
        assert_eq!(instance.margin.top, 4);
        assert_eq!(instance.margin.bottom, 1);
        assert_eq!(instance.background, BackgroundRole::Warning);
        assert_eq!(instance.foreground, ForegroundRole::Approval);
    }

    #[test]
    fn horizontal_role_overrides_keep_inheriting_global_vertical_margins() {
        let sheet = CellStyleSheet::default().with_global_patch(CellStylePatch {
            margin: InsetsPatch {
                top: Some(2),
                bottom: Some(3),
                ..InsetsPatch::default()
            },
            ..CellStylePatch::default()
        });

        let style = sheet.resolve(
            CellRole::Diff,
            CellVisualState::Normal,
            CellStylePatch::default(),
        );

        assert_eq!(style.margin.top, 2);
        assert_eq!(style.margin.right, 0);
        assert_eq!(style.margin.bottom, 3);
        assert_eq!(style.margin.left, 0);
        assert_eq!(sheet.generation(), 2);
    }

    #[test]
    fn instance_patch_can_override_one_edge_without_replacing_other_edges() {
        let sheet = CellStyleSheet::default().with_global_patch(CellStylePatch {
            padding: InsetsPatch {
                top: Some(1),
                right: Some(2),
                bottom: Some(3),
                left: Some(4),
            },
            ..CellStylePatch::default()
        });

        let style = sheet.resolve(
            CellRole::Notice,
            CellVisualState::Normal,
            CellStylePatch {
                padding: InsetsPatch {
                    left: Some(9),
                    ..InsetsPatch::default()
                },
                ..CellStylePatch::default()
            },
        );

        assert_eq!(style.padding.top, 1);
        assert_eq!(style.padding.right, 2);
        assert_eq!(style.padding.bottom, 3);
        assert_eq!(style.padding.left, 9);
    }

    #[test]
    fn production_presets_cover_every_role_and_visual_state() {
        let sheet = CellStyleSheet::default();
        let role_cases = [
            (CellRole::UserMessage, ForegroundRole::Composer),
            (CellRole::AssistantMessage, ForegroundRole::Assistant),
            (CellRole::Reasoning, ForegroundRole::Thinking),
            (CellRole::Plan, ForegroundRole::Tool),
            (CellRole::Tool, ForegroundRole::Tool),
            (CellRole::Result, ForegroundRole::Normal),
            (CellRole::Sources, ForegroundRole::Success),
            (CellRole::Diff, ForegroundRole::Normal),
            (CellRole::QueuedInput, ForegroundRole::Muted),
            (CellRole::Notice, ForegroundRole::Normal),
            (CellRole::Decision, ForegroundRole::Normal),
        ];
        for (role, expected) in role_cases {
            assert_eq!(
                sheet
                    .resolve(role, CellVisualState::Normal, CellStylePatch::default())
                    .foreground,
                expected
            );
        }

        let state_cases = [
            (CellVisualState::Normal, ForegroundRole::Normal),
            (CellVisualState::Pending, ForegroundRole::Muted),
            (CellVisualState::Running, ForegroundRole::Tool),
            (CellVisualState::Success, ForegroundRole::Success),
            (CellVisualState::Warning, ForegroundRole::Warning),
            (CellVisualState::Error, ForegroundRole::Error),
        ];
        for (state, expected) in state_cases {
            assert_eq!(
                sheet
                    .resolve(CellRole::Notice, state, CellStylePatch::default())
                    .foreground,
                expected
            );
        }
        assert_eq!(
            sheet
                .resolve(
                    CellRole::Decision,
                    CellVisualState::Pending,
                    CellStylePatch::default(),
                )
                .foreground,
            ForegroundRole::Approval
        );
        assert_eq!(
            sheet
                .resolve(
                    CellRole::Plan,
                    CellVisualState::Pending,
                    CellStylePatch::default(),
                )
                .foreground,
            ForegroundRole::Tool
        );

        for foreground in [ForegroundRole::User, ForegroundRole::Information] {
            assert_eq!(
                sheet
                    .resolve(
                        CellRole::Notice,
                        CellVisualState::Normal,
                        CellStylePatch {
                            foreground: Some(foreground),
                            ..CellStylePatch::default()
                        },
                    )
                    .foreground,
                foreground
            );
        }
        for background in [BackgroundRole::Success, BackgroundRole::Error] {
            assert_eq!(
                sheet
                    .resolve(
                        CellRole::Notice,
                        CellVisualState::Normal,
                        CellStylePatch {
                            background: Some(background),
                            ..CellStylePatch::default()
                        },
                    )
                    .background,
                background
            );
        }
    }
}
