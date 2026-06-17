use std::{collections::HashMap, path::PathBuf, sync::LazyLock};

use ratatui::{
    style::{Color as RatatuiColor, Modifier, Style as RatatuiStyle},
    text::{Line, Span},
};

use serde_json::Value;
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Theme, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
};
use time::OffsetDateTime;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use wonder_of_u_core::{MessageEnvelope, MessagePayload, ToolUseId};

use crate::{
    diff::{FileEditHunkSummary, PathLinkView},
    measure::{line_width, strip_ansi, wrap_text_hard},
    style::{Color, TextStyle},
};

use super::markdown_render;
use super::{MessageLineView, MessageRole, MessageSpanView, render_message};

const MAX_THINKING_LINES: usize = 4;
const MAX_DETAIL_LINES: usize = 3;

/// Ink-style guide prefix for tool result/detail/progress continuation lines.
///
/// Mirrors the Claude Code visual: `"  ⎿  "` (2 sp + U+23BF + 2 sp).
/// The two leading spaces come from `push_wrapped_block`'s `""` prefix
/// logic, so only the `⎿  ` portion is stored here.
const GUIDE_PREFIX: &str = "⎿  ";

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static HIGHLIGHT_THEME: LazyLock<Option<Theme>> = LazyLock::new(|| {
    let themes = ThemeSet::load_defaults();
    themes
        .themes
        .get("base16-ocean.dark")
        .or_else(|| themes.themes.get("Solarized (dark)"))
        .or_else(|| {
            themes
                .themes
                .iter()
                .find(|(name, _)| name.to_ascii_lowercase().contains("dark"))
                .map(|(_, theme)| theme)
        })
        .or_else(|| themes.themes.values().next())
        .cloned()
});

/// Rich, renderer-agnostic message summaries for the TUI transcript.
#[derive(Clone, Debug, PartialEq)]
pub enum RichMessageView {
    /// Represents markdown
    Markdown(MarkdownSummaryView),
    /// Represents thinking
    Thinking(ThinkingBlockView),
    /// Represents tool group
    ToolGroup(GroupedToolCallView),
    /// Represents collapsed read/search group
    CollapsedReadSearch(CollapsedReadSearchGroupView),
    /// Represents file edit reference
    FileEditReference(FileEditReferenceView),
    /// Represents attachment
    Attachment(AttachmentSummaryView),
    /// Represents system error
    SystemError(SystemErrorView),
    /// Represents boundary
    Boundary(TranscriptBoundaryView),
    /// Represents fallback
    Fallback(Vec<MessageLineView>),
}

impl RichMessageView {
    /// Handles display lines
    #[must_use]
    pub fn display_lines(&self, max_width: usize, expand_output: bool) -> Vec<MessageLineView> {
        match self {
            Self::Markdown(view) => view.display_lines(max_width),
            Self::Thinking(view) => view.display_lines(max_width),
            Self::ToolGroup(view) => view.display_lines(max_width, expand_output),
            Self::CollapsedReadSearch(view) => view.display_lines(max_width),
            Self::FileEditReference(view) => view.display_lines(max_width),
            Self::Attachment(view) => view.display_lines(max_width),
            Self::SystemError(view) => view.display_lines(max_width),
            Self::Boundary(view) => view.display_lines(max_width),
            Self::Fallback(lines) => truncate_existing_lines(lines, max_width),
        }
    }
}

/// Builds richer message summaries while preserving legacy `message_lines`.
#[must_use]
pub fn rich_message_views(
    messages: &[MessageEnvelope],
    expand_output: bool,
) -> Vec<RichMessageView> {
    rich_message_views_indexed(messages, expand_output, None)
        .into_iter()
        .map(|(v, _)| v)
        .collect()
}

pub(super) fn rich_message_views_indexed(
    messages: &[MessageEnvelope],
    expand_output: bool,
    streaming_override: Option<&(usize, Vec<MessageLineView>)>,
) -> Vec<(RichMessageView, usize)> {
    let mut views = Vec::new();
    let mut index = 0usize;

    while index < messages.len() {
        if let Some((override_index, lines)) = streaming_override {
            if index == *override_index {
                views.push((RichMessageView::Fallback(lines.clone()), 1));
                index = index.saturating_add(1);
                continue;
            }
        }

        if let Some((group, consumed)) = grouped_tool_call_view(&messages[index..]) {
            views.push((RichMessageView::ToolGroup(group), consumed));
            index = index.saturating_add(consumed);
            continue;
        }

        views.push((single_message_view(&messages[index]), 1));
        index = index.saturating_add(1);
    }

    if expand_output {
        return views;
    }

    collapse_consecutive_tool_groups(views)
}

fn is_collapsible_tool(tool: &str) -> bool {
    let lower = tool.to_ascii_lowercase();
    lower == "file_read"
        || lower == "glob"
        || lower == "grep"
        || lower == "rg"
        || lower == "search"
        || lower == "find"
}

fn merge_tool_group_into_collapsed(
    group: &GroupedToolCallView,
    collapsed: &mut CollapsedReadSearchGroupView,
) {
    let lower = group.tool.to_ascii_lowercase();
    if lower == "file_read" {
        collapsed.read_count += group.calls.len();
        for call in &group.calls {
            if let Some(input) = &call.input {
                if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
                    collapsed.file_paths.push(path.to_string());
                }
            }
        }
    } else {
        collapsed.search_count += group.calls.len();
        for call in &group.calls {
            if let Some(input) = &call.input {
                let pattern = input
                    .get("pattern")
                    .or_else(|| input.get("query"))
                    .or_else(|| input.get("prompt"))
                    .and_then(|v| v.as_str());
                if let Some(p) = pattern {
                    collapsed.search_patterns.push(p.to_string());
                }
            }
        }
    }
}

fn collapse_consecutive_tool_groups(
    views: Vec<(RichMessageView, usize)>,
) -> Vec<(RichMessageView, usize)> {
    let mut result: Vec<(RichMessageView, usize)> = Vec::new();
    let mut i = 0;

    while i < views.len() {
        let current = &views[i];
        if let RichMessageView::ToolGroup(group) = &current.0 {
            if is_collapsible_tool(&group.tool) {
                let mut collapsed = CollapsedReadSearchGroupView::default();
                let mut total_consumed = 0usize;
                let mut group_count = 0usize;

                while i < views.len() {
                    match &views[i].0 {
                        RichMessageView::ToolGroup(next) if is_collapsible_tool(&next.tool) => {
                            merge_tool_group_into_collapsed(next, &mut collapsed);
                            total_consumed += views[i].1;
                            group_count += 1;
                            i += 1;
                        }
                        _ => break,
                    }
                }

                if group_count > 1
                    || (group_count == 1 && collapsed.read_count + collapsed.search_count > 1)
                {
                    result.push((
                        RichMessageView::CollapsedReadSearch(collapsed),
                        total_consumed,
                    ));
                } else {
                    result.push((RichMessageView::ToolGroup(group.clone()), total_consumed));
                }
                continue;
            }
        }
        result.push(views[i].clone());
        i += 1;
    }

    result
}

fn single_message_view(message: &MessageEnvelope) -> RichMessageView {
    match &message.payload {
        MessagePayload::UserText { content } => RichMessageView::Markdown(
            MarkdownSummaryView::new_with_cwd(MessageRole::User, content, message.cwd.clone()),
        ),
        MessagePayload::AssistantText { content } => {
            if let Some(view) = SystemErrorView::detect(MessageRole::Assistant, content) {
                RichMessageView::SystemError(view)
            } else {
                RichMessageView::Markdown(MarkdownSummaryView::with_timestamp_and_cwd(
                    MessageRole::Assistant,
                    content,
                    Some(message.timestamp),
                    message.cwd.clone(),
                ))
            }
        }
        MessagePayload::System { content } => {
            if let Some(view) = SystemErrorView::detect(MessageRole::System, content) {
                RichMessageView::SystemError(view)
            } else {
                RichMessageView::Markdown(MarkdownSummaryView::new_with_cwd(
                    MessageRole::System,
                    content,
                    message.cwd.clone(),
                ))
            }
        }
        MessagePayload::AssistantThinking { content, collapsed } => {
            RichMessageView::Thinking(ThinkingBlockView::new(content, *collapsed))
        }
        MessagePayload::UserAttachment { label, uri } => {
            RichMessageView::Attachment(AttachmentSummaryView::new(label, uri))
        }
        MessagePayload::CompactBoundary { summary } => {
            RichMessageView::Boundary(TranscriptBoundaryView::new(summary))
        }
        MessagePayload::HookProgress {
            event,
            tool_name,
            hook_count,
            success,
        } => {
            let icon = if *success { "⚙" } else { "⚠" };
            let noun = if *hook_count == 1 { "hook" } else { "hooks" };
            RichMessageView::Fallback(vec![MessageLineView::new(
                format!("{icon} {hook_count} {event} {noun} ran for {tool_name}"),
                MessageRole::System,
            )])
        }
        _ => {
            let mut lines = Vec::new();
            render_message(message, &mut lines);
            RichMessageView::Fallback(lines)
        }
    }
}

fn grouped_tool_call_view(messages: &[MessageEnvelope]) -> Option<(GroupedToolCallView, usize)> {
    let first = messages.first()?;
    let tool = match &first.payload {
        MessagePayload::AssistantToolUse { tool, .. } | MessagePayload::ToolResult { tool, .. } => {
            tool.as_str()
        }
        _ => return None,
    };

    let mut view = GroupedToolCallView::new(tool);
    let mut started_at = HashMap::new();
    let mut consumed = 0usize;

    for message in messages {
        match &message.payload {
            MessagePayload::AssistantToolUse {
                tool: current_tool,
                use_id,
                input,
            } if current_tool == tool => {
                started_at.insert(*use_id, message.timestamp);
                view.push_use(*use_id, Some(input.clone()));
                consumed = consumed.saturating_add(1);
            }
            MessagePayload::ToolResult {
                tool: current_tool,
                use_id,
                success,
                content,
            } if current_tool == tool => {
                let elapsed_secs = started_at
                    .get(use_id)
                    .map(|started| (message.timestamp - *started).as_seconds_f64());
                view.push_result(*use_id, *success, content.clone(), elapsed_secs);
                consumed = consumed.saturating_add(1);
            }
            _ => break,
        }
    }

    Some((view, consumed.max(1)))
}

/// A markdown-like assistant or system message summarized for terminal rendering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownSummaryView {
    /// Stores the role
    pub role: MessageRole,
    /// Stores the optional timestamp shown after the first assistant line.
    pub timestamp: Option<OffsetDateTime>,
    /// Stores the normalized markdown source used by the renderer.
    pub source: String,
    /// Stores the working directory used to shorten local file links.
    pub cwd: Option<PathBuf>,
    /// Stores the blocks
    pub blocks: Vec<MarkdownBlockView>,
}

impl MarkdownSummaryView {
    /// Creates a new value
    #[must_use]
    pub fn new(role: MessageRole, text: &str) -> Self {
        Self::with_timestamp(role, text, None)
    }

    #[must_use]
    fn new_with_cwd(role: MessageRole, text: &str, cwd: Option<PathBuf>) -> Self {
        Self::with_timestamp_and_cwd(role, text, None, cwd)
    }

    #[must_use]
    fn with_timestamp(role: MessageRole, text: &str, timestamp: Option<OffsetDateTime>) -> Self {
        Self::with_timestamp_and_cwd(role, text, timestamp, None)
    }

    #[must_use]
    fn with_timestamp_and_cwd(
        role: MessageRole,
        text: &str,
        timestamp: Option<OffsetDateTime>,
        cwd: Option<PathBuf>,
    ) -> Self {
        let source = normalized_markdown_source(role, text);
        Self {
            role,
            timestamp,
            blocks: parse_markdown_blocks(&source),
            source,
            cwd,
        }
    }

    /// Handles display lines
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        let prefix = role_prefix(self.role);
        let mut lines = markdown_render::render_markdown_text_with_width_and_cwd(
            &self.source,
            max_width,
            self.cwd.as_deref(),
            self.role,
            prefix,
        );

        if lines.is_empty() {
            lines.push(MessageLineView::new(prefix.to_string(), self.role));
        }

        if let Some(timestamp) = self
            .timestamp
            .filter(|_| self.role == MessageRole::Assistant)
        {
            if self.blocks.iter().any(markdown_block_has_content) {
                lines.insert(1.min(lines.len()), timestamp_line(timestamp, max_width));
            }
        }

        lines
    }
}

fn normalized_markdown_source(role: MessageRole, text: &str) -> String {
    if role == MessageRole::Assistant {
        markdown_render::normalize_agent_markdown_source(text, false)
    } else {
        text.to_string()
    }
}

/// A parsed markdown summary block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkdownBlockView {
    /// Represents paragraph
    Paragraph(String),
    /// Represents code
    Code(MarkdownCodeBlockView),
    /// Represents table
    Table(MarkdownTableView),
    /// A markdown heading (`# `, `## `, `### `).
    Heading {
        /// Heading level: 1 for `#`, 2 for `##`, 3 for `###`.
        level: u8,
        /// Heading text with marker stripped.
        text: String,
    },
    /// A blockquote (`> text`).
    Blockquote(String),
}

/// A summarized fenced code block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownCodeBlockView {
    /// Stores the language
    pub language: Option<String>,
    /// Stores the code
    pub code: String,
    /// Stores the line count
    pub line_count: usize,
}

/// A summarized markdown table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownTableView {
    /// Stores the header cells.
    pub headers: Vec<String>,
    /// Stores the row cells.
    pub rows: Vec<Vec<String>>,
}

