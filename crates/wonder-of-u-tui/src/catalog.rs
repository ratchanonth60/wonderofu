//! Reusable catalog/list screen view models.
//!
//! These models capture the shared shape of list-driven screens such as agents,
//! MCP servers, plugins, skills, tasks, and settings/help style browsers while
//! staying renderer agnostic.

/// Semantic emphasis for badges and other status-like decorations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CatalogTone {
    /// Neutral content with no special emphasis.
    #[default]
    Neutral,
    /// Subdued informational content.
    Muted,
    /// Active but non-error status.
    Info,
    /// Positive status.
    Success,
    /// Attention-worthy status.
    Warning,
    /// Error or failure status.
    Danger,
    /// Primary accent content.
    Accent,
}

/// A short status label attached to a header, list item, or preview.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusBadge {
    pub label: String,
    pub tone: CatalogTone,
}

impl StatusBadge {
    #[must_use]
    pub fn new(label: impl Into<String>, tone: CatalogTone) -> Self {
        Self {
            label: label.into(),
            tone,
        }
    }

    #[must_use]
    pub fn connected() -> Self {
        Self::new("connected", CatalogTone::Success)
    }

    #[must_use]
    pub fn pending() -> Self {
        Self::new("pending", CatalogTone::Info)
    }

    #[must_use]
    pub fn disabled() -> Self {
        Self::new("disabled", CatalogTone::Muted)
    }

    #[must_use]
    pub fn needs_auth() -> Self {
        Self::new("needs auth", CatalogTone::Warning)
    }

    #[must_use]
    pub fn failed() -> Self {
        Self::new("failed", CatalogTone::Danger)
    }
}

/// An action shown in a screen header, panel, or empty state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CatalogAction {
    pub label: String,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub primary: bool,
}

impl CatalogAction {
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            shortcut: None,
            enabled: true,
            primary: false,
        }
    }

    #[must_use]
    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    #[must_use]
    pub const fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    #[must_use]
    pub const fn primary(mut self) -> Self {
        self.primary = true;
        self
    }
}

/// Shared header metadata for a catalog-style screen.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CatalogHeader {
    pub title: String,
    pub subtitle: Option<String>,
    pub badges: Vec<StatusBadge>,
    pub actions: Vec<CatalogAction>,
}

impl CatalogHeader {
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            badges: Vec::new(),
            actions: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }
}

/// Shared panel metadata for list and detail panes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PanelDescriptor {
    pub title: String,
    pub subtitle: Option<String>,
    pub footer: Option<String>,
    pub actions: Vec<CatalogAction>,
}

impl PanelDescriptor {
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            footer: None,
            actions: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    #[must_use]
    pub fn with_footer(mut self, footer: impl Into<String>) -> Self {
        self.footer = Some(footer.into());
        self
    }
}

/// Content to show when a catalog has no visible items.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EmptyState {
    pub title: String,
    pub body: Vec<String>,
    pub actions: Vec<CatalogAction>,
}

impl EmptyState {
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: Vec::new(),
            actions: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_body_line(mut self, line: impl Into<String>) -> Self {
        self.body.push(line.into());
        self
    }
}

/// Detail data shown for the currently selected item.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DetailPreview {
    pub panel: PanelDescriptor,
    pub badges: Vec<StatusBadge>,
    pub lines: Vec<String>,
}

impl DetailPreview {
    #[must_use]
    pub fn new(panel: PanelDescriptor) -> Self {
        Self {
            panel,
            badges: Vec::new(),
            lines: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_line(mut self, line: impl Into<String>) -> Self {
        self.lines.push(line.into());
        self
    }
}

/// A selectable list entry inside a catalog.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CatalogEntry {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub badges: Vec<StatusBadge>,
    pub disabled_reason: Option<String>,
    pub preview: Option<DetailPreview>,
    pub keywords: Vec<String>,
    pub actions: Vec<CatalogAction>,
}

impl CatalogEntry {
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            badges: Vec::new(),
            disabled_reason: None,
            preview: None,
            keywords: Vec::new(),
            actions: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub fn with_keyword(mut self, keyword: impl Into<String>) -> Self {
        self.keywords.push(keyword.into());
        self
    }

    #[must_use]
    pub fn with_badge(mut self, badge: StatusBadge) -> Self {
        self.badges.push(badge);
        self
    }

