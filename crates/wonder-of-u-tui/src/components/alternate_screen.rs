//! Alternate-screen viewport primitives.

use super::terminal::TerminalSize;
use crate::frame::Rect;

/// View-model state for an alternate-screen surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlternateScreenView {
    pub size: TerminalSize,
    pub mouse_tracking: bool,
}

impl AlternateScreenView {
    #[must_use]
    pub const fn new(size: TerminalSize) -> Self {
        Self {
            size,
            mouse_tracking: true,
        }
    }

    #[must_use]
    pub const fn with_mouse_tracking(mut self, mouse_tracking: bool) -> Self {
        self.mouse_tracking = mouse_tracking;
        self
    }

    #[must_use]
    pub const fn viewport(self) -> Rect {
        self.size.area()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternate_screen_defaults_to_mouse_tracking() {
        let view = AlternateScreenView::new(TerminalSize::new(80, 24));

        assert!(view.mouse_tracking);
        assert_eq!(view.viewport(), Rect::new(0, 0, 80, 24));
    }
}
