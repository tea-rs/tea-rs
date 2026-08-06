#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Insets {
    pub(crate) top: u16,
    pub(crate) right: u16,
    pub(crate) bottom: u16,
    pub(crate) left: u16,
}

// Instance patches may select these backgrounds even when built-in presets currently do not.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum BackgroundRole {
    #[default]
    Transparent,
    Composer,
    Success,
    Warning,
    Error,
}
