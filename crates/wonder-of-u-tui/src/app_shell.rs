//! App shell summaries inspired by `claude-leak/components/App*`, onboarding, and tab shells.

use crate::{
    design_system::BylineView,
    dialog::{DialogActionView, DialogView},
};

/// Developer/status bar shown near the prompt.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DevBarView {
    /// Stores the profile
    pub profile: Option<String>,
    /// Stores the sandbox
    pub sandbox: Option<String>,
    /// Stores the hook mode
    pub hook_mode: Option<String>,
    /// Stores the fast mode
    pub fast_mode: bool,
    /// Stores the mcp servers
    pub mcp_servers: usize,
}

impl DevBarView {
    /// Renders line
    #[must_use]
    pub fn render_line(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(profile) = &self.profile {
            parts.push(profile.clone());
        }
        if self.fast_mode {
            parts.push("FAST".into());
        }
        if let Some(sandbox) = &self.sandbox {
            parts.push(format!("sandbox: {sandbox}"));
        }
        if let Some(hook_mode) = &self.hook_mode {
            parts.push(format!("hooks: {hook_mode}"));
        }
        if self.mcp_servers > 0 {
            parts.push(format!("mcp: {}", self.mcp_servers));
        }
        BylineView::from_items(parts).render_line()
    }
}

/// Exit confirmation state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExitFlowView {
    /// Stores the running tasks
    pub running_tasks: usize,
    /// Stores the unsaved buffers
    pub unsaved_buffers: usize,
}

impl ExitFlowView {
    /// Handles to dialog view
    #[must_use]
    pub fn to_dialog_view(&self) -> DialogView {
        let mut body = vec!["Are you sure you want to leave Claude Code?".into()];
        if self.running_tasks > 0 {
            body.push(format!(
                "{} running task{} will be interrupted.",
                self.running_tasks,
                if self.running_tasks == 1 { "" } else { "s" }
            ));
        }
        if self.unsaved_buffers > 0 {
            body.push(format!(
                "{} draft buffer{} may be lost.",
                self.unsaved_buffers,
                if self.unsaved_buffers == 1 { "" } else { "s" }
            ));
        }
        DialogView {
            title: "Exit Claude Code".into(),
            body,
            actions: vec![
                DialogActionView::new("Exit", true),
                DialogActionView::new("Stay", false),
            ],
        }
    }
}

/// Device-code or console OAuth handoff details.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsoleOAuthFlowView {
    /// Stores the url
    pub url: String,
    /// Stores the code
    pub code: String,
    /// Stores the expires in seconds
    pub expires_in_seconds: Option<u64>,
}

impl ConsoleOAuthFlowView {
    /// Handles instructions
    #[must_use]
    pub fn instructions(&self) -> Vec<String> {
        let mut lines = vec![
            format!("Open: {}", self.url),
            format!("Enter code: {}", self.code),
        ];
        if let Some(expires_in_seconds) = self.expires_in_seconds {
            lines.push(format!("Code expires in {}s", expires_in_seconds));
        }
        lines
    }
}

/// A single onboarding checklist step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OnboardingStepView {
    /// Stores the title
    pub title: String,
    /// Stores the description
    pub description: String,
    /// Stores the complete
    pub complete: bool,
}

impl OnboardingStepView {
    /// Creates a new value
    #[must_use]
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            complete: false,
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn complete(mut self, complete: bool) -> Self {
        self.complete = complete;
        self
    }
}

/// Onboarding screen state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OnboardingView {
    /// Stores the title
    pub title: String,
    /// Stores the steps
    pub steps: Vec<OnboardingStepView>,
    /// Stores the footer
    pub footer: Option<BylineView>,
}

impl OnboardingView {
    /// Creates a new value
    #[must_use]
    pub fn new(title: impl Into<String>, steps: Vec<OnboardingStepView>) -> Self {
        Self {
            title: title.into(),
            steps,
            footer: None,
        }
    }
    /// Handles footer
    #[must_use]
    pub fn footer(mut self, footer: BylineView) -> Self {
        self.footer = Some(footer);
        self
    }
    /// Renders lines
    #[must_use]
    pub fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![self.title.clone()];
        lines.extend(self.steps.iter().map(|step| {
            format!(
                "{} {} — {}",
                if step.complete { '✓' } else { '•' },
                step.title,
                step.description
            )
        }));
        if let Some(footer) = &self.footer
            && let Some(line) = footer.render_line()
        {
            lines.push(String::new());
            lines.push(line);
        }
        lines
    }
}

/// A tab label with an optional tag badge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagTabView {
    /// Stores the label
    pub label: String,
    /// Stores the tag
    pub tag: Option<String>,
    /// Stores the active
    pub active: bool,
}

impl TagTabView {
    /// Creates a new value
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            tag: None,
            active: false,
        }
    }
    /// Handles tag
    #[must_use]
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
    /// Handles display label
    #[must_use]
    pub fn display_label(&self) -> String {
        match &self.tag {
            Some(tag) => format!("{} [{}]", self.label, tag),
            None => self.label.clone(),
        }
    }
}

/// App shell tab row.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TagTabsView {
    /// Stores the tabs
    pub tabs: Vec<TagTabView>,
}

