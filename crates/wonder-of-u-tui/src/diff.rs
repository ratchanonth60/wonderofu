//! Renderer-agnostic view models for diff, code, and file content.

use unicode_segmentation::UnicodeSegmentation;

use crate::measure::{line_width, strip_ansi};

/// A width-limited line ready for renderer-specific styling.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualLine {
    pub kind: VisualLineKind,
    pub text: String,
}

impl VisualLine {
    #[must_use]
    pub fn new(kind: VisualLineKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }
}

/// Categorizes renderer-agnostic diff and code lines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualLineKind {
    Path,
    Meta,
    Context,
    Addition,
    Removal,
    Code,
    Placeholder,
    NoDiff,
}

/// The rendered text together with a truncation marker.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TruncatedText {
    pub text: String,
    pub truncated: bool,
}

/// Displays a file path or path-like label with an optional link target.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PathLinkView {
    pub label: String,
    pub target: Option<String>,
}

impl PathLinkView {
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            target: None,
        }
    }

    #[must_use]
    pub fn with_target(label: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            target: Some(target.into()),
        }
    }

    #[must_use]
    pub fn display_text(&self, max_width: usize) -> TruncatedText {
        truncate_path_like(&self.label, max_width)
    }
}

/// Summarizes a single file-edit hunk.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FileEditHunkSummary {
    pub additions: usize,
    pub removals: usize,
    pub context: usize,
}

impl FileEditHunkSummary {
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.additions == 0 && self.removals == 0 && self.context == 0
    }

    #[must_use]
    pub fn label(self) -> String {
        format!("+{} -{} ~{}", self.additions, self.removals, self.context)
    }
}

/// A single structured diff line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructuredDiffLine {
    pub kind: StructuredDiffLineKind,
    pub old_line_number: Option<usize>,
    pub new_line_number: Option<usize>,
    pub text: String,
}

impl StructuredDiffLine {
    #[must_use]
    pub fn context(
        old_line_number: Option<usize>,
        new_line_number: Option<usize>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            kind: StructuredDiffLineKind::Context,
            old_line_number,
            new_line_number,
            text: text.into(),
        }
    }

    #[must_use]
    pub fn added(new_line_number: Option<usize>, text: impl Into<String>) -> Self {
        Self {
            kind: StructuredDiffLineKind::Addition,
            old_line_number: None,
            new_line_number,
            text: text.into(),
        }
    }

    #[must_use]
    pub fn removed(old_line_number: Option<usize>, text: impl Into<String>) -> Self {
        Self {
            kind: StructuredDiffLineKind::Removal,
            old_line_number,
            new_line_number: None,
            text: text.into(),
        }
    }

    #[must_use]
    pub fn prefix(&self) -> char {
        match self.kind {
            StructuredDiffLineKind::Context => ' ',
            StructuredDiffLineKind::Addition => '+',
            StructuredDiffLineKind::Removal => '-',
        }
    }
}

/// The semantic role of a diff line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StructuredDiffLineKind {
    Context,
    Addition,
    Removal,
}

/// A single diff hunk with renderer-agnostic summary data.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StructuredDiffHunk {
    pub header: String,
    pub lines: Vec<StructuredDiffLine>,
}

impl StructuredDiffHunk {
    #[must_use]
    pub fn summary(&self) -> FileEditHunkSummary {
        let mut summary = FileEditHunkSummary::default();
        for line in &self.lines {
            match line.kind {
                StructuredDiffLineKind::Context => summary.context += 1,
                StructuredDiffLineKind::Addition => summary.additions += 1,
                StructuredDiffLineKind::Removal => summary.removals += 1,
            }
        }
        summary
    }
}

/// A structured diff for a single file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StructuredDiffView {
    pub old_path: Option<PathLinkView>,
    pub path: Option<PathLinkView>,
    pub hunks: Vec<StructuredDiffHunk>,
    pub truncated: bool,
}

