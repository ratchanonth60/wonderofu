//! Tabs view model — mirrors `claude-leak/components/design-system/Tabs.tsx`.

/// A single tab descriptor.
#[derive(Clone, Debug)]
pub struct TabEntry {
    /// Unique identifier (also used as display label if no `label`).
    pub id: String,
    /// Display label. Falls back to `id` when absent.
    pub label: Option<String>,
    /// Whether this tab is currently selected.
    pub selected: bool,
    /// Whether this tab is disabled (non-interactive).
    pub disabled: bool,
}

impl TabEntry {
    /// Creates a new value
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: None,
            selected: false,
            disabled: false,
        }
    }
    /// Handles label
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
    /// Handles selected
    #[must_use]
    pub fn selected(mut self) -> Self {
        self.selected = true;
        self
    }
    /// Handles disabled
    #[must_use]
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    /// Returns the text to display in the tab bar.
    #[must_use]
    pub fn display_label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.id)
    }
}

/// State for a tab-bar widget.
#[derive(Clone, Debug, Default)]
pub struct TabsView {
    /// Stores the tabs
    pub tabs: Vec<TabEntry>,
    /// Optional header title shown left of the tab strip.
    pub title: Option<String>,
    /// Whether to stretch tabs to fill full terminal width.
    pub use_full_width: bool,
    /// Whether tab navigation is keyboard-disabled (child owns arrow keys).
    pub disable_navigation: bool,
    /// Whether the tab header row has focus (highlighted).
    pub header_focused: bool,
    /// Theme color key.
    pub color: Option<String>,
}

impl TabsView {
    /// Creates a new value
    #[must_use]
    pub fn new(tabs: impl IntoIterator<Item = TabEntry>) -> Self {
        Self {
            tabs: tabs.into_iter().collect(),
            header_focused: true,
            ..Default::default()
        }
    }

    /// Returns the index of the currently selected tab, if any.
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        self.tabs.iter().position(|t| t.selected)
    }

    /// Returns the id of the currently selected tab, if any.
    #[must_use]
    pub fn selected_id(&self) -> Option<&str> {
        self.tabs.iter().find(|t| t.selected).map(|t| t.id.as_str())
    }

    /// Select a tab by id, deselecting all others. Returns `true` if changed.
    pub fn select(&mut self, id: &str) -> bool {
        let prev = self.selected_id().map(|s| s.to_owned());
        for tab in &mut self.tabs {
            tab.selected = tab.id == id;
        }
        self.selected_id().map(|s| s.to_owned()) != prev
    }

    /// Move selection to the next enabled tab. Wraps around.
    pub fn select_next(&mut self) -> bool {
        let count = self.tabs.len();
        if count == 0 {
            return false;
        }
        let current = self.selected_index().unwrap_or(0);
        for offset in 1..=count {
            let idx = (current + offset) % count;
            if !self.tabs[idx].disabled {
                let id = self.tabs[idx].id.clone();
                return self.select(&id);
            }
        }
        false
    }

    /// Move selection to the previous enabled tab. Wraps around.
    pub fn select_prev(&mut self) -> bool {
        let count = self.tabs.len();
        if count == 0 {
            return false;
        }
        let current = self.selected_index().unwrap_or(0);
        for offset in 1..=count {
            let idx = (current + count - offset) % count;
            if !self.tabs[idx].disabled {
                let id = self.tabs[idx].id.clone();
                return self.select(&id);
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_by_id() {
        let mut tabs = TabsView::new(vec![
            TabEntry::new("a"),
            TabEntry::new("b").selected(),
            TabEntry::new("c"),
        ]);
        assert_eq!(tabs.selected_id(), Some("b"));
        tabs.select("c");
        assert_eq!(tabs.selected_id(), Some("c"));
        assert!(!tabs.tabs[1].selected);
    }

    #[test]
    fn next_wraps() {
        let mut tabs = TabsView::new(vec![
            TabEntry::new("a"),
            TabEntry::new("b"),
            TabEntry::new("c").selected(),
        ]);
        tabs.select_next();
        assert_eq!(tabs.selected_id(), Some("a"));
    }

    #[test]
    fn skips_disabled() {
        let mut tabs = TabsView::new(vec![
            TabEntry::new("a").selected(),
            TabEntry::new("b").disabled(),
            TabEntry::new("c"),
        ]);
        tabs.select_next();
        assert_eq!(tabs.selected_id(), Some("c"));
    }
}