    #[must_use]
    pub fn with_preview(mut self, preview: DetailPreview) -> Self {
        self.preview = Some(preview);
        self
    }

    #[must_use]
    pub fn disabled(mut self, reason: impl Into<String>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }

    #[must_use]
    pub fn is_disabled(&self) -> bool {
        self.disabled_reason.is_some()
    }

    fn matches_query(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }

        matches_query(&self.id, query)
            || matches_query(&self.label, query)
            || self
                .description
                .as_deref()
                .is_some_and(|description| matches_query(description, query))
            || self
                .disabled_reason
                .as_deref()
                .is_some_and(|reason| matches_query(reason, query))
            || self
                .badges
                .iter()
                .any(|badge| matches_query(&badge.label, query))
            || self
                .keywords
                .iter()
                .any(|keyword| matches_query(keyword, query))
            || self.preview.as_ref().is_some_and(|preview| {
                matches_query(&preview.panel.title, query)
                    || preview
                        .panel
                        .subtitle
                        .as_deref()
                        .is_some_and(|subtitle| matches_query(subtitle, query))
                    || preview.lines.iter().any(|line| matches_query(line, query))
                    || preview
                        .badges
                        .iter()
                        .any(|badge| matches_query(&badge.label, query))
            })
    }
}

/// A logical list section within a catalog.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CatalogSection {
    pub panel: PanelDescriptor,
    pub entries: Vec<CatalogEntry>,
}

impl CatalogSection {
    #[must_use]
    pub fn new(panel: PanelDescriptor, entries: Vec<CatalogEntry>) -> Self {
        Self { panel, entries }
    }
}

/// A filtered section view with entry selection state attached.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogSectionView<'a> {
    pub panel: &'a PanelDescriptor,
    pub entries: Vec<CatalogEntryView<'a>>,
}

/// A filtered list entry view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogEntryView<'a> {
    pub entry: &'a CatalogEntry,
    pub visible_index: usize,
    pub selected: bool,
}

/// The currently selected entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogSelection<'a> {
    pub entry: &'a CatalogEntry,
    pub visible_index: usize,
}

/// The list slice a renderer should keep in view.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScrollWindow {
    pub offset: usize,
    pub visible_len: usize,
    pub total_len: usize,
}

impl ScrollWindow {
    #[must_use]
    pub fn for_selection(
        total_len: usize,
        selected_index: Option<usize>,
        viewport_len: usize,
    ) -> Self {
        if total_len == 0 || viewport_len == 0 {
            return Self {
                offset: 0,
                visible_len: 0,
                total_len,
            };
        }

        let visible_len = total_len.min(viewport_len);
        let selected_index = selected_index.unwrap_or(0).min(total_len.saturating_sub(1));
        let max_offset = total_len.saturating_sub(visible_len);
        let preferred_offset = selected_index.saturating_sub(visible_len / 2);

        Self {
            offset: preferred_offset.min(max_offset),
            visible_len,
            total_len,
        }
    }

    #[must_use]
    pub fn end(self) -> usize {
        self.offset.saturating_add(self.visible_len)
    }
}

/// Shared state for catalog/list screens with selection, filtering, and preview support.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogModel {
    pub header: CatalogHeader,
    pub list_panel: PanelDescriptor,
    pub detail_panel: PanelDescriptor,
    pub empty_state: EmptyState,
    pub filtered_empty_state: EmptyState,
    pub sections: Vec<CatalogSection>,
    filter_query: String,
    selected_index: usize,
}

impl CatalogModel {
    #[must_use]
    pub fn new(
        header: CatalogHeader,
        list_panel: PanelDescriptor,
        detail_panel: PanelDescriptor,
        empty_state: EmptyState,
        filtered_empty_state: EmptyState,
        sections: Vec<CatalogSection>,
    ) -> Self {
        let mut model = Self {
            header,
            list_panel,
            detail_panel,
            empty_state,
            filtered_empty_state,
            sections,
            filter_query: String::new(),
            selected_index: 0,
        };
        model.clamp_selected_index();
        model
    }

    #[must_use]
    pub fn filter_query(&self) -> &str {
        &self.filter_query
    }

    pub fn set_filter_query(&mut self, query: impl Into<String>) {
        self.filter_query = query.into();
        self.clamp_selected_index();
    }

