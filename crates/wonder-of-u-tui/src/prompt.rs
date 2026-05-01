//! Prompt input view models shared by terminal renderers.

use crate::{input::TextBuffer, measure::measure_text, message::HistorySearchView, vim::VimMode};
use wonder_of_u_core::{InputMode, QueuedCommand};

const DEFAULT_QUEUE_PREVIEW_CHARS: usize = 32;
const DEFAULT_VISIBLE_QUEUED_COMMANDS: usize = 3;

/// Describes the prompt glyph for the active input mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromptModeIndicator {
    pub symbol: char,
    pub label: &'static str,
}

impl PromptModeIndicator {
    #[must_use]
    pub const fn from_mode(mode: InputMode) -> Self {
        match mode {
            InputMode::Bash => Self {
                symbol: '!',
                label: "bash",
            },
            _ => Self {
                symbol: '❯',
                label: "prompt",
            },
        }
    }
}

/// Aggregates visible paste and attachment counts for the prompt footer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PromptAttachmentIndicator {
    pub pastes: usize,
    pub attachments: usize,
}

impl PromptAttachmentIndicator {
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.pastes == 0 && self.attachments == 0
    }

    #[must_use]
    pub fn summary(self) -> Option<String> {
        if self.is_empty() {
            return None;
        }

        let mut parts = Vec::with_capacity(2);
        if self.pastes > 0 {
            parts.push(count_label(self.pastes, "paste", "pastes"));
        }
        if self.attachments > 0 {
            parts.push(count_label(self.attachments, "attachment", "attachments"));
        }
        Some(parts.join(" • "))
    }
}

/// Footer text emitted by the prompt input family.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PromptFooterHint {
    ExitConfirmation(String),
    Pasting,
    HistorySearch { query: String, failed: bool },
    VimInsert,
    Mode(InputMode),
    Shortcuts,
}

impl PromptFooterHint {
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::ExitConfirmation(key) => format!("Press {key} again to exit"),
            Self::Pasting => "Pasting text…".into(),
            Self::HistorySearch { query, failed } => {
                let status = if *failed { " (no match)" } else { "" };
                format!("history> {query}{status}")
            }
            Self::VimInsert => "-- INSERT --".into(),
            Self::Mode(InputMode::Bash) => "! for bash mode".into(),
            Self::Mode(InputMode::PermissionPending) => "awaiting permission".into(),
            Self::Mode(InputMode::TaskNotification) => "task notification".into(),
            Self::Mode(InputMode::Prompt) | Self::Shortcuts => "? for shortcuts".into(),
        }
    }
}

/// Captures footer state without tying it to any specific renderer.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PromptFooterModel {
    pub exit_confirmation_key: Option<String>,
    pub history_search: Option<HistorySearchView>,
    pub is_pasting: bool,
    pub show_shortcuts_hint: bool,
}

impl PromptFooterModel {
    #[must_use]
    pub fn hints(&self, mode: InputMode, vim_mode: Option<VimMode>) -> Vec<PromptFooterHint> {
        if let Some(key) = &self.exit_confirmation_key {
            return vec![PromptFooterHint::ExitConfirmation(key.clone())];
        }

        if self.is_pasting {
            return vec![PromptFooterHint::Pasting];
        }

        let mut hints = Vec::with_capacity(2);

        if let Some(search) = &self.history_search {
            hints.push(PromptFooterHint::HistorySearch {
                query: search.query.clone(),
                failed: search.match_total == 0,
            });
        } else if matches!(vim_mode, Some(VimMode::Insert)) {
            hints.push(PromptFooterHint::VimInsert);
        }

        if mode != InputMode::Prompt {
            hints.push(PromptFooterHint::Mode(mode));
        } else if self.show_shortcuts_hint && hints.is_empty() {
            hints.push(PromptFooterHint::Shortcuts);
        }

        hints
    }
}

/// Wrapped prompt sizing data for renderer layout decisions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromptLayout {
    pub line_count: usize,
    pub visible_line_count: usize,
    pub clipped_line_count: usize,
}

impl PromptLayout {
    #[must_use]
    pub fn from_buffer(
        buffer: &TextBuffer,
        width: usize,
        max_visible_lines: Option<usize>,
    ) -> Self {
        Self::from_text(&buffer.text(), width, max_visible_lines)
    }

    #[must_use]
    pub fn from_text(text: &str, width: usize, max_visible_lines: Option<usize>) -> Self {
        let line_count = measure_text(text, width).height.max(1);
        let visible_line_count = max_visible_lines
            .map(|limit| line_count.min(limit.max(1)))
            .unwrap_or(line_count);

        Self {
            line_count,
            visible_line_count,
            clipped_line_count: line_count.saturating_sub(visible_line_count),
        }
    }

    #[must_use]
    pub const fn is_multiline(self) -> bool {
        self.line_count > 1
    }
}

/// A single queued command preview line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptQueuedCommandView {
    pub index: usize,
    pub preview: String,
}

/// Visible queued-command state for prompt footers and overlays.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PromptQueueView {
    pub commands: Vec<PromptQueuedCommandView>,
    pub hidden_count: usize,
}