const MIN_TABLE_COLUMN_WIDTH: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TableRenderStyle {
    Box,
    Simple,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TableLayout {
    widths: Vec<usize>,
    hard_wrap: bool,
}

impl MarkdownTableView {
    pub(super) fn display_lines(
        &self,
        max_width: usize,
        role: MessageRole,
    ) -> Vec<MessageLineView> {
        let column_count = self
            .headers
            .len()
            .max(self.rows.iter().map(Vec::len).max().unwrap_or(0));
        if column_count == 0 {
            return Vec::new();
        }

        if !self.rows.is_empty()
            && (table_min_width(self, TableRenderStyle::Simple) > max_width
                || (column_count > 2 && table_min_width(self, TableRenderStyle::Box) > max_width))
        {
            return render_key_value_table(self, max_width)
                .into_iter()
                .map(|line| MessageLineView::new(line, role))
                .collect();
        }

        let style = if table_min_width(self, TableRenderStyle::Box) <= max_width {
            TableRenderStyle::Box
        } else {
            TableRenderStyle::Simple
        };
        let layout = compute_table_layout(self, max_width, style);
        let rendered = match style {
            TableRenderStyle::Box => render_box_table(self, &layout.widths, layout.hard_wrap),
            TableRenderStyle::Simple => render_simple_table(self, &layout.widths, layout.hard_wrap),
        };

        rendered
            .into_iter()
            .map(|line| MessageLineView::new(line, role))
            .collect()
    }
}

fn render_key_value_table(table: &MarkdownTableView, max_width: usize) -> Vec<String> {
    let width = max_width.max(1);
    let mut lines = Vec::new();
    for (row_index, row) in table.rows.iter().enumerate() {
        if row_index > 0 {
            lines.push("─".repeat(width.min(24)));
        }
        let column_count = table.headers.len().max(row.len());
        for column_index in 0..column_count {
            let header = table
                .headers
                .get(column_index)
                .map_or_else(|| format!("Column {}", column_index + 1), Clone::clone);
            let value = row.get(column_index).map_or("", String::as_str);
            let text = if header.trim().is_empty() {
                value.to_string()
            } else {
                format!("{}: {}", header.trim(), value.trim())
            };
            let mut wrapped = wrap_text_hard(&text, width);
            if wrapped.is_empty() {
                wrapped.push(String::new());
            }
            lines.extend(wrapped);
        }
    }

    lines
}

fn table_min_width(table: &MarkdownTableView, style: TableRenderStyle) -> usize {
    let widths = table_column_measurements(table)
        .into_iter()
        .map(|(min_width, _)| min_width)
        .collect::<Vec<_>>();
    widths.iter().sum::<usize>() + table_overhead(widths.len(), style)
}

fn compute_table_layout(
    table: &MarkdownTableView,
    max_width: usize,
    style: TableRenderStyle,
) -> TableLayout {
    let measurements = table_column_measurements(table);
    let min_widths = measurements
        .iter()
        .map(|(min_width, _)| *min_width)
        .collect::<Vec<_>>();
    let ideal_widths = measurements
        .into_iter()
        .map(|(_, ideal_width)| ideal_width)
        .collect::<Vec<_>>();
    let overhead = table_overhead(min_widths.len(), style);
    let min_total = min_widths.iter().sum::<usize>();
    let ideal_total = ideal_widths.iter().sum::<usize>();
    let available = max_width.saturating_sub(overhead);

    if ideal_total <= available {
        return TableLayout {
            widths: ideal_widths,
            hard_wrap: false,
        };
    }

    if min_total <= available {
        return TableLayout {
            widths: distribute_widths(&min_widths, &ideal_widths, available),
            hard_wrap: false,
        };
    }

    let baseline = vec![MIN_TABLE_COLUMN_WIDTH; min_widths.len()];
    let widths = if available <= baseline.iter().sum() {
        baseline
    } else {
        distribute_widths(&baseline, &min_widths, available)
    };

    TableLayout {
        widths,
        hard_wrap: true,
    }
}

fn table_column_measurements(table: &MarkdownTableView) -> Vec<(usize, usize)> {
    let column_count = table
        .headers
        .len()
        .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
    (0..column_count)
        .map(|index| {
            let mut min_width = cell_min_width(table.headers.get(index).map_or("", String::as_str));
            let mut ideal_width =
                cell_ideal_width(table.headers.get(index).map_or("", String::as_str));
            for row in &table.rows {
                let cell = row.get(index).map_or("", String::as_str);
                min_width = min_width.max(cell_min_width(cell));
                ideal_width = ideal_width.max(cell_ideal_width(cell));
            }
            (min_width, ideal_width)
        })
        .collect()
}

fn cell_min_width(cell: &str) -> usize {
    cell.split_whitespace()
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0)
        .max(MIN_TABLE_COLUMN_WIDTH)
}

fn cell_ideal_width(cell: &str) -> usize {
    cell.lines()
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0)
        .max(MIN_TABLE_COLUMN_WIDTH)
}

fn table_overhead(column_count: usize, style: TableRenderStyle) -> usize {
    match style {
        TableRenderStyle::Box => 1 + column_count * 3,
        TableRenderStyle::Simple => column_count.saturating_sub(1) * 3,
    }
}

fn distribute_widths(min_widths: &[usize], ideal_widths: &[usize], available: usize) -> Vec<usize> {
    let base_total = min_widths.iter().sum::<usize>();
    if available <= base_total {
        return min_widths.to_vec();
    }

    let extra_space = available - base_total;
    let overflows = ideal_widths
        .iter()
        .zip(min_widths.iter())
        .map(|(ideal_width, min_width)| ideal_width.saturating_sub(*min_width))
        .collect::<Vec<_>>();
    let overflow_total = overflows.iter().sum::<usize>();
    if overflow_total == 0 {
        return min_widths.to_vec();
    }

    let mut widths = min_widths.to_vec();
    let mut allocated = 0usize;
    let mut remainders = Vec::new();
    for (index, overflow) in overflows.iter().copied().enumerate() {
        let raw = overflow.saturating_mul(extra_space);
        let extra = raw / overflow_total;
        widths[index] = widths[index].saturating_add(extra);
        allocated = allocated.saturating_add(extra);
        remainders.push((index, raw % overflow_total));
    }

    remainders.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    for (index, _) in remainders
        .into_iter()
        .take(extra_space.saturating_sub(allocated))
    {
        widths[index] = widths[index].saturating_add(1);
    }

    widths
}

fn render_box_table(table: &MarkdownTableView, widths: &[usize], hard_wrap: bool) -> Vec<String> {
    let mut lines = vec![table_border(widths, '┌', '┬', '┐')];
    lines.extend(render_table_row(
        widths,
        &table.headers,
        hard_wrap,
        TableRenderStyle::Box,
    ));
    lines.push(table_border(widths, '├', '┼', '┤'));
    for row in &table.rows {
        lines.extend(render_table_row(
            widths,
            row,
            hard_wrap,
            TableRenderStyle::Box,
        ));
    }
    lines.push(table_border(widths, '└', '┴', '┘'));
    lines
}

fn render_simple_table(
    table: &MarkdownTableView,
    widths: &[usize],
    hard_wrap: bool,
) -> Vec<String> {
    let mut lines = render_table_row(widths, &table.headers, hard_wrap, TableRenderStyle::Simple);
    lines.push(
        widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join(" | "),
    );
    for row in &table.rows {
        lines.extend(render_table_row(
            widths,
            row,
            hard_wrap,
            TableRenderStyle::Simple,
        ));
    }
    lines
}

fn render_table_row(
    widths: &[usize],
    cells: &[String],
    hard_wrap: bool,
    style: TableRenderStyle,
) -> Vec<String> {
    let wrapped_cells = widths
        .iter()
        .enumerate()
        .map(|(index, width)| {
            wrap_table_cell(
                cells.get(index).map_or("", String::as_str),
                *width,
                hard_wrap,
            )
        })
        .collect::<Vec<_>>();
    let height = wrapped_cells.iter().map(Vec::len).max().unwrap_or(1);
    let mut lines = Vec::with_capacity(height);

    for line_index in 0..height {
        let segments = wrapped_cells
            .iter()
            .zip(widths.iter())
            .map(|(cell_lines, width)| {
                let content = cell_lines.get(line_index).map_or("", String::as_str);
                pad_table_cell(content, *width)
            })
            .collect::<Vec<_>>();
        let line = match style {
            TableRenderStyle::Box => format!("│ {} │", segments.join(" │ ")),
            TableRenderStyle::Simple => segments.join(" | "),
        };
        lines.push(line);
    }

    lines
}

fn wrap_table_cell(cell: &str, width: usize, _hard_wrap: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let source = cell.trim();
    if source.is_empty() {
        return vec![String::new()];
    }

    for raw_line in source.lines() {
        let mut wrapped = wrap_text_hard(raw_line, width.max(1));
        if wrapped.is_empty() {
            wrapped.push(String::new());
        }
        lines.extend(wrapped);
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

fn pad_table_cell(cell: &str, width: usize) -> String {
    let padding = width.saturating_sub(UnicodeWidthStr::width(cell));
    format!("{cell}{}", " ".repeat(padding))
}

fn table_border(widths: &[usize], left: char, middle: char, right: char) -> String {
    let mut line = String::new();
    line.push(left);
    for (index, width) in widths.iter().enumerate() {
        line.push_str(&"─".repeat(width.saturating_add(2)));
        if index + 1 == widths.len() {
            line.push(right);
        } else {
            line.push(middle);
        }
    }
    line
}

/// Highlights a fenced code block for transcript rendering.
#[must_use]
pub fn highlight_code_block(lang: &str, code: &str) -> Vec<Line<'static>> {
    let Some(theme) = HIGHLIGHT_THEME.as_ref() else {
        return plain_code_lines(code);
    };
    let Some(syntax) = syntax_for_language(lang) else {
        return plain_code_lines(code);
    };

    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut lines = Vec::new();
    for raw_line in code.split('\n') {
        let Ok(highlighted) = highlighter.highlight_line(raw_line, &SYNTAX_SET) else {
            return plain_code_lines(code);
        };
        let spans: Vec<_> = highlighted
            .into_iter()
            .map(|(style, text)| Span::styled(text.to_string(), syntect_style_to_ratatui(style)))
            .collect();
        lines.push(Line::from(spans));
    }

    if lines.is_empty() {
        lines.push(Line::default());
    }

    lines
}

fn syntax_for_language(lang: &str) -> Option<&'static SyntaxReference> {
    let token = lang.split_whitespace().next().unwrap_or_default().trim();
    if token.is_empty() {
        return None;
    }

    SYNTAX_SET
        .find_syntax_by_token(token)
        .or_else(|| SYNTAX_SET.find_syntax_by_name(token))
        .or_else(|| SYNTAX_SET.find_syntax_by_extension(token))
}

fn plain_code_lines(code: &str) -> Vec<Line<'static>> {
    let mut lines = code
        .split('\n')
        .map(|line| Line::from(Span::raw(line.to_string())))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push(Line::default());
    }
    lines
}

fn syntect_style_to_ratatui(style: syntect::highlighting::Style) -> RatatuiStyle {
    let mut out = RatatuiStyle::default().fg(RatatuiColor::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ));
    if style.font_style.contains(FontStyle::BOLD) {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        out = out.add_modifier(Modifier::UNDERLINED);
    }
    out
}

pub(super) fn highlighted_code_lines(
    code: &MarkdownCodeBlockView,
    role: MessageRole,
    max_width: usize,
) -> Vec<MessageLineView> {
    highlight_code_block(code.language.as_deref().unwrap_or_default(), &code.code)
        .into_iter()
        .flat_map(|line| wrap_highlighted_line(line, role, max_width))
        .collect()
}

fn wrap_highlighted_line(
    line: Line<'static>,
    role: MessageRole,
    max_width: usize,
) -> Vec<MessageLineView> {
    let width = max_width.max(1);
    let mut wrapped = Vec::new();
    let mut current = Vec::new();
    let mut current_width = 0usize;

    for span in line.spans {
        let style = text_style_from_ratatui(span.style);
        let mut chunk = String::new();
        for symbol in span.content.chars() {
            if current_width == width {
                if !chunk.is_empty() {
                    current.push(MessageSpanView::new(std::mem::take(&mut chunk), style));
                }
                wrapped.push(MessageLineView::with_spans(role, current));
                current = Vec::new();
                current_width = 0;
            }
            chunk.push(symbol);
            current_width = current_width.saturating_add(1);
        }
        if !chunk.is_empty() {
            current.push(MessageSpanView::new(chunk, style));
        }
    }

    if !current.is_empty() {
        wrapped.push(MessageLineView::with_spans(role, current));
    }

    if wrapped.is_empty() {
        wrapped.push(MessageLineView::new(String::new(), role));
    }

    wrapped
}

fn text_style_from_ratatui(style: RatatuiStyle) -> Option<TextStyle> {
    let mut out = TextStyle::default();
    if let Some(fg) = style.fg {
        out.fg = Some(color_from_ratatui(fg));
    }
    // Deliberately skip `style.bg`: syntect themes apply grey block
    // backgrounds that look wrong against the terminal background color.
    // Keeping only fg + modifiers gives clean, Ink-style code spans.
    out.bold = style.add_modifier.contains(Modifier::BOLD);
    out.dim = style.add_modifier.contains(Modifier::DIM);
    out.italic = style.add_modifier.contains(Modifier::ITALIC);
    out.underlined = style.add_modifier.contains(Modifier::UNDERLINED);
    out.reversed = style.add_modifier.contains(Modifier::REVERSED);

    (out != TextStyle::default()).then_some(out)
}

fn color_from_ratatui(color: RatatuiColor) -> Color {
    match color {
        RatatuiColor::Reset => Color::Reset,
        RatatuiColor::Black => Color::Black,
        RatatuiColor::Red | RatatuiColor::LightRed => Color::Red,
        RatatuiColor::Green | RatatuiColor::LightGreen => Color::Green,
        RatatuiColor::Yellow | RatatuiColor::LightYellow => Color::Yellow,
        RatatuiColor::Blue | RatatuiColor::LightBlue => Color::Blue,
        RatatuiColor::Magenta | RatatuiColor::LightMagenta => Color::Magenta,
        RatatuiColor::Cyan | RatatuiColor::LightCyan => Color::Cyan,
        RatatuiColor::Gray => Color::Grey,
        RatatuiColor::DarkGray => Color::DarkGrey,
        RatatuiColor::White => Color::White,
        RatatuiColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        RatatuiColor::Indexed(index) => xterm_256_color(index),
    }
}

fn xterm_256_color(index: u8) -> Color {
    match index {
        0 => Color::Black,
        1 => Color::DarkRed,
        2 => Color::DarkGreen,
        3 => Color::DarkYellow,
        4 => Color::DarkBlue,
        5 => Color::DarkMagenta,
        6 => Color::DarkCyan,
        7 => Color::Grey,
        8 => Color::DarkGrey,
        9 => Color::Red,
        10 => Color::Green,
        11 => Color::Yellow,
        12 => Color::Blue,
        13 => Color::Magenta,
        14 => Color::Cyan,
        15 => Color::White,
        16..=231 => {
            let index = index.saturating_sub(16);
            let red = index / 36;
            let green = (index % 36) / 6;
            let blue = index % 6;
            let channel = |value| if value == 0 { 0 } else { 55 + value * 40 };
            Color::Rgb(channel(red), channel(green), channel(blue))
        }
        232..=255 => {
            let level = 8u8.saturating_add(index.saturating_sub(232).saturating_mul(10));
            Color::Rgb(level, level, level)
        }
    }
}

/// Summarizes assistant thinking blocks without rendering full markdown.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThinkingBlockView {
    /// Stores the content
    pub content: String,
    /// Stores the collapsed
    pub collapsed: bool,
}

impl ThinkingBlockView {
    /// Creates a new value
    #[must_use]
    pub fn new(content: impl Into<String>, collapsed: bool) -> Self {
        Self {
            content: content.into(),
            collapsed,
        }
    }
    /// Handles display lines
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        let line_count = self.content.lines().count().max(1);
        if self.collapsed {
            return push_line(
                format!("thinking> ∴ Thinking · collapsed · {line_count} lines hidden"),
                MessageRole::Progress,
                max_width,
            );
        }

        let mut lines = push_line("thinking> ∴ Thinking…", MessageRole::Progress, max_width);
        let preview = wrap_summary_lines(&self.content, max_width, MAX_THINKING_LINES, true);
        push_wrapped_block(&mut lines, "", &preview, MessageRole::Progress, max_width);
        lines
    }
}

