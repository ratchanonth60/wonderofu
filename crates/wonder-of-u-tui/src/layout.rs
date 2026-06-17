use crate::frame::Rect;

/// Height of the compact bottom chrome strip (one row: the compact footer hint).
///
/// Reduced from 2 to 1 to match the Claude Code fullscreen layout: the transcript
/// starts at the very top of the terminal, the prompt sits near the bottom with a
/// rounded border, and a single compact footer row carries keybinding hints and
/// the optional scroll indicator.  The loading/status information is shown inline
/// inside the transcript area (via the spinner line) rather than in a separate
/// chrome row.
pub const CHROME_HEIGHT: u16 = 1;

/// Represents shell layout
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellLayout {
    /// Transcript message area.
    pub messages: Rect,
    /// Prompt input box area.
    pub prompt: Rect,
    /// The single compact footer row carrying keybinding hints.
    pub footer: Rect,
}

impl ShellLayout {
    /// Splits `area` into transcript, prompt, and a single compact footer row.
    ///
    /// The footer always occupies the last [`CHROME_HEIGHT`] rows (currently 1).
    /// `requested_prompt_height` is capped so the transcript is never crowded out.
    #[must_use]
    pub fn split(area: Rect, requested_prompt_height: u16) -> Self {
        let chrome_height = area.height.min(CHROME_HEIGHT);
        let available = area.height.saturating_sub(chrome_height);

        let mut prompt_height = if available == 0 {
            0
        } else {
            requested_prompt_height.max(1).min(available)
        };
        let mut message_height = available.saturating_sub(prompt_height);

        if available > 1 && message_height == 0 {
            prompt_height = prompt_height.saturating_sub(1);
            message_height = 1;
        }

        let messages = Rect::new(area.x, area.y, area.width, message_height);
        let prompt = Rect::new(
            area.x,
            area.y.saturating_add(message_height),
            area.width,
            prompt_height,
        );
        let footer = Rect::new(
            area.x,
            area.y
                .saturating_add(message_height)
                .saturating_add(prompt_height),
            area.width,
            chrome_height,
        );

        Self {
            messages,
            prompt,
            footer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_creates_three_primary_zones_and_one_compact_footer() {
        // CHROME_HEIGHT = 1: the entire chrome strip is the compact footer row.
        // height=12, prompt=3 → available=11, messages=8, prompt at y=8.
        let layout = ShellLayout::split(Rect::new(0, 0, 80, 12), 3);

        assert_eq!(layout.messages, Rect::new(0, 0, 80, 8));
        assert_eq!(layout.prompt, Rect::new(0, 8, 80, 3));
        assert_eq!(layout.footer, Rect::new(0, 11, 80, 1));
    }

    #[test]
    fn split_reserves_message_space_when_height_allows() {
        // height=4, chrome=1, available=3.  requested_prompt=3 → messages=0 →
        // guard fires → prompt=2, messages=1.
        let layout = ShellLayout::split(Rect::new(0, 0, 40, 4), 3);

        assert_eq!(layout.messages, Rect::new(0, 0, 40, 1));
        assert_eq!(layout.prompt, Rect::new(0, 1, 40, 2));
        assert_eq!(layout.footer, Rect::new(0, 3, 40, 1));
    }
}
