//! Text primitives and text-local styling metadata.

use crate::style::{Color, TextStyle};

/// Wrapping and truncation behavior for text content.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextWrap {
    /// Represents wrap
    #[default]
    Wrap,
    /// Represents wrap trim
    WrapTrim,
    /// Represents end
    End,
    /// Represents middle
    Middle,
    /// Represents truncate end
    TruncateEnd,
    /// Represents truncate
    Truncate,
    /// Represents truncate middle
    TruncateMiddle,
    /// Represents truncate start
    TruncateStart,
}

/// Terminal weight choices. Ink treats bold and dim as mutually exclusive.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextWeight {
    /// Represents normal
    #[default]
    Normal,
    /// Represents bold
    Bold,
    /// Represents dim
    Dim,
}

/// Text-specific styling that extends the crate's shared `TextStyle`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextAttributes {
    /// Stores the color
    pub color: Option<Color>,
    /// Stores the background color
    pub background_color: Option<Color>,
    /// Stores the weight
    pub weight: TextWeight,
    /// Stores the italic
    pub italic: bool,
    /// Stores the underline
    pub underline: bool,
    /// Stores the strikethrough
    pub strikethrough: bool,
    /// Stores the inverse
    pub inverse: bool,
}

impl TextAttributes {
    /// Constant fn
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
    /// Stores the content
    pub content: String,
    /// Stores the wrap
    pub wrap: TextWrap,
    /// Stores the attributes
    pub attributes: TextAttributes,
}

impl TextView {
    /// Creates a new value
    #[must_use]
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            wrap: TextWrap::Wrap,
            attributes: TextAttributes::default(),
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn wrap(mut self, wrap: TextWrap) -> Self {
        self.wrap = wrap;
        self
    }
    /// Constant fn
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
