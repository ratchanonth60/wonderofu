/// Enumerates dialog kind
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DialogKind {
    /// Represents confirm
    Confirm,
    /// Represents permission
    Permission,
    /// Represents notice
    Notice,
}
/// Represents dialog action view
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogActionView {
    /// Stores the label
    pub label: String,
    /// Stores the primary
    pub primary: bool,
}

impl DialogActionView {
    /// Creates a new value
    #[must_use]
    pub fn new(label: impl Into<String>, primary: bool) -> Self {
        Self {
            label: label.into(),
            primary,
        }
    }
}
/// Represents dialog view
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DialogView {
    /// Stores the title
    pub title: String,
    /// Stores the body
    pub body: Vec<String>,
    /// Stores the actions
    pub actions: Vec<DialogActionView>,
    /// Index of the currently focused action (arrow-key navigable).
    /// Always clamped to `[0, actions.len())`.
    pub selected_action: usize,
}

impl DialogView {
    /// Handles confirm
    #[must_use]
    pub fn confirm(
        title: impl Into<String>,
        body: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            title: title.into(),
            body: body.into_iter().map(Into::into).collect(),
            actions: vec![
                DialogActionView::new("Confirm", true),
                DialogActionView::new("Cancel", false),
            ],
            selected_action: 0,
        }
    }
    /// Handles permission
    #[must_use]
    pub fn permission(tool: impl Into<String>, reason: impl Into<String>) -> Self {
        let tool = tool.into();
        let reason = reason.into();
        Self {
            title: format!("Permission: {tool}"),
            body: vec![
                format!("Tool `{tool}` needs approval to continue."),
                reason,
                "Allow to continue, or deny to continue without running it.".into(),
            ],
            actions: vec![
                DialogActionView::new("Allow", true),
                DialogActionView::new("Deny", false),
            ],
            selected_action: 0,
        }
    }
    /// Handles notice
    #[must_use]
    pub fn notice(
        title: impl Into<String>,
        body: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            title: title.into(),
            body: body.into_iter().map(Into::into).collect(),
            actions: vec![DialogActionView::new("Close", true)],
            selected_action: 0,
        }
    }
    /// Moves focus to the previous action (wraps around).
    pub fn focus_prev(&mut self) {
        if self.actions.is_empty() {
            return;
        }
        self.selected_action = if self.selected_action == 0 {
            self.actions.len() - 1
        } else {
            self.selected_action - 1
        };
    }

    /// Moves focus to the next action (wraps around).
    pub fn focus_next(&mut self) {
        if self.actions.is_empty() {
            return;
        }
        self.selected_action = (self.selected_action + 1) % self.actions.len();
    }

    /// Returns `true` when the currently focused action is the primary (first) action.
    #[must_use]
    pub fn selected_is_primary(&self) -> bool {
        self.actions
            .get(self.selected_action)
            .is_some_and(|a| a.primary)
    }

    /// Handles kind
    #[must_use]
    pub fn kind(&self) -> DialogKind {
        if self.actions.len() == 1 && self.actions[0].label == "Close" {
            DialogKind::Notice
        } else if self.title.starts_with("Permission: ") {
            DialogKind::Permission
        } else {
            DialogKind::Confirm
        }
    }
    /// Handles action hint
    #[must_use]
    pub fn action_hint(&self) -> String {
        self.actions
            .iter()
            .map(|action| {
                if action.primary {
                    format!("[{}]", action.label)
                } else {
                    action.label.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("  ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_dialog_uses_allow_deny_actions() {
        let dialog = DialogView::permission("bash", "Needs approval.");

        assert_eq!(dialog.kind(), DialogKind::Permission);
        assert_eq!(dialog.body[0], "Tool `bash` needs approval to continue.");
        assert_eq!(dialog.body[1], "Needs approval.");
        assert_eq!(dialog.actions[0], DialogActionView::new("Allow", true));
        assert!(dialog.action_hint().contains("[Allow]"));
    }

    #[test]
    fn notice_dialog_uses_close_action() {
        let dialog = DialogView::notice("Task update", ["Tests finished"]);

        assert_eq!(dialog.kind(), DialogKind::Notice);
        assert_eq!(dialog.actions, vec![DialogActionView::new("Close", true)]);
    }
}
