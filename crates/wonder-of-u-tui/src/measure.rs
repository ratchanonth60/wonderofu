//! Text measurement helpers for terminal layout.
//!
//! These helpers measure visible terminal width instead of counting Unicode
//! scalar values, and ignore ANSI escape sequences when possible.

use std::{borrow::Cow, mem};

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Visible text dimensions in terminal cells.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextMeasurement {
    /// Stores the width
    pub width: usize,
    /// Stores the height
    pub height: usize,
}

/// Removes ANSI escape sequences from text.
#[must_use]
pub fn strip_ansi(text: &str) -> Cow<'_, str> {
    if !text.as_bytes().contains(&b'\x1b') {
        return Cow::Borrowed(text);
    }

    let bytes = text.as_bytes();
    let mut stripped = String::with_capacity(text.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == 0x1b {
            index += 1;
            if index >= bytes.len() {
                break;
            }

            match bytes[index] {
                b'[' => {
                    index += 1;
                    while index < bytes.len() {
                        let byte = bytes[index];
                        index += 1;
                        if (0x40..=0x7e).contains(&byte) {
                            break;
                        }
                    }
                }
                b']' => {
                    index += 1;
                    while index < bytes.len() {
                        match bytes[index] {
                            0x07 => {
                                index += 1;
                                break;
                            }
                            0x1b if bytes.get(index + 1) == Some(&b'\\') => {
                                index += 2;
                                break;
                            }
                            _ => index += 1,
                        }
                    }
                }
                _ => {
                    index += 1;
                }
            }

            continue;
        }

        let mut chars = text[index..].chars();
        if let Some(ch) = chars.next() {
            stripped.push(ch);
            index += ch.len_utf8();
        } else {
            break;
        }
    }

    Cow::Owned(stripped)
}

/// Returns the visible width of a single logical line.
#[must_use]
pub fn line_width(line: &str) -> usize {
    UnicodeWidthStr::width(strip_ansi(line).as_ref())
}

/// Returns the widest visible line in a multi-line string.
#[must_use]
pub fn widest_line(text: &str) -> usize {
    text.split('\n').map(line_width).max().unwrap_or(0)
}

/// Measures visible width and wrapped height for text.
#[must_use]
pub fn measure_text(text: &str, max_width: usize) -> TextMeasurement {
    if text.is_empty() {
        return TextMeasurement::default();
    }

    let height = if max_width == 0 {
        text.split('\n').count()
    } else {
        wrap_text_hard(text, max_width).len()
    };

    TextMeasurement {
        width: widest_line(text),
        height,
    }
}

/// Hard-wraps text by visible terminal cell width.
#[must_use]
pub fn wrap_text_hard(text: &str, max_width: usize) -> Vec<String> {
    let stripped = strip_ansi(text);
    let text = stripped.as_ref();

    if max_width == 0 {
        return text.split('\n').map(ToString::to_string).collect();
    }

    let mut lines = Vec::new();

    for logical_line in text.split('\n') {
        if logical_line.is_empty() {
            lines.push(String::new());
            continue;
        }

        let mut current = String::new();
        let mut current_width = 0usize;

        for grapheme in logical_line.graphemes(true) {
            let grapheme_width = UnicodeWidthStr::width(grapheme);

            if grapheme_width > 0
                && current_width > 0
                && current_width.saturating_add(grapheme_width) > max_width
            {
                lines.push(mem::take(&mut current));
                current_width = 0;
            }

            current.push_str(grapheme);
            current_width = current_width.saturating_add(grapheme_width);

            if current_width >= max_width {
                lines.push(mem::take(&mut current));
                current_width = 0;
            }
        }

        if !current.is_empty() {
            lines.push(current);
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: &str = "\u{1b}[31m";
    const RESET: &str = "\u{1b}[0m";

    #[test]
    fn strip_ansi_removes_csi_and_osc_sequences() {
        let text = format!("{RED}red{RESET}\u{1b}]8;;https://example.com\u{7}link\u{1b}]8;;\u{7}");

        assert_eq!(strip_ansi(&text), "redlink");
    }

    #[test]
    fn line_width_handles_ascii_thai_and_wide_unicode() {
        assert_eq!(line_width("hello"), 5);
        assert_eq!(line_width("วั"), 1);
        assert_eq!(line_width("コン"), 4);
    }

    #[test]
    fn line_width_ignores_ansi_sequences() {
        assert_eq!(line_width(&format!("{RED}warn{RESET}")), 4);
    }

    #[test]
    fn widest_line_uses_visible_width() {
        let text = format!("ok\n{RED}コン{RESET}\nno");

        assert_eq!(widest_line(&text), 4);
    }

    #[test]
    fn measure_text_counts_wrapped_height_for_emoji() {
        let measurement = measure_text("😄x", 2);

        assert_eq!(
            measurement,
            TextMeasurement {
                width: 3,
                height: 2,
            }
        );
    }

    #[test]
    fn measure_text_returns_zero_for_empty_text() {
        assert_eq!(measure_text("", 10), TextMeasurement::default());
    }

    #[test]
    fn wrap_text_hard_preserves_empty_lines() {
        assert_eq!(wrap_text_hard("a\n\nb", 10), vec!["a", "", "b"]);
    }

    #[test]
    fn wrap_text_hard_splits_ascii_lines() {
        assert_eq!(wrap_text_hard("abcdef", 4), vec!["abcd", "ef"]);
    }

    #[test]
    fn wrap_text_hard_splits_long_words_by_display_width() {
        assert_eq!(
            wrap_text_hard("コンピュータ", 4),
            vec!["コン", "ピュ", "ータ"]
        );
    }

    #[test]
    fn wrap_text_hard_strips_ansi_before_wrapping() {
        let text = format!("{RED}abcd{RESET}ef");

        assert_eq!(wrap_text_hard(&text, 3), vec!["abc", "def"]);
    }
}
