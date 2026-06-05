//! Prompt input view models shared by terminal renderers.

use crate::{input::TextBuffer, measure::measure_text, message::HistorySearchView, vim::VimMode};
use wonder_of_u_core::{InputMode, QueuedCommand};

const DEFAULT_QUEUE_PREVIEW_CHARS: usize = 32;
const DEFAULT_VISIBLE_QUEUED_COMMANDS: usize = 3;

/// Describes the prompt glyph for the active input mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromptModeIndicator {
    /// Stores the symbol
    pub symbol: char,
    /// Stores the label
    pub label: &'static str,
}

impl PromptModeIndicator {
    /// Constant fn
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
    /// Stores the pastes
    pub pastes: usize,
    /// Stores the attachments
    pub attachments: usize,
}

impl PromptAttachmentIndicator {
    /// Constant fn
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.pastes == 0 && self.attachments == 0
    }
    /// Handles summary
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
    /// Represents exit confirmation
    ExitConfirmation(String),
    /// Represents pasting
    Pasting,
    /// Represents history search
    HistorySearch {
        /// Stores the query
        query: String,
        /// Stores the failed
        failed: bool,
    },
    /// Represents vim insert
    VimInsert,
    /// Represents mode
    Mode(InputMode),
    /// Represents shortcuts
    Shortcuts,
}

impl PromptFooterHint {
    /// Handles text
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
    /// Stores the exit confirmation key
    pub exit_confirmation_key: Option<String>,
    /// Stores the history search
    pub history_search: Option<HistorySearchView>,
    /// Stores whether pasting
    pub is_pasting: bool,
    /// Stores the show shortcuts hint
    pub show_shortcuts_hint: bool,
}

impl PromptFooterModel {
    /// Handles hints
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
    /// Stores the line count
    pub line_count: usize,
    /// Stores the visible line count
    pub visible_line_count: usize,
    /// Stores the clipped line count
    pub clipped_line_count: usize,
}

impl PromptLayout {
    /// Handles from buffer
    #[must_use]
    pub fn from_buffer(
        buffer: &TextBuffer,
        width: usize,
        max_visible_lines: Option<usize>,
    ) -> Self {
        Self::from_text(&buffer.text(), width, max_visible_lines)
    }
    /// Handles from text
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
    /// Constant fn
    #[must_use]
    pub const fn is_multiline(self) -> bool {
        self.line_count > 1
    }
}

/// A single queued command preview line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptQueuedCommandView {
    /// Stores the index
    pub index: usize,
    /// Stores the preview
    pub preview: String,
}

/// Visible queued-command state for prompt footers and overlays.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PromptQueueView {
    /// Stores the commands
    pub commands: Vec<PromptQueuedCommandView>,
    /// Stores the hidden count
    pub hidden_count: usize,
}

impl PromptQueueView {
    /// Handles from commands
    #[must_use]
    pub fn from_commands(commands: &[QueuedCommand]) -> Option<Self> {
        Self::from_commands_with_limits(
            commands,
            DEFAULT_VISIBLE_QUEUED_COMMANDS,
            DEFAULT_QUEUE_PREVIEW_CHARS,
        )
    }
    /// Handles from commands with limits
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
    /// Handles lines
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
    /// Handles overflow label
    #[must_use]
    pub fn overflow_label(&self) -> Option<String> {
        (self.hidden_count > 0).then(|| format!("+{} more queued", self.hidden_count))
    }
}

/// A single command suggestion shown below the prompt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptSuggestion {
    /// Stores the id
    pub id: String,
    /// Stores the display text
    pub display_text: String,
    /// Stores the replacement
    pub replacement: String,
    /// Stores the description
    pub description: Option<String>,
    /// Stores the tag
    pub tag: Option<String>,
    /// Stores the keywords
    pub keywords: Vec<String>,
}

impl PromptSuggestion {
    /// Creates a new value
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
    /// Handles with description
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
    /// Handles with tag
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }
    /// Handles with keywords
    #[must_use]
    pub fn with_keywords(mut self, keywords: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.keywords = keywords.into_iter().map(Into::into).collect();
        self
    }

    fn match_score(&self, filter: &str) -> Option<i64> {
        // Strip leading '/' so "the" scores as a prefix of "theme", not a mid-string hit.
        // Score each field independently and take the max so the visible command name
        // drives ranking rather than the opaque id field.
        let display = self.display_text.strip_prefix('/').unwrap_or(&self.display_text);
        let replacement = self.replacement.strip_prefix('/').unwrap_or(&self.replacement);
        let description = self.description.as_deref().unwrap_or_default();
        let tag = self.tag.as_deref().unwrap_or_default();
        let keywords = self.keywords.join(" ");

        [
            (display, 0i64),
            (replacement, -100),
            (self.id.as_str(), -2000),
            (description, -3000),
            (tag, -4000),
            (keywords.as_str(), -4000),
        ]
        .iter()
        .filter(|(f, _)| !f.is_empty())
        .filter_map(|(f, penalty)| fuzzy_match_score(filter, f).map(|s| s + penalty))
        .max()
    }
}

