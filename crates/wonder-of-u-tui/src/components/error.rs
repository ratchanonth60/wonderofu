//! Error overview view models.

/// Source location for an error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorLocationView {
    /// Stores the file path
    pub file_path: String,
    /// Stores the line
    pub line: u32,
    /// Stores the column
    pub column: u32,
}

impl ErrorLocationView {
    /// Creates a new value
    #[must_use]
    pub fn new(file_path: impl Into<String>, line: u32, column: u32) -> Self {
        Self {
            file_path: file_path.into(),
            line,
            column,
        }
    }
    /// Handles display label
    #[must_use]
    pub fn display_label(&self) -> String {
        format!("{}:{}:{}", self.file_path, self.line, self.column)
    }
}

/// One highlighted source line in an error excerpt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorExcerptLineView {
    /// Stores the line
    pub line: u32,
    /// Stores the value
    pub value: String,
    /// Stores the highlighted
    pub highlighted: bool,
}

impl ErrorExcerptLineView {
    /// Creates a new value
    #[must_use]
    pub fn new(line: u32, value: impl Into<String>) -> Self {
        Self {
            line,
            value: value.into(),
            highlighted: false,
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn highlighted(mut self, highlighted: bool) -> Self {
        self.highlighted = highlighted;
        self
    }
}

/// One parsed stack frame row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorStackFrameView {
    /// Stores the function
    pub function: Option<String>,
    /// Stores the file path
    pub file_path: Option<String>,
    /// Stores the line
    pub line: Option<u32>,
    /// Stores the column
    pub column: Option<u32>,
    /// Stores the raw
    pub raw: String,
}

impl ErrorStackFrameView {
    /// Handles raw
    #[must_use]
    pub fn raw(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        Self {
            function: None,
            file_path: None,
            line: None,
            column: None,
            raw,
        }
    }
    /// Handles parsed
    #[must_use]
    pub fn parsed(
        function: impl Into<String>,
        file_path: impl Into<String>,
        line: u32,
        column: u32,
    ) -> Self {
        let function = function.into();
        let file_path = file_path.into();
        let raw = format!("{function} ({file_path}:{line}:{column})");
        Self {
            function: Some(function),
            file_path: Some(file_path),
            line: Some(line),
            column: Some(column),
            raw,
        }
    }
    /// Handles display label
    #[must_use]
    pub fn display_label(&self) -> String {
        match (&self.function, &self.file_path, self.line, self.column) {
            (Some(function), Some(file_path), Some(line), Some(column)) => {
                format!("{function} ({file_path}:{line}:{column})")
            }
            _ => self.raw.clone(),
        }
    }
}

/// Fully prepared error panel data.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ErrorOverviewView {
    /// Stores the message
    pub message: String,
    /// Stores the location
    pub location: Option<ErrorLocationView>,
    /// Stores the excerpt
    pub excerpt: Vec<ErrorExcerptLineView>,
    /// Stores the stack
    pub stack: Vec<ErrorStackFrameView>,
}

impl ErrorOverviewView {
    /// Creates a new value
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ..Self::default()
        }
    }
    /// Handles with location
    #[must_use]
    pub fn with_location(mut self, location: ErrorLocationView) -> Self {
        self.location = Some(location);
        self
    }
    /// Handles with excerpt
    #[must_use]
    pub fn with_excerpt(mut self, excerpt: impl IntoIterator<Item = ErrorExcerptLineView>) -> Self {
        self.excerpt = excerpt.into_iter().collect();
        self
    }
    /// Handles with stack
    #[must_use]
    pub fn with_stack(mut self, stack: impl IntoIterator<Item = ErrorStackFrameView>) -> Self {
        self.stack = stack.into_iter().collect();
        self
    }
    /// Handles line number width
    #[must_use]
    pub fn line_number_width(&self) -> usize {
        self.excerpt
            .iter()
            .map(|line| line.line.to_string().len())
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_number_width_tracks_widest_excerpt_line() {
        let error = ErrorOverviewView::new("boom").with_excerpt([
            ErrorExcerptLineView::new(8, "let a = 1;"),
            ErrorExcerptLineView::new(120, "panic!();").highlighted(true),
        ]);

        assert_eq!(error.line_number_width(), 3);
    }

    #[test]
    fn parsed_stack_frame_formats_location() {
        let frame = ErrorStackFrameView::parsed("render", "src/main.rs", 14, 9);

        assert_eq!(frame.display_label(), "render (src/main.rs:14:9)");
    }
}
