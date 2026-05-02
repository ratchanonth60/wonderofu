//! ANSI style and escape-policy models for renderer-independent TUI code.

use crate::style::{Color, TextStyle};

const NAMED_COLORS: [AnsiNamedColor; 16] = [
    AnsiNamedColor::Black,
    AnsiNamedColor::Red,
    AnsiNamedColor::Green,
    AnsiNamedColor::Yellow,
    AnsiNamedColor::Blue,
    AnsiNamedColor::Magenta,
    AnsiNamedColor::Cyan,
    AnsiNamedColor::White,
    AnsiNamedColor::BrightBlack,
    AnsiNamedColor::BrightRed,
    AnsiNamedColor::BrightGreen,
    AnsiNamedColor::BrightYellow,
    AnsiNamedColor::BrightBlue,
    AnsiNamedColor::BrightMagenta,
    AnsiNamedColor::BrightCyan,
    AnsiNamedColor::BrightWhite,
];

/// Controls how a renderer should treat incoming raw ANSI escape sequences.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RawAnsiPolicy {
    /// Parse ANSI into structured styles before rendering.
    #[default]
    Parse,
    /// Pass ANSI through unchanged when the active renderer can safely do so.
    Passthrough,
    /// Drop ANSI control sequences and keep only printable text.
    Strip,
}

/// Named ANSI colors from the 16-color palette.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnsiNamedColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
}

/// A semantic ANSI color value.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AnsiColor {
    Named(AnsiNamedColor),
    Indexed(u8),
    Rgb {
        r: u8,
        g: u8,
        b: u8,
    },
    #[default]
    Default,
}

impl AnsiColor {
    #[must_use]
    pub fn to_text_color(self) -> Option<Color> {
        match self {
            Self::Default | Self::Indexed(_) => None,
            Self::Rgb { r, g, b } => Some(Color::Rgb(r, g, b)),
            Self::Named(color) => Some(match color {
                AnsiNamedColor::Black => Color::Black,
                AnsiNamedColor::Red => Color::DarkRed,
                AnsiNamedColor::Green => Color::DarkGreen,
                AnsiNamedColor::Yellow => Color::DarkYellow,
                AnsiNamedColor::Blue => Color::DarkBlue,
                AnsiNamedColor::Magenta => Color::DarkMagenta,
                AnsiNamedColor::Cyan => Color::DarkCyan,
                AnsiNamedColor::White => Color::Grey,
                AnsiNamedColor::BrightBlack => Color::DarkGrey,
                AnsiNamedColor::BrightRed => Color::Red,
                AnsiNamedColor::BrightGreen => Color::Green,
                AnsiNamedColor::BrightYellow => Color::Yellow,
                AnsiNamedColor::BrightBlue => Color::Blue,
                AnsiNamedColor::BrightMagenta => Color::Magenta,
                AnsiNamedColor::BrightCyan => Color::Cyan,
                AnsiNamedColor::BrightWhite => Color::White,
            }),
        }
    }
}

impl From<Color> for AnsiColor {
    fn from(value: Color) -> Self {
        match value {
            Color::Reset => Self::Default,
            Color::Black => Self::Named(AnsiNamedColor::Black),
            Color::DarkGrey => Self::Named(AnsiNamedColor::BrightBlack),
            Color::Red => Self::Named(AnsiNamedColor::BrightRed),
            Color::DarkRed => Self::Named(AnsiNamedColor::Red),
            Color::Green => Self::Named(AnsiNamedColor::BrightGreen),
            Color::DarkGreen => Self::Named(AnsiNamedColor::Green),
            Color::Yellow => Self::Named(AnsiNamedColor::BrightYellow),
            Color::DarkYellow => Self::Named(AnsiNamedColor::Yellow),
            Color::Blue => Self::Named(AnsiNamedColor::BrightBlue),
            Color::DarkBlue => Self::Named(AnsiNamedColor::Blue),
            Color::Magenta => Self::Named(AnsiNamedColor::BrightMagenta),
            Color::DarkMagenta => Self::Named(AnsiNamedColor::Magenta),
            Color::Cyan => Self::Named(AnsiNamedColor::BrightCyan),
            Color::DarkCyan => Self::Named(AnsiNamedColor::Cyan),
            Color::Grey => Self::Named(AnsiNamedColor::White),
            Color::White => Self::Named(AnsiNamedColor::BrightWhite),
            Color::Rgb(r, g, b) => Self::Rgb { r, g, b },
        }
    }
}

/// ANSI underline variants carried by SGR state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UnderlineStyle {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// Structured ANSI style state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AnsiStyle {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: UnderlineStyle,
    pub blink: bool,
    pub inverse: bool,
    pub hidden: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub fg: AnsiColor,
    pub bg: AnsiColor,
    pub underline_color: AnsiColor,
}

