//! Task, tool-loader, and auxiliary utility views inspired by the remaining TUI parity set.

use crate::{SpinnerMode, SpinnerView};

/// High-level task state shown in task lists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskStatusView {
    /// Represents pending
    Pending,
    /// Represents running
    Running,
    /// Represents completed
    Completed,
    /// Represents blocked
    Blocked,
    /// Represents failed
    Failed,
}

impl TaskStatusView {
    /// Constant fn
    #[must_use]
    pub const fn icon(self) -> char {
        match self {
            Self::Pending => '•',
            Self::Running => '◐',
            Self::Completed => '✓',
            Self::Blocked => '⏸',
            Self::Failed => '✗',
        }
    }
}

/// A single task row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskEntryView {
    /// Stores the title
    pub title: String,
    /// Stores the detail
    pub detail: Option<String>,
    /// Stores the status
    pub status: TaskStatusView,
    /// Stores the active
    pub active: bool,
}

impl TaskEntryView {
    /// Creates a new value
    #[must_use]
    pub fn new(title: impl Into<String>, status: TaskStatusView) -> Self {
        Self {
            title: title.into(),
            detail: None,
            status,
            active: false,
        }
    }
    /// Handles detail
    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
}

/// Task list summary.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TaskListView {
    /// Stores the tasks
    pub tasks: Vec<TaskEntryView>,
}

impl TaskListView {
    /// Renders lines
    #[must_use]
    pub fn render_lines(&self) -> Vec<String> {
        self.tasks
            .iter()
            .flat_map(|task| {
                let mut lines = vec![format!(
                    "{} {} {}",
                    if task.active { '❯' } else { ' ' },
                    task.status.icon(),
                    task.title
                )];
                if let Some(detail) = &task.detail {
                    lines.push(format!("    {detail}"));
                }
                lines
            })
            .collect()
    }
}

/// Loading indicator for a tool invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolUseLoaderView {
    /// Stores the spinner
    pub spinner: SpinnerView,
    /// Stores the label
    pub label: String,
}

impl ToolUseLoaderView {
    /// Creates a new value
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            spinner: SpinnerView::new(SpinnerMode::ToolUse, "using tool"),
            label: label.into(),
        }
    }
    /// Renders line
    #[must_use]
    pub fn render_line(&self) -> String {
        format!("{} — {}", self.spinner.render_line(), self.label)
    }
}

/// Saved teleport/session entries shown in stash views.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeleportStashEntryView {
    /// Stores the session
    pub session: String,
    /// Stores the target
    pub target: String,
    /// Stores the summary
    pub summary: Option<String>,
}

impl TeleportStashEntryView {
    /// Creates a new value
    #[must_use]
    pub fn new(session: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            session: session.into(),
            target: target.into(),
            summary: None,
        }
    }
    /// Handles summary
    #[must_use]
    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }
}

/// Teleport stash overview.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TeleportStashView {
    /// Stores the entries
    pub entries: Vec<TeleportStashEntryView>,
}

impl TeleportStashView {
    /// Renders lines
    #[must_use]
    pub fn render_lines(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|entry| match &entry.summary {
                Some(summary) => format!("{} → {} ({summary})", entry.session, entry.target),
                None => format!("{} → {}", entry.session, entry.target),
            })
            .collect()
    }
}

/// Caches the last visible content while off-screen.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OffscreenFreezeView {
    /// Stores the cached lines
    pub cached_lines: Vec<String>,
    /// Stores the frozen
    pub frozen: bool,
}

impl OffscreenFreezeView {
    /// Handles update
    pub fn update<I>(&mut self, lines: I)
    where
        I: IntoIterator<Item = String>,
    {
        if !self.frozen {
            self.cached_lines = lines.into_iter().collect();
        }
    }
    /// Handles render
    #[must_use]
    pub fn render<'a>(&'a self, live_lines: &'a [String]) -> &'a [String] {
        if self.frozen {
            &self.cached_lines
        } else {
            live_lines
        }
    }
}

/// Compact fast-mode badge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FastIconView {
    /// Stores the enabled
    pub enabled: bool,
}

impl FastIconView {
    /// Constant fn
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        if self.enabled { "⚡" } else { "" }
    }
}

/// Hook mode label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookModeView {
    /// Represents disabled
    Disabled,
    /// Represents enabled
    Enabled,
    /// Represents passthrough
    Passthrough,
}

impl HookModeView {
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "hooks disabled",
            Self::Enabled => "hooks enabled",
            Self::Passthrough => "hooks passthrough",
        }
    }
}

/// A single MCP tool row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpToolEntryView {
    /// Stores the name
    pub name: String,
    /// Stores the server
    pub server: String,
}

impl McpToolEntryView {
    /// Creates a new value
    #[must_use]
    pub fn new(name: impl Into<String>, server: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            server: server.into(),
        }
    }
}

/// MCP tool list summary.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct McpToolListView {
    /// Stores the tools
    pub tools: Vec<McpToolEntryView>,
}

impl McpToolListView {
    /// Renders lines
    #[must_use]
    pub fn render_lines(&self) -> Vec<String> {
        self.tools
            .iter()
            .map(|tool| format!("• {} ({})", tool.name, tool.server))
            .collect()
    }
}

/// Referral/pass summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PassesView {
    /// Stores the available
    pub available: usize,
    /// Stores the note
    pub note: Option<String>,
}

impl PassesView {
    /// Renders lines
    #[must_use]
    pub fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} pass{} available",
            self.available,
            if self.available == 1 { "" } else { "es" }
        )];
        if let Some(note) = &self.note {
            lines.push(note.clone());
        }
        lines
    }
}

/// Agent type wizard option.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentTypeOptionView {
    /// Stores the label
    pub label: String,
    /// Stores the selected
    pub selected: bool,
}

impl AgentTypeOptionView {
    /// Creates a new value
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            selected: false,
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}

/// Agent type selection step.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentTypeStepView {
    /// Stores the options
    pub options: Vec<AgentTypeOptionView>,
}

impl AgentTypeStepView {
    /// Renders lines
    #[must_use]
    pub fn render_lines(&self) -> Vec<String> {
        self.options
            .iter()
            .map(|option| {
                format!(
                    "{} {}",
                    if option.selected { '◉' } else { '○' },
                    option.label
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_list_renders_status_and_details() {
        let lines = TaskListView {
            tasks: vec![
                TaskEntryView::new("Collect context", TaskStatusView::Running)
                    .detail("Searching files")
                    .active(true),
            ],
        }
        .render_lines();

        assert_eq!(lines[0], "❯ ◐ Collect context");
        assert_eq!(lines[1], "    Searching files");
    }

    #[test]
    fn offscreen_freeze_holds_cached_content() {
        let mut freeze = OffscreenFreezeView::default();
        freeze.update(vec!["a".into(), "b".into()]);
        freeze.frozen = true;
        let live = vec!["c".into()];

        assert_eq!(freeze.render(&live), &["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn agent_type_step_marks_selected_option() {
        let lines = AgentTypeStepView {
            options: vec![
                AgentTypeOptionView::new("Research").selected(true),
                AgentTypeOptionView::new("Editor"),
            ],
        }
        .render_lines();

        assert_eq!(lines, vec!["◉ Research", "○ Editor"]);
    }
}