impl StructuredDiffView {
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<VisualLine> {
        let mut lines = Vec::new();
        if let Some(path_line) = self.path_line(max_width) {
            lines.push(path_line);
        }

        if self.hunks.is_empty() {
            lines.extend(NoDiffView::unchanged().display_lines(max_width));
            return lines;
        }

        let (old_width, new_width) = self.line_number_widths();
        for hunk in &self.hunks {
            let mut header = hunk.header.clone();
            let summary = hunk.summary();
            if !summary.is_empty() {
                if !header.is_empty() {
                    header.push_str(" • ");
                }
                header.push_str(&summary.label());
            }
            lines.push(render_line(
                VisualLineKind::Meta,
                truncate_visible_end(&header, max_width),
            ));

            for line in &hunk.lines {
                let old = line
                    .old_line_number
                    .map_or_else(|| String::from(""), |value| value.to_string());
                let new = line
                    .new_line_number
                    .map_or_else(|| String::from(""), |value| value.to_string());
                let text = format!(
                    "{old:>old_width$} {new:>new_width$} {} {}",
                    line.prefix(),
                    line.text
                );
                let kind = match line.kind {
                    StructuredDiffLineKind::Context => VisualLineKind::Context,
                    StructuredDiffLineKind::Addition => VisualLineKind::Addition,
                    StructuredDiffLineKind::Removal => VisualLineKind::Removal,
                };
                lines.push(render_line(kind, truncate_visible_end(&text, max_width)));
            }
        }

        if self.truncated {
            lines.push(render_line(
                VisualLineKind::Placeholder,
                truncate_visible_end("… diff truncated", max_width),
            ));
        }

        lines
    }

    fn path_line(&self, max_width: usize) -> Option<VisualLine> {
        match (&self.old_path, &self.path) {
            (Some(old_path), Some(path)) if old_path.label != path.label => Some(render_line(
                VisualLineKind::Path,
                truncate_visible_start(&format!("{} → {}", old_path.label, path.label), max_width),
            )),
            (_, Some(path)) => Some(render_line(
                VisualLineKind::Path,
                path.display_text(max_width),
            )),
            (Some(path), None) => Some(render_line(
                VisualLineKind::Path,
                path.display_text(max_width),
            )),
            (None, None) => None,
        }
    }

    fn line_number_widths(&self) -> (usize, usize) {
        let mut old_max = 0usize;
        let mut new_max = 0usize;
        for hunk in &self.hunks {
            for line in &hunk.lines {
                old_max = old_max.max(line.old_line_number.unwrap_or_default());
                new_max = new_max.max(line.new_line_number.unwrap_or_default());
            }
        }
        (digit_width(old_max), digit_width(new_max))
    }
}

/// A plain-text fallback for syntax-highlighted code.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HighlightedCodeView {
    pub path: Option<PathLinkView>,
    pub language: Option<String>,
    pub code: String,
    pub first_line_number: usize,
    pub truncated: bool,
}

impl HighlightedCodeView {
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<VisualLine> {
        let mut lines = Vec::new();
        if let Some(path) = &self.path {
            lines.push(render_line(
                VisualLineKind::Path,
                path.display_text(max_width),
            ));
        }
        if let Some(language) = self.language.as_deref() {
            lines.push(render_line(
                VisualLineKind::Meta,
                truncate_visible_end(&format!("language: {language}"), max_width),
            ));
        }

        let source_lines = split_preserving_empty_line(&self.code);
        if source_lines.is_empty() {
            lines.push(render_line(
                VisualLineKind::NoDiff,
                truncate_visible_end("No code to display.", max_width),
            ));
            return lines;
        }

        let last_line_number = self
            .first_line_number
            .saturating_add(source_lines.len().saturating_sub(1));
        let number_width = digit_width(last_line_number);
        for (index, source_line) in source_lines.iter().enumerate() {
            let line_number = self.first_line_number.saturating_add(index);
            let text = format!("{line_number:>number_width$} │ {source_line}");
            lines.push(render_line(
                VisualLineKind::Code,
                truncate_visible_end(&text, max_width),
            ));
        }

        if self.truncated {
            lines.push(render_line(
                VisualLineKind::Placeholder,
                truncate_visible_end("… code truncated", max_width),
            ));
        }

        lines
    }
}

