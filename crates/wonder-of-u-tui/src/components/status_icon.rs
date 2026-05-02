//! Status icon view model — mirrors `claude-leak/components/design-system/StatusIcon.tsx`.

/// Visual state of a status icon (spinner, success, error, etc.).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum StatusIconKind {
    #[default]
    Idle,
    Running,
    Success,
    Warning,
    Error,
    Paused,
}

impl StatusIconKind {
    /// Returns the single-character symbol for this status.
    #[must_use]
    pub const fn symbol(self) -> char {
        match self {
            Self::Idle => '·',
            Self::Running => '◎',
            Self::Success => '✓',
            Self::Warning => '⚠',
            Self::Error => '✗',
            Self::Paused => '⏸',
        }
    }

    /// Returns the theme color key for this status.
    #[must_use]
    pub const fn color_key(self) -> &'static str {
        match self {
            Self::Idle => "secondaryText",
            Self::Running => "claude",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Paused => "secondaryText",
        }
    }
}

/// Spinner frame sequence (braille dots).
pub const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// View model for a status icon / spinner.
#[derive(Clone, Debug, Default)]
pub struct StatusIconView {
    pub kind: StatusIconKind,
    /// Current spinner frame index (used only when `kind == Running`).
    pub spinner_frame: usize,
}

impl StatusIconView {
    #[must_use]
    pub fn new(kind: StatusIconKind) -> Self {
        Self {
            kind,
            spinner_frame: 0,
        }
    }

    /// Advance the spinner frame by one tick.
    pub fn tick(&mut self) {
        self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
    }

    /// The character to render (spinner when running, static otherwise).
    #[must_use]
    pub fn render_char(&self) -> char {
        if self.kind == StatusIconKind::Running {
            SPINNER_FRAMES[self.spinner_frame % SPINNER_FRAMES.len()]
        } else {
            self.kind.symbol()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_advances() {
        let mut icon = StatusIconView::new(StatusIconKind::Running);
        let first = icon.render_char();
        icon.tick();
        let second = icon.render_char();
        assert_ne!(first, second);
    }

    #[test]
    fn success_does_not_animate() {
        let mut icon = StatusIconView::new(StatusIconKind::Success);
        let before = icon.render_char();
        icon.tick();
        assert_eq!(before, icon.render_char());
    }
}
