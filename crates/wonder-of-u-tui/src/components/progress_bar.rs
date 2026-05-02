//! Progress bar view model — mirrors `claude-leak/components/design-system/ProgressBar.tsx`.

/// Direction that a progress bar fills.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum ProgressDirection {
    #[default]
    LeftToRight,
    RightToLeft,
}

/// View model for a horizontal progress bar.
#[derive(Clone, Debug)]
pub struct ProgressBarView {
    /// Current progress as a fraction in `[0.0, 1.0]`.
    pub fraction: f32,
    /// Width in terminal columns. `None` uses the available layout width.
    pub width: Option<u16>,
    /// Color key for the filled portion.
    pub color: Option<String>,
    /// Color key for the unfilled portion.
    pub background_color: Option<String>,
    /// Direction the bar fills.
    pub direction: ProgressDirection,
    /// Whether to show a percentage label.
    pub show_label: bool,
}

impl ProgressBarView {
    /// Create a progress bar with the given fraction (clamped to `[0, 1]`).
    #[must_use]
    pub fn new(fraction: f32) -> Self {
        Self {
            fraction: fraction.clamp(0.0, 1.0),
            width: None,
            color: None,
            background_color: None,
            direction: ProgressDirection::LeftToRight,
            show_label: false,
        }
    }

    #[must_use]
    pub fn width(mut self, width: u16) -> Self {
        self.width = Some(width);
        self
    }

    #[must_use]
    pub fn color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }

    #[must_use]
    pub fn show_label(mut self) -> Self {
        self.show_label = true;
        self
    }

    /// Returns the percentage as an integer string (e.g. `"42%"`).
    #[must_use]
    pub fn label_text(&self) -> String {
        format!("{}%", (self.fraction * 100.0).round() as u32)
    }

    /// Compute filled and unfilled column counts for a given total width.
    #[must_use]
    pub fn columns(&self, total: u16) -> (u16, u16) {
        let filled = (f32::from(total) * self.fraction).round() as u16;
        let filled = filled.min(total);
        (filled, total - filled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_text() {
        assert_eq!(ProgressBarView::new(0.5).label_text(), "50%");
        assert_eq!(ProgressBarView::new(1.0).label_text(), "100%");
        assert_eq!(ProgressBarView::new(0.0).label_text(), "0%");
    }

    #[test]
    fn columns_split() {
        let bar = ProgressBarView::new(0.25);
        let (filled, empty) = bar.columns(20);
        assert_eq!(filled, 5);
        assert_eq!(empty, 15);
    }
}