/// Explains why file contents are not rendered inline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePlaceholderView {
    pub path: Option<PathLinkView>,
    pub kind: FilePlaceholderKind,
}

impl FilePlaceholderView {
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<VisualLine> {
        let mut lines = Vec::new();
        if let Some(path) = &self.path {
            lines.push(render_line(
                VisualLineKind::Path,
                path.display_text(max_width),
            ));
        }

        match self.kind {
            FilePlaceholderKind::Binary => lines.push(render_line(
                VisualLineKind::Placeholder,
                truncate_visible_end("Binary file not shown.", max_width),
            )),
            FilePlaceholderKind::LargeFile {
                size_bytes,
                limit_bytes,
            } => {
                let text = match limit_bytes {
                    Some(limit) => format!(
                        "File too large to display ({}, limit {}).",
                        format_bytes(size_bytes),
                        format_bytes(limit)
                    ),
                    None => format!("File too large to display ({}).", format_bytes(size_bytes)),
                };
                lines.push(render_line(
                    VisualLineKind::Placeholder,
                    truncate_visible_end(&text, max_width),
                ));
            }
        }

        lines
    }
}

/// A file placeholder state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilePlaceholderKind {
    Binary,
    LargeFile {
        size_bytes: usize,
        limit_bytes: Option<usize>,
    },
}

/// Explains why there is no diff output.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NoDiffView {
    pub path: Option<PathLinkView>,
    pub state: NoDiffState,
    pub detail: Option<String>,
}

impl NoDiffView {
    #[must_use]
    pub fn unchanged() -> Self {
        Self {
            path: None,
            state: NoDiffState::Unchanged,
            detail: None,
        }
    }

    #[must_use]
    pub fn empty() -> Self {
        Self {
            path: None,
            state: NoDiffState::Empty,
            detail: None,
        }
    }

    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            path: None,
            state: NoDiffState::Unavailable,
            detail: None,
        }
    }

    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<VisualLine> {
        let mut lines = Vec::new();
        if let Some(path) = &self.path {
            lines.push(render_line(
                VisualLineKind::Path,
                path.display_text(max_width),
            ));
        }

        lines.push(render_line(
            VisualLineKind::NoDiff,
            truncate_visible_end(self.state.label(), max_width),
        ));
        if let Some(detail) = self.detail.as_deref() {
            lines.push(render_line(
                VisualLineKind::Meta,
                truncate_visible_end(detail, max_width),
            ));
        }
        lines
    }
}

/// Describes the absence of diff output.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NoDiffState {
    #[default]
    Unchanged,
    Empty,
    Unavailable,
}

impl NoDiffState {
    fn label(self) -> &'static str {
        match self {
            Self::Unchanged => "No changes.",
            Self::Empty => "Nothing to display.",
            Self::Unavailable => "No diff available.",
        }
    }
}

/// A single file-oriented visual block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileVisualView {
    Diff(StructuredDiffView),
    Code(HighlightedCodeView),
    Placeholder(FilePlaceholderView),
    NoDiff(NoDiffView),
}

impl FileVisualView {
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<VisualLine> {
        match self {
            Self::Diff(view) => view.display_lines(max_width),
            Self::Code(view) => view.display_lines(max_width),
            Self::Placeholder(view) => view.display_lines(max_width),
            Self::NoDiff(view) => view.display_lines(max_width),
        }
    }
}

fn render_line(kind: VisualLineKind, text: TruncatedText) -> VisualLine {
    VisualLine::new(kind, text.text)
}

fn split_preserving_empty_line(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n').collect()
}

fn digit_width(value: usize) -> usize {
    value.max(1).to_string().len()
}

fn truncate_visible_end(text: &str, max_width: usize) -> TruncatedText {
    truncate_visible(text, max_width, false)
}

fn truncate_visible_start(text: &str, max_width: usize) -> TruncatedText {
    truncate_visible(text, max_width, true)
}

