//! Spinner and shimmer view models inspired by `claude-leak/components/Spinner*`.

use unicode_width::UnicodeWidthStr;

const GLIMMER_PADDING: isize = 10;
const TOOL_FLASH_PERIOD_FRAMES: u64 = 10;

/// Default spinner glyphs used by the Claude Code loading animation.
pub const SPINNER_GLYPHS: &[char] = &['·', '✢', '✳', '✶', '✻', '✽'];

/// Expanded frame list for the leading spinner glyph.
pub const SPINNER_FRAMES: &[char] = &['·', '✢', '✳', '✶', '✻', '✽', '✻', '✶', '✳', '✢'];

/// High-level animation mode for the spinner row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpinnerMode {
    /// Represents idle
    Idle,
    /// Represents requesting
    Requesting,
    /// Represents thinking
    Thinking,
    /// Represents tool use
    ToolUse,
    /// Represents stalled
    Stalled,
}

/// Visual emphasis applied to a character in the message row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpinnerCharStyle {
    /// Represents base
    Base,
    /// Represents near highlight
    NearHighlight,
    /// Represents highlight
    Highlight,
    /// Represents flash
    Flash,
}

/// A single visible character in the animated message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpinnerCharView {
    /// Stores the ch
    pub ch: char,
    /// Stores the style
    pub style: SpinnerCharStyle,
}

/// A single rendered spinner frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpinnerFrameView {
    /// Stores the glyph
    pub glyph: char,
    /// Stores the message
    pub message: Vec<SpinnerCharView>,
    /// Stores the metadata
    pub metadata: Vec<String>,
    /// Stores whether stalled
    pub is_stalled: bool,
}

impl SpinnerFrameView {
    /// Handles message text
    #[must_use]
    pub fn message_text(&self) -> String {
        self.message.iter().map(|part| part.ch).collect()
    }
}

/// Renderer-neutral spinner state for frame-driven animation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpinnerView {
    /// Stores the mode
    pub mode: SpinnerMode,
    /// Stores the message
    pub message: String,
    /// Stores the frame
    pub frame: u64,
    /// Stores the suffix
    pub suffix: Option<String>,
    /// Stores the elapsed ms
    pub elapsed_ms: Option<u64>,
    /// Stores the token count
    pub token_count: Option<usize>,
    /// Stores the reduced motion
    pub reduced_motion: bool,
}

