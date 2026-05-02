//! Newline text helpers.

/// One or more line breaks emitted inside a text flow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NewlineView {
    /// Stores the count
    pub count: u16,
}

impl Default for NewlineView {
    fn default() -> Self {
        Self { count: 1 }
    }
}

impl NewlineView {
    /// Constant fn
    #[must_use]
    pub const fn new(count: u16) -> Self {
        Self { count }
    }
    /// Handles as text
    #[must_use]
    pub fn as_text(self) -> String {
        "\n".repeat(usize::from(self.count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newline_view_repeats_requested_count() {
        assert_eq!(NewlineView::new(3).as_text(), "\n\n\n");
    }
}