    pub fn set_selected_index(&mut self, selected_index: usize) {
        self.selected_index = selected_index;
        self.clamp_selected_index();
    }

    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        let visible_count = self.visible_count();
        (visible_count > 0).then(|| self.selected_index.min(visible_count - 1))
    }

    #[must_use]
    pub fn visible_count(&self) -> usize {
        self.visible_entries().len()
    }

    #[must_use]
    pub fn has_filter(&self) -> bool {
        !self.normalized_query().is_empty()
    }

    #[must_use]
    pub fn active_empty_state(&self) -> Option<&EmptyState> {
        if self.visible_count() > 0 {
            None
        } else if self.has_filter() {
            Some(&self.filtered_empty_state)
        } else {
            Some(&self.empty_state)
        }
    }

    #[must_use]
    pub fn selected_entry(&self) -> Option<&CatalogEntry> {
        self.selected_entry_with_index()
            .map(|selection| selection.entry)
    }

    #[must_use]
    pub fn selection(&self) -> Option<CatalogSelection<'_>> {
        self.selected_entry_with_index()
    }

    #[must_use]
    pub fn detail_preview(&self) -> Option<&DetailPreview> {
        self.selected_entry()?.preview.as_ref()
    }

    #[must_use]
    pub fn scroll_window(&self, viewport_len: usize) -> ScrollWindow {
        ScrollWindow::for_selection(self.visible_count(), self.selected_index(), viewport_len)
    }

    #[must_use]
    pub fn visible_sections(&self) -> Vec<CatalogSectionView<'_>> {
        let mut visible_index = 0;
        let query = self.normalized_query();
        let selected_index = self.selected_index();
        let mut sections = Vec::new();

        for section in &self.sections {
            let mut entries = Vec::new();

            for entry in &section.entries {
                if !entry.matches_query(&query) {
                    continue;
                }

                entries.push(CatalogEntryView {
                    entry,
                    visible_index,
                    selected: selected_index == Some(visible_index),
                });
                visible_index += 1;
            }

            if !entries.is_empty() {
                sections.push(CatalogSectionView {
                    panel: &section.panel,
                    entries,
                });
            }
        }

        sections
    }

    #[must_use]
    pub fn visible_entries(&self) -> Vec<&CatalogEntry> {
        let query = self.normalized_query();
        let mut entries = Vec::new();

        for section in &self.sections {
            for entry in &section.entries {
                if entry.matches_query(&query) {
                    entries.push(entry);
                }
            }
        }

        entries
    }

    fn normalized_query(&self) -> String {
        self.filter_query.trim().to_lowercase()
    }

    fn clamp_selected_index(&mut self) {
        let visible_count = self.visible_count();
        if visible_count == 0 {
            self.selected_index = 0;
        } else {
            self.selected_index = self.selected_index.min(visible_count - 1);
        }
    }

    fn selected_entry_with_index(&self) -> Option<CatalogSelection<'_>> {
        let selected_index = self.selected_index()?;
        self.visible_entries()
            .into_iter()
            .nth(selected_index)
            .map(|entry| CatalogSelection {
                entry,
                visible_index: selected_index,
            })
    }
}

