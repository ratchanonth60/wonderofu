//! Text primitives and text-local styling metadata.

use crate::style::{Color, TextStyle};

/// Wrapping and truncation behavior for text content.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextWrap {
    #[default]
    Wrap,
    WrapTrim,
    End,
    Middle,
    TruncateEnd,
    Truncate,
    TruncateMiddle,
    TruncateStart,
}

/// Terminal weight choices. Ink treats bold and dim as mutually exclusive.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextWeight {
    #[default]
    Normal,
    Bold,
    Dim,
}

/// Text-specific styling that extends the crate's shared `TextStyle`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextAttributes {
    pub color: Option<Color>,
    pub background_color: Option<Color>,
    pub weight: TextWeight,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub inverse: bool,
}

impl TextAttributes {
    #[must_use]
    pub const fn base_style(self) -> TextStyle {
        let mut style = TextStyle {
            fg: self.color,
            bg: self.background_color,
            bold: matches!(self.weight, TextWeight::Bold),
            dim: matches!(self.weight, TextWeight::Dim),
            italic: self.italic,
            underlined: self.underline,
            reversed: self.inverse,
        };
        if matches!(self.weight, TextWeight::Bold) {
            style.dim = false;
        }
        style
    }
}

/// Text content plus wrapping and style metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextView {
    pub content: String,
    pub wrap: TextWrap,
    pub attributes: TextAttributes,
}

impl TextView {
    #[must_use]
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            wrap: TextWrap::Wrap,
            attributes: TextAttributes::default(),
        }
    }

    #[must_use]
    pub const fn wrap(mut self, wrap: TextWrap) -> Self {
        self.wrap = wrap;
        self
    }

    #[must_use]
    pub const fn attributes(mut self, attributes: TextAttributes) -> Self {
        self.attributes = attributes;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_weight_excludes_dim_in_base_style() {
        let attributes = TextAttributes {
            color: Some(Color::Green),
            weight: TextWeight::Bold,
            italic: true,
            underline: true,
            inverse: true,
            strikethrough: true,
            background_color: Some(Color::Black),
        };
        let style = attributes.base_style();

        assert_eq!(style.fg, Some(Color::Green));
        assert_eq!(style.bg, Some(Color::Black));
        assert!(style.bold);
        assert!(!style.dim);
        assert!(style.italic);
        assert!(style.underlined);
        assert!(style.reversed);
    }

    #[test]
    fn text_view_defaults_to_wrapping_content() {
        let text = TextView::new("hello");

        assert_eq!(text.wrap, TextWrap::Wrap);
        assert_eq!(text.content, "hello");
    }
}