/// Groups contiguous tool calls and their results.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupedToolCallView {
    /// Stores the tool
    pub tool: String,
    /// Stores the calls
    pub calls: Vec<ToolCallView>,
}

impl GroupedToolCallView {
    /// Creates a new value
    #[must_use]
    pub fn new(tool: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            calls: Vec::new(),
        }
    }

    fn push_use(&mut self, use_id: ToolUseId, input: Option<Value>) {
        if let Some(existing) = self.calls.iter_mut().find(|call| call.use_id == use_id) {
            existing.input = input;
            return;
        }

        self.calls.push(ToolCallView {
            use_id,
            input,
            result: None,
            elapsed_secs: None,
        });
    }

    fn push_result(
        &mut self,
        use_id: ToolUseId,
        success: bool,
        content: String,
        elapsed_secs: Option<f64>,
    ) {
        if let Some(existing) = self.calls.iter_mut().find(|call| call.use_id == use_id) {
            existing.result = Some(ToolCallView::result_for(success, content));
            existing.elapsed_secs = elapsed_secs;
            return;
        }

        self.calls.push(ToolCallView {
            use_id,
            input: None,
            result: Some(ToolCallView::result_for(success, content)),
            elapsed_secs,
        });
    }
    /// Handles display lines
    #[must_use]
    pub fn display_lines(&self, max_width: usize, expand_output: bool) -> Vec<MessageLineView> {
        let mut lines = Vec::new();
        let summary = tool_group_summary_verb(&self.tool, &self.calls);
        if !summary.is_empty() {
            let elapsed = self.calls.iter().try_fold(0.0, |total, call| {
                call.elapsed_secs.map(|secs| total + secs)
            });
            let headline = elapsed.map_or_else(
                || format!("⤿ {summary}"),
                |secs| format!("⤿ {summary} ({secs:.1}s)"),
            );
            lines.push(MessageLineView::new(headline, MessageRole::Tool));
        }
        for (index, call) in self.calls.iter().enumerate() {
            if index > 0 {
                lines.push(MessageLineView::new(String::new(), MessageRole::System));
            }
            lines.extend(call.display_lines(&self.tool, max_width, expand_output));
        }
        if lines.is_empty() {
            lines.extend(push_line(
                format!("tools[{}]> 0 calls", self.tool),
                MessageRole::Tool,
                max_width,
            ));
        }
        lines
    }
}

fn tool_group_summary_verb(tool: &str, calls: &[ToolCallView]) -> String {
    let tool = tool.to_ascii_lowercase();
    let read_count =
        if tool.starts_with("file_read") || tool.starts_with("read") || tool.starts_with("list") {
            calls.len()
        } else {
            0
        };
    let search_count = if tool.contains("search") || tool.contains("grep") || tool.contains("glob")
    {
        calls.len()
    } else {
        0
    };

    match (read_count, search_count) {
        (0, 0) => String::new(),
        (reads, 0) => format!("Read {reads} {}", if reads == 1 { "file" } else { "files" }),
        (0, searches) => format!(
            "Searched {searches} {}",
            if searches == 1 { "path" } else { "paths" }
        ),
        (reads, searches) => format!(
            "Read {reads} {}, searched {searches} {}",
            if reads == 1 { "file" } else { "files" },
            if searches == 1 { "path" } else { "paths" }
        ),
    }
}

/// Collapses consecutive file read and search tool calls into a single compact summary.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollapsedReadSearchGroupView {
    /// Number of file read operations in this group.
    pub read_count: usize,
    /// Number of search/grep/glob/find operations in this group.
    pub search_count: usize,
    /// Paths of files that were read.
    pub file_paths: Vec<String>,
    /// Patterns that were searched for.
    pub search_patterns: Vec<String>,
}

impl CollapsedReadSearchGroupView {
    /// Renders the collapsed summary as display lines.
    pub fn display_lines(&self, _max_width: usize) -> Vec<MessageLineView> {
        let mut parts = Vec::new();
        if self.read_count > 0 {
            let noun = if self.read_count == 1 {
                "file"
            } else {
                "files"
            };
            parts.push(format!("Read {} {}", self.read_count, noun));
        }
        if self.search_count > 0 {
            let noun = if self.search_count == 1 {
                "pattern"
            } else {
                "patterns"
            };
            parts.push(format!("searched for {} {}", self.search_count, noun));
        }
        if parts.is_empty() {
            return Vec::new();
        }
        vec![MessageLineView::new(
            format!("\u{293f}  {}", parts.join(", ")),
            MessageRole::Tool,
        )]
    }
}

/// A summarized tool invocation and optional result.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallView {
    /// Stores the use identifier
    pub use_id: ToolUseId,
    /// Stores the input
    pub input: Option<Value>,
    /// Stores the result
    pub result: Option<RejectedToolMessageView>,
    /// Elapsed wall-clock seconds from tool invocation to result, if known.
    pub elapsed_secs: Option<f64>,
}

impl ToolCallView {
    fn result_for(success: bool, content: String) -> RejectedToolMessageView {
        RejectedToolMessageView::from_result(content, success)
    }

    fn display_lines(
        &self,
        tool: &str,
        max_width: usize,
        expand_output: bool,
    ) -> Vec<MessageLineView> {
        let summary = ToolActivitySummary::from_call(tool, self, expand_output);
        let mut lines = push_line(summary.headline, self.role(), max_width);
        for row in summary.rows {
            match row {
                ToolActivityRow::Detail {
                    label: _,
                    value,
                    role,
                } => push_wrapped_block(
                    &mut lines,
                    "",
                    &[format!("{GUIDE_PREFIX}{value}")],
                    role,
                    max_width,
                ),
                ToolActivityRow::Preview {
                    label: _,
                    lines: preview,
                } => {
                    let formatted = preview
                        .iter()
                        .enumerate()
                        .map(|(index, line)| {
                            if index == 0 {
                                format!("{GUIDE_PREFIX}{line}")
                            } else {
                                format!("  {line}")
                            }
                        })
                        .collect::<Vec<_>>();
                    if !formatted.is_empty() {
                        push_wrapped_block(
                            &mut lines,
                            "",
                            &formatted,
                            MessageRole::System,
                            max_width,
                        );
                    }
                }
            }
        }
        lines
    }

    fn generic_label(&self) -> String {
        let id = short_use_id(self.use_id);
        let input = self
            .input
            .as_ref()
            .map_or_else(|| "no input recorded".into(), summarize_tool_input);
        let (status, detail) = self
            .result
            .as_ref()
            .map_or((ToolResultStatus::Pending, None), |result| {
                (result.status, Some(result.detail.as_str()))
            });
        let mut text = format!("• #{id} {} · {input}", status.label());
        if let Some(detail) = detail.filter(|detail| !detail.is_empty()) {
            text.push_str(" → ");
            text.push_str(detail);
        }
        text
    }

    fn is_success(&self) -> bool {
        matches!(
            self.result.as_ref().map(|result| result.status),
            Some(ToolResultStatus::Success)
        )
    }

    fn result_detail(&self) -> Option<&str> {
        self.result.as_ref().map(|result| result.detail.as_str())
    }

