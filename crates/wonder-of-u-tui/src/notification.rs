//! Notification queue and view models for shell overlay toasts.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationSeverity {
    Info,
    Success,
    Warning,
    Error,
}

impl NotificationSeverity {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "ok",
            Self::Warning => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationLifetime {
    Persistent,
    Ticks(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationInput {
    pub key: String,
    pub title: String,
    pub lines: Vec<String>,
    pub severity: NotificationSeverity,
    pub lifetime: NotificationLifetime,
    pub focus: bool,
}

impl NotificationInput {
    #[must_use]
    pub fn new(
        key: impl Into<String>,
        title: impl Into<String>,
        lines: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            lines: lines.into_iter().map(Into::into).collect(),
            severity: NotificationSeverity::Info,
            lifetime: NotificationLifetime::Persistent,
            focus: false,
        }
    }

    #[must_use]
    pub fn severity(mut self, severity: NotificationSeverity) -> Self {
        self.severity = severity;
        self
    }

    #[must_use]
    pub fn lifetime(mut self, lifetime: NotificationLifetime) -> Self {
        self.lifetime = lifetime;
        self
    }

    #[must_use]
    pub fn focused(mut self, focus: bool) -> Self {
        self.focus = focus;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationView {
    pub key: String,
    pub title: String,
    pub lines: Vec<String>,
    pub severity: NotificationSeverity,
    pub focused: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NotificationEntry {
    key: String,
    title: String,
    lines: Vec<String>,
    severity: NotificationSeverity,
    remaining_ticks: Option<u32>,
    focused: bool,
}

impl NotificationEntry {
    #[must_use]
    fn from_input(input: NotificationInput) -> Self {
        Self {
            key: input.key,
            title: input.title,
            lines: input.lines,
            severity: input.severity,
            remaining_ticks: match input.lifetime {
                NotificationLifetime::Persistent => None,
                NotificationLifetime::Ticks(ticks) => Some(ticks),
            },
            focused: input.focus,
        }
    }

    #[must_use]
    fn view(&self) -> NotificationView {
        NotificationView {
            key: self.key.clone(),
            title: self.title.clone(),
            lines: self.lines.clone(),
            severity: self.severity,
            focused: self.focused,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationQueue {
    entries: Vec<NotificationEntry>,
    window_focused: bool,
}

impl Default for NotificationQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationQueue {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            window_focused: true,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn push(&mut self, input: NotificationInput) {
        let previous = self
            .entries
            .iter()
            .position(|entry| entry.key == input.key)
            .map(|index| self.entries.remove(index));

        let should_focus = input.focus || previous.as_ref().is_some_and(|entry| entry.focused);
        let mut entry = NotificationEntry::from_input(input);
        entry.focused = should_focus;

        if should_focus {
            self.clear_focus();
        }

        self.entries.push(entry);
    }

    pub fn tick(&mut self) -> bool {
        if !self.window_focused || self.entries.is_empty() {
            return false;
        }

        let focused_key = self
            .entries
            .iter()
            .find(|entry| entry.focused)
            .map(|entry| entry.key.clone());
        let original_len = self.entries.len();
        for entry in &mut self.entries {
            if let Some(remaining) = &mut entry.remaining_ticks {
                *remaining = remaining.saturating_sub(1);
            }
        }
        self.entries
            .retain(|entry| entry.remaining_ticks != Some(0));
        let changed = self.entries.len() != original_len;
        if changed {
            self.restore_focus_after_prune(focused_key.as_deref());
        }
        changed
    }

    pub fn set_window_focused(&mut self, focused: bool) -> bool {
        let changed = self.window_focused != focused;
        self.window_focused = focused;
        changed
    }

    pub fn focus_latest(&mut self) -> bool {
        let Some(last) = self.entries.len().checked_sub(1) else {
            return false;
        };
        self.focus_index(last)
    }

    pub fn focus_next(&mut self) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let current = self
            .focused_index()
            .unwrap_or_else(|| self.entries.len() - 1);
        self.focus_index((current + 1) % self.entries.len())
    }

    pub fn focus_previous(&mut self) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let current = self.focused_index().unwrap_or(0);
        self.focus_index(if current == 0 {
            self.entries.len() - 1
        } else {
            current - 1
        })
    }

    pub fn dismiss_focused(&mut self) -> Option<NotificationView> {
        let index = self.focused_index()?;
        self.dismiss_at(index)
    }

    pub fn dismiss(&mut self, key: &str) -> Option<NotificationView> {
        let index = self.entries.iter().position(|entry| entry.key == key)?;
        self.dismiss_at(index)
    }

    #[must_use]
    pub fn view(&self, limit: usize) -> Vec<NotificationView> {
        self.entries
            .iter()
            .rev()
            .take(limit)
            .map(NotificationEntry::view)
            .collect()
    }

    fn dismiss_at(&mut self, index: usize) -> Option<NotificationView> {
        if index >= self.entries.len() {
            return None;
        }
        let removed = self.entries.remove(index);
        if removed.focused {
            let replacement = index
                .saturating_sub(1)
                .min(self.entries.len().saturating_sub(1));
            if let Some(entry) = self.entries.get_mut(replacement) {
                entry.focused = true;
            }
        }
        Some(removed.view())
    }

    fn clear_focus(&mut self) {
        for entry in &mut self.entries {
            entry.focused = false;
        }
    }

    fn focus_index(&mut self, index: usize) -> bool {
        if index >= self.entries.len() {
            return false;
        }
        let changed = self.focused_index() != Some(index);
        self.clear_focus();
        if let Some(entry) = self.entries.get_mut(index) {
            entry.focused = true;
        }
        changed
    }

    fn focused_index(&self) -> Option<usize> {
        self.entries.iter().position(|entry| entry.focused)
    }

    fn restore_focus_after_prune(&mut self, key: Option<&str>) {
        if let Some(key) = key {
            if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
                let _ = self.focus_index(index);
                return;
            }
        }
        if self.focused_index().is_none() {
            let _ = self.focus_latest();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_expires_notifications_after_ttl_ticks() {
        let mut queue = NotificationQueue::new();
        queue.push(
            NotificationInput::new("task:1", "Task update", ["done"])
                .lifetime(NotificationLifetime::Ticks(2))
                .focused(true),
        );

        assert_eq!(queue.len(), 1);
        assert!(!queue.tick(), "first tick should only decrement TTL");
        assert_eq!(queue.len(), 1);
        assert!(queue.tick(), "second tick should expire the notification");
        assert!(queue.is_empty());
    }

    #[test]
    fn queue_dismisses_the_focused_notification() {
        let mut queue = NotificationQueue::new();
        queue.push(NotificationInput::new("one", "One", ["first"]).focused(true));
        queue.push(NotificationInput::new("two", "Two", ["second"]).focused(true));

        let removed = queue.dismiss_focused().expect("focused notification");

        assert_eq!(removed.key, "two");
        let view = queue.view(10);
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].key, "one");
        assert!(
            view[0].focused,
            "focus should move to the remaining notification"
        );
    }

    #[test]
    fn queue_dedupes_by_key_and_refreshes_content() {
        let mut queue = NotificationQueue::new();
        queue.push(
            NotificationInput::new("source-status", "Source status", ["initial"])
                .lifetime(NotificationLifetime::Ticks(1)),
        );
        queue.push(
            NotificationInput::new("source-status", "Source status", ["updated"])
                .severity(NotificationSeverity::Warning)
                .lifetime(NotificationLifetime::Ticks(2))
                .focused(true),
        );

        let view = queue.view(10);
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].lines, vec!["updated"]);
        assert_eq!(view[0].severity, NotificationSeverity::Warning);
        assert!(view[0].focused);

        assert!(!queue.tick());
        assert_eq!(
            queue.len(),
            1,
            "updated TTL should keep the notification alive"
        );
        assert!(queue.tick());
        assert!(queue.is_empty());
    }

    #[test]
    fn queue_pauses_ttl_while_window_is_unfocused() {
        let mut queue = NotificationQueue::new();
        queue.push(
            NotificationInput::new("task:1", "Task update", ["done"])
                .lifetime(NotificationLifetime::Ticks(1)),
        );

        assert!(queue.set_window_focused(false));
        assert!(
            !queue.tick(),
            "ticks should not expire notifications while the window is unfocused"
        );
        assert_eq!(queue.len(), 1);

        assert!(queue.set_window_focused(true));
        assert!(queue.tick());
        assert!(queue.is_empty());
    }

    #[test]
    fn queue_cycles_focus_across_visible_notifications() {
        let mut queue = NotificationQueue::new();
        queue.push(NotificationInput::new("one", "One", ["first"]).focused(true));
        queue.push(NotificationInput::new("two", "Two", ["second"]));
        queue.push(NotificationInput::new("three", "Three", ["third"]));

        assert!(queue.focus_next());
        let focused = queue
            .view(10)
            .into_iter()
            .find(|entry| entry.focused)
            .expect("focused notification");
        assert_eq!(focused.key, "two");

        assert!(queue.focus_previous());
        let focused = queue
            .view(10)
            .into_iter()
            .find(|entry| entry.focused)
            .expect("focused notification");
        assert_eq!(focused.key, "one");
    }
}
