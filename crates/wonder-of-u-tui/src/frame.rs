use crate::style::TextStyle;
/// Represents rect
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rect {
    /// Stores the x
    pub x: u16,
    /// Stores the y
    pub y: u16,
    /// Stores the width
    pub width: u16,
    /// Stores the height
    pub height: u16,
}

impl Rect {
    /// Constant fn
    #[must_use]
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn right(self) -> u16 {
        self.x.saturating_add(self.width)
    }
    /// Constant fn
    #[must_use]
    pub const fn bottom(self) -> u16 {
        self.y.saturating_add(self.height)
    }

    /// Returns `true` when `(col, row)` lies inside the rectangle.
    ///
    /// Uses half-open intervals `[x, x+width)` × `[y, y+height)`, which
    /// matches terminal coordinate conventions.
    #[must_use]
    pub const fn contains(self, col: u16, row: u16) -> bool {
        col >= self.x && col < self.right() && row >= self.y && row < self.bottom()
    }
    /// Constant fn
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
    /// Constant fn
    #[must_use]
    pub const fn inset(self, margin: u16) -> Self {
        let offset = margin.saturating_mul(2);
        Self {
            x: self.x.saturating_add(margin),
            y: self.y.saturating_add(margin),
            width: self.width.saturating_sub(offset),
            height: self.height.saturating_sub(offset),
        }
    }
}
/// Represents cell
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cell {
    /// Stores the symbol
    pub symbol: char,
    /// Stores the style
    pub style: TextStyle,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            symbol: ' ',
            style: TextStyle::default(),
        }
    }
}
/// Represents frame buffer
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameBuffer {
    width: u16,
    height: u16,
    cells: Vec<Cell>,
}

impl FrameBuffer {
    /// Creates a new value
    #[must_use]
    pub fn new(width: u16, height: u16) -> Self {
        let size = usize::from(width) * usize::from(height);
        Self {
            width,
            height,
            cells: vec![Cell::default(); size],
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn width(&self) -> u16 {
        self.width
    }
    /// Constant fn
    #[must_use]
    pub const fn height(&self) -> u16 {
        self.height
    }
    /// Constant fn
    #[must_use]
    pub const fn area(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }
    /// Handles cell
    #[must_use]
    pub fn cell(&self, x: u16, y: u16) -> Option<&Cell> {
        self.index(x, y).map(|index| &self.cells[index])
    }

    /// Handles put
    pub fn put(&mut self, x: u16, y: u16, symbol: char, style: TextStyle) {
        if let Some(index) = self.index(x, y) {
            self.cells[index] = Cell { symbol, style };
        }
    }

    /// Handles fill rect
    pub fn fill_rect(&mut self, area: Rect, symbol: char, style: TextStyle) {
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                self.put(x, y, symbol, style);
            }
        }
    }

    /// Writes str
    pub fn write_str(&mut self, x: u16, y: u16, text: &str, style: TextStyle, max_width: u16) {
        if y >= self.height || x >= self.width || max_width == 0 {
            return;
        }

        let mut cursor_x = x;
        for symbol in text.chars().take(usize::from(max_width)) {
            if cursor_x >= self.width {
                break;
            }
            self.put(cursor_x, y, symbol, style);
            cursor_x = cursor_x.saturating_add(1);
        }
    }

    /// Handles draw border
    pub fn draw_border(&mut self, area: Rect, style: TextStyle) {
        if area.is_empty() {
            return;
        }

        if area.width == 1 && area.height == 1 {
            self.put(area.x, area.y, '+', style);
            return;
        }

        if area.height == 1 {
            for x in area.x..area.right() {
                self.put(x, area.y, '-', style);
            }
            return;
        }

        if area.width == 1 {
            for y in area.y..area.bottom() {
                self.put(area.x, y, '|', style);
            }
            return;
        }

        let right = area.right().saturating_sub(1);
        let bottom = area.bottom().saturating_sub(1);

        self.put(area.x, area.y, '+', style);
        self.put(right, area.y, '+', style);
        self.put(area.x, bottom, '+', style);
        self.put(right, bottom, '+', style);

        for x in area.x.saturating_add(1)..right {
            self.put(x, area.y, '-', style);
            self.put(x, bottom, '-', style);
        }

        for y in area.y.saturating_add(1)..bottom {
            self.put(area.x, y, '|', style);
            self.put(right, y, '|', style);
        }
    }
    /// Handles lines
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(usize::from(self.height));
        for y in 0..self.height {
            let mut line = String::with_capacity(usize::from(self.width));
            for x in 0..self.width {
                let index = usize::from(y) * usize::from(self.width) + usize::from(x);
                line.push(self.cells[index].symbol);
            }
            let trimmed = line.trim_end_matches(' ').to_string();
            lines.push(trimmed);
        }
        lines
    }
    /// Handles to plain text
    #[must_use]
    pub fn to_plain_text(&self) -> String {
        self.lines().join("\n")
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        if x < self.width && y < self.height {
            Some(usize::from(y) * usize::from(self.width) + usize::from(x))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Color;

    #[test]
    fn border_drawing_marks_corners_and_edges() {
        let mut frame = FrameBuffer::new(8, 4);
        let style = TextStyle::default().fg(Color::Blue);

        frame.draw_border(Rect::new(1, 0, 6, 4), style);

        assert_eq!(frame.cell(1, 0).expect("top-left").symbol, '+');
        assert_eq!(frame.cell(6, 3).expect("bottom-right").symbol, '+');
        assert_eq!(frame.cell(3, 0).expect("top-edge").symbol, '-');
        assert_eq!(frame.cell(1, 2).expect("left-edge").symbol, '|');
        assert_eq!(
            frame.cell(1, 0).expect("styled").style.fg,
            Some(Color::Blue)
        );
    }

    #[test]
    fn line_snapshot_trims_trailing_space_only() {
        let mut frame = FrameBuffer::new(5, 2);
        frame.write_str(0, 0, "abc", TextStyle::default(), 5);
        frame.write_str(1, 1, "z", TextStyle::default(), 5);

        assert_eq!(frame.lines(), vec!["abc".to_string(), " z".to_string()]);
    }
}