    fn role(&self) -> MessageRole {
        match self.result.as_ref().map(|result| result.status) {
            Some(
                ToolResultStatus::Error | ToolResultStatus::Rejected | ToolResultStatus::Cancelled,
            ) => MessageRole::Error,
            Some(ToolResultStatus::Pending) | None => MessageRole::Progress,
            _ => MessageRole::Tool,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ToolActivitySummary {
    headline: String,
    rows: Vec<ToolActivityRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ToolActivityRow {
    Detail {
        label: &'static str,
        value: String,
        role: MessageRole,
    },
    Preview {
        label: &'static str,
        lines: Vec<String>,
    },
}

impl ToolActivitySummary {
    fn from_call(tool: &str, call: &ToolCallView, expand_output: bool) -> Self {
        match tool {
            "bash" => summarize_bash_call(call, expand_output),
            "file_read" => summarize_file_read_call(call, expand_output),
            "file_write" => summarize_file_write_call(call, expand_output),
            "glob" => summarize_glob_call(call, expand_output),
            "grep" | "rg" | "search" | "find" => summarize_search_call(tool, call, expand_output),
            _ => summarize_generic_tool_call(tool, call, expand_output),
        }
    }
}

fn summarize_bash_call(call: &ToolCallView, expand_output: bool) -> ToolActivitySummary {
    let command = call
        .input
        .as_ref()
        .and_then(|input| input.get("command").and_then(Value::as_str))
        .map(normalize_inline_text)
        .unwrap_or_default();
    let elapsed = call.elapsed_secs.map(format_elapsed).unwrap_or_default();
    let headline = if command.is_empty() {
        format!("● Bash{elapsed}")
    } else {
        format!("● {}{elapsed}", bash_activity_title(&command))
    };

    let mut rows = status_rows(call);
    if !command.is_empty() {
        rows.push(ToolActivityRow::Detail {
            label: "command",
            value: truncate_visible_end(&command, 72),
            role: MessageRole::System,
        });
    }
    append_result_rows(call, &mut rows, expand_output);

    ToolActivitySummary { headline, rows }
}

fn summarize_file_read_call(call: &ToolCallView, expand_output: bool) -> ToolActivitySummary {
    let path = call
        .input
        .as_ref()
        .and_then(|input| input.get("path").and_then(Value::as_str))
        .unwrap_or("unknown");
    let mut rows = status_rows(call);
    rows.push(ToolActivityRow::Detail {
        label: "path",
        value: truncate_visible_end(path, 72),
        role: MessageRole::System,
    });
    if call.is_success() {
        let max_lines = if expand_output { usize::MAX } else { 20 };
        if let Some(preview) = preview_lines(call.result_detail(), max_lines, 72, !expand_output) {
            rows.push(ToolActivityRow::Preview {
                label: "preview",
                lines: preview,
            });
        }
    } else {
        append_result_rows(call, &mut rows, expand_output);
    }

    ToolActivitySummary {
        headline: format!(
            "● Read({}){}",
            compact_target_label(path),
            call.elapsed_secs.map(format_elapsed).unwrap_or_default()
        ),
        rows,
    }
}

fn summarize_file_write_call(call: &ToolCallView, expand_output: bool) -> ToolActivitySummary {
    let (path, input_preview) = call
        .input
        .as_ref()
        .map(|input| {
            (
                input
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                input.get("content").and_then(Value::as_str),
            )
        })
        .unwrap_or_else(|| ("unknown".into(), None));
    let mut rows = status_rows(call);
    rows.push(ToolActivityRow::Detail {
        label: "path",
        value: truncate_visible_end(&path, 72),
        role: MessageRole::System,
    });
    let max_lines = if expand_output { usize::MAX } else { 20 };
    if let Some(preview) =
        input_preview.and_then(|text| preview_lines(Some(text), max_lines, 72, !expand_output))
    {
        rows.push(ToolActivityRow::Preview {
            label: "preview",
            lines: preview,
        });
    }
    if !call.is_success() {
        append_result_rows(call, &mut rows, expand_output);
    } else if let Some(detail) = short_result_detail(call.result_detail()) {
        rows.push(ToolActivityRow::Detail {
            label: "result",
            value: detail,
            role: MessageRole::System,
        });
    }

    ToolActivitySummary {
        headline: format!(
            "● Edit({}){}",
            compact_target_label(&path),
            call.elapsed_secs.map(format_elapsed).unwrap_or_default()
        ),
        rows,
    }
}

fn summarize_glob_call(call: &ToolCallView, expand_output: bool) -> ToolActivitySummary {
    let (pattern, path) = call
        .input
        .as_ref()
        .map(|input| {
            (
                input.get("pattern").and_then(Value::as_str).unwrap_or("*"),
                input.get("path").and_then(Value::as_str).unwrap_or("."),
            )
        })
        .unwrap_or(("*", "."));
    let mut rows = status_rows(call);
    rows.push(ToolActivityRow::Detail {
        label: "pattern",
        value: truncate_visible_end(pattern, 72),
        role: MessageRole::System,
    });
    if path != "." {
        rows.push(ToolActivityRow::Detail {
            label: "path",
            value: truncate_visible_end(path, 72),
            role: MessageRole::System,
        });
    }
    append_result_rows(call, &mut rows, expand_output);

    ToolActivitySummary {
        headline: format!(
            "● List({}){}",
            infer_list_target(pattern, path),
            call.elapsed_secs.map(format_elapsed).unwrap_or_default()
        ),
        rows,
    }
}

fn summarize_search_call(
    tool: &str,
    call: &ToolCallView,
    expand_output: bool,
) -> ToolActivitySummary {
    let query = call
        .input
        .as_ref()
        .and_then(|input| {
            input
                .get("query")
                .or_else(|| input.get("pattern"))
                .or_else(|| input.get("prompt"))
                .and_then(Value::as_str)
        })
        .unwrap_or("*");
    let mut rows = status_rows(call);
    rows.push(ToolActivityRow::Detail {
        label: if tool == "find" { "path" } else { "query" },
        value: truncate_visible_end(query, 72),
        role: MessageRole::System,
    });
    append_result_rows(call, &mut rows, expand_output);

    ToolActivitySummary {
        headline: format!(
            "● {}({}){}",
            if tool == "find" { "List" } else { "Search" },
            compact_target_label(query),
            call.elapsed_secs.map(format_elapsed).unwrap_or_default()
        ),
        rows,
    }
}

fn summarize_generic_tool_call(
    tool: &str,
    call: &ToolCallView,
    expand_output: bool,
) -> ToolActivitySummary {
    let mut rows = status_rows(call);
    rows.push(ToolActivityRow::Detail {
        label: "summary",
        value: call.generic_label(),
        role: MessageRole::System,
    });
    if let Some(input) = &call.input {
        rows.push(ToolActivityRow::Detail {
            label: "input",
            value: summarize_tool_input(input),
            role: MessageRole::System,
        });
    }
    append_result_rows(call, &mut rows, expand_output);

    ToolActivitySummary {
        headline: format!(
            "● {tool}{}",
            call.elapsed_secs.map(format_elapsed).unwrap_or_default()
        ),
        rows,
    }
}

fn format_elapsed(secs: f64) -> String {
    if secs < 0.05 {
        return String::new();
    }
    if secs < 10.0 {
        format!(" · {:.1}s", secs)
    } else if secs < 60.0 {
        format!(" · {:.0}s", secs)
    } else {
        format!(" · {:.0}m {:.0}s", secs / 60.0, secs % 60.0)
    }
}

fn status_rows(call: &ToolCallView) -> Vec<ToolActivityRow> {
    if call.result.is_none() {
        vec![ToolActivityRow::Detail {
            label: "status",
            value: "running…".into(),
            role: MessageRole::Progress,
        }]
    } else {
        Vec::new()
    }
}

fn append_result_rows(call: &ToolCallView, rows: &mut Vec<ToolActivityRow>, expand_output: bool) {
    let Some(result) = &call.result else {
        return;
    };
    let max_lines = if expand_output { usize::MAX } else { 20 };
    if let Some(preview) = preview_lines(Some(&result.detail), max_lines, 72, !expand_output) {
        if preview.len() > 1 || result.detail.contains('\n') || looks_like_preview(&preview[0]) {
            rows.push(ToolActivityRow::Preview {
                label: if call.is_success() {
                    "output"
                } else {
                    "detail"
                },
                lines: preview,
            });
            return;
        }
    }
    if let Some(detail) = short_result_detail(Some(&result.detail)) {
        rows.push(ToolActivityRow::Detail {
            label: if call.is_success() {
                "result"
            } else {
                "detail"
            },
            value: detail,
            role: if call.is_success() {
                MessageRole::System
            } else {
                MessageRole::Error
            },
        });
    }
}

fn preview_lines(
    text: Option<&str>,
    max_lines: usize,
    max_width: usize,
    show_expand_hint: bool,
) -> Option<Vec<String>> {
    let text = text?;
    let all_lines = text
        .lines()
        .map(str::trim_end)
        .skip_while(|line| line.trim().is_empty())
        .map(|line| truncate_visible_end(line, max_width))
        .collect::<Vec<_>>();
    if all_lines.is_empty() {
        return None;
    }

    let total = all_lines.len();
    let overflow = total > max_lines;
    let mut lines = all_lines.into_iter().take(max_lines).collect::<Vec<_>>();
    if overflow {
        let remaining = total - max_lines;
        let mut overflow_label = format!("… +{remaining} lines");
        if show_expand_hint {
            overflow_label.push_str(" (ctrl+o to expand)");
        }
        lines.push(overflow_label);
    }
    Some(lines)
}

fn short_result_detail(text: Option<&str>) -> Option<String> {
    let detail = text?.trim();
    if detail.is_empty() {
        return None;
    }
    if matches!(
        detail.to_ascii_lowercase().as_str(),
        "ok" | "done" | "completed" | "success"
    ) {
        return None;
    }
    Some(truncate_visible_end(
        detail.lines().next().unwrap_or(detail).trim(),
        72,
    ))
}

fn looks_like_preview(line: &str) -> bool {
    line.contains('/') || line.contains("::") || line.contains("@@") || line.contains("fn ")
}

fn compact_target_label(value: &str) -> String {
    let trimmed = value.trim().trim_matches('"');
    if trimmed.is_empty() {
        return "unknown".into();
    }
    trimmed
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .unwrap_or(trimmed)
        .to_string()
}

fn normalize_inline_text(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn bash_activity_title(command: &str) -> String {
    let lower = command.to_ascii_lowercase();
    if is_list_command(&lower) {
        return format!("List({})", infer_bash_list_target(command));
    }
    if is_test_command(&lower) {
        return "Run(Tests)".into();
    }
    if is_read_command(&lower) {
        return format!(
            "Read({})",
            infer_path_target(command).unwrap_or_else(|| "File".into())
        );
    }
    if is_search_command(&lower) {
        return format!(
            "Search({})",
            infer_search_target(command).unwrap_or_else(|| "Matches".into())
        );
    }
    if lower.starts_with("git diff") || lower.starts_with("diff ") {
        return format!(
            "Diff({})",
            infer_path_target(command).unwrap_or_else(|| "Workspace".into())
        );
    }
    if lower.contains("apply_patch") {
        return "Edit(Patch)".into();
    }
    format!("Bash({})", truncate_visible_end(command, 28))
}

fn infer_bash_list_target(command: &str) -> String {
    let lower = command.to_ascii_lowercase();
    if lower.contains("test") || lower.contains("spec") {
        return "Tests".into();
    }
    if lower.contains("src")
        || lower.contains("crate")
        || lower.contains("cargo")
        || lower.contains("mod")
    {
        return "Modules".into();
    }
    infer_path_target(command).unwrap_or_else(|| "Files".into())
}

fn infer_list_target(pattern: &str, path: &str) -> String {
    let combined = format!("{pattern} {path}");
    let lower = combined.to_ascii_lowercase();
    if lower.contains("test") || lower.contains("spec") {
        return "Tests".into();
    }
    if lower.contains("src")
        || lower.contains("crate")
        || lower.contains("cargo")
        || lower.contains("mod")
    {
        return "Modules".into();
    }
    let path_target = compact_target_label(path);
    if path_target != "." && path_target != "*" && path_target != "unknown" {
        return path_target;
    }
    compact_target_label(pattern)
}

fn infer_search_target(command: &str) -> Option<String> {
    let tokens = command.split_whitespace().collect::<Vec<_>>();
    tokens
        .windows(2)
        .find_map(|pair| (pair[0] == "rg" || pair[0] == "grep").then(|| pair[1]))
        .map(|token| token.trim_matches('"').trim_matches('\''))
        .filter(|token| !token.starts_with('-') && !token.is_empty())
        .map(compact_target_label)
}

fn infer_path_target(command: &str) -> Option<String> {
    command
        .split_whitespace()
        .rev()
        .map(|token| token.trim_matches('"').trim_matches('\''))
        .find(|token| {
            !token.starts_with('-')
                && !token.contains('=')
                && *token != "."
                && *token != "|"
                && *token != "&&"
                && token.chars().any(|ch| ch.is_alphanumeric())
        })
        .map(compact_target_label)
}

fn is_list_command(lower: &str) -> bool {
    lower.starts_with("ls ")
        || lower == "ls"
        || lower.starts_with("find ")
        || lower.starts_with("fd ")
        || lower.starts_with("tree ")
        || lower.contains("rg --files")
        || lower.starts_with("git ls-files")
}

fn is_test_command(lower: &str) -> bool {
    lower.starts_with("cargo test")
        || lower.starts_with("pytest")
        || lower.starts_with("npm test")
        || lower.starts_with("pnpm test")
        || lower.starts_with("yarn test")
        || lower.starts_with("go test")
}

fn is_read_command(lower: &str) -> bool {
    lower.starts_with("cat ")
        || lower.starts_with("sed -n")
        || lower.starts_with("head ")
        || lower.starts_with("tail ")
}

fn is_search_command(lower: &str) -> bool {
    lower.starts_with("rg ") || lower.starts_with("grep ")
}

/// Summarizes a file edit reference using diff summaries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEditReferenceView {
    /// Stores the path
    pub path: PathLinkView,
    /// Stores the summary
    pub summary: FileEditHunkSummary,
    /// Stores the note
    pub note: Option<String>,
}

impl FileEditReferenceView {
    /// Creates a new value
    #[must_use]
    pub fn new(path: PathLinkView, summary: FileEditHunkSummary) -> Self {
        Self {
            path,
            summary,
            note: None,
        }
    }
    /// Handles with note
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
    /// Handles display lines
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        let path = self.path.display_text(max_width).text;
        let mut lines = push_line(format!("● Edit({path})"), MessageRole::Tool, max_width);
        push_wrapped_block(
            &mut lines,
            "",
            &[format!("{GUIDE_PREFIX}{}", self.summary.label())],
            MessageRole::System,
            max_width,
        );
        if let Some(note) = self.note.as_deref() {
            push_wrapped_block(
                &mut lines,
                "",
                &[truncate_visible_end(note, max_width.max(1))],
                MessageRole::System,
                max_width,
            );
        }
        lines
    }
}

/// Summarizes a user attachment reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachmentSummaryView {
    /// Stores the label
    pub label: String,
    /// Stores the uri
    pub uri: String,
    /// Stores the kind
    pub kind: AttachmentKind,
}

impl AttachmentSummaryView {
    /// Creates a new value
    #[must_use]
    pub fn new(label: impl Into<String>, uri: impl Into<String>) -> Self {
        let label = label.into();
        let uri = uri.into();
        Self {
            kind: AttachmentKind::detect(&label, &uri),
            label,
            uri,
        }
    }
    /// Handles display lines
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        let mut lines = push_line(
            format!("attachment> {} {}", self.kind.label(), self.label),
            MessageRole::User,
            max_width,
        );
        if self.uri != self.label {
            push_wrapped_block(
                &mut lines,
                "",
                std::slice::from_ref(&self.uri),
                MessageRole::User,
                max_width,
            );
        }
        lines
    }
}

/// Categorizes attachment references for compact transcript summaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttachmentKind {
    /// Represents file
    File,
    /// Represents image
    Image,
    /// Represents pdf
    Pdf,
    /// Represents link
    Link,
    /// Represents directory
    Directory,
    /// Represents generic
    Generic,
}

impl AttachmentKind {
    fn detect(label: &str, uri: &str) -> Self {
        let source = format!("{label} {uri}").to_ascii_lowercase();
        if source.contains("file:///") {
            if source.ends_with('/') {
                return Self::Directory;
            }
            return match extension_kind(&source) {
                Some(kind) => kind,
                None => Self::File,
            };
        }
        if source.starts_with("http://") || source.starts_with("https://") {
            return Self::Link;
        }
        extension_kind(&source).unwrap_or(Self::Generic)
    }

    fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Image => "image",
            Self::Pdf => "pdf",
            Self::Link => "link",
            Self::Directory => "directory",
            Self::Generic => "item",
        }
    }
}

/// Summarizes system, API, and rate-limit failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemErrorView {
    /// Stores the role
    pub role: MessageRole,
    /// Stores the kind
    pub kind: SystemErrorKind,
    /// Stores the detail
    pub detail: String,
}

impl SystemErrorView {
    /// Handles detect
    #[must_use]
    pub fn detect(role: MessageRole, text: &str) -> Option<Self> {
        let kind = SystemErrorKind::detect(text)?;
        Some(Self {
            role,
            kind,
            detail: text.trim().to_string(),
        })
    }
    /// Handles display lines
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        let summary = wrap_summary_lines(&self.detail, max_width, MAX_DETAIL_LINES, true);
        let headline = summary
            .first()
            .cloned()
            .unwrap_or_else(|| self.kind.label().to_string());
        let mut lines = push_line(
            format!("error> {}: {headline}", self.kind.label()),
            MessageRole::Error,
            max_width,
        );
        if summary.len() > 1 {
            push_wrapped_block(&mut lines, "", &summary[1..], MessageRole::Error, max_width);
        }
        lines
    }
}

/// Coarse system error categories that match the current transcript surfaces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemErrorKind {
    /// Represents rate limit
    RateLimit,
    /// Represents timeout
    Timeout,
    /// Represents authentication
    Authentication,
    /// Represents api
    Api,
    /// Represents system
    System,
}

impl SystemErrorKind {
    fn detect(text: &str) -> Option<Self> {
        let headline = text.lines().map(str::trim).find(|line| !line.is_empty())?;
        let lower = headline.to_ascii_lowercase();
        if lower.is_empty() {
            return None;
        }
        if lower.contains("rate limit") || lower.contains("usage limit") {
            return Some(Self::RateLimit);
        }
        if lower.contains("timed out") || lower.contains("timeout") {
            return Some(Self::Timeout);
        }
        if lower.contains("invalid api key")
            || lower.contains("token revoked")
            || lower.contains("unauthorized")
        {
            return Some(Self::Authentication);
        }
        if lower.contains("api error")
            || lower.contains("api:")
            || lower.contains("status code")
            || lower.contains("server error")
        {
            return Some(Self::Api);
        }
        if lower.starts_with("error")
            || lower.starts_with("failed")
            || lower.contains(" error:")
            || lower.contains(" failed:")
        {
            return Some(Self::System);
        }
        None
    }

    fn label(self) -> &'static str {
        match self {
            Self::RateLimit => "rate limit",
            Self::Timeout => "timeout",
            Self::Authentication => "auth",
            Self::Api => "api",
            Self::System => "system",
        }
    }
}

/// Summarizes compact transcript boundaries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptBoundaryView {
    /// Stores the summary
    pub summary: String,
}

impl TranscriptBoundaryView {
    /// Creates a new value
    #[must_use]
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
        }
    }
    /// Handles display lines
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        let summary = self.summary.trim();
        let text = if summary.is_empty() || summary.eq_ignore_ascii_case("conversation compacted") {
            "summary> ✻ Conversation compacted".to_string()
        } else {
            format!("summary> ✻ Conversation compacted · {summary}")
        };
        push_line(text, MessageRole::System, max_width)
    }
}

/// A summarized tool result or rejection state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectedToolMessageView {
    /// Stores the kind
    pub kind: RejectedToolMessageKind,
    /// Stores the status
    pub status: ToolResultStatus,
    /// Stores the detail
    pub detail: String,
}

impl RejectedToolMessageView {
    /// Handles from result
    #[must_use]
    pub fn from_result(detail: impl Into<String>, success: bool) -> Self {
        let detail = detail.into().trim().to_string();
        if success {
            return Self {
                kind: RejectedToolMessageKind::Success,
                status: ToolResultStatus::Success,
                detail,
            };
        }

        let lower = detail.to_ascii_lowercase();
        let (kind, status) =
            if lower.contains("cancel") || lower.contains("interrupted") || lower.contains("abort")
            {
                (
                    RejectedToolMessageKind::Cancelled,
                    ToolResultStatus::Cancelled,
                )
            } else if lower.contains("plan rejected") || lower.contains("rejected plan") {
                (
                    RejectedToolMessageKind::PlanRejected,
                    ToolResultStatus::Rejected,
                )
            } else if lower.contains("rejected")
                || lower.contains("deny")
                || lower.contains("classifier")
            {
                (
                    RejectedToolMessageKind::Rejected,
                    ToolResultStatus::Rejected,
                )
            } else {
                (RejectedToolMessageKind::Error, ToolResultStatus::Error)
            };

        Self {
            kind,
            status,
            detail,
        }
    }
}

/// Tool result classification used by grouped tool summaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolResultStatus {
    /// Represents pending
    Pending,
    /// Represents success
    Success,
    /// Represents error
    Error,
    /// Represents rejected
    Rejected,
    /// Represents cancelled
    Cancelled,
}

impl ToolResultStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Success => "ok",
            Self::Error => "error",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Tool-specific rejection states rendered as compact transcript summaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectedToolMessageKind {
    /// Represents success
    Success,
    /// Represents rejected
    Rejected,
    /// Represents plan rejected
    PlanRejected,
    /// Represents cancelled
    Cancelled,
    /// Represents error
    Error,
}

