//! Divider view model — mirrors `claude-leak/components/design-system/Divider.tsx`.

/// Orientation of a divider line.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum DividerOrientation {
    /// Represents horizontal
    #[default]
    Horizontal,
    /// Represents vertical
    Vertical,
}

/// View model for a divider / separator widget.
#[derive(Clone, Debug)]
pub struct DividerView {
    /// Stores the orientation
    pub orientation: DividerOrientation,
    /// Fill character. Defaults to `'─'` (horizontal) or `'│'` (vertical).
    pub fill_char: Option<char>,
    /// Theme color key.
    pub color: Option<String>,
    /// Explicit length in columns/rows. `None` → fills available space.
    pub length: Option<u16>,
    /// Optional label to embed in the middle of the divider.
    pub label: Option<String>,
}

impl DividerView {
    /// Handles horizontal
    #[must_use]
    pub fn horizontal() -> Self {
        Self {
            orientation: DividerOrientation::Horizontal,
            fill_char: None,
            color: None,
            length: None,
            label: None,
        }
    }
    /// Handles vertical
    #[must_use]
    pub fn vertical() -> Self {
        Self {
            orientation: DividerOrientation::Vertical,
            fill_char: None,
            color: None,
            length: None,
            label: None,
        }
    }
    /// Handles label
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
    /// Handles color
    #[must_use]
    pub fn color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Returns the fill character appropriate for the orientation.
    #[must_use]
    pub fn effective_fill_char(&self) -> char {
        self.fill_char.unwrap_or(match self.orientation {
            DividerOrientation::Horizontal => '─',
            DividerOrientation::Vertical => '│',
        })
    }

    /// Render the divider as a string of the given length, embedding a label
    /// in the center when present.
    #[must_use]
    pub fn render(&self, total_len: u16) -> String {
        let fill = self.effective_fill_char();
        let total = total_len as usize;
        if let Some(ref lbl) = self.label {
            let label_len = lbl.len() + 2; // " label "
            if label_len + 2 <= total {
                let sides = total - label_len;
                let left = sides / 2;
                let right = sides - left;
                return format!(
                    "{} {} {}",
                    fill.to_string().repeat(left),
                    lbl,
                    fill.to_string().repeat(right)
                );
            }
        }
        fill.to_string().repeat(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_fill() {
        let d = DividerView::horizontal();
        assert_eq!(d.render(5), "─────");
    }

    #[test]
    fn label_centered() {
        let d = DividerView::horizontal().label("hi");
        let out = d.render(10);
        assert!(out.contains("hi"));
    }
}
