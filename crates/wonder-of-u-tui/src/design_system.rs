//! Design-system view models inspired by `claude-leak/components/design-system/*`.

use crate::components::TextWrap;

/// Shared themed text fragment metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThemedTextView {
    /// Stores the content
    pub content: String,
    /// Stores the color key
    pub color_key: Option<String>,
    /// Stores the background color key
    pub background_color_key: Option<String>,
    /// Stores the dim
    pub dim: bool,
    /// Stores the bold
    pub bold: bool,
    /// Stores the wrap
    pub wrap: TextWrap,
}

impl ThemedTextView {
    /// Creates a new value
    #[must_use]
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            color_key: None,
            background_color_key: None,
            dim: false,
            bold: false,
            wrap: TextWrap::Wrap,
        }
    }
    /// Handles color key
    #[must_use]
    pub fn color_key(mut self, color_key: impl Into<String>) -> Self {
        self.color_key = Some(color_key.into());
        self
    }
    /// Handles background color key
    #[must_use]
    pub fn background_color_key(mut self, background_color_key: impl Into<String>) -> Self {
        self.background_color_key = Some(background_color_key.into());
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn dim(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn bold(mut self, bold: bool) -> Self {
        self.bold = bold;
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn wrap(mut self, wrap: TextWrap) -> Self {
        self.wrap = wrap;
        self
    }
    /// Handles resolved color key
    #[must_use]
    pub fn resolved_color_key<'a>(&'a self, hover_color_key: Option<&'a str>) -> Option<&'a str> {
        self.color_key
            .as_deref()
            .or(hover_color_key)
            .or(self.dim.then_some("inactive"))
    }
}

/// Inline metadata joined with a middot separator.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BylineView {
    /// Stores the items
    pub items: Vec<String>,
}

impl BylineView {
    /// Handles from items
    #[must_use]
    pub fn from_items(items: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            items: items.into_iter().map(Into::into).collect(),
        }
    }
    /// Renders line
    #[must_use]
    pub fn render_line(&self) -> Option<String> {
        (!self.items.is_empty()).then(|| self.items.join(" · "))
    }
}

/// Row metadata for selection lists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListItemView {
    /// Stores the label
    pub label: String,
    /// Stores the description
    pub description: Option<String>,
    /// Stores whether focused
    pub is_focused: bool,
    /// Stores whether selected
    pub is_selected: bool,
    /// Stores the show scroll down
    pub show_scroll_down: bool,
    /// Stores the show scroll up
    pub show_scroll_up: bool,
    /// Stores the styled
    pub styled: bool,
    /// Stores the disabled
    pub disabled: bool,
}

impl ListItemView {
    /// Creates a new value
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: None,
            is_focused: false,
            is_selected: false,
            show_scroll_down: false,
            show_scroll_up: false,
            styled: true,
            disabled: false,
        }
    }
    /// Handles description
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn focused(mut self, is_focused: bool) -> Self {
        self.is_focused = is_focused;
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn selected(mut self, is_selected: bool) -> Self {
        self.is_selected = is_selected;
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn styled(mut self, styled: bool) -> Self {
        self.styled = styled;
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn show_scroll_down(mut self, show_scroll_down: bool) -> Self {
        self.show_scroll_down = show_scroll_down;
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn show_scroll_up(mut self, show_scroll_up: bool) -> Self {
        self.show_scroll_up = show_scroll_up;
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn indicator(&self) -> char {
        if self.disabled {
            ' '
        } else if self.is_focused {
            '❯'
        } else if self.show_scroll_down {
            '↓'
        } else if self.show_scroll_up {
            '↑'
        } else {
            ' '
        }
    }
    /// Handles color key
    #[must_use]
    pub fn color_key(&self) -> Option<&'static str> {
        if self.disabled {
            Some("inactive")
        } else if !self.styled {
            None
        } else if self.is_selected {
            Some("success")
        } else if self.is_focused {
            Some("suggestion")
        } else {
            None
        }
    }
    /// Renders lines
    #[must_use]
    pub fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} {}{}",
            self.indicator(),
            self.label,
            if self.is_selected && !self.disabled {
                " ✓"
            } else {
                ""
            }
        )];
        if let Some(description) = &self.description {
            lines.push(format!("  {description}"));
        }
        lines
    }
}

/// Minimal bordered pane summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneView {
    /// Stores the title
    pub title: Option<String>,
    /// Stores the color key
    pub color_key: Option<String>,
    /// Stores the body
    pub body: Vec<String>,
    /// Stores the padding x
    pub padding_x: usize,
}