impl AnsiStyle {
    /// Applies one SGR parameter string to the current style state.
    #[must_use]
    pub fn apply_sgr(mut self, params: &str) -> Self {
        let params = parse_params(params);
        let mut index = 0;

        while index < params.len() {
            let param = &params[index];
            let code = param.value.unwrap_or(0);

            match code {
                0 => self = Self::default(),
                1 => self.bold = true,
                2 => self.dim = true,
                3 => self.italic = true,
                4 => {
                    self.underline = if param.uses_colons {
                        param
                            .subparams
                            .first()
                            .copied()
                            .map(UnderlineStyle::from_sgr_subparam)
                            .unwrap_or(UnderlineStyle::Single)
                    } else {
                        UnderlineStyle::Single
                    };
                }
                5 | 6 => self.blink = true,
                7 => self.inverse = true,
                8 => self.hidden = true,
                9 => self.strikethrough = true,
                21 => self.underline = UnderlineStyle::Double,
                22 => {
                    self.bold = false;
                    self.dim = false;
                }
                23 => self.italic = false,
                24 => self.underline = UnderlineStyle::None,
                25 => self.blink = false,
                27 => self.inverse = false,
                28 => self.hidden = false,
                29 => self.strikethrough = false,
                30..=37 => self.fg = AnsiColor::Named(NAMED_COLORS[usize::from(code - 30)]),
                39 => self.fg = AnsiColor::Default,
                40..=47 => self.bg = AnsiColor::Named(NAMED_COLORS[usize::from(code - 40)]),
                49 => self.bg = AnsiColor::Default,
                53 => self.overline = true,
                55 => self.overline = false,
                58 => {
                    if let Some((color, consumed)) = parse_extended_color(&params, index) {
                        self.underline_color = color;
                        index += consumed.saturating_sub(1);
                    }
                }
                59 => self.underline_color = AnsiColor::Default,
                90..=97 => self.fg = AnsiColor::Named(NAMED_COLORS[usize::from(code - 90 + 8)]),
                100..=107 => self.bg = AnsiColor::Named(NAMED_COLORS[usize::from(code - 100 + 8)]),
                38 => {
                    if let Some((color, consumed)) = parse_extended_color(&params, index) {
                        self.fg = color;
                        index += consumed.saturating_sub(1);
                    }
                }
                48 => {
                    if let Some((color, consumed)) = parse_extended_color(&params, index) {
                        self.bg = color;
                        index += consumed.saturating_sub(1);
                    }
                }
                _ => {}
            }

            index += 1;
        }

        self
    }

    #[must_use]
    pub fn to_text_style(self) -> TextStyle {
        let mut style = TextStyle::default();

        if let Some(fg) = self.fg.to_text_color() {
            style = style.fg(fg);
        }
        if let Some(bg) = self.bg.to_text_color() {
            style = style.bg(bg);
        }
        if self.bold {
            style = style.bold();
        }
        if self.dim {
            style = style.dim();
        }
        if self.italic {
            style = style.italic();
        }
        if self.underline != UnderlineStyle::None {
            style = style.underlined();
        }
        if self.inverse {
            style = style.reversed();
        }

        style
    }
}

impl From<TextStyle> for AnsiStyle {
    fn from(value: TextStyle) -> Self {
        Self {
            bold: value.bold,
            dim: value.dim,
            italic: value.italic,
            underline: if value.underlined {
                UnderlineStyle::Single
            } else {
                UnderlineStyle::None
            },
            inverse: value.reversed,
            fg: value.fg.map_or(AnsiColor::Default, AnsiColor::from),
            bg: value.bg.map_or(AnsiColor::Default, AnsiColor::from),
            ..Self::default()
        }
    }
}

