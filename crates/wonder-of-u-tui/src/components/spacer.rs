//! Flexible spacer primitives.

/// A flex item that absorbs remaining space on the major axis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpacerView {
    /// Stores the flex grow
    pub flex_grow: u16,
}

impl Default for SpacerView {
    fn default() -> Self {
        Self { flex_grow: 1 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacer_defaults_to_growing() {
        assert_eq!(SpacerView::default().flex_grow, 1);
    }
}
