//! Scroll-box state primitives.

use std::ops::Range;

/// Imperative scroll state tracked independently from rendering.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScrollBoxState {
    pub scroll_top: u16,
    pub pending_delta: i32,
    pub scroll_height: u16,
    pub viewport_height: u16,
    pub viewport_top: u16,
    pub sticky: bool,
    clamp_min: Option<u16>,
    clamp_max: Option<u16>,
}

impl ScrollBoxState {
    #[must_use]
    pub const fn new(viewport_height: u16, scroll_height: u16) -> Self {
        Self {
            scroll_top: 0,
            pending_delta: 0,
            scroll_height,
            viewport_height,
            viewport_top: 0,
            sticky: false,
            clamp_min: None,
            clamp_max: None,
        }
    }

    #[must_use]
    pub const fn max_scroll_top(self) -> u16 {
        self.scroll_height.saturating_sub(self.viewport_height)
    }

    #[must_use]
    pub fn visible_range(self) -> Range<u16> {
        let top = self.effective_scroll_top();
        let bottom = top
            .saturating_add(self.viewport_height)
            .min(self.scroll_height);
        top..bottom
    }

    pub fn scroll_to(&mut self, y: u16) {
        self.sticky = false;
        self.pending_delta = 0;
        self.scroll_top = self.clamp_scroll_top(y);
    }

    pub fn scroll_by(&mut self, delta: i32) {
        self.sticky = false;
        self.pending_delta = self.pending_delta.saturating_add(delta);
    }

    pub fn apply_pending_delta(&mut self) {
        let top = i32::from(self.scroll_top).saturating_add(self.pending_delta);
        self.scroll_top = self.clamp_scroll_top(top.max(0) as u16);
        self.pending_delta = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.pending_delta = 0;
        self.sticky = true;
        self.scroll_top = self.max_scroll_top();
    }

    pub fn set_clamp_bounds(&mut self, min: Option<u16>, max: Option<u16>) {
        self.clamp_min = min;
        self.clamp_max = max;
    }

    #[must_use]
    pub fn effective_scroll_top(self) -> u16 {
        self.clamp_scroll_top(self.scroll_top)
    }

    #[must_use]
    fn clamp_scroll_top(self, value: u16) -> u16 {
        let mut clamped = value.min(self.max_scroll_top());
        if let Some(min) = self.clamp_min {
            clamped = clamped.max(min);
        }
        if let Some(max) = self.clamp_max {
            clamped = clamped.min(max);
        }
        clamped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_delta_applies_with_scroll_limits() {
        let mut state = ScrollBoxState::new(5, 20);
        state.scroll_to(4);
        state.scroll_by(3);
        state.apply_pending_delta();

        assert_eq!(state.scroll_top, 7);
        assert_eq!(state.pending_delta, 0);
    }

    #[test]
    fn clamp_bounds_limit_effective_range() {
        let mut state = ScrollBoxState::new(5, 20);
        state.set_clamp_bounds(Some(3), Some(6));
        state.scroll_to(12);

        assert_eq!(state.effective_scroll_top(), 6);
        assert_eq!(state.visible_range(), 6..11);
    }
}
