use crossterm::style::Color as CrosstermColor;
/// Enumerates color
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Color {
    /// Represents reset
    Reset,
    /// Represents black
    Black,
    /// Represents dark grey
    DarkGrey,
    /// Represents red
    Red,
    /// Represents dark red
    DarkRed,
    /// Represents green
    Green,
    /// Represents dark green
    DarkGreen,
    /// Represents yellow
    Yellow,
    /// Represents dark yellow
    DarkYellow,
    /// Represents blue
    Blue,
    /// Represents dark blue
    DarkBlue,
    /// Represents magenta
    Magenta,
    /// Represents dark magenta
    DarkMagenta,
    /// Represents cyan
    Cyan,
    /// Represents dark cyan
    DarkCyan,
    /// Represents grey
    Grey,
    /// Represents white
    White,
    /// Represents rgb
    Rgb(u8, u8, u8),
}

impl From<Color> for CrosstermColor {
    fn from(value: Color) -> Self {
        match value {
            Color::Reset => Self::Reset,
            Color::Black => Self::Black,
            Color::DarkGrey => Self::DarkGrey,
            Color::Red => Self::Red,
            Color::DarkRed => Self::DarkRed,
            Color::Green => Self::Green,
            Color::DarkGreen => Self::DarkGreen,
            Color::Yellow => Self::Yellow,
            Color::DarkYellow => Self::DarkYellow,
            Color::Blue => Self::Blue,
            Color::DarkBlue => Self::DarkBlue,
            Color::Magenta => Self::Magenta,
            Color::DarkMagenta => Self::DarkMagenta,
            Color::Cyan => Self::Cyan,
            Color::DarkCyan => Self::DarkCyan,
            Color::Grey => Self::Grey,
            Color::White => Self::White,
            Color::Rgb(r, g, b) => Self::Rgb { r, g, b },
        }
    }
}
/// Represents text style
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextStyle {
    /// Stores the fg
    pub fg: Option<Color>,
    /// Stores the bg
    pub bg: Option<Color>,
    /// Stores the bold
    pub bold: bool,
    /// Stores the dim
    pub dim: bool,
    /// Stores the italic
    pub italic: bool,
    /// Stores the underlined
    pub underlined: bool,
    /// Stores the reversed
    pub reversed: bool,
}

impl TextStyle {
    /// Constant fn
    #[must_use]
    pub const fn fg(mut self, color: Color) -> Self {
        self.fg = Some(color);
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn underlined(mut self) -> Self {
        self.underlined = true;
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn reversed(mut self) -> Self {
        self.reversed = true;
        self
    }
}
/// Represents theme
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Theme {
    /// Stores the background
    pub background: TextStyle,
    /// Stores the border
    pub border: TextStyle,
    /// Stores the title
    pub title: TextStyle,
    /// Stores the messages
    pub messages: TextStyle,
    /// Stores the prompt
    pub prompt: TextStyle,
    /// Stores the status
    pub status: TextStyle,
    /// Stores the footer
    pub footer: TextStyle,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            // No explicit background colour: let the terminal emulator's own
            // background show through (Ink/Claude Code visual parity).
            background: TextStyle::default().fg(Color::Grey),
            border: TextStyle::default().fg(Color::DarkBlue),
            title: TextStyle::default().fg(Color::Blue).bold(),
            messages: TextStyle::default().fg(Color::White),
            prompt: TextStyle::default().fg(Color::Cyan),
            status: TextStyle::default().fg(Color::Yellow).bold(),
            footer: TextStyle::default().fg(Color::DarkGrey),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_builder_sets_expected_flags() {
        let style = TextStyle::default()
            .fg(Color::Green)
            .bg(Color::Black)
            .bold()
            .underlined()
            .reversed();

        assert_eq!(style.fg, Some(Color::Green));
        assert_eq!(style.bg, Some(Color::Black));
        assert!(style.bold);
        assert!(style.underlined);
        assert!(style.reversed);
        assert!(!style.italic);
    }

    #[test]
    fn default_theme_uses_distinct_shell_regions() {
        let theme = Theme::default();

        assert_ne!(theme.messages.fg, theme.prompt.fg);
        assert_ne!(theme.status.fg, theme.footer.fg);
        assert!(theme.title.bold);
    }
}