/// Mutable suggestion state including filter text and active selection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PromptSuggestionState {
    /// Stores the suggestions
    pub suggestions: Vec<PromptSuggestion>,
    /// Stores the filter
    pub filter: String,
    /// Stores the selected index
    pub selected_index: usize,
}

impl PromptSuggestionState {
    /// Creates a new value
    #[must_use]
    pub fn new(suggestions: impl IntoIterator<Item = PromptSuggestion>) -> Self {
        Self {
            suggestions: suggestions.into_iter().collect(),
            filter: String::new(),
            selected_index: 0,
        }
    }

    /// Handles set filter
    pub fn set_filter(&mut self, filter: impl Into<String>) {
        self.filter = filter.into();
        self.selected_index = 0;
    }
    /// Handles filtered
    #[must_use]
    pub fn filtered(&self) -> Vec<&PromptSuggestion> {
        let mut filtered = self
            .suggestions
            .iter()
            .enumerate()
            .filter_map(|(index, suggestion)| {
                suggestion
                    .match_score(&self.filter)
                    .map(|score| (index, score, suggestion))
            })
            .collect::<Vec<_>>();
        filtered.sort_by(
            |(left_index, left_score, _), (right_index, right_score, _)| {
                right_score
                    .cmp(left_score)
                    .then_with(|| left_index.cmp(right_index))
            },
        );
        filtered
            .into_iter()
            .map(|(_, _, suggestion)| suggestion)
            .collect()
    }
    /// Handles selected
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
    /// Stores the buffer
    pub buffer: TextBuffer,
    /// Stores the input mode
    pub input_mode: InputMode,
    /// Stores the vim mode
    pub vim_mode: Option<VimMode>,
    /// Stores the footer
    pub footer: PromptFooterModel,
    /// Stores the attachments
    pub attachments: PromptAttachmentIndicator,
    /// Stores the queued commands
    pub queued_commands: Vec<QueuedCommand>,
    /// Stores the suggestions
    pub suggestions: PromptSuggestionState,
}

impl Default for PromptInputModel {
    fn default() -> Self {
        Self::new(false)
    }
}

