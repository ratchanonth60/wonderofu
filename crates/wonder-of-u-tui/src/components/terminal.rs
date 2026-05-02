//! Terminal-size and focus primitives shared by component view models.

use crate::{frame::Rect, terminal::TerminalCapabilities};

/// Terminal dimensions in character cells.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TerminalSize {
    /// Stores the columns
    pub columns: u16,
    /// Stores the rows
    pub rows: u16,
}

impl TerminalSize {
    /// Constant fn
    #[must_use]
    pub const fn new(columns: u16, rows: u16) -> Self {
        Self { columns, rows }
    }
    /// Constant fn
    #[must_use]
    pub const fn area(self) -> Rect {
        Rect::new(0, 0, self.columns, self.rows)
    }
}

/// High-level terminal focus status.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TerminalFocusState {
    /// Represents focused
    Focused,
    /// Represents blurred
    Blurred,
    /// Represents unknown
    #[default]
    Unknown,
}

impl TerminalFocusState {
    /// Constant fn
    #[must_use]
    pub const fn is_focused(self) -> bool {
        matches!(self, Self::Focused)
    }
}

/// Focus context data exposed to component trees.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalFocusView {
    /// Stores whether terminal focused
    pub is_terminal_focused: bool,
    /// Stores the terminal focus state
    pub terminal_focus_state: TerminalFocusState,
}

impl Default for TerminalFocusView {
    fn default() -> Self {
        Self {
            is_terminal_focused: true,
            terminal_focus_state: TerminalFocusState::Unknown,
        }
    }
}

impl TerminalFocusView {
    /// Constant fn
    #[must_use]
    pub const fn new(terminal_focus_state: TerminalFocusState) -> Self {
        Self {
            is_terminal_focused: terminal_focus_state.is_focused(),
            terminal_focus_state,
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn from_bool(is_terminal_focused: bool) -> Self {
        Self {
            is_terminal_focused,
            terminal_focus_state: if is_terminal_focused {
                TerminalFocusState::Focused
            } else {
                TerminalFocusState::Blurred
            },
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn hyperlink_enabled(self, capabilities: TerminalCapabilities) -> bool {
        self.is_terminal_focused && capabilities.hyperlinks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_size_maps_to_frame_area() {
        let size = TerminalSize::new(120, 40);

        assert_eq!(size.area(), Rect::new(0, 0, 120, 40));
    }

    #[test]
    fn focused_view_tracks_focus_state() {
        let focused = TerminalFocusView::new(TerminalFocusState::Focused);
        let blurred = TerminalFocusView::from_bool(false);

        assert!(focused.is_terminal_focused);
        assert_eq!(blurred.terminal_focus_state, TerminalFocusState::Blurred);
    }
}
