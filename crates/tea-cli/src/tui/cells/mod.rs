mod decision;
mod diff;
mod host;
mod lifecycle;
mod list;
mod message;
mod notice;
mod plan;
mod queued_input;
mod reasoning;
mod result;
mod sources;
mod style;

pub(crate) use host::{CellBlock, CellContext, CellHost, CellPresenter, CellSpec};
pub(crate) use list::CellList;
#[cfg(test)]
pub(crate) use plan::MAX_VISIBLE_PLAN_BODY_ROWS;
#[cfg(test)]
pub(crate) use style::{CellStylePatch, CellStyleSheet, InsetsPatch};

use super::presentation::CellContent;

impl CellContent {
    pub(crate) fn presenter(&self) -> &dyn CellPresenter {
        match self {
            Self::Message(cell) => cell,
            Self::Reasoning(cell) => cell,
            Self::Plan(cell) => cell,
            Self::Lifecycle(cell) => cell,
            Self::Result(cell) => cell,
            Self::Sources(cell) => cell,
            Self::Diff(cell) => cell,
            Self::QueuedInput(cell) => cell,
            Self::Notice(cell) => cell,
            Self::Decision(cell) => cell,
        }
    }

    pub(crate) fn spec(&self) -> CellSpec<'_> {
        CellSpec::new(self.presenter())
    }
}
