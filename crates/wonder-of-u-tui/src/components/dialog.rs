//! Dialog view model — mirrors `claude-leak/components/design-system/Dialog.tsx`.

/// State for a modal dialog overlay.
#[derive(Clone, Debug)]
pub struct DialogView {
    /// Title bar text.
    pub title: String,
    /// Optional subtitle shown below the title.
    pub subtitle: Option<String>,
    /// Whether Esc/n cancel keybindings are active.
    pub is_cancel_active: bool,
    /// Whether to show the bottom input-guide bar.
    pub show_input_guide: bool,
    /// Whether to draw a border around the dialog.
    pub show_border: bool,
    /// Theme color key (from the app theme map).
    pub color: Option<String>,
}

impl DialogView {
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            is_cancel_active: true,
            show_input_guide: true,
            show_border: true,
            color: None,
        }
    }

    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    #[must_use]
    pub fn color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }

    #[must_use]
    pub fn hide_border(mut self) -> Self {
        self.show_border = false;
        self
    }

    #[must_use]
    pub fn disable_cancel(mut self) -> Self {
        self.is_cancel_active = false;
        self
    }
}