impl TagTabsView {
    /// Renders line
    #[must_use]
    pub fn render_line(&self) -> String {
        self.tabs
            .iter()
            .map(|tab| {
                if tab.active {
                    format!("[{}]", tab.display_label())
                } else {
                    tab.display_label()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Top-level shell composition helpers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AppShellView {
    /// Stores the title
    pub title: String,
    /// Stores the dev bar
    pub dev_bar: Option<DevBarView>,
    /// Stores the onboarding
    pub onboarding: Option<OnboardingView>,
    /// Stores the footer
    pub footer: Option<BylineView>,
}

impl AppShellView {
    /// Renders lines
    #[must_use]
    pub fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![self.title.clone()];
        if let Some(dev_bar) = &self.dev_bar
            && let Some(line) = dev_bar.render_line()
        {
            lines.push(line);
        }
        if let Some(onboarding) = &self.onboarding {
            lines.push(String::new());
            lines.extend(onboarding.render_lines());
        }
        if let Some(footer) = &self.footer
            && let Some(line) = footer.render_line()
        {
            lines.push(String::new());
            lines.push(line);
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_bar_skips_empty_sections() {
        let line = DevBarView {
            profile: Some("debug".into()),
            fast_mode: true,
            sandbox: Some("workspace-write".into()),
            hook_mode: None,
            mcp_servers: 2,
        }
        .render_line()
        .unwrap();

        assert_eq!(line, "debug · FAST · sandbox: workspace-write · mcp: 2");
    }

    #[test]
    fn exit_flow_mentions_running_work() {
        let dialog = ExitFlowView {
            running_tasks: 2,
            unsaved_buffers: 1,
        }
        .to_dialog_view();

        assert!(
            dialog
                .body
                .iter()
                .any(|line| line.contains("2 running tasks"))
        );
        assert!(
            dialog
                .body
                .iter()
                .any(|line| line.contains("1 draft buffer"))
        );
        assert_eq!(dialog.actions[0], DialogActionView::new("Exit", true));
    }

    #[test]
    fn tag_tabs_highlight_active_tab() {
        let line = TagTabsView {
            tabs: vec![
                TagTabView::new("Chat").active(true),
                TagTabView::new("Tasks").tag("3"),
            ],
        }
        .render_line();

        assert_eq!(line, "[Chat] Tasks [3]");
    }

    // ── ConsoleOAuthFlowView ──────────────────────────────────────────────────

    #[test]
    fn console_oauth_instructions_without_expiry() {
        // When `expires_in_seconds` is absent the instruction list has exactly
        // two lines: URL and code.
        let view = ConsoleOAuthFlowView {
            url: "https://github.com/login/device".into(),
            code: "WXYZ-5678".into(),
            expires_in_seconds: None,
        };
        let lines = view.instructions();
        assert_eq!(
            lines.len(),
            2,
            "should produce exactly 2 lines without expiry"
        );
        assert!(
            lines[0].contains("https://github.com/login/device"),
            "first line must contain the URL; got: {:?}",
            lines[0]
        );
        assert!(
            lines[1].contains("WXYZ-5678"),
            "second line must contain the user code; got: {:?}",
            lines[1]
        );
    }

    #[test]
    fn console_oauth_instructions_with_expiry_adds_third_line() {
        // When the code has an expiry the third line must mention the seconds.
        let view = ConsoleOAuthFlowView {
            url: "https://example.invalid/device".into(),
            code: "ABCD-0000".into(),
            expires_in_seconds: Some(900),
        };
        let lines = view.instructions();
        assert_eq!(
            lines.len(),
            3,
            "should produce 3 lines when expiry is present"
        );
        assert!(
            lines[2].contains("900"),
            "third line must mention the expiry duration; got: {:?}",
            lines[2]
        );
    }

    // ── OnboardingView ────────────────────────────────────────────────────────

    #[test]
    fn onboarding_renders_completed_and_pending_steps() {
        // Completed steps use '✓'; pending steps use '•'.
        let view = OnboardingView::new(
            "Get started",
            vec![
                OnboardingStepView::new("Install", "already done").complete(true),
                OnboardingStepView::new("Configure", "needs action").complete(false),
            ],
        );
        let lines = view.render_lines();
        // First line is always the title.
        assert_eq!(lines[0], "Get started");
        let complete_line = lines.iter().find(|l| l.contains("Install")).unwrap();
        assert!(
            complete_line.contains('✓'),
            "completed step must use '✓'; got: {complete_line:?}"
        );
        let pending_line = lines.iter().find(|l| l.contains("Configure")).unwrap();
        assert!(
            pending_line.contains('•'),
            "pending step must use '•'; got: {pending_line:?}"
        );
    }

    #[test]
    fn onboarding_renders_footer_when_present() {
        use crate::design_system::BylineView;
        // A footer BylineView with one item should appear as an extra line.
        let footer = BylineView::from_items(vec!["docs: https://example.com".to_string()]);
        let view = OnboardingView::new("Title", vec![OnboardingStepView::new("Step", "desc")])
            .footer(footer);
        let lines = view.render_lines();
        // The footer separator blank line and footer content must be present.
        let joined = lines.join("\n");
        assert!(
            joined.contains("docs: https://example.com"),
            "footer text must appear in rendered output; got: {joined:?}"
        );
    }

    #[test]
    fn onboarding_with_no_steps_renders_only_title() {
        let view = OnboardingView::new("Empty", vec![]);
        let lines = view.render_lines();
        assert_eq!(lines, vec!["Empty".to_string()]);
    }
}
