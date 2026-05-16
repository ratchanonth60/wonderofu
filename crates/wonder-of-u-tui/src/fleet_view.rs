//! Pure view-model for fleet run status rendering.
//!
//! This module is **completely dependency-free** with respect to storage,
//! CLI, and async runtimes.  It holds only plain data structs and produces
//! plain `Vec<String>` lines that a shell renderer, test harness, or CLI
//! printer can consume without further transformation.
//!
//! # Design
//!
//! ```text
//! FleetRunView
//!   ├── fleet_id     : String
//!   ├── description  : String
//!   ├── status       : FleetStatusView (pending|running|completed|failed|cancelled)
//!   └── members      : Vec<FleetMemberEntryView>
//!         ├── task_id     : String
//!         ├── name        : Option<String>
//!         ├── request_id  : Option<String>
//!         ├── status      : FleetMemberStatusView
//!         └── excerpt     : Option<String>
//! ```
//!
//! Call [`FleetRunView::render_lines`] to get a compact, human-readable
//! text representation suitable for terminal output or snapshot tests.
//!
//! # Examples
//!
//! ```
//! use wonder_of_u_tui::fleet_view::{
//!     FleetMemberEntryView, FleetMemberStatusView, FleetRunView, FleetStatusView,
//! };
//!
//! let view = FleetRunView {
//!     fleet_id: "fleet-001".into(),
//!     description: "my fleet".into(),
//!     status: FleetStatusView::Running,
//!     members: vec![
//!         FleetMemberEntryView::new("task-001", FleetMemberStatusView::Running)
//!             .name("worker-1"),
//!     ],
//! };
//! let lines = view.render_lines();
//! assert!(lines.iter().any(|l| l.contains("◐")));
//! ```

// ── Status enums ─────────────────────────────────────────────────────────────

/// Overall status of a fleet run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FleetStatusView {
    /// No members dispatched yet.
    Pending,
    /// At least one member is actively running.
    Running,
    /// All members completed successfully.
    Completed,
    /// All members terminal, at least one failed.
    Failed,
    /// All members were cancelled.
    Cancelled,
}

impl FleetStatusView {
    /// Returns `true` for terminal states (completed, failed, cancelled).
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Short human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Single-character status icon consistent with [`crate::task_view::TaskStatusView`].
    #[must_use]
    pub const fn icon(self) -> char {
        match self {
            Self::Pending => '•',
            Self::Running => '◐',
            Self::Completed => '✓',
            Self::Failed => '✗',
            Self::Cancelled => '⏸',
        }
    }
}

/// Per-member status within a fleet run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FleetMemberStatusView {
    /// Member not yet dispatched or task record missing.
    Pending,
    /// Member is actively running.
    Running,
    /// Member completed successfully.
    Completed,
    /// Member failed, was killed, or was cancelled.
    Failed,
}

impl FleetMemberStatusView {
    /// Returns `true` for terminal states (completed, failed).
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }

    /// Short human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    /// Single-character status icon.
    #[must_use]
    pub const fn icon(self) -> char {
        match self {
            Self::Pending => '•',
            Self::Running => '◐',
            Self::Completed => '✓',
            Self::Failed => '✗',
        }
    }
}

// ── View data structs ─────────────────────────────────────────────────────────

/// A single fleet member row displayed in the fleet view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FleetMemberEntryView {
    /// Task identifier string (from `TaskId::to_string()`).
    pub task_id: String,
    /// Human-readable member name, if available.
    pub name: Option<String>,
    /// The fleet member request id that triggered this task, if known.
    pub request_id: Option<String>,
    /// Derived member status.
    pub status: FleetMemberStatusView,
    /// Short output excerpt from the result sidecar, if available.
    pub excerpt: Option<String>,
}

impl FleetMemberEntryView {
    /// Creates a minimal member entry with just a task id and status.
    #[must_use]
    pub fn new(task_id: impl Into<String>, status: FleetMemberStatusView) -> Self {
        Self {
            task_id: task_id.into(),
            name: None,
            request_id: None,
            status,
            excerpt: None,
        }
    }

    /// Attaches a human-readable name to this member.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Attaches a fleet request id to this member.
    #[must_use]
    pub fn request_id(mut self, id: impl Into<String>) -> Self {
        self.request_id = Some(id.into());
        self
    }