impl PromptQueueView {
    #[must_use]
    pub fn from_commands(commands: &[QueuedCommand]) -> Option<Self> {
        Self::from_commands_with_limits(
            commands,
            DEFAULT_VISIBLE_QUEUED_COMMANDS,
            DEFAULT_QUEUE_PREVIEW_CHARS,
        )
    }

    #[must_use]
    pub fn from_commands_with_limits(
        commands: &[QueuedCommand],
        max_visible: usize,
        max_preview_chars: usize,
    ) -> Option<Self> {
        if commands.is_empty() || max_visible == 0 {
            return None;
        }

        let visible = commands
            .iter()
            .take(max_visible)
            .enumerate()
            .map(|(index, command)| PromptQueuedCommandView {
                index: index + 1,
                preview: queued_command_preview(command, max_preview_chars),
            })
            .collect();

        Some(Self {
            commands: visible,
            hidden_count: commands.len().saturating_sub(max_visible),
        })
    }

    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut lines = self
            .commands
            .iter()
            .map(|command| format!("{}. {}", command.index, command.preview))
            .collect::<Vec<_>>();
        if let Some(overflow) = self.overflow_label() {
            lines.push(overflow);
        }
        lines
    }

    #[must_use]
    pub fn overflow_label(&self) -> Option<String> {
        (self.hidden_count > 0).then(|| format!("+{} more queued", self.hidden_count))
    }
}

/// A single command suggestion shown below the prompt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptSuggestion {
    pub id: String,
    pub display_text: String,
    pub replacement: String,
    pub description: Option<String>,
    pub tag: Option<String>,
    pub keywords: Vec<String>,
}

impl PromptSuggestion {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        display_text: impl Into<String>,
        replacement: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            display_text: display_text.into(),
            replacement: replacement.into(),
            description: None,
            tag: None,
            keywords: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    #[must_use]
    pub fn with_keywords(mut self, keywords: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.keywords = keywords.into_iter().map(Into::into).collect();
        self
    }

    fn matches_filter(&self, filter_terms: &[String]) -> bool {
        if filter_terms.is_empty() {
            return true;
        }

        let search_text = normalize_for_match(
            &[
                self.id.as_str(),
                self.display_text.as_str(),
                self.replacement.as_str(),
                self.description.as_deref().unwrap_or_default(),
                self.tag.as_deref().unwrap_or_default(),
                &self.keywords.join(" "),
            ]
            .join(" "),
        );

        filter_terms
            .iter()
            .all(|term| search_text.contains(term.as_str()))
    }
}

/// Mutable suggestion state including filter text and active selection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PromptSuggestionState {
    pub suggestions: Vec<PromptSuggestion>,
    pub filter: String,
    pub selected_index: usize,
}

impl PromptSuggestionState {
    #[must_use]
    pub fn new(suggestions: impl IntoIterator<Item = PromptSuggestion>) -> Self {
        Self {
            suggestions: suggestions.into_iter().collect(),
            filter: String::new(),
            selected_index: 0,
        }
    }

    pub fn set_filter(&mut self, filter: impl Into<String>) {
        self.filter = filter.into();
        self.selected_index = 0;
    }

    #[must_use]
    pub fn filtered(&self) -> Vec<&PromptSuggestion> {
        let terms = self
            .filter
            .split_whitespace()
            .map(normalize_for_match)
            .filter(|term| !term.is_empty())
            .collect::<Vec<_>>();

        self.suggestions
            .iter()
            .filter(|suggestion| suggestion.matches_filter(&terms))
            .collect()
    }

    #[must_use]
    pub fn selected(&self) -> Option<&PromptSuggestion> {
        let filtered = self.filtered();
        filtered
            .get(self.selected_index.min(filtered.len().saturating_sub(1)))
            .copied()
    }
}

/// Renderer-agnostic state for the prompt input family.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptInputModel {
    pub buffer: TextBuffer,
    pub input_mode: InputMode,
    pub vim_mode: Option<VimMode>,
    pub footer: PromptFooterModel,
    pub attachments: PromptAttachmentIndicator,
    pub queued_commands: Vec<QueuedCommand>,
    pub suggestions: PromptSuggestionState,
}

impl Default for PromptInputModel {
    fn default() -> Self {
        Self::new(false)
    }
}

impl PromptInputModel {
    #[must_use]
    pub fn new(multiline: bool) -> Self {
        Self {
            buffer: TextBuffer::new(multiline),
            input_mode: InputMode::Prompt,
            vim_mode: None,
            footer: PromptFooterModel::default(),
            attachments: PromptAttachmentIndicator::default(),
            queued_commands: Vec::new(),
            suggestions: PromptSuggestionState::default(),
        }
    }

    #[must_use]
    pub fn from_text(text: impl AsRef<str>, multiline: bool) -> Self {
        Self {
            buffer: TextBuffer::from_text(text, multiline),
            ..Self::new(multiline)
        }
    }

    #[must_use]
    pub fn mode_indicator(&self) -> PromptModeIndicator {
        PromptModeIndicator::from_mode(self.input_mode)
    }