fn truncate_path_like(text: &str, max_width: usize) -> TruncatedText {
    let width = line_width(text);
    if width <= max_width {
        return TruncatedText {
            text: text.to_string(),
            truncated: false,
        };
    }

    let separator = if text.contains('/') {
        Some('/')
    } else if text.contains('\\') {
        Some('\\')
    } else {
        None
    };
    let Some(separator) = separator else {
        return truncate_visible_start(text, max_width);
    };

    let parts: Vec<&str> = text
        .split(separator)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return truncate_visible_start(text, max_width);
    }

    let ellipsis = "…";
    let mut candidate = parts[parts.len() - 1].to_string();
    for part in parts[..parts.len() - 1].iter().rev() {
        let proposed = format!("{part}{separator}{candidate}");
        let truncated = format!("{ellipsis}{separator}{proposed}");
        if line_width(&truncated) > max_width {
            break;
        }
        candidate = proposed;
    }

    let truncated = format!("{ellipsis}{separator}{candidate}");
    if line_width(&truncated) > max_width {
        truncate_visible_start(&candidate, max_width)
    } else {
        TruncatedText {
            text: truncated,
            truncated: true,
        }
    }
}

fn truncate_visible(text: &str, max_width: usize, from_start: bool) -> TruncatedText {
    let stripped = strip_ansi(text);
    let text = stripped.as_ref();
    let width = line_width(text);
    if width <= max_width {
        return TruncatedText {
            text: text.to_string(),
            truncated: false,
        };
    }
    if max_width == 0 {
        return TruncatedText {
            text: String::new(),
            truncated: !text.is_empty(),
        };
    }
    if max_width == 1 {
        return TruncatedText {
            text: "…".into(),
            truncated: true,
        };
    }

    let mut kept = Vec::new();
    let mut kept_width = line_width("…");
    if from_start {
        for grapheme in text.graphemes(true).rev() {
            let grapheme_width = line_width(grapheme);
            if kept_width.saturating_add(grapheme_width) > max_width {
                break;
            }
            kept.push(grapheme);
            kept_width = kept_width.saturating_add(grapheme_width);
        }
        kept.reverse();
        let mut truncated = String::from("…");
        for grapheme in kept {
            truncated.push_str(grapheme);
        }
        TruncatedText {
            text: truncated,
            truncated: true,
        }
    } else {
        for grapheme in text.graphemes(true) {
            let grapheme_width = line_width(grapheme);
            if kept_width.saturating_add(grapheme_width) > max_width {
                break;
            }
            kept.push(grapheme);
            kept_width = kept_width.saturating_add(grapheme_width);
        }
        let mut truncated = kept.concat();
        truncated.push('…');
        TruncatedText {
            text: truncated,
            truncated: true,
        }
    }
}

