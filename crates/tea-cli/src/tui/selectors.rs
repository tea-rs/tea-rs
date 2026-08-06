use tea_protocol::{BranchId, ModelRef, ReasoningEffort, SessionId};

const MAX_SELECTOR_ITEMS: usize = 512;
const MAX_SELECTOR_QUERY_BYTES: usize = 256;

/// Typed value returned by a local selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorValue {
    /// Durable session selection.
    Session(SessionId),
    /// Provider model selection.
    Model(ModelRef),
    /// Provider-neutral reasoning effort selection.
    Reasoning(ReasoningEffort),
    /// Durable branch selection.
    Branch(BranchId),
}

/// One bounded selector row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorItem {
    label: String,
    value: SelectorValue,
}

impl SelectorItem {
    /// Creates one selector row.
    #[must_use]
    pub fn new(label: impl Into<String>, value: SelectorValue) -> Self {
        Self {
            label: label.into(),
            value,
        }
    }

    /// Returns the display label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the typed value.
    #[must_use]
    pub const fn value(&self) -> &SelectorValue {
        &self.value
    }
}

/// Invalid selector construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SelectorError {
    /// The title, row label, or collection bound is invalid.
    #[error("selector is invalid")]
    Invalid,
}

/// Filterable deterministic local selection view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    title: String,
    items: Vec<SelectorItem>,
    visible: Vec<usize>,
    query: String,
    selected: usize,
}

impl Selector {
    /// Creates a selector with stable source order.
    ///
    /// # Errors
    ///
    /// Rejects empty/oversized/control labels and oversized item collections.
    pub fn new<I>(title: impl Into<String>, items: I) -> Result<Self, SelectorError>
    where
        I: IntoIterator<Item = SelectorItem>,
    {
        let title = title.into();
        let items = items.into_iter().collect::<Vec<_>>();
        if !valid_text(&title)
            || items.len() > MAX_SELECTOR_ITEMS
            || items.iter().any(|item| !valid_text(&item.label))
        {
            return Err(SelectorError::Invalid);
        }
        let visible = (0..items.len()).collect();
        Ok(Self {
            title,
            items,
            visible,
            query: String::new(),
            selected: 0,
        })
    }

    /// Returns the selector title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the current filter query.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Replaces the case-insensitive query with a bounded value.
    pub fn set_query(&mut self, query: &str) {
        let mut boundary = query.len().min(MAX_SELECTOR_QUERY_BYTES);
        while !query.is_char_boundary(boundary) {
            boundary -= 1;
        }
        query[..boundary].clone_into(&mut self.query);
        let query = self.query.to_lowercase();
        self.visible = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| item.label.to_lowercase().contains(&query).then_some(index))
            .collect();
        self.selected = 0;
    }

    /// Returns currently visible rows.
    #[must_use]
    pub fn visible_items(&self) -> Vec<&SelectorItem> {
        self.visible
            .iter()
            .map(|index| &self.items[*index])
            .collect()
    }

    /// Moves to the next visible row with wraparound.
    pub fn move_next(&mut self) {
        if !self.visible.is_empty() {
            self.selected = (self.selected + 1) % self.visible.len();
        }
    }

    /// Moves to the previous visible row with wraparound.
    pub fn move_previous(&mut self) {
        if !self.visible.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.visible.len() - 1);
        }
    }

    /// Selects one currently visible typed value when present.
    pub fn select_value(&mut self, value: &SelectorValue) {
        if let Some(index) = self
            .visible
            .iter()
            .position(|index| self.items[*index].value == *value)
        {
            self.selected = index;
        }
    }

    /// Returns the selected row label.
    #[must_use]
    pub fn selected_label(&self) -> Option<&str> {
        self.visible
            .get(self.selected)
            .map(|index| self.items[*index].label.as_str())
    }

    /// Returns a clone of the selected typed value.
    #[must_use]
    pub fn accept(&self) -> Option<SelectorValue> {
        self.visible
            .get(self.selected)
            .map(|index| self.items[*index].value.clone())
    }
}

fn valid_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}