fn parse_markdown_blocks(text: &str) -> Vec<MarkdownBlockView> {
    let mut blocks = Vec::new();
    let mut paragraph = Vec::new();
    let mut in_code_block = false;
    let mut code_language = None;
    let mut code_lines = Vec::new();
    let lines = text.lines().collect::<Vec<_>>();
    let mut index = 0usize;

    while let Some(line) = lines.get(index).copied() {
        if let Some(language) = line.strip_prefix("```") {
            if in_code_block {
                blocks.push(MarkdownBlockView::Code(MarkdownCodeBlockView {
                    language: code_language.take(),
                    code: code_lines.join("\n"),
                    line_count: code_lines.len(),
                }));
                code_lines.clear();
                in_code_block = false;
            } else {
                flush_paragraph(&mut blocks, &mut paragraph);
                let language = language.trim();
                code_language = (!language.is_empty()).then(|| language.to_string());
                in_code_block = true;
            }
            index = index.saturating_add(1);
            continue;
        }

        if in_code_block {
            code_lines.push(line.to_string());
            index = index.saturating_add(1);
            continue;
        }

        if line.trim().is_empty() {
            flush_paragraph(&mut blocks, &mut paragraph);
            index = index.saturating_add(1);
            continue;
        }

        if line.trim_start().starts_with('|') {
            flush_paragraph(&mut blocks, &mut paragraph);
            let mut table_lines = Vec::new();
            while let Some(table_line) = lines.get(index).copied() {
                if !table_line.trim_start().starts_with('|') {
                    break;
                }
                table_lines.push(table_line);
                index = index.saturating_add(1);
            }
            if let Some(table) = parse_markdown_table(&table_lines) {
                blocks.push(MarkdownBlockView::Table(table));
            }
            continue;
        }

        // Detect headings (### before ## before # to avoid false prefix match).
        let trimmed_line = line.trim_start();
        if let Some(text) = trimmed_line
            .strip_prefix("### ")
            .or_else(|| trimmed_line.strip_prefix("## "))
            .or_else(|| trimmed_line.strip_prefix("# "))
        {
            let level = if trimmed_line.starts_with("### ") {
                3u8
            } else if trimmed_line.starts_with("## ") {
                2
            } else {
                1
            };
            flush_paragraph(&mut blocks, &mut paragraph);
            blocks.push(MarkdownBlockView::Heading {
                level,
                text: text.trim().to_string(),
            });
            index = index.saturating_add(1);
            continue;
        }

        // Detect blockquotes — accumulate consecutive `> ` lines.
        if let Some(first_quote) = trimmed_line.strip_prefix("> ") {
            flush_paragraph(&mut blocks, &mut paragraph);
            let mut quote_lines = vec![first_quote.to_string()];
            index = index.saturating_add(1);
            while let Some(next_line) = lines.get(index).copied() {
                if let Some(q) = next_line.trim_start().strip_prefix("> ") {
                    quote_lines.push(q.to_string());
                    index = index.saturating_add(1);
                } else {
                    break;
                }
            }
            blocks.push(MarkdownBlockView::Blockquote(quote_lines.join(" ")));
            continue;
        }

        paragraph.push(normalize_markdown_line(line));
        index = index.saturating_add(1);
    }

    flush_paragraph(&mut blocks, &mut paragraph);

    if in_code_block || code_language.is_some() || !code_lines.is_empty() {
        blocks.push(MarkdownBlockView::Code(MarkdownCodeBlockView {
            language: code_language,
            code: code_lines.join("\n"),
            line_count: code_lines.len(),
        }));
    }

    blocks
}

fn flush_paragraph(blocks: &mut Vec<MarkdownBlockView>, paragraph: &mut Vec<String>) {
    if paragraph.is_empty() {
        return;
    }

    blocks.push(MarkdownBlockView::Paragraph(paragraph.join(" ")));
    paragraph.clear();
}

fn parse_markdown_table(lines: &[&str]) -> Option<MarkdownTableView> {
    let mut headers = None;
    let mut rows = Vec::new();

    for line in lines {
        if is_table_separator(line) {
            continue;
        }
        let cells = parse_table_line(line);
        if headers.is_none() {
            headers = Some(cells);
        } else {
            rows.push(cells);
        }
    }

    headers.map(|headers| MarkdownTableView { headers, rows })
}

fn is_table_separator(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') {
        return false;
    }
    let content = trimmed.trim_start_matches('|').trim_end_matches('|').trim();
    if content.is_empty() {
        return false;
    }

    content.split('|').all(|cell| {
        cell.trim()
            .chars()
            .all(|character| matches!(character, '-' | ':' | ' '))
    })
}

fn parse_table_line(line: &str) -> Vec<String> {
    line.trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

fn normalize_markdown_line(line: &str) -> String {
    // Headings and blockquotes are detected upstream in parse_markdown_blocks;
    // this function only handles lines that end up in Paragraph blocks.
    let trimmed = line.trim();
    trimmed
        .strip_prefix("- ")
        .map(|value| format!("• {value}"))
        .or_else(|| trimmed.strip_prefix("* ").map(|value| format!("• {value}")))
        .unwrap_or_else(|| trimmed.to_string())
}

/// Parses inline markdown markers from an already-wrapped text segment.
///
/// Handles: `**bold**`, `*italic*`, `` `code` ``, `~~strikethrough~~`.
/// Returns styled spans. Falls back to a single unstyled span when no markers
/// are found, which the caller can detect via [`spans_have_markup`].
fn parse_inline_spans(text: &str) -> Vec<MessageSpanView> {
    let mut spans: Vec<MessageSpanView> = Vec::new();
    let mut rest = text;
    let mut plain = String::new();

    while !rest.is_empty() {
        let next_marker = rest
            .char_indices()
            .find(|(_, c)| matches!(c, '`' | '*' | '~'));

        let Some((pos, ch)) = next_marker else {
            plain.push_str(rest);
            break;
        };

        plain.push_str(&rest[..pos]);
        rest = &rest[pos..];

        match ch {
            '`' => {
                if let Some(end) = rest[1..].find('`') {
                    let code = &rest[1..1 + end];
                    if !code.is_empty() {
                        let code_owned = code.to_string();
                        if !plain.is_empty() {
                            spans.push(MessageSpanView::new(std::mem::take(&mut plain), None));
                        }
                        spans.push(MessageSpanView::new(
                            code_owned,
                            Some(TextStyle::default().fg(Color::DarkCyan)),
                        ));
                        rest = &rest[1 + end + 1..];
                    } else {
                        plain.push('`');
                        rest = &rest[1..];
                    }
                } else {
                    plain.push('`');
                    rest = &rest[1..];
                }
            }
            '*' => {
                if rest.starts_with("**") {
                    if let Some(end) = rest[2..].find("**") {
                        let bold = &rest[2..2 + end];
                        if !bold.is_empty() {
                            let bold_owned = bold.to_string();
                            if !plain.is_empty() {
                                spans.push(MessageSpanView::new(std::mem::take(&mut plain), None));
                            }
                            spans.push(MessageSpanView::new(
                                bold_owned,
                                Some(TextStyle::default().bold()),
                            ));
                            rest = &rest[2 + end + 2..];
                        } else {
                            plain.push_str("**");
                            rest = &rest[2..];
                        }
                    } else {
                        plain.push_str("**");
                        rest = &rest[2..];
                    }
                } else if let Some(end) = rest[1..].find('*') {
                    let italic = &rest[1..1 + end];
                    if !italic.is_empty() && !italic.starts_with('*') {
                        let italic_owned = italic.to_string();
                        if !plain.is_empty() {
                            spans.push(MessageSpanView::new(std::mem::take(&mut plain), None));
                        }
                        spans.push(MessageSpanView::new(
                            italic_owned,
                            Some(TextStyle::default().italic()),
                        ));
                        rest = &rest[1 + end + 1..];
                    } else {
                        plain.push('*');
                        rest = &rest[1..];
                    }
                } else {
                    plain.push('*');
                    rest = &rest[1..];
                }
            }
            '~' => {
                if rest.starts_with("~~") {
                    if let Some(end) = rest[2..].find("~~") {
                        let strike = &rest[2..2 + end];
                        if !strike.is_empty() {
                            let strike_owned = strike.to_string();
                            if !plain.is_empty() {
                                spans.push(MessageSpanView::new(std::mem::take(&mut plain), None));
                            }
                            spans.push(MessageSpanView::new(
                                strike_owned,
                                Some(TextStyle::default().dim()),
                            ));
                            rest = &rest[2 + end + 2..];
                        } else {
                            plain.push_str("~~");
                            rest = &rest[2..];
                        }
                    } else {
                        plain.push_str("~~");
                        rest = &rest[2..];
                    }
                } else {
                    plain.push('~');
                    rest = &rest[1..];
                }
            }
            _ => {
                let c_len = ch.len_utf8();
                plain.push_str(&rest[..c_len]);
                rest = &rest[c_len..];
            }
        }
    }

    if !plain.is_empty() {
        spans.push(MessageSpanView::new(plain, None));
    }

    spans
}

fn spans_have_markup(spans: &[MessageSpanView]) -> bool {
    spans.iter().any(|s| s.style.is_some())
}

fn push_wrapped_block(
    output: &mut Vec<MessageLineView>,
    prefix: &str,
    source_lines: &[String],
    role: MessageRole,
    max_width: usize,
) {
    if source_lines.is_empty() {
        return;
    }

    let indent = " ".repeat(prefix.chars().count());
    let continuation_prefix = if prefix.is_empty() {
        "  "
    } else {
        indent.as_str()
    };
    let available = max_width.saturating_sub(line_width(prefix)).max(1);
    let continuation_available = max_width
        .saturating_sub(line_width(continuation_prefix))
        .max(1);
    let mut used_prefix = false;

    for (index, source) in source_lines.iter().enumerate() {
        let block_prefix = if !used_prefix && !prefix.is_empty() {
            prefix
        } else if prefix.is_empty() && index == 0 {
            "  "
        } else {
            continuation_prefix
        };
        let width = if !used_prefix && !prefix.is_empty() {
            available
        } else {
            continuation_available
        };
        let wrapped = wrap_text_hard(source, width);
        if wrapped.is_empty() {
            output.push(MessageLineView::new(block_prefix.to_string(), role));
            continue;
        }
        for (wrapped_index, segment) in wrapped.into_iter().enumerate() {
            let segment_prefix = if wrapped_index == 0 {
                block_prefix
            } else {
                continuation_prefix
            };
            let spans = parse_inline_spans(&segment);
            if spans_have_markup(&spans) {
                let mut all_spans = Vec::with_capacity(spans.len() + 1);
                if !segment_prefix.is_empty() {
                    all_spans.push(MessageSpanView::new(segment_prefix.to_string(), None));
                }
                all_spans.extend(spans);
                output.push(MessageLineView::with_spans(role, all_spans));
            } else {
                output.push(MessageLineView::new(
                    format!("{segment_prefix}{segment}"),
                    role,
                ));
            }
        }
        used_prefix = true;
    }
}

fn push_line(text: impl Into<String>, role: MessageRole, max_width: usize) -> Vec<MessageLineView> {
    vec![MessageLineView::new(
        truncate_visible_end(&text.into(), max_width.max(1)),
        role,
    )]
}

fn truncate_existing_lines(lines: &[MessageLineView], max_width: usize) -> Vec<MessageLineView> {
    let width = max_width.max(1);
    let mut out = Vec::new();
    for line in lines {
        if line_width(&line.text) <= width {
            out.push(line.clone());
            continue;
        }

        if let Some((prefix_head, body)) = line.text.split_once("> ") {
            push_wrapped_block(
                &mut out,
                &format!("{prefix_head}> "),
                &[body.to_string()],
                line.role,
                width,
            );
            continue;
        }

        if line.spans.is_empty() {
            out.push(MessageLineView::new(
                truncate_visible_end(&line.text, width),
                line.role,
            ));
        } else {
            out.push(MessageLineView::with_spans(
                line.role,
                truncate_spans_to_width(&line.spans, width),
            ));
        }
    }
    out
}

fn truncate_spans_to_width(spans: &[MessageSpanView], max_width: usize) -> Vec<MessageSpanView> {
    let mut out = Vec::new();
    let mut remaining = max_width;
    for span in spans {
        let span_width = line_width(&span.text);
        if span_width <= remaining {
            out.push(span.clone());
            remaining = remaining.saturating_sub(span_width);
        } else if remaining > 0 {
            let truncated = truncate_visible_end(&span.text, remaining);
            if !truncated.is_empty() {
                out.push(MessageSpanView::new(truncated, span.style));
            }
            break;
        } else {
            break;
        }
    }
    out
}

fn wrap_summary_lines(
    text: &str,
    max_width: usize,
    max_lines: usize,
    summarize: bool,
) -> Vec<String> {
    let stripped = strip_ansi(text);
    let width = max_width.max(1);

    // Wrap each logical line independently so that paragraphs, lists, and
    // blank separators are preserved rather than collapsed into a single
    // space-joined run.
    let mut lines: Vec<String> = stripped
        .split('\n')
        .flat_map(|logical| {
            let trimmed = logical.trim_end();
            if trimmed.is_empty() {
                vec![String::new()]
            } else {
                wrap_text_hard(trimmed, width)
            }
        })
        .collect();

    // Drop leading/trailing blank lines so callers get clean output.
    while lines.first().is_some_and(|l| l.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }

    if lines.is_empty() {
        return vec![String::new()];
    }

    // Only truncate when the caller explicitly requests a summary; when
    // summarize=false the full content is returned regardless of max_lines.
    if summarize && lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            *last = append_ellipsis(last, width);
        }
    }

    lines
}

fn role_prefix(role: MessageRole) -> &'static str {
    match role {
        // ▶ for user input (right-pointing triangle = "going in"),
        // ◆ for assistant output (diamond = AI response).
        // Continuation blocks (second paragraph onward) are rendered flush-left via
        // the empty-prefix branch in `display_lines`.
        MessageRole::User => "▶ ",
        MessageRole::Assistant => "◆ ",
        // Tool headlines already carry their own "● Tool(args)" bullet, so no extra
        // prefix is needed here.
        MessageRole::Tool => "",
        MessageRole::System => "system> ",
        MessageRole::Progress => "progress> ",
        MessageRole::Error => "error> ",
    }
}

fn markdown_block_has_content(block: &MarkdownBlockView) -> bool {
    match block {
        MarkdownBlockView::Paragraph(text) => !text.trim().is_empty(),
        MarkdownBlockView::Code(code) => !code.code.trim().is_empty(),
        MarkdownBlockView::Table(table) => {
            table.headers.iter().any(|cell| !cell.trim().is_empty())
                || table
                    .rows
                    .iter()
                    .flatten()
                    .any(|cell| !cell.trim().is_empty())
        }
        MarkdownBlockView::Heading { text, .. } => !text.trim().is_empty(),
        MarkdownBlockView::Blockquote(text) => !text.trim().is_empty(),
    }
}

fn timestamp_line(timestamp: OffsetDateTime, max_width: usize) -> MessageLineView {
    let timestamp = format_message_timestamp(timestamp);
    let padding = " ".repeat(max_width.saturating_sub(line_width(&timestamp)));
    MessageLineView::with_spans(
        MessageRole::System,
        vec![
            MessageSpanView::new(padding, None),
            MessageSpanView::new(
                timestamp,
                Some(TextStyle::default().fg(Color::DarkGrey).dim()),
            ),
        ],
    )
}

fn format_message_timestamp(timestamp: OffsetDateTime) -> String {
    let hour = timestamp.hour();
    let minute = timestamp.minute();
    let meridiem = if hour < 12 { "AM" } else { "PM" };
    let hour = match hour % 12 {
        0 => 12,
        value => value,
    };
    format!("{hour:02}:{minute:02} {meridiem}")
}

