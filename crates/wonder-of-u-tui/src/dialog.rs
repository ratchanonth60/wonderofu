#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DialogKind {
    Confirm,
    Permission,
    Notice,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogActionView {
    pub label: String,
    pub primary: bool,
}

impl DialogActionView {
    #[must_use]
    pub fn new(label: impl Into<String>, primary: bool) -> Self {
        Self {
            label: label.into(),
            primary,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DialogView {
    pub title: String,
    pub body: Vec<String>,
    pub actions: Vec<DialogActionView>,
}

impl DialogView {
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
        }
    }

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
        }
    }

    #[must_use]
    pub fn notice(
        title: impl Into<String>,
        body: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            title: title.into(),
            body: body.into_iter().map(Into::into).collect(),
            actions: vec![DialogActionView::new("Close", true)],
        }
    }

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