    /// Attaches an output excerpt to this member.
    #[must_use]
    pub fn excerpt(mut self, excerpt: impl Into<String>) -> Self {
        self.excerpt = Some(excerpt.into());
        self
    }

    /// Renders one or two compact lines for this member entry.
    ///
    /// Format:
    /// ```text
    ///   ◐ task-001 (worker-1) — running
    ///     └ <excerpt>
    /// ```
    #[must_use]
    pub fn render_lines(&self) -> Vec<String> {
        let label = match &self.name {
            Some(name) => format!("{} ({})", self.task_id, name),
            None => self.task_id.clone(),
        };
        let mut lines = vec![format!(
            "  {} {} — {}",
            self.status.icon(),
            label,
            self.status.label()
        )];
        if let Some(ref ex) = self.excerpt {
            lines.push(format!("    └ {ex}"));
        }
        lines
    }
}

/// Snapshot view of an entire fleet run including its member rows.
///
/// Constructed from storage observations or test fixtures; carries no
/// references to storage types so it can be used in pure TUI render paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FleetRunView {
    /// Fleet run identifier string.
    pub fleet_id: String,
    /// Human-readable description of what the fleet is doing.
    pub description: String,
    /// Overall fleet status.
    pub status: FleetStatusView,
    /// Ordered list of member entries (preserves member_task_ids order).
    pub members: Vec<FleetMemberEntryView>,
}

impl FleetRunView {
    /// Creates an empty pending fleet run view.
    #[must_use]
    pub fn new(
        fleet_id: impl Into<String>,
        description: impl Into<String>,
        status: FleetStatusView,
    ) -> Self {
        Self {
            fleet_id: fleet_id.into(),
            description: description.into(),
            status,
            members: Vec::new(),
        }
    }

    /// Returns summary counts for each member status.
    #[must_use]
    pub fn counts(&self) -> FleetRunCounts {
        FleetRunCounts {
            pending: self
                .members
                .iter()
                .filter(|m| m.status == FleetMemberStatusView::Pending)
                .count(),
            running: self
                .members
                .iter()
                .filter(|m| m.status == FleetMemberStatusView::Running)
                .count(),
            completed: self
                .members
                .iter()
                .filter(|m| m.status == FleetMemberStatusView::Completed)
                .count(),
            failed: self
                .members
                .iter()
                .filter(|m| m.status == FleetMemberStatusView::Failed)
                .count(),
        }
    }

    /// Renders the fleet run as a list of human-readable lines.
    ///
    /// Output format:
    /// ```text
    /// ◐ Fleet fleet-001 — running (2 members)
    ///   Description: my fleet
    ///   ✓ task-aaa (worker-1) — completed
    ///   ◐ task-bbb — running
    ///
    /// summary: pending=0 running=1 completed=1 failed=0
    /// ```
    #[must_use]
    pub fn render_lines(&self) -> Vec<String> {
        let counts = self.counts();
        let mut lines = vec![
            format!(
                "{} Fleet {} — {} ({} members)",
                self.status.icon(),
                self.fleet_id,
                self.status.label(),
                self.members.len()
            ),
            format!("  Description: {}", self.description),
        ];

        for member in &self.members {
            lines.extend(member.render_lines());
        }

        if !self.members.is_empty() {
            lines.push(String::new());
            lines.push(format!(
                "summary: pending={} running={} completed={} failed={}",
                counts.pending, counts.running, counts.completed, counts.failed
            ));
        }

        lines
    }
}