fn summarize_tool_input(value: &Value) -> String {
    if let Some(command) = value
        .get("command")
        .or_else(|| value.get("cmd"))
        .and_then(Value::as_str)
    {
        return format!("command={}", quoted(command));
    }
    if let Some(path) = value
        .get("path")
        .or_else(|| value.get("file_path"))
        .or_else(|| value.get("uri"))
        .and_then(Value::as_str)
    {
        return format!("path={}", quoted(path));
    }
    if let Some(query) = value
        .get("query")
        .or_else(|| value.get("pattern"))
        .or_else(|| value.get("prompt"))
        .and_then(Value::as_str)
    {
        return format!("query={}", quoted(query));
    }

    let compact = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
    truncate_visible_end(&compact, 40)
}

fn short_use_id(use_id: ToolUseId) -> String {
    use_id
        .to_string()
        .split('-')
        .next()
        .map_or_else(|| "unknown".into(), ToString::to_string)
}

fn quoted(text: &str) -> String {
    format!("\"{}\"", truncate_visible_end(text, 24))
}

fn extension_kind(text: &str) -> Option<AttachmentKind> {
    if text.ends_with(".png")
        || text.ends_with(".jpg")
        || text.ends_with(".jpeg")
        || text.ends_with(".gif")
        || text.ends_with(".webp")
    {
        return Some(AttachmentKind::Image);
    }
    if text.ends_with(".pdf") {
        return Some(AttachmentKind::Pdf);
    }
    None
}

fn truncate_visible_end(text: &str, max_width: usize) -> String {
    let stripped = strip_ansi(text);
    let stripped = stripped.as_ref();
    if max_width == 0 {
        return String::new();
    }
    if line_width(stripped) <= max_width {
        return stripped.to_string();
    }

    let mut truncated = String::new();
    let mut width = 0usize;
    let target = max_width.saturating_sub(1);

    for grapheme in stripped.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if width.saturating_add(grapheme_width) > target {
            break;
        }
        truncated.push_str(grapheme);
        width = width.saturating_add(grapheme_width);
    }
    truncated.push('…');
    truncated
}

fn append_ellipsis(text: &str, max_width: usize) -> String {
    if text.ends_with('…') {
        return text.to_string();
    }

    let mut candidate = text.to_string();
    candidate.push('…');
    truncate_visible_end(&candidate, max_width)
}

#[cfg(test)]
mod tests {
    use ::time::{Duration, OffsetDateTime};
    use wonder_of_u_core::{MessagePayload, SessionId};

    use super::*;

    fn use_id(value: &str) -> ToolUseId {
        ToolUseId::parse(value).expect("valid tool use id")
    }

