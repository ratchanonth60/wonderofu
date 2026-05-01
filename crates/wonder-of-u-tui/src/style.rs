use crossterm::style::Color as CrosstermColor;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Color {
    Reset,
    Black,
    DarkGrey,
    Red,
    DarkRed,
    Green,
    DarkGreen,
    Yellow,
    DarkYellow,
    Blue,
    DarkBlue,
    Magenta,
    DarkMagenta,
    Cyan,
    DarkCyan,
    Grey,
    White,
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextStyle {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underlined: bool,
    pub reversed: bool,
}

impl TextStyle {
    #[must_use]
    pub const fn fg(mut self, color: Color) -> Self {
        self.fg = Some(color);
        self
    }

    #[must_use]
    pub const fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }

    #[must_use]
    pub const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    #[must_use]
    pub const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    #[must_use]
    pub const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    #[must_use]
    pub const fn underlined(mut self) -> Self {
        self.underlined = true;
        self
    }

    #[must_use]
    pub const fn reversed(mut self) -> Self {
        self.reversed = true;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Theme {
    pub background: TextStyle,
    pub border: TextStyle,
    pub title: TextStyle,
    pub messages: TextStyle,
    pub prompt: TextStyle,
    pub status: TextStyle,
    pub footer: TextStyle,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: TextStyle::default().bg(Color::Black).fg(Color::Grey),
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