/// Aggregated member counts for a [`FleetRunView`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FleetRunCounts {
    /// Members in the pending state.
    pub pending: usize,
    /// Members actively running.
    pub running: usize,
    /// Members completed successfully.
    pub completed: usize,
    /// Members that failed or were killed/cancelled.
    pub failed: usize,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── FleetStatusView ───────────────────────────────────────────────────────

    #[test]
    fn fleet_status_terminal_states() {
        assert!(FleetStatusView::Completed.is_terminal());
        assert!(FleetStatusView::Failed.is_terminal());
        assert!(FleetStatusView::Cancelled.is_terminal());
        assert!(!FleetStatusView::Pending.is_terminal());
        assert!(!FleetStatusView::Running.is_terminal());
    }

    #[test]
    fn fleet_status_icons_are_distinct() {
        let statuses = [
            FleetStatusView::Pending,
            FleetStatusView::Running,
            FleetStatusView::Completed,
            FleetStatusView::Failed,
            FleetStatusView::Cancelled,
        ];
        let icons: Vec<char> = statuses.iter().map(|s| s.icon()).collect();
        // All icons distinct.
        let unique: std::collections::HashSet<char> = icons.iter().copied().collect();
        assert_eq!(unique.len(), icons.len(), "duplicate icon detected");
    }

    #[test]
    fn fleet_status_labels_match_variants() {
        assert_eq!(FleetStatusView::Pending.label(), "pending");
        assert_eq!(FleetStatusView::Running.label(), "running");
        assert_eq!(FleetStatusView::Completed.label(), "completed");
        assert_eq!(FleetStatusView::Failed.label(), "failed");
        assert_eq!(FleetStatusView::Cancelled.label(), "cancelled");
    }

    // ── FleetMemberStatusView ─────────────────────────────────────────────────

    #[test]
    fn member_status_terminal_states() {
        assert!(FleetMemberStatusView::Completed.is_terminal());
        assert!(FleetMemberStatusView::Failed.is_terminal());
        assert!(!FleetMemberStatusView::Pending.is_terminal());
        assert!(!FleetMemberStatusView::Running.is_terminal());
    }

    #[test]
    fn member_status_icons_cover_all_variants() {
        let icons: Vec<char> = [
            FleetMemberStatusView::Pending,
            FleetMemberStatusView::Running,
            FleetMemberStatusView::Completed,
            FleetMemberStatusView::Failed,
        ]
        .iter()
        .map(|s| s.icon())
        .collect();
        let unique: std::collections::HashSet<char> = icons.iter().copied().collect();
        assert_eq!(unique.len(), icons.len(), "duplicate member icon detected");
    }

    // ── FleetMemberEntryView rendering ────────────────────────────────────────

    #[test]
    fn member_entry_pending_renders_bullet() {
        let entry = FleetMemberEntryView::new("task-001", FleetMemberStatusView::Pending);
        let lines = entry.render_lines();
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].contains('•'),
            "expected • icon; got: {:?}",
            lines[0]
        );
        assert!(lines[0].contains("task-001"), "got: {:?}", lines[0]);
        assert!(lines[0].contains("pending"), "got: {:?}", lines[0]);
    }

    #[test]
    fn member_entry_running_renders_spinner() {
        let entry = FleetMemberEntryView::new("task-002", FleetMemberStatusView::Running);
        let lines = entry.render_lines();
        assert!(
            lines[0].contains('◐'),
            "expected ◐ icon; got: {:?}",
            lines[0]
        );
        assert!(lines[0].contains("running"), "got: {:?}", lines[0]);
    }

    #[test]
    fn member_entry_completed_renders_checkmark() {
        let entry = FleetMemberEntryView::new("task-003", FleetMemberStatusView::Completed);
        let lines = entry.render_lines();
        assert!(
            lines[0].contains('✓'),
            "expected ✓ icon; got: {:?}",
            lines[0]
        );
    }

    #[test]
    fn member_entry_failed_renders_cross() {
        let entry = FleetMemberEntryView::new("task-004", FleetMemberStatusView::Failed);
        let lines = entry.render_lines();
        assert!(
            lines[0].contains('✗'),
            "expected ✗ icon; got: {:?}",
            lines[0]
        );
    }

    #[test]
    fn member_entry_name_appears_in_parens() {
        let entry =
            FleetMemberEntryView::new("task-001", FleetMemberStatusView::Running).name("worker-1");
        let lines = entry.render_lines();
        assert!(
            lines[0].contains("task-001 (worker-1)"),
            "expected name in parens; got: {:?}",
            lines[0]
        );
    }

    #[test]
    fn member_entry_excerpt_on_second_line() {
        let entry = FleetMemberEntryView::new("task-001", FleetMemberStatusView::Completed)
            .excerpt("all tests passed");
        let lines = entry.render_lines();
        assert_eq!(lines.len(), 2, "expected 2 lines; got: {lines:?}");
        assert!(lines[1].contains("all tests passed"), "got: {:?}", lines[1]);
        assert!(
            lines[1].contains('└'),
            "expected └ leader; got: {:?}",
            lines[1]
        );
    }

    // ── FleetRunView rendering ────────────────────────────────────────────────

    fn make_fleet_view_mixed() -> FleetRunView {
        FleetRunView {
            fleet_id: "fleet-abc123".into(),
            description: "integration test fleet".into(),
            status: FleetStatusView::Running,
            members: vec![
                FleetMemberEntryView::new("task-aaa", FleetMemberStatusView::Completed)
                    .name("step-1"),
                FleetMemberEntryView::new("task-bbb", FleetMemberStatusView::Running)
                    .name("step-2"),
                FleetMemberEntryView::new("task-ccc", FleetMemberStatusView::Pending),
                FleetMemberEntryView::new("task-ddd", FleetMemberStatusView::Failed)
                    .name("step-4")
                    .excerpt("error: build failed"),
            ],
        }
    }

    #[test]
    fn fleet_run_view_header_line_contains_fleet_id_and_status() {
        let view = make_fleet_view_mixed();
        let lines = view.render_lines();
        let header = &lines[0];
        assert!(
            header.contains("fleet-abc123"),
            "header missing fleet_id; got: {header}"
        );
        assert!(
            header.contains("running"),
            "header missing status; got: {header}"
        );
        assert!(
            header.contains("4 members"),
            "header missing member count; got: {header}"
        );
    }

    #[test]
    fn fleet_run_view_description_on_second_line() {
        let view = make_fleet_view_mixed();
        let lines = view.render_lines();
        assert!(
            lines[1].contains("integration test fleet"),
            "description missing; got: {:?}",
            lines[1]
        );
    }

    #[test]
    fn fleet_run_view_all_four_statuses_present() {
        let view = make_fleet_view_mixed();
        let text = view.render_lines().join("\n");
        assert!(text.contains('✓'), "missing completed icon");
        assert!(text.contains('◐'), "missing running icon");
        assert!(text.contains('•'), "missing pending icon");
        assert!(text.contains('✗'), "missing failed icon");
    }

    #[test]
    fn fleet_run_view_summary_line_counts_correct() {
        let view = make_fleet_view_mixed();
        let text = view.render_lines().join("\n");
        assert!(
            text.contains("pending=1"),
            "wrong pending count; text:\n{text}"
        );
        assert!(
            text.contains("running=1"),
            "wrong running count; text:\n{text}"
        );
        assert!(
            text.contains("completed=1"),
            "wrong completed count; text:\n{text}"
        );
        assert!(
            text.contains("failed=1"),
            "wrong failed count; text:\n{text}"
        );
    }

    #[test]
    fn fleet_run_view_empty_members_omits_summary() {
        let view = FleetRunView::new("fleet-empty", "nothing yet", FleetStatusView::Pending);
        let lines = view.render_lines();
        // Only header + description, no summary line for empty fleet.
        assert_eq!(
            lines.len(),
            2,
            "expected 2 lines for empty fleet; got: {lines:?}"
        );
        assert!(!lines.iter().any(|l| l.starts_with("summary:")));
    }

    #[test]
    fn fleet_run_counts_correct() {
        let view = make_fleet_view_mixed();
        let counts = view.counts();
        assert_eq!(counts.pending, 1);
        assert_eq!(counts.running, 1);
        assert_eq!(counts.completed, 1);
        assert_eq!(counts.failed, 1);
    }

    #[test]
    fn fleet_run_view_failed_excerpt_present() {
        let view = make_fleet_view_mixed();
        let text = view.render_lines().join("\n");
        assert!(
            text.contains("error: build failed"),
            "excerpt missing; text:\n{text}"
        );
    }

    #[test]
    fn fleet_run_view_terminal_completed() {
        let view = FleetRunView::new("fleet-x", "done", FleetStatusView::Completed);
        assert!(view.status.is_terminal());
        let text = view.render_lines().join("\n");
        assert!(text.contains('✓'));
    }

    #[test]
    fn fleet_run_view_terminal_failed() {
        let view = FleetRunView::new("fleet-y", "broken", FleetStatusView::Failed);
        assert!(view.status.is_terminal());
        let text = view.render_lines().join("\n");
        assert!(text.contains('✗'));
    }
}