fn matches_query(haystack: &str, query: &str) -> bool {
    haystack.to_lowercase().contains(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_model() -> CatalogModel {
        CatalogModel::new(
            CatalogHeader::new("Catalog").with_subtitle("Reusable screens"),
            PanelDescriptor::new("Items").with_subtitle("Visible entries"),
            PanelDescriptor::new("Details").with_subtitle("Current selection"),
            EmptyState::new("Nothing here").with_body_line("Add an item to get started."),
            EmptyState::new("No matches").with_body_line("Try a different filter."),
            vec![
                CatalogSection::new(
                    PanelDescriptor::new("Agents"),
                    vec![
                        CatalogEntry::new("agent-alpha", "Alpha Agent")
                            .with_description("General automation")
                            .with_keyword("automation")
                            .with_preview(
                                DetailPreview::new(
                                    PanelDescriptor::new("Alpha Agent")
                                        .with_subtitle("General automation"),
                                )
                                .with_line("Coordinates work across tools.")
                                .with_line("Enabled for project use.")
                                .with_line("Last updated recently."),
                            ),
                        CatalogEntry::new("agent-built-in", "Built-in Agent")
                            .with_badge(StatusBadge::disabled())
                            .disabled("Built-in agents cannot be edited"),
                    ],
                ),
                CatalogSection::new(
                    PanelDescriptor::new("Plugins"),
                    vec![
                        CatalogEntry::new("plugin-lint", "Lint Plugin")
                            .with_description("Static analysis")
                            .with_keyword("plugin")
                            .with_badge(StatusBadge::connected()),
                        CatalogEntry::new("plugin-auth", "Auth Plugin")
                            .with_badge(StatusBadge::needs_auth())
                            .with_preview(
                                DetailPreview::new(
                                    PanelDescriptor::new("Auth Plugin")
                                        .with_subtitle("Authentication required"),
                                )
                                .with_line("Reconnect the backing service.")
                                .with_line("Review stored credentials."),
                            ),
                    ],
                ),
            ],
        )
    }

    #[test]
    fn empty_states_switch_between_base_and_filtered_messages() {
        let mut model = CatalogModel::new(
            CatalogHeader::new("Skills"),
            PanelDescriptor::new("Skills"),
            PanelDescriptor::new("Preview"),
            EmptyState::new("No skills found")
                .with_body_line("Create skills in .claude/skills or ~/.claude/skills."),
            EmptyState::new("No matching skills").with_body_line("Clear the search and try again."),
            Vec::new(),
        );

        assert_eq!(
            model.active_empty_state().map(|state| state.title.as_str()),
            Some("No skills found")
        );

        model.set_filter_query("mcp");

        assert_eq!(
            model.active_empty_state().map(|state| state.title.as_str()),
            Some("No matching skills")
        );
        assert_eq!(model.selected_index(), None);
    }

    #[test]
    fn filtered_lists_only_keep_matching_sections_and_entries() {
        let mut model = sample_model();

        model.set_filter_query("plugin");

        let sections = model.visible_sections();

        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].panel.title, "Plugins");
        assert_eq!(sections[0].entries.len(), 2);
        assert_eq!(sections[0].entries[0].entry.label, "Lint Plugin");
        assert!(sections[0].entries[0].selected);
    }

    #[test]
    fn disabled_entries_preserve_reason_and_selection() {
        let mut model = sample_model();
        model.set_selected_index(1);

        let selected = model.selected_entry().expect("selected entry");

        assert_eq!(selected.label, "Built-in Agent");
        assert!(selected.is_disabled());
        assert_eq!(
            selected.disabled_reason.as_deref(),
            Some("Built-in agents cannot be edited")
        );
    }

    #[test]
    fn detail_preview_follows_current_selection() {
        let mut model = sample_model();
        model.set_selected_index(3);

        let preview = model.detail_preview().expect("detail preview");

        assert_eq!(preview.panel.title, "Auth Plugin");
        assert_eq!(
            preview.panel.subtitle.as_deref(),
            Some("Authentication required")
        );
        assert_eq!(
            preview.lines,
            vec![
                "Reconnect the backing service.".to_string(),
                "Review stored credentials.".to_string(),
            ]
        );
    }

    #[test]
    fn selected_index_clamps_after_filtering() {
        let mut model = sample_model();
        model.set_selected_index(3);

        model.set_filter_query("alpha");

        assert_eq!(model.visible_count(), 1);
        assert_eq!(model.selected_index(), Some(0));
        assert_eq!(
            model.selected_entry().map(|entry| entry.label.as_str()),
            Some("Alpha Agent")
        );
    }

    #[test]
    fn status_badge_helpers_use_expected_labels_and_tones() {
        assert_eq!(StatusBadge::connected().tone, CatalogTone::Success);
        assert_eq!(
            StatusBadge::pending(),
            StatusBadge::new("pending", CatalogTone::Info)
        );
        assert_eq!(
            StatusBadge::disabled(),
            StatusBadge::new("disabled", CatalogTone::Muted)
        );
        assert_eq!(StatusBadge::needs_auth().tone, CatalogTone::Warning);
        assert_eq!(StatusBadge::failed().tone, CatalogTone::Danger);
    }

    #[test]
    fn scroll_window_centers_on_selected_item_when_possible() {
        let mut model = sample_model();
        model.set_selected_index(3);

        let window = model.scroll_window(2);

        assert_eq!(
            window,
            ScrollWindow {
                offset: 2,
                visible_len: 2,
                total_len: 4,
            }
        );
        assert_eq!(window.end(), 4);
    }
}