    #[must_use]
    pub fn footer_hints(&self) -> Vec<PromptFooterHint> {
        self.footer.hints(self.input_mode, self.vim_mode)
    }

    #[must_use]
    pub fn layout(&self, width: usize, max_visible_lines: Option<usize>) -> PromptLayout {
        PromptLayout::from_buffer(&self.buffer, width, max_visible_lines)
    }

    #[must_use]
    pub fn queue_view(&self) -> Option<PromptQueueView> {
        PromptQueueView::from_commands(&self.queued_commands)
    }

    #[must_use]
    pub fn attachment_summary(&self) -> Option<String> {
        self.attachments.summary()
    }
}

fn count_label(count: usize, singular: &str, plural: &str) -> String {
    let label = if count == 1 { singular } else { plural };
    format!("{count} {label}")
}

fn normalize_for_match(text: &str) -> String {
    text.to_lowercase()
}

fn queued_command_preview(command: &QueuedCommand, max_preview_chars: usize) -> String {
    let normalized = command
        .command
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return "(empty command)".into();
    }

    truncate_with_ellipsis(&normalized, max_preview_chars)
}

fn truncate_with_ellipsis(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }

    if max_chars <= 1 {
        return "…".into();
    }

    let mut truncated = text.chars().take(max_chars - 1).collect::<String>();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use wonder_of_u_core::QueuePlacement;

    use super::*;

    #[test]
    fn empty_prompt_uses_single_visible_line() {
        let mut prompt = PromptInputModel::new(true);
        prompt.footer.show_shortcuts_hint = true;

        assert_eq!(
            prompt.layout(80, Some(4)),
            PromptLayout {
                line_count: 1,
                visible_line_count: 1,
                clipped_line_count: 0,
            }
        );
        assert_eq!(prompt.footer_hints(), vec![PromptFooterHint::Shortcuts]);
        assert_eq!(
            prompt.mode_indicator(),
            PromptModeIndicator {
                symbol: '❯',
                label: "prompt",
            }
        );
    }

    #[test]
    fn multiline_prompt_sizing_counts_wrap_and_clamps_visibility() {
        let prompt = PromptInputModel::from_text("abcd\nefghij", true);

        assert_eq!(
            prompt.layout(4, Some(2)),
            PromptLayout {
                line_count: 3,
                visible_line_count: 2,
                clipped_line_count: 1,
            }
        );
    }

    #[test]
    fn queued_command_overflow_reports_hidden_entries() {
        let mut prompt = PromptInputModel::new(false);
        prompt.queued_commands = vec![
            queued("/status"),
            queued("draft the migration plan"),
            queued("/theme midnight"),
            queued("summarize the open tasks in detail"),
        ];

        let queue = prompt.queue_view().expect("queued commands");

        assert_eq!(
            queue.lines(),
            vec![
                "1. /status",
                "2. draft the migration plan",
                "3. /theme midnight",
                "+1 more queued",
            ]
        );
    }

    #[test]
    fn mode_changes_update_indicator_and_footer_hints() {
        let mut prompt = PromptInputModel::new(false);
        prompt.footer.show_shortcuts_hint = true;
        assert_eq!(prompt.footer_hints(), vec![PromptFooterHint::Shortcuts]);

        prompt.input_mode = InputMode::Bash;
        assert_eq!(
            prompt.mode_indicator(),
            PromptModeIndicator {
                symbol: '!',
                label: "bash",
            }
        );
        assert_eq!(
            prompt.footer_hints(),
            vec![PromptFooterHint::Mode(InputMode::Bash)]
        );

        prompt.input_mode = InputMode::Prompt;
        prompt.footer.show_shortcuts_hint = false;
        prompt.vim_mode = Some(VimMode::Insert);
        assert_eq!(prompt.footer_hints(), vec![PromptFooterHint::VimInsert]);
    }

    #[test]
    fn paste_and_attachment_indicators_summarize_counts() {
        let indicator = PromptAttachmentIndicator {
            pastes: 2,
            attachments: 1,
        };

        assert_eq!(
            indicator.summary().as_deref(),
            Some("2 pastes • 1 attachment")
        );
        assert!(PromptAttachmentIndicator::default().summary().is_none());
    }

    #[test]
    fn suggestion_filtering_is_case_insensitive_and_clamps_selection() {
        let mut suggestions = PromptSuggestionState::new([
            PromptSuggestion::new("command-status", "/status", "/status")
                .with_description("Show current session status"),
            PromptSuggestion::new("command-theme", "/theme", "/theme")
                .with_description("Switch the theme")
                .with_keywords(["appearance", "colors"]),
            PromptSuggestion::new("file-plan", "plan.md", "plan.md")
                .with_tag("file")
                .with_keywords(["workspace"]),
        ]);

        suggestions.set_filter("THEME");
        suggestions.selected_index = 8;

        let filtered = suggestions.filtered();

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "command-theme");
        assert_eq!(
            suggestions.selected().map(|item| item.id.as_str()),
            Some("command-theme")
        );
    }

    fn queued(command: &str) -> QueuedCommand {
        QueuedCommand {
            command: command.into(),
            placement: QueuePlacement::Later,
        }
    }
}