impl PromptInputModel {
    /// Creates a new value
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
    /// Handles from text
    #[must_use]
    pub fn from_text(text: impl AsRef<str>, multiline: bool) -> Self {
        Self {
            buffer: TextBuffer::from_text(text, multiline),
            ..Self::new(multiline)
        }
    }
    /// Handles mode indicator
    #[must_use]
    pub fn mode_indicator(&self) -> PromptModeIndicator {
        PromptModeIndicator::from_mode(self.input_mode)
    }
    /// Handles footer hints
    #[must_use]
    pub fn footer_hints(&self) -> Vec<PromptFooterHint> {
        self.footer.hints(self.input_mode, self.vim_mode)
    }
    /// Handles layout
    #[must_use]
    pub fn layout(&self, width: usize, max_visible_lines: Option<usize>) -> PromptLayout {
        PromptLayout::from_buffer(&self.buffer, width, max_visible_lines)
    }
    /// Handles queue view
    #[must_use]
    pub fn queue_view(&self) -> Option<PromptQueueView> {
        PromptQueueView::from_commands(&self.queued_commands)
    }
    /// Handles attachment summary
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

fn normalized_filter_terms(filter: &str) -> Vec<String> {
    filter
        .split_whitespace()
        .map(normalize_for_match)
        .filter(|term| !term.is_empty())
        .collect()
}

/// Returns a fuzzy-match score for `filter` against `text`.
///
/// Higher scores are better. The matcher prefers exact prefix matches, then
/// exact substring matches, then fuzzy subsequence matches. Every whitespace-
/// separated query term must match for the overall score to be returned.
#[must_use]
pub fn fuzzy_match_score(filter: &str, text: &str) -> Option<i64> {
    let terms = normalized_filter_terms(filter);
    if terms.is_empty() {
        return Some(0);
    }

    let haystack = normalize_for_match(text);
    terms.iter().try_fold(0_i64, |score, term| {
        score_term(&haystack, term).map(|term_score| score + term_score)
    })
}

fn score_term(haystack: &str, term: &str) -> Option<i64> {
    if term.is_empty() {
        return Some(0);
    }

    if haystack == term {
        return Some(100_000);
    }

    if haystack.starts_with(term) {
        return Some(90_000 - haystack.chars().count() as i64);
    }

    if let Some(index) = haystack.find(term) {
        return Some(
            75_000
                - (index as i64 * 100)
                - (haystack
                    .chars()
                    .count()
                    .saturating_sub(term.chars().count()) as i64),
        );
    }

    fuzzy_subsequence_score(haystack, term)
}

fn fuzzy_subsequence_score(haystack: &str, needle: &str) -> Option<i64> {
    let haystack = haystack.chars().collect::<Vec<_>>();
    let needle = needle.chars().collect::<Vec<_>>();
    let mut search_start = 0usize;
    let mut previous_index = None;
    let mut score = 10_000_i64;

    for ch in &needle {
        let relative_index = haystack
            .iter()
            .enumerate()
            .skip(search_start)
            .find_map(|(index, candidate)| (*candidate == *ch).then_some(index))?;
        if let Some(previous_index) = previous_index {
            let gap = relative_index.saturating_sub(previous_index + 1);
            if gap == 0 {
                score += 150;
            } else {
                score -= gap as i64 * 10;
            }
        } else {
            score -= relative_index as i64 * 25;
        }

        if relative_index == 0
            || !haystack
                .get(relative_index.saturating_sub(1))
                .is_some_and(|value| value.is_alphanumeric())
        {
            score += 100;
        }

        previous_index = Some(relative_index);
        search_start = relative_index + 1;
    }

    score -= haystack.len().saturating_sub(needle.len()) as i64;
    Some(score)
}

/// Returns char-index ranges in `display` that should be underlined to show the match.
///
/// Tries substring match first (a contiguous range); falls back to fuzzy subsequence
/// positions. The leading `/` in slash commands is skipped during search but its index
/// is preserved in the returned ranges.
#[must_use]
pub fn find_match_chars(display: &str, filter: &str) -> Vec<(usize, usize)> {
    if filter.is_empty() || display.is_empty() {
        return Vec::new();
    }

    let char_offset = usize::from(display.starts_with('/'));
    let search_target = &display[char_offset..];
    let target_lower = search_target.to_lowercase();
    let filter_lower = filter.to_lowercase();

    if let Some(byte_idx) = target_lower.find(&filter_lower) {
        let char_start = char_offset + target_lower[..byte_idx].chars().count();
        let char_end = char_start + filter_lower.chars().count();
        return vec![(char_start, char_end)];
    }

    let display_chars: Vec<char> = display.chars().collect();
    let filter_chars: Vec<char> = filter_lower.chars().collect();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut search_start = char_offset;

    for needle_ch in &filter_chars {
        let found = display_chars[search_start..]
            .iter()
            .enumerate()
            .find_map(|(i, c)| {
                (c.to_lowercase().next() == Some(*needle_ch)).then_some(search_start + i)
            });
        match found {
            Some(pos) => {
                ranges.push((pos, pos + 1));
                search_start = pos + 1;
            }
            None => return Vec::new(),
        }
    }

    merge_char_ranges(ranges)
}

fn merge_char_ranges(ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    if ranges.len() <= 1 {
        return ranges;
    }
    let mut merged: Vec<(usize, usize)> = Vec::new();
    let mut current = ranges[0];
    for &(start, end) in &ranges[1..] {
        if start <= current.1 {
            current.1 = current.1.max(end);
        } else {
            merged.push(current);
            current = (start, end);
        }
    }
    merged.push(current);
    merged
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

    #[test]
    fn fuzzy_match_score_prefers_prefix_then_substring_then_subsequence() {
        let prefix = fuzzy_match_score("the", "/theme").expect("prefix match");
        let substring = fuzzy_match_score("heme", "/theme").expect("substring match");
        let subsequence = fuzzy_match_score("thm", "/theme").expect("subsequence match");

        assert!(prefix > substring);
        assert!(substring > subsequence);
        assert!(fuzzy_match_score("zzz", "/theme").is_none());
    }

    #[test]
    fn suggestion_filtering_supports_fuzzy_abbreviations_and_ranking() {
        let mut suggestions = PromptSuggestionState::new([
            PromptSuggestion::new("command-theme", "/theme", "/theme")
                .with_description("Switch the theme"),
            PromptSuggestion::new("command-status", "/status", "/status"),
            PromptSuggestion::new(
                "command-terminal-setup",
                "/terminal-setup",
                "/terminal-setup",
            ),
        ]);

        suggestions.set_filter("thm");
        let filtered = suggestions.filtered();

        assert_eq!(
            filtered
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["command-theme"]
        );

        suggestions.set_filter("st");
        let filtered = suggestions.filtered();
        assert_eq!(
            filtered
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["command-status", "command-terminal-setup", "command-theme"]
        );
    }

    fn queued(command: &str) -> QueuedCommand {
        QueuedCommand {
            command: command.into(),
            placement: QueuePlacement::Later,
        }
    }
}