impl SpinnerView {
    /// Creates a new value
    #[must_use]
    pub fn new(mode: SpinnerMode, message: impl Into<String>) -> Self {
        Self {
            mode,
            message: message.into(),
            frame: 0,
            suffix: None,
            elapsed_ms: None,
            token_count: None,
            reduced_motion: false,
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn frame(mut self, frame: u64) -> Self {
        self.frame = frame;
        self
    }
    /// Handles suffix
    #[must_use]
    pub fn suffix(mut self, suffix: impl Into<String>) -> Self {
        self.suffix = Some(suffix.into());
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn elapsed_ms(mut self, elapsed_ms: u64) -> Self {
        self.elapsed_ms = Some(elapsed_ms);
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn token_count(mut self, token_count: usize) -> Self {
        self.token_count = Some(token_count);
        self
    }
    /// Constant fn
    #[must_use]
    pub const fn reduced_motion(mut self, reduced_motion: bool) -> Self {
        self.reduced_motion = reduced_motion;
        self
    }
    /// Renders frame
    #[must_use]
    pub fn render_frame(&self) -> SpinnerFrameView {
        let glyph = match self.mode {
            SpinnerMode::Idle => '·',
            _ => SPINNER_FRAMES[self.frame as usize % SPINNER_FRAMES.len()],
        };
        let glimmer_index = self.glimmer_index();
        let is_flash_active = self.mode == SpinnerMode::ToolUse
            && !self.reduced_motion
            && (self.frame / TOOL_FLASH_PERIOD_FRAMES) % 2 == 0;
        let message = self
            .message
            .chars()
            .enumerate()
            .map(|(index, ch)| SpinnerCharView {
                ch,
                style: if is_flash_active {
                    SpinnerCharStyle::Flash
                } else if self.reduced_motion {
                    SpinnerCharStyle::Base
                } else {
                    match (index as isize - glimmer_index).abs() {
                        0 => SpinnerCharStyle::Highlight,
                        1 => SpinnerCharStyle::NearHighlight,
                        _ => SpinnerCharStyle::Base,
                    }
                },
            })
            .collect();

        SpinnerFrameView {
            glyph,
            message,
            metadata: self.metadata_parts(),
            is_stalled: self.mode == SpinnerMode::Stalled,
        }
    }
    /// Renders line
    #[must_use]
    pub fn render_line(&self) -> String {
        let frame = self.render_frame();
        let mut line = format!("{} {}", frame.glyph, frame.message_text());
        if !frame.metadata.is_empty() {
            line.push_str(" · ");
            line.push_str(&frame.metadata.join(" · "));
        }
        line
    }

    fn metadata_parts(&self) -> Vec<String> {
        let mut parts = Vec::with_capacity(3);
        if let Some(suffix) = &self.suffix {
            parts.push(suffix.clone());
        }
        if let Some(elapsed_ms) = self.elapsed_ms {
            parts.push(format_duration(elapsed_ms));
        }
        if let Some(token_count) = self.token_count {
            parts.push(format!("{token_count} tokens"));
        }
        parts
    }

    fn glimmer_index(&self) -> isize {
        if matches!(
            self.mode,
            SpinnerMode::Idle | SpinnerMode::ToolUse | SpinnerMode::Stalled
        ) || self.reduced_motion
        {
            return -100;
        }

        let width = UnicodeWidthStr::width(self.message.as_str()) as isize;
        let cycle_length = width + GLIMMER_PADDING * 2;
        let cycle_position = self.frame as isize % cycle_length.max(1);

        match self.mode {
            SpinnerMode::Requesting => cycle_position - GLIMMER_PADDING,
            SpinnerMode::Thinking => width + GLIMMER_PADDING - cycle_position,
            SpinnerMode::Idle | SpinnerMode::ToolUse | SpinnerMode::Stalled => -100,
        }
    }
}

fn format_duration(elapsed_ms: u64) -> String {
    let total_seconds = elapsed_ms / 1_000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requesting_spinner_moves_glimmer_left_to_right() {
        let first = SpinnerView::new(SpinnerMode::Requesting, "hello")
            .frame(0)
            .render_frame();
        let later = SpinnerView::new(SpinnerMode::Requesting, "hello")
            .frame(12)
            .render_frame();

        let first_highlight = first
            .message
            .iter()
            .position(|part| part.style == SpinnerCharStyle::Highlight);
        let later_highlight = later
            .message
            .iter()
            .position(|part| part.style == SpinnerCharStyle::Highlight);

        assert_eq!(first_highlight, None);
        assert_eq!(later_highlight, Some(2));
    }

    #[test]
    fn thinking_spinner_moves_glimmer_right_to_left() {
        let earlier = SpinnerView::new(SpinnerMode::Thinking, "hello")
            .frame(11)
            .render_frame();
        let later = SpinnerView::new(SpinnerMode::Thinking, "hello")
            .frame(13)
            .render_frame();

        let earlier_highlight = earlier
            .message
            .iter()
            .position(|part| part.style == SpinnerCharStyle::Highlight);
        let later_highlight = later
            .message
            .iter()
            .position(|part| part.style == SpinnerCharStyle::Highlight);

        assert_eq!(earlier_highlight, Some(4));
        assert_eq!(later_highlight, Some(2));
    }

    #[test]
    fn tool_use_spinner_flashes_entire_message() {
        let flashing = SpinnerView::new(SpinnerMode::ToolUse, "tool")
            .frame(0)
            .render_frame();

        assert!(
            flashing
                .message
                .iter()
                .all(|part| part.style == SpinnerCharStyle::Flash)
        );
    }

    #[test]
    fn reduced_motion_disables_message_highlight() {
        let frame = SpinnerView::new(SpinnerMode::Requesting, "hello")
            .frame(15)
            .reduced_motion(true)
            .render_frame();

        assert!(
            frame
                .message
                .iter()
                .all(|part| part.style == SpinnerCharStyle::Base)
        );
    }

    #[test]
    fn render_line_includes_metadata() {
        let line = SpinnerView::new(SpinnerMode::Requesting, "Syncing")
            .frame(3)
            .suffix("thinking")
            .elapsed_ms(61_000)
            .token_count(128)
            .render_line();

        assert!(line.contains("Syncing"));
        assert!(line.contains("thinking"));
        assert!(line.contains("01:01"));
        assert!(line.contains("128 tokens"));
    }
}