/// Applies SGR parameters and converts the resulting ANSI state to the crate's text style.
#[must_use]
pub fn apply_sgr_to_text_style(params: &str, base: TextStyle) -> TextStyle {
    AnsiStyle::from(base).apply_sgr(params).to_text_style()
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedParam {
    value: Option<u16>,
    subparams: Vec<u16>,
    uses_colons: bool,
}

impl UnderlineStyle {
    fn from_sgr_subparam(value: u16) -> Self {
        match value {
            2 => Self::Double,
            3 => Self::Curly,
            4 => Self::Dotted,
            5 => Self::Dashed,
            _ => Self::Single,
        }
    }
}

fn parse_params(input: &str) -> Vec<ParsedParam> {
    if input.is_empty() {
        return vec![ParsedParam {
            value: Some(0),
            subparams: Vec::new(),
            uses_colons: false,
        }];
    }

    let mut params = Vec::new();
    let mut current = ParsedParam {
        value: None,
        subparams: Vec::new(),
        uses_colons: false,
    };
    let mut digits = String::new();
    let mut in_subparams = false;

    for ch in input.chars().chain(std::iter::once(';')) {
        match ch {
            ';' => {
                let parsed = (!digits.is_empty())
                    .then(|| digits.parse::<u16>().ok())
                    .flatten();
                if in_subparams {
                    if let Some(value) = parsed {
                        current.subparams.push(value);
                    }
                } else {
                    current.value = parsed;
                }
                params.push(current);
                current = ParsedParam {
                    value: None,
                    subparams: Vec::new(),
                    uses_colons: false,
                };
                digits.clear();
                in_subparams = false;
            }
            ':' => {
                let parsed = (!digits.is_empty())
                    .then(|| digits.parse::<u16>().ok())
                    .flatten();
                if in_subparams {
                    if let Some(value) = parsed {
                        current.subparams.push(value);
                    }
                } else {
                    current.value = parsed;
                    current.uses_colons = true;
                    in_subparams = true;
                }
                digits.clear();
            }
            '0'..='9' => digits.push(ch),
            _ => {}
        }
    }

    params
}

fn parse_extended_color(params: &[ParsedParam], index: usize) -> Option<(AnsiColor, usize)> {
    let param = params.get(index)?;

    if param.uses_colons && !param.subparams.is_empty() {
        match param.subparams[0] {
            5 if param.subparams.len() >= 2 => {
                return Some((
                    AnsiColor::Indexed(u8::try_from(param.subparams[1]).ok()?),
                    1,
                ));
            }
            2 if param.subparams.len() >= 4 => {
                let offset = usize::from(param.subparams.len() >= 5);
                return Some((
                    AnsiColor::Rgb {
                        r: u8::try_from(*param.subparams.get(1 + offset)?).ok()?,
                        g: u8::try_from(*param.subparams.get(2 + offset)?).ok()?,
                        b: u8::try_from(*param.subparams.get(3 + offset)?).ok()?,
                    },
                    1,
                ));
            }
            _ => {}
        }
    }

    match params.get(index + 1)?.value? {
        5 => Some((
            AnsiColor::Indexed(u8::try_from(params.get(index + 2)?.value?).ok()?),
            3,
        )),
        2 => Some((
            AnsiColor::Rgb {
                r: u8::try_from(params.get(index + 2)?.value?).ok()?,
                g: u8::try_from(params.get(index + 3)?.value?).ok()?,
                b: u8::try_from(params.get(index + 4)?.value?).ok()?,
            },
            5,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_parser_handles_named_and_truecolor_sequences() {
        let style = AnsiStyle::default().apply_sgr("1;31;48;2;12;34;56");

        assert!(style.bold);
        assert_eq!(style.fg, AnsiColor::Named(AnsiNamedColor::Red));
        assert_eq!(
            style.bg,
            AnsiColor::Rgb {
                r: 12,
                g: 34,
                b: 56
            }
        );
    }

    #[test]
    fn sgr_parser_supports_colon_forms_and_resets() {
        let style = AnsiStyle::default().apply_sgr("4:3;58:2::10:20:30;38:5:200;24;59");

        assert_eq!(style.underline, UnderlineStyle::None);
        assert_eq!(style.underline_color, AnsiColor::Default);
        assert_eq!(style.fg, AnsiColor::Indexed(200));
    }

    #[test]
    fn ansi_style_maps_to_text_style_palette() {
        let style = AnsiStyle::default().apply_sgr("91;104;3;4").to_text_style();

        assert_eq!(style.fg, Some(Color::Red));
        assert_eq!(style.bg, Some(Color::Blue));
        assert!(style.italic);
        assert!(style.underlined);
    }

    #[test]
    fn text_style_round_trips_through_ansi_style() {
        let style = TextStyle::default()
            .fg(Color::DarkCyan)
            .bg(Color::Rgb(1, 2, 3))
            .bold()
            .dim()
            .italic()
            .underlined()
            .reversed();

        let ansi = AnsiStyle::from(style);

        assert_eq!(ansi.fg, AnsiColor::Named(AnsiNamedColor::Cyan));
        assert_eq!(ansi.bg, AnsiColor::Rgb { r: 1, g: 2, b: 3 });
        assert!(ansi.bold);
        assert!(ansi.dim);
        assert!(ansi.italic);
        assert_eq!(ansi.underline, UnderlineStyle::Single);
        assert!(ansi.inverse);
    }

    #[test]
    fn apply_sgr_to_text_style_preserves_existing_flags_until_reset() {
        let base = TextStyle::default().bold().fg(Color::Green);

        let styled = apply_sgr_to_text_style("3;4", base);
        let reset = apply_sgr_to_text_style("0;31", styled);

        assert!(styled.bold);
        assert!(styled.italic);
        assert!(styled.underlined);
        assert_eq!(styled.fg, Some(Color::Green));

        assert!(!reset.bold);
        assert_eq!(reset.fg, Some(Color::DarkRed));
    }

    #[test]
    fn raw_ansi_policy_defaults_to_parsing() {
        assert_eq!(RawAnsiPolicy::default(), RawAnsiPolicy::Parse);
    }
}