    #[test]
    fn markdown_summary_renders_text_and_highlighted_code_blocks() {
        let view = MarkdownSummaryView::new(
            MessageRole::Assistant,
            "# Heading\n- first item\n```rust\nfn main() {}\nprintln!(\"hi\");\n```",
        );

        // # Heading → Heading block (bold cyan span, no role prefix)
        // - first item → Paragraph block ("◆ • first item")
        // code block → 2 syntax-highlighted lines
        let lines = view.display_lines(80);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].text, "Heading");
        assert!(!lines[0].spans.is_empty(), "heading must have styled span");
        assert_eq!(
            lines[1],
            MessageLineView::new("◆ • first item", MessageRole::Assistant)
        );
        assert_eq!(lines[2].text, "fn main() {}");
        assert_eq!(lines[3].text, "println!(\"hi\");");
        assert!(!lines[2].spans.is_empty());
        assert!(
            lines[2].spans.iter().any(|span| span.style.is_some()),
            "expected syntect to style Rust code: {lines:?}"
        );
    }

    #[test]
    fn markdown_paragraph_wraps_once_without_orphan_fragments() {
        let max_width = 20;
        // 30 'a's: wraps at the prefix-adjusted width (18), not at the full
        // width first. Double-wrapping used to leave 1-2 char orphan lines.
        let view = MarkdownSummaryView::new(MessageRole::Assistant, &"a".repeat(30));

        let lines = view.display_lines(max_width);
        assert_eq!(lines[0].text, format!("◆ {}", "a".repeat(18)));
        assert_eq!(lines[1].text, format!("  {}", "a".repeat(12)));
        assert_eq!(lines.len(), 2);
        assert!(
            lines.iter().all(|line| line_width(&line.text) <= max_width),
            "no line may exceed max_width: {lines:?}"
        );
    }

    #[test]
    fn markdown_heading_wraps_at_max_width() {
        let max_width = 10;
        let view =
            MarkdownSummaryView::new(MessageRole::Assistant, &format!("# {}", "h".repeat(25)));

        let lines = view.display_lines(max_width);
        assert!(lines.len() >= 3, "long heading must wrap: {lines:?}");
        for line in &lines {
            assert!(
                line_width(&line.text) <= max_width,
                "heading line overflows: {:?}",
                line.text
            );
            assert!(
                line.spans.iter().any(|span| span.style.is_some()),
                "wrapped heading segments keep their style"
            );
        }
    }

    #[test]
    fn markdown_table_renders_simple_two_column_table() {
        let view = MarkdownSummaryView::new(
            MessageRole::Assistant,
            "| Col A | Col B |\n|-------|-------|\n| val 1 | val 2 |",
        );

        let lines = view.display_lines(80);
        assert!(lines.iter().any(|line| line.text.contains("Col A")));
        assert!(lines.iter().any(|line| line.text.contains("Col B")));
        assert!(lines.iter().any(|line| line.text.contains("val 1")));
        assert!(lines.iter().any(|line| line.text.contains("val 2")));
        assert!(lines.iter().all(|line| line.role == MessageRole::Assistant));
    }

    #[test]
    fn markdown_table_with_separator_row_is_skipped() {
        let view = MarkdownSummaryView::new(
            MessageRole::Assistant,
            "| Col A | Col B |\n|:------|------:|\n| val 1 | val 2 |",
        );

        let lines = view.display_lines(80);
        assert!(lines.iter().any(|line| line.text.contains("val 1")));
        assert!(!lines.iter().any(|line| line.text.contains(":------")));
        assert!(!lines.iter().any(|line| line.text.contains("------:")));
    }

    #[test]
    fn markdown_fenced_markdown_table_is_unwrapped_for_agent_messages() {
        let view = MarkdownSummaryView::new(
            MessageRole::Assistant,
            "```markdown\n| Name | Role |\n| --- | --- |\n| Ada | Engineer |\n```",
        );

        assert!(
            view.blocks
                .iter()
                .any(|block| matches!(block, MarkdownBlockView::Table(_))),
            "markdown table fence should become a table block: {:?}",
            view.blocks
        );
        let lines = view.display_lines(80);
        assert!(lines.iter().any(|line| line.text.contains("Ada")));
        assert!(!lines.iter().any(|line| line.text.contains("```markdown")));
    }

    #[test]
    fn non_markdown_fenced_code_stays_code() {
        let view = MarkdownSummaryView::new(MessageRole::Assistant, "```rust\nfn main() {}\n```");

        assert!(
            view.blocks
                .iter()
                .any(|block| matches!(block, MarkdownBlockView::Code(_))),
            "rust fence should stay as code: {:?}",
            view.blocks
        );
        let lines = view.display_lines(80);
        assert_eq!(lines[0].text, "fn main() {}");
    }

    #[test]
    fn markdown_inline_styles_and_web_links_are_styled() {
        let view = MarkdownSummaryView::new(
            MessageRole::Assistant,
            "Use **bold**, *italic*, `code`, and https://example.com.",
        );

        let lines = view.display_lines(120);
        let spans = &lines[0].spans;
        assert!(
            spans
                .iter()
                .any(|span| span.text == "bold" && span.style.is_some_and(|style| style.bold)),
            "bold span missing: {spans:?}"
        );
        assert!(
            spans
                .iter()
                .any(|span| span.text == "italic" && span.style.is_some_and(|style| style.italic)),
            "italic span missing: {spans:?}"
        );
        assert!(
            spans.iter().any(|span| span.text == "code"
                && span
                    .style
                    .is_some_and(|style| style.fg == Some(Color::Cyan))),
            "inline code span missing: {spans:?}"
        );
        assert!(
            spans.iter().any(|span| span.text == "https://example.com"
                && span
                    .style
                    .is_some_and(|style| style.fg == Some(Color::Cyan) && style.underlined)),
            "web link span missing: {spans:?}"
        );
    }

    #[test]
    fn local_markdown_links_render_relative_to_cwd() {
        let view = MarkdownSummaryView::new_with_cwd(
            MessageRole::Assistant,
            "See [entry](/workspace/src/main.rs:42).",
            Some(std::path::PathBuf::from("/workspace")),
        );

        let lines = view.display_lines(80);
        assert_eq!(lines[0].text, "◆ See src/main.rs:42.");
        assert!(
            lines[0]
                .spans
                .iter()
                .any(|span| span.text == "src/main.rs:42"
                    && span.style.is_some_and(|style| style.underlined)),
            "local link span missing: {:?}",
            lines[0].spans
        );
    }

    #[test]
    fn wide_tables_fall_back_to_key_value_rows_when_narrow() {
        let view = MarkdownSummaryView::new(
            MessageRole::Assistant,
            "| File | Status | Notes |\n| --- | --- | --- |\n| src/message/rich.rs | changed | renderer path |\n",
        );

        let lines = view.display_lines(24);
        assert!(lines.iter().any(|line| line.text.starts_with("File:")));
        assert!(lines.iter().any(|line| line.text.starts_with("Status:")));
        assert!(lines.iter().any(|line| line.text.starts_with("Notes:")));
        assert!(
            lines.iter().all(|line| line_width(&line.text) <= 24),
            "narrow table fallback overflowed: {lines:?}"
        );
    }

    #[test]
    fn highlight_code_block_styles_known_language() {
        let lines = highlight_code_block("rust", "fn main() { let value = 1; }\n");

        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].spans.iter().any(|span| span.style.fg.is_some()),
            "expected at least one colored span for Rust code: {lines:?}"
        );
    }

    #[test]
    fn highlight_code_block_falls_back_to_plain_text_for_unknown_language() {
        let lines = highlight_code_block("not-a-real-language", "plain text\n");

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans.len(), 1);
        assert_eq!(lines[0].spans[0].content.as_ref(), "plain text");
        assert_eq!(lines[0].spans[0].style, RatatuiStyle::default());
    }

    #[test]
    fn markdown_summary_user_text_renders_with_bullet_prefix() {
        // User messages use "▶ " prefix; assistant uses "◆ ".
        let view = MarkdownSummaryView::new(MessageRole::User, "review the diff and continue");

        assert_eq!(
            view.display_lines(80),
            vec![MessageLineView::new(
                "▶ review the diff and continue",
                MessageRole::User,
            )]
        );
    }

    #[test]
    fn rich_message_views_show_timestamp_line_for_assistant_text_messages() {
        let session_id = SessionId::new();
        let mut message = MessageEnvelope::new(
            session_id,
            MessagePayload::AssistantText {
                content: "all set".into(),
            },
        );
        message.timestamp =
            OffsetDateTime::UNIX_EPOCH + Duration::hours(12) + Duration::minutes(45);

        let views = rich_message_views(&[message], false);
        let lines = views[0].display_lines(20, false);

        assert_eq!(
            lines[0],
            MessageLineView::new("◆ all set", MessageRole::Assistant)
        );
        assert_eq!(
            lines[1],
            MessageLineView::with_spans(
                MessageRole::System,
                vec![
                    MessageSpanView::new(" ".repeat(12), None),
                    MessageSpanView::new(
                        "12:45 PM",
                        Some(TextStyle::default().fg(Color::DarkGrey).dim()),
                    ),
                ],
            )
        );
    }

    #[test]
    fn thinking_summary_supports_collapsed_and_expanded_states() {
        let collapsed = ThinkingBlockView::new("step one\nstep two", true);
        assert_eq!(
            collapsed.display_lines(80),
            vec![MessageLineView::new(
                "thinking> ∴ Thinking · collapsed · 2 lines hidden",
                MessageRole::Progress,
            )]
        );

        let expanded = ThinkingBlockView::new("step one\nstep two", false);
        assert_eq!(
            expanded.display_lines(80),
            vec![
                MessageLineView::new("thinking> ∴ Thinking…", MessageRole::Progress),
                MessageLineView::new("  step one", MessageRole::Progress),
                MessageLineView::new("  step two", MessageRole::Progress),
            ]
        );
    }

    #[test]
    fn hook_progress_messages_render_as_non_empty_system_lines() {
        let session_id = SessionId::new();
        let views = rich_message_views(
            &[MessageEnvelope::new(
                session_id,
                MessagePayload::HookProgress {
                    event: "PreToolUse".into(),
                    tool_name: "bash".into(),
                    hook_count: 1,
                    success: true,
                },
            )],
            false,
        );
        let lines = views[0].display_lines(80, false);

        assert!(!lines.is_empty());
        assert_eq!(lines[0].role, MessageRole::System);
        assert!(!lines[0].text.trim().is_empty());
        assert!(lines[0].text.contains("hook"));
    }

    #[test]
    fn rich_message_views_group_tool_uses_and_results() {
        let session_id = SessionId::new();
        let mut messages = vec![
            MessageEnvelope::new(
                session_id,
                MessagePayload::AssistantToolUse {
                    tool: "bash".into(),
                    use_id: use_id("00000000-0000-0000-0000-000000000001"),
                    input: serde_json::json!({ "command": "cargo test -p wonder-of-u-tui" }),
                },
            ),
            MessageEnvelope::new(
                session_id,
                MessagePayload::AssistantToolUse {
                    tool: "bash".into(),
                    use_id: use_id("00000000-0000-0000-0000-000000000002"),
                    input: serde_json::json!({ "command": "rm -rf /tmp/build" }),
                },
            ),
            MessageEnvelope::new(
                session_id,
                MessagePayload::ToolResult {
                    tool: "bash".into(),
                    use_id: use_id("00000000-0000-0000-0000-000000000001"),
                    success: true,
                    content: "tests passed".into(),
                },
            ),
            MessageEnvelope::new(
                session_id,
                MessagePayload::ToolResult {
                    tool: "bash".into(),
                    use_id: use_id("00000000-0000-0000-0000-000000000002"),
                    success: false,
                    content: "Tool use rejected by the user".into(),
                },
            ),
        ];
        let base = OffsetDateTime::UNIX_EPOCH;
        messages[0].timestamp = base;
        messages[1].timestamp = base + Duration::seconds(1);
        messages[2].timestamp = base + Duration::seconds(2);
        messages[3].timestamp = base + Duration::seconds(4);

        let views = rich_message_views(&messages, false);
        assert_eq!(views.len(), 1);
        let RichMessageView::ToolGroup(group) = &views[0] else {
            panic!("expected grouped tool view, got {views:?}");
        };
        assert_eq!(group.tool, "bash");
        assert_eq!(group.calls.len(), 2);
        assert_eq!(
            group.calls[0].use_id,
            use_id("00000000-0000-0000-0000-000000000001")
        );
        assert_eq!(
            group.calls[0].input,
            Some(serde_json::json!({ "command": "cargo test -p wonder-of-u-tui" }))
        );
        assert_eq!(
            group.calls[0].result,
            Some(RejectedToolMessageView {
                kind: RejectedToolMessageKind::Success,
                status: ToolResultStatus::Success,
                detail: "tests passed".into(),
            })
        );
        assert_eq!(group.calls[0].elapsed_secs, Some(2.0));
        assert_eq!(
            group.calls[1].use_id,
            use_id("00000000-0000-0000-0000-000000000002")
        );
        assert_eq!(
            group.calls[1].input,
            Some(serde_json::json!({ "command": "rm -rf /tmp/build" }))
        );
        assert_eq!(
            group.calls[1].result,
            Some(RejectedToolMessageView {
                kind: RejectedToolMessageKind::Rejected,
                status: ToolResultStatus::Rejected,
                detail: "Tool use rejected by the user".into(),
            })
        );
        assert_eq!(group.calls[1].elapsed_secs, Some(3.0));

        let lines = views
            .into_iter()
            .flat_map(|view| view.display_lines(120, false))
            .collect::<Vec<_>>();
        assert_eq!(
            lines,
            vec![
                MessageLineView::new("● Run(Tests) · 2.0s", MessageRole::Tool,),
                MessageLineView::new("  ⎿  cargo test -p wonder-of-u-tui", MessageRole::System,),
                MessageLineView::new("  ⎿  tests passed", MessageRole::System),
                MessageLineView::new("", MessageRole::System),
                MessageLineView::new("● Bash(rm -rf /tmp/build) · 3.0s", MessageRole::Error,),
                MessageLineView::new("  ⎿  rm -rf /tmp/build", MessageRole::System,),
                MessageLineView::new("  ⎿  Tool use rejected by the user", MessageRole::Error,),
            ]
        );
    }

    #[test]
    fn tool_activity_summaries_include_inline_previews_for_read_and_write_tools() {
        let read = ToolCallView {
            use_id: use_id("00000000-0000-0000-0000-000000000001"),
            input: Some(serde_json::json!({ "path": "src/lib.rs" })),
            result: Some(RejectedToolMessageView::from_result(
                "pub fn demo() {\n    println!(\"hi\");\n}\n",
                true,
            )),
            elapsed_secs: None,
        };
        assert_eq!(
            read.display_lines("file_read", 120, false),
            vec![
                MessageLineView::new("● Read(lib.rs)", MessageRole::Tool),
                MessageLineView::new("  ⎿  src/lib.rs", MessageRole::System),
                MessageLineView::new("  ⎿  pub fn demo() {", MessageRole::System),
                MessageLineView::new("        println!(\"hi\");", MessageRole::System),
                MessageLineView::new("    }", MessageRole::System),
            ]
        );

        let write = ToolCallView {
            use_id: use_id("00000000-0000-0000-0000-000000000002"),
            input: Some(serde_json::json!({
                "path": "src/lib.rs",
                "content": "pub fn demo() {\n    println!(\"updated\");\n}\n",
            })),
            result: Some(RejectedToolMessageView::from_result(
                "wrote src/lib.rs",
                true,
            )),
            elapsed_secs: None,
        };
        assert_eq!(
            write.display_lines("file_write", 120, false),
            vec![
                MessageLineView::new("● Edit(lib.rs)", MessageRole::Tool),
                MessageLineView::new("  ⎿  src/lib.rs", MessageRole::System),
                MessageLineView::new("  ⎿  pub fn demo() {", MessageRole::System),
                MessageLineView::new("        println!(\"updated\");", MessageRole::System),
                MessageLineView::new("    }", MessageRole::System),
                MessageLineView::new("  ⎿  wrote src/lib.rs", MessageRole::System),
            ]
        );
    }

    #[test]
    fn grouped_file_reads_render_summary_header() {
        let view = GroupedToolCallView {
            tool: "file_read".into(),
            calls: vec![
                ToolCallView {
                    use_id: use_id("00000000-0000-0000-0000-000000000011"),
                    input: Some(serde_json::json!({ "path": "src/lib.rs" })),
                    result: Some(RejectedToolMessageView::from_result(
                        "pub fn one() {}",
                        true,
                    )),
                    elapsed_secs: Some(0.8),
                },
                ToolCallView {
                    use_id: use_id("00000000-0000-0000-0000-000000000012"),
                    input: Some(serde_json::json!({ "path": "src/main.rs" })),
                    result: Some(RejectedToolMessageView::from_result("fn main() {}", true)),
                    elapsed_secs: Some(1.3),
                },
            ],
        };

        let lines = view.display_lines(120, false);

        assert_eq!(lines[0].role, MessageRole::Tool);
        assert!(lines[0].text.starts_with("⤿ Read 2 files"));
    }

    #[test]
    fn grouped_searches_render_summary_header() {
        let view = GroupedToolCallView {
            tool: "search".into(),
            calls: vec![ToolCallView {
                use_id: use_id("00000000-0000-0000-0000-000000000021"),
                input: Some(serde_json::json!({ "query": "display_lines" })),
                result: Some(RejectedToolMessageView::from_result(
                    "src/message/rich.rs:933",
                    true,
                )),
                elapsed_secs: Some(0.8),
            }],
        };

        let lines = view.display_lines(120, false);

        assert_eq!(lines[0].role, MessageRole::Tool);
        assert!(lines[0].text.starts_with("⤿ Searched"));
    }

    #[test]
    fn grouped_bash_calls_do_not_render_summary_header() {
        let view = GroupedToolCallView {
            tool: "bash".into(),
            calls: vec![ToolCallView {
                use_id: use_id("00000000-0000-0000-0000-000000000031"),
                input: Some(serde_json::json!({ "command": "cargo test" })),
                result: Some(RejectedToolMessageView::from_result("ok", true)),
                elapsed_secs: Some(1.0),
            }],
        };

        let lines = view.display_lines(120, false);

        assert!(!lines[0].text.starts_with("⤿"));
    }

    #[test]
    fn attachment_summary_detects_images() {
        let view =
            AttachmentSummaryView::new("diagram.png", "file:///workspace/assets/diagram.png");

        assert_eq!(
            view.display_lines(80),
            vec![
                MessageLineView::new("attachment> image diagram.png", MessageRole::User),
                MessageLineView::new("  file:///workspace/assets/diagram.png", MessageRole::User,),
            ]
        );
    }

    #[test]
    fn elapsed_time_appended_to_tool_headline() {
        let call = ToolCallView {
            use_id: use_id("00000000-0000-0000-0000-000000000001"),
            input: None,
            result: None,
            elapsed_secs: Some(2.5),
        };

        let summary = ToolActivitySummary::from_call("bash", &call, false);
        assert!(
            summary.headline.contains("2.5s"),
            "expected elapsed in headline: {}",
            summary.headline
        );
    }

    #[test]
    fn system_error_summary_detects_rate_limit_and_api_failures() {
        let rate_limit = SystemErrorView::detect(
            MessageRole::Assistant,
            "Rate limit reached. Try again in 3 minutes.",
        )
        .expect("rate limit view");
        assert_eq!(rate_limit.kind, SystemErrorKind::RateLimit);
        assert_eq!(
            rate_limit.display_lines(80),
            vec![MessageLineView::new(
                "error> rate limit: Rate limit reached. Try again in 3 minutes.",
                MessageRole::Error,
            )]
        );

        let timeout = SystemErrorView::detect(
            MessageRole::System,
            "API request timed out after 30s while contacting the provider.",
        )
        .expect("timeout view");
        assert_eq!(timeout.kind, SystemErrorKind::Timeout);
        assert_eq!(
            timeout.display_lines(80),
            vec![MessageLineView::new(
                "error> timeout: API request timed out after 30s while contacting the provider.",
                MessageRole::Error,
            )]
        );
    }

    #[test]
    fn transcript_boundary_renders_compact_summary() {
        let view = TranscriptBoundaryView::new("Older tool output hidden");

        assert_eq!(
            view.display_lines(80),
            vec![MessageLineView::new(
                "summary> ✻ Conversation compacted · Older tool output hidden",
                MessageRole::System,
            )]
        );
    }

    #[test]
    fn file_edit_reference_uses_diff_summary_labels() {
        let view = FileEditReferenceView::new(
            PathLinkView::new("crates/wonder-of-u-tui/src/message/mod.rs"),
            FileEditHunkSummary {
                additions: 12,
                removals: 1,
                context: 4,
            },
        )
        .with_note("Rich transcript summaries");

        assert_eq!(
            view.display_lines(120),
            vec![
                MessageLineView::new(
                    "● Edit(crates/wonder-of-u-tui/src/message/mod.rs)",
                    MessageRole::Tool,
                ),
                MessageLineView::new("  ⎿  +12 -1 ~4", MessageRole::System),
                MessageLineView::new("  Rich transcript summaries", MessageRole::System),
            ]
        );
    }

    // ── no-prefix transcript tests (Claude Code parity) ──────────────────────

    #[test]
    fn user_text_renders_without_user_prefix() {
        let session_id = SessionId::new();
        let messages = vec![MessageEnvelope::new(
            session_id,
            MessagePayload::UserText {
                content: "hi there".into(),
            },
        )];
        let lines = super::super::message_lines(&messages, false);
        assert!(
            lines.iter().all(|l| !l.text.starts_with("user>")),
            "UserText must not render with 'user>' prefix; lines: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.text.contains("hi there")),
            "UserText content must appear in transcript; lines: {lines:?}"
        );
        // Visual parity: first user line must start with "▶" prefix.
        assert!(
            lines.iter().any(|l| l.text.starts_with('▶')),
            "UserText first line must start with ▶ prefix; lines: {lines:?}"
        );
    }

    #[test]
    fn assistant_text_renders_without_assistant_prefix() {
        let session_id = SessionId::new();
        let messages = vec![MessageEnvelope::new(
            session_id,
            MessagePayload::AssistantText {
                content: "Hello! How can I help?".into(),
            },
        )];
        let lines = super::super::message_lines(&messages, false);
        assert!(
            lines.iter().all(|l| !l.text.starts_with("assistant>")),
            "AssistantText must not render with 'assistant>' prefix; lines: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.text.contains("Hello!")),
            "AssistantText content must appear in transcript; lines: {lines:?}"
        );
        // Visual parity: first assistant line must start with "◆" prefix.
        assert!(
            lines.iter().any(|l| l.text.starts_with('◆')),
            "AssistantText first line must start with ◆ prefix; lines: {lines:?}"
        );
    }

    #[test]
    fn assistant_text_with_error_topic_does_not_render_as_system_error() {
        let session_id = SessionId::new();
        let messages = vec![MessageEnvelope::new(
            session_id,
            MessagePayload::AssistantText {
                content: "Perfect! Here is an overview.\n\n## Error handling\nThe project uses structured errors.".into(),
            },
        )];
        let lines = super::super::message_lines(&messages, false);

        assert!(
            lines.iter().all(|line| line.role != MessageRole::Error),
            "Assistant overview text must not be promoted to error UI; lines: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.text.contains("Error handling")),
            "Assistant content must still render; lines: {lines:?}"
        );
    }

    #[test]
    fn tool_activity_renders_with_bullet_and_subordinate_result() {
        let session_id = SessionId::new();
        let uid = use_id("00000000-0000-0000-0000-000000000001");
        let messages = vec![
            MessageEnvelope::new(
                session_id,
                MessagePayload::AssistantToolUse {
                    tool: "file_read".into(),
                    use_id: uid,
                    input: serde_json::json!({ "path": "src/lib.rs" }),
                },
            ),
            MessageEnvelope::new(
                session_id,
                MessagePayload::ToolResult {
                    tool: "file_read".into(),
                    use_id: uid,
                    success: true,
                    content: "pub fn main() {}".into(),
                },
            ),
        ];
        let lines = super::super::message_lines(&messages, false);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert!(
            texts.iter().any(|t| t.starts_with('●')),
            "Tool headline must start with ● bullet; lines: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains('⎿')),
            "Tool result must contain ⎿ guide prefix; lines: {texts:?}"
        );
        assert!(
            texts.iter().all(|t| !t.starts_with("tool[")),
            "Old tool[name] format must not appear; lines: {texts:?}"
        );
    }

    #[test]
    fn collapsed_tool_output_shows_expand_hint() {
        let session_id = SessionId::new();
        let uid = use_id("00000000-0000-0000-0000-000000000002");
        let content = (1..=22)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let messages = vec![
            MessageEnvelope::new(
                session_id,
                MessagePayload::AssistantToolUse {
                    tool: "file_read".into(),
                    use_id: uid,
                    input: serde_json::json!({ "path": "src/lib.rs" }),
                },
            ),
            MessageEnvelope::new(
                session_id,
                MessagePayload::ToolResult {
                    tool: "file_read".into(),
                    use_id: uid,
                    success: true,
                    content,
                },
            ),
        ];

        let lines = super::super::message_lines(&messages, false);
        assert!(
            lines
                .iter()
                .any(|line| line.text.contains("… +2 lines (ctrl+o to expand)")),
            "Collapsed transcript must expose the ctrl+o hint; lines: {lines:?}"
        );
    }

    #[test]
    fn expanded_tool_output_renders_all_preview_lines() {
        let session_id = SessionId::new();
        let uid = use_id("00000000-0000-0000-0000-000000000003");
        let content = (1..=22)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let messages = vec![
            MessageEnvelope::new(
                session_id,
                MessagePayload::AssistantToolUse {
                    tool: "file_read".into(),
                    use_id: uid,
                    input: serde_json::json!({ "path": "src/lib.rs" }),
                },
            ),
            MessageEnvelope::new(
                session_id,
                MessagePayload::ToolResult {
                    tool: "file_read".into(),
                    use_id: uid,
                    success: true,
                    content,
                },
            ),
        ];

        let lines = super::super::message_lines(&messages, true);
        let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert!(
            texts.iter().any(|line| line.contains("line 22")),
            "Expanded transcript must include the full preview; lines: {texts:?}"
        );
        assert!(
            texts
                .iter()
                .all(|line| !line.contains("(ctrl+o to expand)") && !line.contains("… +")),
            "Expanded transcript must not render overflow hints; lines: {texts:?}"
        );
    }

    #[test]
    fn full_conversation_has_no_legacy_role_prefixes() {
        let session_id = SessionId::new();
        let uid = use_id("00000000-0000-0000-0000-000000000099");
        let messages = vec![
            MessageEnvelope::new(
                session_id,
                MessagePayload::UserText {
                    content: "list my files".into(),
                },
            ),
            MessageEnvelope::new(
                session_id,
                MessagePayload::AssistantToolUse {
                    tool: "bash".into(),
                    use_id: uid,
                    input: serde_json::json!({ "command": "ls ." }),
                },
            ),
            MessageEnvelope::new(
                session_id,
                MessagePayload::ToolResult {
                    tool: "bash".into(),
                    use_id: uid,
                    success: true,
                    content: "Cargo.toml\nsrc/".into(),
                },
            ),
            MessageEnvelope::new(
                session_id,
                MessagePayload::AssistantText {
                    content: "Done, here are your files.".into(),
                },
            ),
        ];
        let lines = super::super::message_lines(&messages, false);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        // None of the legacy prefixes must appear.
        for prefix in ["user> ", "assistant> ", "tool[bash]"] {
            assert!(
                texts.iter().all(|t| !t.starts_with(prefix)),
                "Legacy prefix '{prefix}' must not appear in transcript; lines: {texts:?}"
            );
        }
        // Content must be present.
        assert!(
            texts.iter().any(|t| t.contains("list my files")),
            "User content must be present; lines: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains("Done")),
            "Assistant response must be present; lines: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.starts_with('●')),
            "Tool call must use ● bullet; lines: {texts:?}"
        );
        // User messages open with "▶ ", assistant messages with "◆ ".
        assert!(
            texts
                .iter()
                .any(|t| t.starts_with('▶') && t.contains("list my files")),
            "User message must open with ▶ prefix; lines: {texts:?}"
        );
        assert!(
            texts
                .iter()
                .any(|t| t.starts_with('◆') && t.contains("Done")),
            "Assistant message must open with ◆ prefix; lines: {texts:?}"
        );
    }

    // ── Claude Code visual parity: new focused tests ─────────────────────────

    #[test]
    fn user_message_first_paragraph_gets_bullet_prefix() {
        let view = MarkdownSummaryView::new(MessageRole::User, "implement the feature");

        let lines = view.display_lines(80);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "▶ implement the feature");
        assert_eq!(lines[0].role, MessageRole::User);
    }

    #[test]
    fn assistant_message_first_paragraph_gets_bullet_prefix() {
        let view = MarkdownSummaryView::new(MessageRole::Assistant, "I'll take care of that.");

        let lines = view.display_lines(80);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "◆ I'll take care of that.");
        assert_eq!(lines[0].role, MessageRole::Assistant);
    }

    #[test]
    fn user_multiblock_only_first_block_has_bullet() {
        // A user message with two paragraphs (separated by a blank line in the
        // source) must show "▶ " only on the first block's first line; the
        // second paragraph renders flush-left.
        let view =
            MarkdownSummaryView::new(MessageRole::User, "first paragraph\n\nsecond paragraph");

        let lines = view.display_lines(80);
        assert!(
            lines.iter().any(|l| l.text.starts_with('▶')),
            "At least one line must have the ▶ prefix; lines: {lines:?}"
        );
        // Second block must NOT start with "▶".
        assert!(
            lines
                .iter()
                .skip(1)
                .any(|l| l.text.contains("second paragraph") && !l.text.starts_with('▶')),
            "Second paragraph must be flush-left (no prefix); lines: {lines:?}"
        );
    }

    #[test]
    fn assistant_bullet_continuation_is_indented_two_spaces_when_wrapped() {
        // A very long assistant paragraph that must wrap at a narrow width
        // should have the first line starting with "◆ " and subsequent wrapped
        // lines indented by two spaces to align with the bullet body.
        let view = MarkdownSummaryView::new(
            MessageRole::Assistant,
            "This is a very long assistant paragraph that will need to wrap.",
        );

        // Use a narrow width so the text wraps.
        let lines = view.display_lines(20);
        assert!(
            lines[0].text.starts_with("◆ "),
            "First wrapped line must begin with ◆ ; lines: {lines:?}"
        );
        if lines.len() > 1 {
            assert!(
                lines[1].text.starts_with("  "),
                "Continuation wrapped line must be indented two spaces; lines: {lines:?}"
            );
        }
    }

    #[test]
    fn tool_list_row_shows_bullet_name_and_collapsed_detail() {
        // Glob / list tool should render as "● List(target)" headline with a
        // "└ ..." subordinate detail row, matching the Claude Code reference.
        let call = ToolCallView {
            use_id: use_id("00000000-0000-0000-0000-000000000010"),
            input: Some(serde_json::json!({ "pattern": "**/*.rs", "path": "src" })),
            result: Some(RejectedToolMessageView::from_result("57 paths", true)),
            elapsed_secs: None,
        };

        let lines = call.display_lines("glob", 120, false);
        assert!(
            lines[0].text.starts_with("● List("),
            "List tool headline must start with ● List(; lines: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.text.contains('⎿')),
            "List tool must emit a ⎿ guide prefix detail row; lines: {lines:?}"
        );
    }

    #[test]
    fn tool_read_row_shows_bullet_read_and_collapsed_detail() {
        // File read should render as "● Read(filename)" with a "└ path" and
        // preview detail row.
        let call = ToolCallView {
            use_id: use_id("00000000-0000-0000-0000-000000000011"),
            input: Some(serde_json::json!({ "path": "crates/core/src/lib.rs" })),
            result: Some(RejectedToolMessageView::from_result("pub mod core;", true)),
            elapsed_secs: Some(0.3),
        };

        let lines = call.display_lines("file_read", 120, false);
        assert!(
            lines[0].text.starts_with("● Read("),
            "Read tool headline must start with ● Read(; lines: {lines:?}"
        );
        assert_eq!(lines[0].role, MessageRole::Tool);
        assert!(
            lines.iter().any(|l| l.text.contains('⎿')),
            "Read tool must emit at least one ⎿ guide prefix detail row; lines: {lines:?}"
        );
    }

    #[test]
    fn tool_update_row_shows_bullet_edit_and_detail() {
        // File write/edit should render as "● Edit(filename)" with "└ path"
        // and an optional result detail row.
        let call = ToolCallView {
            use_id: use_id("00000000-0000-0000-0000-000000000012"),
            input: Some(serde_json::json!({
                "path": "crates/core/src/lib.rs",
                "content": "pub mod new_core;",
            })),
            result: Some(RejectedToolMessageView::from_result(
                "wrote crates/core/src/lib.rs",
                true,
            )),
            elapsed_secs: None,
        };

        let lines = call.display_lines("file_write", 120, false);
        assert!(
            lines[0].text.starts_with("● Edit("),
            "Write tool headline must start with ● Edit(; lines: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.text.contains('⎿')),
            "Write tool must emit at least one ⎿ guide prefix detail row; lines: {lines:?}"
        );
    }

    #[test]
    fn expand_hint_uses_ctrl_o_keybinding() {
        // The expand hint text must reference the actual keybinding ("ctrl+o")
        // that is configured in wonder-of-u so the UI doesn't lie to the user.
        let call = ToolCallView {
            use_id: use_id("00000000-0000-0000-0000-000000000020"),
            input: Some(serde_json::json!({ "path": "src/lib.rs" })),
            result: Some(RejectedToolMessageView::from_result(
                (1..=25)
                    .map(|i| format!("line {i}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                true,
            )),
            elapsed_secs: None,
        };

        let lines = call.display_lines("file_read", 120, false);
        assert!(
            lines
                .iter()
                .any(|l| l.text.contains("ctrl+o") && l.text.contains("expand")),
            "Overflow hint must say 'ctrl+o' (the real keybinding) to expand; lines: {lines:?}"
        );
    }

    #[test]
    fn search_tool_renders_bullet_search_headline() {
        // Grep / search tool should render as "● Search(query)".
        let call = ToolCallView {
            use_id: use_id("00000000-0000-0000-0000-000000000030"),
            input: Some(serde_json::json!({ "query": "display_lines" })),
            result: Some(RejectedToolMessageView::from_result(
                "src/message/rich.rs:933",
                true,
            )),
            elapsed_secs: Some(0.5),
        };

        let lines = call.display_lines("search", 120, false);
        assert!(
            lines[0].text.starts_with("● Search("),
            "Search tool headline must start with ● Search(; lines: {lines:?}"
        );
    }

    // ── ink-message-cards: new focused tests ─────────────────────────────────

    #[test]
    fn wrap_summary_lines_summarize_false_never_truncates() {
        // When summarize=false, wrap_summary_lines must return ALL wrapped lines
        // regardless of max_lines, and must NOT append an ellipsis.
        let long_text = "word ".repeat(200); // produces many wrapped lines at width 40
        let lines = wrap_summary_lines(long_text.trim(), 40, 6, false);
        assert!(
            lines.len() > 6,
            "summarize=false must not truncate; got {} lines",
            lines.len()
        );
        assert!(
            lines.iter().all(|l| !l.ends_with('…')),
            "summarize=false must not append ellipsis; lines: {lines:?}"
        );
    }

    #[test]
    fn wrap_summary_lines_summarize_true_truncates_with_ellipsis() {
        // When summarize=true and content exceeds max_lines, the result must be
        // capped at max_lines and the last line must end with '…'.
        let long_text = "word ".repeat(200);
        let lines = wrap_summary_lines(long_text.trim(), 40, 4, true);
        assert_eq!(lines.len(), 4, "summarize=true must truncate to max_lines");
        assert!(
            lines.last().unwrap().ends_with('…'),
            "summarize=true must append ellipsis on the last line; lines: {lines:?}"
        );
    }

    #[test]
    fn wrap_summary_lines_preserves_newlines_as_separate_lines() {
        // Multi-line content (e.g. thinking block, error detail) must keep each
        // logical line on its own output line rather than collapsing to a single
        // space-separated run.
        let text = "first line\nsecond line\n\nfourth line";
        let lines = wrap_summary_lines(text, 80, 20, false);
        assert!(
            lines.iter().any(|l| l == "first line"),
            "first line must be present; lines: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l == "second line"),
            "second line must be present; lines: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l == "fourth line"),
            "fourth line must be present after blank; lines: {lines:?}"
        );
        // Blank separating line must be preserved.
        assert!(
            lines.iter().any(|l| l.is_empty()),
            "blank separator line must be preserved; lines: {lines:?}"
        );
        // Old collapsing behavior must NOT appear.
        assert!(
            !lines.iter().any(|l| l.contains("first line second line")),
            "lines must not be collapsed together; lines: {lines:?}"
        );
    }

    #[test]
    fn text_style_from_ratatui_strips_syntect_bg_color() {
        // Syntect themes carry grey block backgrounds; text_style_from_ratatui
        // must discard bg so code spans do not render with grey blocks.
        use ratatui::style::Color as RC;
        let ratatui_style = RatatuiStyle::default()
            .fg(RC::Green)
            .bg(RC::Rgb(40, 40, 40));

        let result = text_style_from_ratatui(ratatui_style);
        let style = result.expect("fg=Green must yield a non-default TextStyle");
        assert!(style.fg.is_some(), "fg must be preserved");
        assert!(
            style.bg.is_none(),
            "bg must be stripped by text_style_from_ratatui"
        );
    }

    #[test]
    fn tool_guide_prefix_uses_ink_character() {
        // All tool detail/result subordinate rows must use the Ink guide prefix
        // "  ⎿  " (2 sp + U+23BF + 2 sp) instead of the old "  └ " prefix.
        let call = ToolCallView {
            use_id: use_id("00000000-0000-0000-0000-000000000099"),
            input: Some(serde_json::json!({ "command": "cargo fmt" })),
            result: Some(RejectedToolMessageView::from_result(
                "formatted 3 files",
                true,
            )),
            elapsed_secs: Some(0.4),
        };

        let lines = call.display_lines("bash", 120, false);
        // Every subordinate line must use ⎿ and must NOT contain the old └.
        let sub_lines: Vec<&str> = lines[1..].iter().map(|l| l.text.as_str()).collect();
        assert!(
            sub_lines.iter().any(|t| t.contains('⎿')),
            "Subordinate detail rows must use ⎿ guide prefix; sub_lines: {sub_lines:?}"
        );
        assert!(
            sub_lines.iter().all(|t| !t.contains('└')),
            "Old └ marker must not appear in subordinate rows; sub_lines: {sub_lines:?}"
        );
    }

    #[test]
    fn grouped_tool_calls_guide_prefix_on_all_detail_rows() {
        // In a multi-call tool group, every detail/result row across all calls
        // must use the ⎿ guide prefix.
        let view = GroupedToolCallView {
            tool: "bash".into(),
            calls: vec![
                ToolCallView {
                    use_id: use_id("00000000-0000-0000-0000-000000000041"),
                    input: Some(serde_json::json!({ "command": "cargo check" })),
                    result: Some(RejectedToolMessageView::from_result("ok", true)),
                    elapsed_secs: Some(1.0),
                },
                ToolCallView {
                    use_id: use_id("00000000-0000-0000-0000-000000000042"),
                    input: Some(serde_json::json!({ "command": "cargo clippy" })),
                    result: Some(RejectedToolMessageView::from_result("1 warning", true)),
                    elapsed_secs: Some(2.0),
                },
            ],
        };

        let lines = view.display_lines(120, false);
        // No line anywhere should contain the old └.
        assert!(
            lines.iter().all(|l| !l.text.contains('└')),
            "Old └ must not appear in any grouped tool line; lines: {lines:?}"
        );
    }

    #[test]
    fn completed_table_renders_normally() {
        let view = MarkdownSummaryView::new(
            MessageRole::Assistant,
            "| Name | Age |\n|------|-----|\n| Alice | 30 |",
        );
        let has_table = view
            .blocks
            .iter()
            .any(|b| matches!(b, MarkdownBlockView::Table(_)));
        assert!(
            has_table,
            "Table with body row must render as Table: {:?}",
            view.blocks
        );
    }

    #[test]
    fn non_streaming_code_block_renders_as_code() {
        let view =
            MarkdownSummaryView::new(MessageRole::Assistant, "before\n```rust\nfn main() {}");
        let has_code_block = view
            .blocks
            .iter()
            .any(|b| matches!(b, MarkdownBlockView::Code(_)));
        assert!(
            has_code_block,
            "Non-streaming path must still render partial code fence as Code block: {:?}",
            view.blocks
        );
    }

    #[test]
    fn truncate_existing_lines_clips_wide_table_without_garbling() {
        let wide_source = "| Column A | Column B | Column C | Column D | Column E | Column F |\n\
                           |----------|----------|----------|----------|----------|----------|\n\
                           | value 1  | value 2  | value 3  | value 4  | value 5  | value 6  |";
        let view = MarkdownSummaryView::new(MessageRole::Assistant, wide_source);
        let lines = view.display_lines(40);

        let truncated = truncate_existing_lines(&lines, 40);
        assert!(
            truncated.iter().all(|line| line_width(&line.text) <= 40),
            "every truncated line must fit within width: {truncated:?}"
        );
        assert!(
            lines.iter().all(|line| line_width(&line.text) <= 40),
            "every non-streaming line must fit within width: {lines:?}"
        );
        assert_eq!(truncated.len(), lines.len());
    }

    #[test]
    fn truncate_existing_lines_clips_styled_line_without_splitting() {
        let styled = vec![MessageLineView::with_spans(
            MessageRole::Assistant,
            vec![
                MessageSpanView::new("pre ", None),
                MessageSpanView::new("styled bold text here", Some(TextStyle::default().bold())),
                MessageSpanView::new(" suffix longer", None),
            ],
        )];
        let truncated = truncate_existing_lines(&styled, 20);
        assert_eq!(truncated.len(), 1, "styled wide line is clipped, not split");
        assert!(
            line_width(&truncated[0].text) <= 20,
            "clipped line must fit: {:?}",
            truncated[0].text
        );
        assert!(
            truncated[0].spans.iter().any(|s| s.style.is_some()),
            "styled spans are preserved in clip: {:?}",
            truncated[0].spans
        );
    }
}
