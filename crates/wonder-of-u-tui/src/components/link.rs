//! Hyperlink-friendly text primitives.

/// Link label, destination, and fallback text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkView {
    pub label: String,
    pub url: String,
    pub fallback_label: Option<String>,
}

impl LinkView {
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        let url = url.into();
        Self {
            label: url.clone(),
            url,
            fallback_label: None,
        }
    }

    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    #[must_use]
    pub fn with_fallback(mut self, fallback_label: impl Into<String>) -> Self {
        self.fallback_label = Some(fallback_label.into());
        self
    }

    #[must_use]
    pub fn display_label(&self, supports_hyperlinks: bool) -> &str {
        if supports_hyperlinks {
            &self.label
        } else {
            self.fallback_label.as_deref().unwrap_or(&self.label)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_uses_url_as_default_label() {
        let link = LinkView::new("https://example.invalid");

        assert_eq!(link.label, "https://example.invalid");
    }

    #[test]
    fn link_uses_fallback_when_hyperlinks_are_disabled() {
        let link = LinkView::new("https://example.invalid")
            .with_label("Docs")
            .with_fallback("Docs (https://example.invalid)");

        assert_eq!(link.display_label(true), "Docs");
        assert_eq!(link.display_label(false), "Docs (https://example.invalid)");
    }
}
