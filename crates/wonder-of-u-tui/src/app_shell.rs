//! App shell summaries inspired by `claude-leak/components/App*`, onboarding, and tab shells.

use crate::{
    design_system::BylineView,
    dialog::{DialogActionView, DialogView},
};

/// Developer/status bar shown near the prompt.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DevBarView {
    pub profile: Option<String>,
    pub sandbox: Option<String>,
    pub hook_mode: Option<String>,
    pub fast_mode: bool,
    pub mcp_servers: usize,
}

impl DevBarView {
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
    pub running_tasks: usize,
    pub unsaved_buffers: usize,
}

impl ExitFlowView {
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
    pub url: String,
    pub code: String,
    pub expires_in_seconds: Option<u64>,
}

impl ConsoleOAuthFlowView {
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
    pub title: String,
    pub description: String,
    pub complete: bool,
}

impl OnboardingStepView {
    #[must_use]
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            complete: false,
        }
    }

    #[must_use]
    pub const fn complete(mut self, complete: bool) -> Self {
        self.complete = complete;
        self
    }
}

/// Onboarding screen state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OnboardingView {
    pub title: String,
    pub steps: Vec<OnboardingStepView>,
    pub footer: Option<BylineView>,
}

impl OnboardingView {
    #[must_use]
    pub fn new(title: impl Into<String>, steps: Vec<OnboardingStepView>) -> Self {
        Self {
            title: title.into(),
            steps,
            footer: None,
        }
    }

    #[must_use]
    pub fn footer(mut self, footer: BylineView) -> Self {
        self.footer = Some(footer);
        self
    }

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
    pub label: String,
    pub tag: Option<String>,
    pub active: bool,
}

impl TagTabView {
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            tag: None,
            active: false,
        }
    }

    #[must_use]
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    #[must_use]
    pub const fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

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
    pub tabs: Vec<TagTabView>,
}

impl TagTabsView {
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
    pub title: String,
    pub dev_bar: Option<DevBarView>,
    pub onboarding: Option<OnboardingView>,
    pub footer: Option<BylineView>,
}

impl AppShellView {
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
}