impl PaneView {
    /// Creates a new value
    #[must_use]
    pub fn new(body: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            title: None,
            color_key: None,
            body: body.into_iter().map(Into::into).collect(),
            padding_x: 2,
        }
    }
    /// Handles title
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
    /// Handles color key
    #[must_use]
    pub fn color_key(mut self, color_key: impl Into<String>) -> Self {
        self.color_key = Some(color_key.into());
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn padding_x(mut self, padding_x: usize) -> Self {
        self.padding_x = padding_x;
        self
    }
    /// Renders lines
    #[must_use]
    pub fn render_lines(&self, width: usize) -> Vec<String> {
        let border = if let Some(title) = &self.title {
            let prefix = format!("─ {title} ");
            if prefix.chars().count() >= width {
                prefix.chars().take(width).collect()
            } else {
                format!("{prefix}{}", "─".repeat(width - prefix.chars().count()))
            }
        } else {
            "─".repeat(width)
        };

        let padding = " ".repeat(self.padding_x);
        let mut lines = vec![border];
        lines.extend(self.body.iter().map(|line| format!("{padding}{line}")));
        lines
    }
}

/// Ratchet lock behavior for growing content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RatchetLock {
    /// Represents always
    Always,
    /// Represents offscreen
    Offscreen,
}

/// Tracks the tallest observed height for stable off-screen rendering.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RatchetView {
    /// Stores the max height
    pub max_height: usize,
}

impl RatchetView {
    /// Handles observe height
    #[must_use]
    pub fn observe_height(
        &mut self,
        current_height: usize,
        viewport_rows: usize,
        is_visible: bool,
        lock: RatchetLock,
    ) -> usize {
        if current_height > self.max_height {
            self.max_height = current_height.min(viewport_rows);
        }

        match lock {
            RatchetLock::Always => self.max_height.max(current_height),
            RatchetLock::Offscreen if !is_visible => self.max_height.max(current_height),
            RatchetLock::Offscreen => current_height,
        }
    }
}

/// Theme-aware bordered box summary.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ThemedBoxView {
    /// Stores the title
    pub title: Option<String>,
    /// Stores the border color key
    pub border_color_key: Option<String>,
    /// Stores the background color key
    pub background_color_key: Option<String>,
    /// Stores the body
    pub body: Vec<ThemedTextView>,
}

impl ThemedBoxView {
    /// Creates a new value
    #[must_use]
    pub fn new(body: impl IntoIterator<Item = ThemedTextView>) -> Self {
        Self {
            title: None,
            border_color_key: None,
            background_color_key: None,
            body: body.into_iter().collect(),
        }
    }
    /// Handles title
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
    /// Handles border color key
    #[must_use]
    pub fn border_color_key(mut self, border_color_key: impl Into<String>) -> Self {
        self.border_color_key = Some(border_color_key.into());
        self
    }
    /// Handles background color key
    #[must_use]
    pub fn background_color_key(mut self, background_color_key: impl Into<String>) -> Self {
        self.background_color_key = Some(background_color_key.into());
        self
    }
    /// Renders lines
    #[must_use]
    pub fn render_lines(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.body.len() + 2);
        let title = self.title.as_deref().unwrap_or("");
        let border = if title.is_empty() {
            "─".repeat(width)
        } else {
            let prefix = format!("┌ {title} ");
            format!(
                "{prefix}{}",
                "─".repeat(width.saturating_sub(prefix.chars().count()))
            )
        };
        lines.push(border);
        lines.extend(
            self.body
                .iter()
                .map(|line| format!("│ {}", truncate(&line.content, width.saturating_sub(2)))),
        );
        lines.push("└".to_string() + &"─".repeat(width.saturating_sub(1)));
        lines
    }
}

fn truncate(value: &str, width: usize) -> String {
    value.chars().take(width).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byline_joins_items_with_middot() {
        let line = BylineView::from_items(["Enter to confirm", "Esc to cancel"])
            .render_line()
            .unwrap();

        assert_eq!(line, "Enter to confirm · Esc to cancel");
    }

    #[test]
    fn list_item_prefers_focus_indicator_over_scroll_hint() {
        let item = ListItemView::new("Sandbox")
            .focused(true)
            .show_scroll_down(true)
            .selected(true);

        assert_eq!(item.indicator(), '❯');
        assert_eq!(item.color_key(), Some("success"));
        assert_eq!(item.render_lines()[0], "❯ Sandbox ✓");
    }

    #[test]
    fn pane_renders_border_and_padding() {
        let pane = PaneView::new(["One", "Two"]).title("Settings").padding_x(1);
        let lines = pane.render_lines(12);

        assert!(lines[0].starts_with("─ Settings "));
        assert_eq!(lines[1], " One");
        assert_eq!(lines[2], " Two");
    }

    #[test]
    fn ratchet_holds_max_height_when_offscreen() {
        let mut ratchet = RatchetView::default();
        assert_eq!(
            ratchet.observe_height(3, 20, true, RatchetLock::Offscreen),
            3
        );
        assert_eq!(
            ratchet.observe_height(8, 20, false, RatchetLock::Offscreen),
            8
        );
        assert_eq!(
            ratchet.observe_height(2, 20, false, RatchetLock::Offscreen),
            8
        );
    }

    #[test]
    fn themed_text_prefers_explicit_color_to_hover_color() {
        let text = ThemedTextView::new("hi").color_key("claude").dim(true);

        assert_eq!(text.resolved_color_key(Some("warning")), Some("claude"));
    }
}
