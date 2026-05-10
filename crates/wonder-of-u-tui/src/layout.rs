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
    /// Stores the area
    pub area: Rect,
    /// Stores the messages
    pub messages: Rect,
    /// Stores the prompt
    pub prompt: Rect,
    /// Stores the chrome
    pub chrome: Rect,
    /// Zero-height placeholder kept for API stability; formerly the status row.
    ///
    /// With [`CHROME_HEIGHT`] = 1 the single chrome row is used entirely by
    /// [`ShellLayout::footer`].  Callers that pass this rect to
    /// `draw_status_line` will be no-ops because the rect is empty.
    pub status: Rect,
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
        let chrome = Rect::new(
            area.x,
            area.y
                .saturating_add(message_height)
                .saturating_add(prompt_height),
            area.width,
            chrome_height,
        );
        // The single chrome row is the compact footer; status is kept as a
        // zero-height placeholder so the layout struct stays backward-compatible.
        let footer_height = chrome_height.min(1);
        let status_height = chrome_height.saturating_sub(footer_height);
        let status = Rect::new(chrome.x, chrome.y, chrome.width, status_height);
        let footer = Rect::new(
            chrome.x,
            chrome.y.saturating_add(status_height),
            chrome.width,
            footer_height,
        );

        Self {
            area,
            messages,
            prompt,
            chrome,
            status,
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
        assert_eq!(layout.chrome, Rect::new(0, 11, 80, 1));
        // status is a zero-height placeholder; footer owns the single chrome row.
        assert_eq!(layout.status, Rect::new(0, 11, 80, 0));
        assert_eq!(layout.footer, Rect::new(0, 11, 80, 1));
    }

    #[test]
    fn split_reserves_message_space_when_height_allows() {
        // height=4, chrome=1, available=3.  requested_prompt=3 → messages=0 →
        // guard fires → prompt=2, messages=1.
        let layout = ShellLayout::split(Rect::new(0, 0, 40, 4), 3);

        assert_eq!(layout.messages, Rect::new(0, 0, 40, 1));
        assert_eq!(layout.prompt, Rect::new(0, 1, 40, 2));
        assert_eq!(layout.chrome, Rect::new(0, 3, 40, 1));
    }
}
