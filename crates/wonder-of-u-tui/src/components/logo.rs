//! Logo and welcome view models inspired by `claude-leak/components/LogoV2/*`.

use super::spinner::SPINNER_GLYPHS;

/// Static mascot poses used by the animated logo.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClawdPose {
    Default,
    ArmsUp,
    LookLeft,
    LookRight,
}

impl ClawdPose {
    #[must_use]
    pub const fn ascii_lines(self) -> [&'static str; 3] {
        match self {
            Self::Default => [" ▐▛███▜▌", "▝▜█████▛▘", "  ▘▘ ▝▝  "],
            Self::LookLeft => [" ▐▟███▟▌", "▝▜█████▛▘", "  ▘▘ ▝▝  "],
            Self::LookRight => [" ▐▙███▙▌", "▝▜█████▛▘", "  ▘▘ ▝▝  "],
            Self::ArmsUp => ["▗▟▛███▜▙▖", " ▜█████▛ ", "  ▘▘ ▝▝  "],
        }
    }
}

/// A single feed item shown beside or below the logo.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogoFeedItem {
    pub title: String,
    pub body: Vec<String>,
}

impl LogoFeedItem {
    #[must_use]
    pub fn new(
        title: impl Into<String>,
        body: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            title: title.into(),
            body: body.into_iter().map(Into::into).collect(),
        }
    }
}

/// Renderer-neutral Claude Code logo view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogoView {
    pub version: String,
    pub condensed: bool,
    pub frame: u64,
    pub feed: Vec<LogoFeedItem>,
    pub emergency_tip: Option<String>,
}

impl LogoView {
    #[must_use]
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            condensed: false,
            frame: 0,
            feed: Vec::new(),
            emergency_tip: None,
        }
    }

    #[must_use]
    pub const fn condensed(mut self, condensed: bool) -> Self {
        self.condensed = condensed;
        self
    }

    #[must_use]
    pub const fn frame(mut self, frame: u64) -> Self {
        self.frame = frame;
        self
    }

    #[must_use]
    pub fn feed(mut self, feed: Vec<LogoFeedItem>) -> Self {
        self.feed = feed;
        self
    }

    #[must_use]
    pub fn emergency_tip(mut self, emergency_tip: impl Into<String>) -> Self {
        self.emergency_tip = Some(emergency_tip.into());
        self
    }

    #[must_use]
    pub fn render_lines(&self, width: usize) -> Vec<String> {
        let mut lines = if self.condensed {
            self.render_condensed_lines()
        } else {
            self.render_full_lines()
        };

        if let Some(tip) = &self.emergency_tip {
            lines.push(String::new());
            lines.push(truncate_line(&format!("Tip: {tip}"), width));
        }

        for item in &self.feed {
            lines.push(String::new());
            lines.push(truncate_line(&format!("• {}", item.title), width));
            lines.extend(
                item.body
                    .iter()
                    .map(|line| truncate_line(&format!("  {line}"), width)),
            );
        }

        lines
            .into_iter()
            .map(|line| truncate_line(&line, width))
            .collect()
    }

    #[must_use]
    pub const fn clawd_pose(&self) -> ClawdPose {
        match (self.frame / 4) % 4 {
            0 => ClawdPose::Default,
            1 => ClawdPose::LookRight,
            2 => ClawdPose::LookLeft,
            _ => ClawdPose::ArmsUp,
        }
    }

    fn render_condensed_lines(&self) -> Vec<String> {
        let asterisk = SPINNER_GLYPHS[self.frame as usize % SPINNER_GLYPHS.len()];
        vec![
            format!("{asterisk} Claude Code v{}", self.version),
            "  Fast terminal coding assistant".into(),
            format!("  {}", self.clawd_pose().ascii_lines()[0]),
        ]
    }

    fn render_full_lines(&self) -> Vec<String> {
        let asterisk = SPINNER_GLYPHS[self.frame as usize % SPINNER_GLYPHS.len()];
        let clawd = self.clawd_pose().ascii_lines();
        vec![
            format!("{asterisk} Welcome to Claude Code v{}", self.version),
            "  ░░░░░░         exploring your workspace".into(),
            "░░░   ░░░░░░░░░  writing, reviewing, and testing".into(),
            format!("      {}", clawd[0]),
            format!("      {}", clawd[1]),
            format!("      {}", clawd[2]),
        ]
    }
}

/// Welcome screen composition built around [`LogoView`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WelcomeView {
    pub title: String,
    pub subtitle: Option<String>,
    pub logo: LogoView,
    pub shortcuts: Vec<String>,
}

impl WelcomeView {
    #[must_use]
    pub fn new(title: impl Into<String>, logo: LogoView) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            logo,
            shortcuts: Vec::new(),
        }
    }

    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    #[must_use]
    pub fn shortcuts(mut self, shortcuts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.shortcuts = shortcuts.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub fn render_lines(&self, width: usize) -> Vec<String> {
        let mut lines = self.logo.render_lines(width);
        lines.push(String::new());
        lines.push(truncate_line(&self.title, width));
        if let Some(subtitle) = &self.subtitle {
            lines.push(truncate_line(subtitle, width));
        }
        if !self.shortcuts.is_empty() {
            lines.push(String::new());
            lines.push("Shortcuts:".into());
            lines.extend(
                self.shortcuts
                    .iter()
                    .map(|shortcut| truncate_line(&format!("  • {shortcut}"), width)),
            );
        }
        lines
    }
}

fn truncate_line(line: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let mut rendered = String::new();
    for ch in line.chars() {
        if rendered.chars().count() >= width {
            break;
        }
        rendered.push(ch);
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clawd_pose_cycles_through_animation_frames() {
        assert_eq!(
            LogoView::new("0.1.0").frame(0).clawd_pose(),
            ClawdPose::Default
        );
        assert_eq!(
            LogoView::new("0.1.0").frame(4).clawd_pose(),
            ClawdPose::LookRight
        );
        assert_eq!(
            LogoView::new("0.1.0").frame(8).clawd_pose(),
            ClawdPose::LookLeft
        );
        assert_eq!(
            LogoView::new("0.1.0").frame(12).clawd_pose(),
            ClawdPose::ArmsUp
        );
    }

    #[test]
    fn condensed_logo_keeps_output_short() {
        let lines = LogoView::new("1.2.3").condensed(true).render_lines(24);

        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("Claude Code"));
        assert!(lines.iter().all(|line| line.chars().count() <= 24));
    }

    #[test]
    fn welcome_view_appends_shortcuts_and_tip() {
        let logo = LogoView::new("1.0.0")
            .emergency_tip("Run /help if startup looks wrong")
            .feed(vec![LogoFeedItem::new(
                "What's new",
                ["Better diffs", "Faster prompts"],
            )]);
        let lines = WelcomeView::new("Ready to pair", logo)
            .subtitle("Ask for changes, tests, or reviews.")
            .shortcuts(["/help", "/config"])
            .render_lines(80);

        assert!(lines.iter().any(|line| line.contains("Tip: Run /help")));
        assert!(lines.iter().any(|line| line.contains("What's new")));
        assert!(lines.iter().any(|line| line.contains("/config")));
    }
}
