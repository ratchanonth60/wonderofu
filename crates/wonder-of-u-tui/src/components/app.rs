//! Top-level app view state.

use super::{ErrorOverviewView, TerminalFocusView, TerminalSize};

/// Renderer-agnostic state shared with a top-level app surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppViewState {
    /// Stores the terminal size
    pub terminal_size: TerminalSize,
    /// Stores the terminal focus
    pub terminal_focus: TerminalFocusView,
    /// Stores the error overview
    pub error_overview: Option<ErrorOverviewView>,
}

impl AppViewState {
    /// Constant fn
    #[must_use]
    pub const fn new(terminal_size: TerminalSize, terminal_focus: TerminalFocusView) -> Self {
        Self {
            terminal_size,
            terminal_focus,
            error_overview: None,
        }
    }
    /// Handles with error
    #[must_use]
    pub fn with_error(mut self, error_overview: ErrorOverviewView) -> Self {
        self.error_overview = Some(error_overview);
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn has_error(&self) -> bool {
        self.error_overview.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::TerminalFocusState;

    #[test]
    fn app_state_tracks_error_overlay_presence() {
        let view = AppViewState::new(
            TerminalSize::new(100, 30),
            TerminalFocusView::new(TerminalFocusState::Focused),
        )
        .with_error(ErrorOverviewView::new("bad"));

        assert!(view.has_error());
    }
}