fn format_bytes(bytes: usize) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len().saturating_sub(1) {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_diff_tracks_added_removed_and_context_lines() {
        let hunk = StructuredDiffHunk {
            header: "@@ -1,2 +1,3 @@".into(),
            lines: vec![
                StructuredDiffLine::context(Some(1), Some(1), "fn keep() {"),
                StructuredDiffLine::removed(Some(2), "    old_call();"),
                StructuredDiffLine::added(Some(2), "    new_call();"),
            ],
        };
        let view = StructuredDiffView {
            old_path: None,
            path: Some(PathLinkView::new("src/lib.rs")),
            hunks: vec![hunk.clone()],
            truncated: false,
        };

        assert_eq!(
            hunk.summary(),
            FileEditHunkSummary {
                additions: 1,
                removals: 1,
                context: 1,
            }
        );

        let lines = view.display_lines(80);
        assert_eq!(
            lines[0],
            VisualLine::new(VisualLineKind::Path, "src/lib.rs")
        );
        assert_eq!(
            lines[1],
            VisualLine::new(VisualLineKind::Meta, "@@ -1,2 +1,3 @@ • +1 -1 ~1")
        );
        assert_eq!(
            lines[2],
            VisualLine::new(VisualLineKind::Context, "1 1   fn keep() {")
        );
        assert_eq!(
            lines[3],
            VisualLine::new(VisualLineKind::Removal, "2   -     old_call();")
        );
        assert_eq!(
            lines[4],
            VisualLine::new(VisualLineKind::Addition, "  2 +     new_call();")
        );
    }

    #[test]
    fn path_and_diff_output_truncate_when_width_is_tight() {
        let path = PathLinkView::new("very/long/path/to/src/lib.rs");
        assert_eq!(
            path.display_text(12),
            TruncatedText {
                text: "…/src/lib.rs".into(),
                truncated: true,
            }
        );

        let view = StructuredDiffView {
            old_path: None,
            path: Some(path),
            hunks: vec![StructuredDiffHunk {
                header: "@@ -10,1 +10,1 @@".into(),
                lines: vec![StructuredDiffLine::added(
                    Some(10),
                    "let answer = forty_two_with_extra_words();",
                )],
            }],
            truncated: true,
        };

        let lines = view.display_lines(18);
        assert_eq!(
            lines[0],
            VisualLine::new(VisualLineKind::Path, "…/to/src/lib.rs")
        );
        assert_eq!(
            lines[1],
            VisualLine::new(VisualLineKind::Meta, "@@ -10,1 +10,1 @@…")
        );
        assert_eq!(
            lines[2],
            VisualLine::new(VisualLineKind::Addition, "  10 + let answer…")
        );
        assert_eq!(
            lines[3],
            VisualLine::new(VisualLineKind::Placeholder, "… diff truncated")
        );
    }

    #[test]
    fn highlighted_code_fallback_renders_line_numbers() {
        let view = HighlightedCodeView {
            path: Some(PathLinkView::with_target(
                "src/main.rs",
                "file:///workspace/src/main.rs",
            )),
            language: Some("rust".into()),
            code: "fn main() {\n    println!(\"hi\");\n}".into(),
            first_line_number: 40,
            truncated: false,
        };

        assert_eq!(
            view.display_lines(80),
            vec![
                VisualLine::new(VisualLineKind::Path, "src/main.rs"),
                VisualLine::new(VisualLineKind::Meta, "language: rust"),
                VisualLine::new(VisualLineKind::Code, "40 │ fn main() {"),
                VisualLine::new(VisualLineKind::Code, "41 │     println!(\"hi\");"),
                VisualLine::new(VisualLineKind::Code, "42 │ }"),
            ]
        );
    }

    #[test]
    fn binary_and_large_file_placeholders_render_expected_messages() {
        let binary = FilePlaceholderView {
            path: Some(PathLinkView::new("assets/logo.png")),
            kind: FilePlaceholderKind::Binary,
        };
        let large = FilePlaceholderView {
            path: Some(PathLinkView::new("logs/build.log")),
            kind: FilePlaceholderKind::LargeFile {
                size_bytes: 12 * 1024,
                limit_bytes: Some(4 * 1024),
            },
        };

        assert_eq!(
            binary.display_lines(80),
            vec![
                VisualLine::new(VisualLineKind::Path, "assets/logo.png"),
                VisualLine::new(VisualLineKind::Placeholder, "Binary file not shown."),
            ]
        );
        assert_eq!(
            large.display_lines(80),
            vec![
                VisualLine::new(VisualLineKind::Path, "logs/build.log"),
                VisualLine::new(
                    VisualLineKind::Placeholder,
                    "File too large to display (12.0 KiB, limit 4.0 KiB)."
                ),
            ]
        );
    }

    #[test]
    fn no_diff_states_render_plain_fallbacks() {
        let mut view = NoDiffView::empty();
        view.path = Some(PathLinkView::new("src/empty.rs"));
        view.detail = Some("The file exists but has no displayable content.".into());

        assert_eq!(
            view.display_lines(80),
            vec![
                VisualLine::new(VisualLineKind::Path, "src/empty.rs"),
                VisualLine::new(VisualLineKind::NoDiff, "Nothing to display."),
                VisualLine::new(
                    VisualLineKind::Meta,
                    "The file exists but has no displayable content."
                ),
            ]
        );
    }
}
