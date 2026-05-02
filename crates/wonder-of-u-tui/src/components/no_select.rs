//! Non-selectable content markers.

/// Selection exclusion behavior for a no-select region.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NoSelectMode {
    #[default]
    CurrentBounds,
    FromLeftEdge,
}

/// Marker used to exclude a region from fullscreen text selection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoSelectView {
    pub mode: NoSelectMode,
}

impl NoSelectView {
    #[must_use]
    pub const fn from_left_edge() -> Self {
        Self {
            mode: NoSelectMode::FromLeftEdge,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_left_edge_uses_wider_exclusion_mode() {
        assert_eq!(
            NoSelectView::from_left_edge().mode,
            NoSelectMode::FromLeftEdge
        );
    }
}
