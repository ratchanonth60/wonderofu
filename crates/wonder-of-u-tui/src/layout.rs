use crate::frame::Rect;

pub const CHROME_HEIGHT: u16 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellLayout {
    pub area: Rect,
    pub messages: Rect,
    pub prompt: Rect,
    pub chrome: Rect,
    pub status: Rect,
    pub footer: Rect,
}

impl ShellLayout {
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
        let status_height = chrome_height.min(1);
        let footer_height = chrome_height.saturating_sub(status_height);
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
    fn split_creates_three_primary_zones_and_two_chrome_lines() {
        let layout = ShellLayout::split(Rect::new(0, 0, 80, 12), 3);

        assert_eq!(layout.messages, Rect::new(0, 0, 80, 7));
        assert_eq!(layout.prompt, Rect::new(0, 7, 80, 3));
        assert_eq!(layout.chrome, Rect::new(0, 10, 80, 2));
        assert_eq!(layout.status, Rect::new(0, 10, 80, 1));
        assert_eq!(layout.footer, Rect::new(0, 11, 80, 1));
    }

    #[test]
    fn split_reserves_message_space_when_height_allows() {
        let layout = ShellLayout::split(Rect::new(0, 0, 40, 4), 3);

        assert_eq!(layout.messages, Rect::new(0, 0, 40, 1));
        assert_eq!(layout.prompt, Rect::new(0, 1, 40, 1));
        assert_eq!(layout.chrome, Rect::new(0, 2, 40, 2));
    }
}
