use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use wonder_of_u_core::{MessageEnvelope, MessagePayload, ToolUseId};

use crate::{
    diff::{FileEditHunkSummary, PathLinkView},
    measure::{line_width, strip_ansi, wrap_text_hard},
};

use super::{MessageLineView, MessageRole, render_message};

const MAX_PARAGRAPH_LINES: usize = 6;
const MAX_THINKING_LINES: usize = 4;
const MAX_DETAIL_LINES: usize = 3;

/// Rich, renderer-agnostic message summaries for the TUI transcript.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RichMessageView {
    /// Represents markdown
    Markdown(MarkdownSummaryView),
    /// Represents thinking
    Thinking(ThinkingBlockView),
    /// Represents tool group
    ToolGroup(GroupedToolCallView),
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
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        match self {
            Self::Markdown(view) => view.display_lines(max_width),
            Self::Thinking(view) => view.display_lines(max_width),
            Self::ToolGroup(view) => view.display_lines(max_width),
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
pub fn rich_message_views(messages: &[MessageEnvelope]) -> Vec<RichMessageView> {
    let mut views = Vec::new();
    let mut index = 0usize;

    while index < messages.len() {
        if let Some((group, consumed)) = grouped_tool_call_view(&messages[index..]) {
            views.push(RichMessageView::ToolGroup(group));
            index = index.saturating_add(consumed);
            continue;
        }

        views.push(single_message_view(&messages[index]));
        index = index.saturating_add(1);
    }

    views
}

fn single_message_view(message: &MessageEnvelope) -> RichMessageView {
    match &message.payload {
        MessagePayload::UserText { content } => {
            RichMessageView::Markdown(MarkdownSummaryView::new(MessageRole::User, content))
        }
        MessagePayload::AssistantText { content } => {
            if let Some(view) = SystemErrorView::detect(MessageRole::Assistant, content) {
                RichMessageView::SystemError(view)
            } else {
                RichMessageView::Markdown(MarkdownSummaryView::new(MessageRole::Assistant, content))
            }
        }
        MessagePayload::System { content } => {
            if let Some(view) = SystemErrorView::detect(MessageRole::System, content) {
                RichMessageView::SystemError(view)
            } else {
                RichMessageView::Markdown(MarkdownSummaryView::new(MessageRole::System, content))
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
    let mut consumed = 0usize;

    for message in messages {
        match &message.payload {
            MessagePayload::AssistantToolUse {
                tool: current_tool,
                use_id,
                input,
            } if current_tool == tool => {
                view.push_use(*use_id, Some(input.clone()));
                consumed = consumed.saturating_add(1);
            }
            MessagePayload::ToolResult {
                tool: current_tool,
                use_id,
                success,
                content,
            } if current_tool == tool => {
                view.push_result(*use_id, *success, content.clone());
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
    /// Stores the blocks
    pub blocks: Vec<MarkdownBlockView>,
}

impl MarkdownSummaryView {
    /// Creates a new value
    #[must_use]
    pub fn new(role: MessageRole, text: &str) -> Self {
        Self {
            role,
            blocks: parse_markdown_blocks(text),
        }
    }
    /// Handles display lines
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        let prefix = role_prefix(self.role);
        let mut lines = Vec::new();
        let mut first_block = true;

        for block in &self.blocks {
            match block {
                MarkdownBlockView::Paragraph(text) => {
                    let block_lines =
                        wrap_summary_lines(text, max_width, MAX_PARAGRAPH_LINES, false);
                    let display_prefix = if first_block { prefix } else { "" };
                    if display_prefix.is_empty() {
                        // Claude Code-style transcript paragraphs stay flush-left for user and
                        // assistant narration instead of inheriting the generic detail indent.
                        push_unprefixed_block(&mut lines, &block_lines, self.role, max_width);
                    } else {
                        push_wrapped_block(
                            &mut lines,
                            display_prefix,
                            &block_lines,
                            self.role,
                            max_width,
                        );
                    }
                }
                MarkdownBlockView::Code(code) => {
                    let label = code.label();
                    let preview = code.preview_text();
                    let mut code_lines = vec![preview];
                    if code.line_count > 1 {
                        let hidden = code.line_count.saturating_sub(1);
                        let noun = if hidden == 1 { "line" } else { "lines" };
                        code_lines.push(format!("+{hidden} more {noun}"));
                    }
                    push_wrapped_block(&mut lines, &label, &code_lines, self.role, max_width);
                }
            }
            first_block = false;
        }

        if lines.is_empty() {
            lines.push(MessageLineView::new(prefix.to_string(), self.role));
        }

        lines
    }
}

/// A parsed markdown summary block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkdownBlockView {
    /// Represents paragraph
    Paragraph(String),
    /// Represents code
    Code(MarkdownCodeBlockView),
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

impl MarkdownCodeBlockView {
    fn label(&self) -> String {
        match self.language.as_deref() {
            Some(language) if !language.is_empty() => format!("code[{language}]> "),
            _ => "code> ".into(),
        }
    }

    fn preview_text(&self) -> String {
        self.code
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map_or_else(|| "(empty)".into(), |line| truncate_visible_end(line, 48))
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
#[derive(Clone, Debug, Eq, PartialEq)]
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
        });
    }

    fn push_result(&mut self, use_id: ToolUseId, success: bool, content: String) {
        if let Some(existing) = self.calls.iter_mut().find(|call| call.use_id == use_id) {
            existing.result = Some(ToolCallView::result_for(success, content));
            return;
        }

        self.calls.push(ToolCallView {
            use_id,
            input: None,
            result: Some(ToolCallView::result_for(success, content)),
        });
    }
    /// Handles display lines
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        let mut lines = Vec::new();
        for (index, call) in self.calls.iter().enumerate() {
            if index > 0 {
                lines.push(MessageLineView::new(String::new(), MessageRole::System));
            }
            lines.extend(call.display_lines(&self.tool, max_width));
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

/// A summarized tool invocation and optional result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCallView {
    /// Stores the use identifier
    pub use_id: ToolUseId,
    /// Stores the input
    pub input: Option<Value>,
    /// Stores the result
    pub result: Option<RejectedToolMessageView>,
}

impl ToolCallView {
    fn result_for(success: bool, content: String) -> RejectedToolMessageView {
        RejectedToolMessageView::from_result(content, success)
    }

    fn display_lines(&self, tool: &str, max_width: usize) -> Vec<MessageLineView> {
        let summary = ToolActivitySummary::from_call(tool, self);
        let mut lines = push_line(summary.headline, self.role(), max_width);
        for row in summary.rows {
            match row {
                ToolActivityRow::Detail {
                    label: _,
                    value,
                    role,
                } => push_wrapped_block(&mut lines, "", &[format!("└ {value}")], role, max_width),
                ToolActivityRow::Preview {
                    label: _,
                    lines: preview,
                } => {
                    let formatted = preview
                        .iter()
                        .enumerate()
                        .map(|(index, line)| {
                            if index == 0 {
                                format!("└ {line}")
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
    fn from_call(tool: &str, call: &ToolCallView) -> Self {
        match tool {
            "bash" => summarize_bash_call(call),
            "file_read" => summarize_file_read_call(call),
            "file_write" => summarize_file_write_call(call),
            "glob" => summarize_glob_call(call),
            "grep" | "rg" | "search" | "find" => summarize_search_call(tool, call),
            _ => summarize_generic_tool_call(tool, call),
        }
    }
}

fn summarize_bash_call(call: &ToolCallView) -> ToolActivitySummary {
    let command = call
        .input
        .as_ref()
        .and_then(|input| input.get("command").and_then(Value::as_str))
        .map(normalize_inline_text)
        .unwrap_or_default();
    let headline = if command.is_empty() {
        "● Bash".to_string()
    } else {
        format!("● {}", bash_activity_title(&command))
    };

    let mut rows = status_rows(call);
    if !command.is_empty() {
        rows.push(ToolActivityRow::Detail {
            label: "command",
            value: truncate_visible_end(&command, 72),
            role: MessageRole::System,
        });
    }
    append_result_rows(call, &mut rows);

    ToolActivitySummary { headline, rows }
}

fn summarize_file_read_call(call: &ToolCallView) -> ToolActivitySummary {
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
        if let Some(preview) = preview_lines(call.result_detail(), 3, 72) {
            rows.push(ToolActivityRow::Preview {
                label: "preview",
                lines: preview,
            });
        }
    } else {
        append_result_rows(call, &mut rows);
    }

    ToolActivitySummary {
        headline: format!("● Read({})", compact_target_label(path)),
        rows,
    }
}

fn summarize_file_write_call(call: &ToolCallView) -> ToolActivitySummary {
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
    if let Some(preview) = input_preview.and_then(|text| preview_lines(Some(text), 3, 72)) {
        rows.push(ToolActivityRow::Preview {
            label: "preview",
            lines: preview,
        });
    }
    if !call.is_success() {
        append_result_rows(call, &mut rows);
    } else if let Some(detail) = short_result_detail(call.result_detail()) {
        rows.push(ToolActivityRow::Detail {
            label: "result",
            value: detail,
            role: MessageRole::System,
        });
    }

    ToolActivitySummary {
        headline: format!("● Edit({})", compact_target_label(&path)),
        rows,
    }
}

fn summarize_glob_call(call: &ToolCallView) -> ToolActivitySummary {
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
    append_result_rows(call, &mut rows);

    ToolActivitySummary {
        headline: format!("● List({})", infer_list_target(pattern, path)),
        rows,
    }
}

fn summarize_search_call(tool: &str, call: &ToolCallView) -> ToolActivitySummary {
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
    append_result_rows(call, &mut rows);

    ToolActivitySummary {
        headline: format!(
            "● {}({})",
            if tool == "find" { "List" } else { "Search" },
            compact_target_label(query)
        ),
        rows,
    }
}

fn summarize_generic_tool_call(tool: &str, call: &ToolCallView) -> ToolActivitySummary {
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
    append_result_rows(call, &mut rows);

    ToolActivitySummary {
        headline: format!("● {tool}"),
        rows,
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

fn append_result_rows(call: &ToolCallView, rows: &mut Vec<ToolActivityRow>) {
    let Some(result) = &call.result else {
        return;
    };
    if let Some(preview) = preview_lines(Some(&result.detail), 3, 72) {
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

fn preview_lines(text: Option<&str>, max_lines: usize, max_width: usize) -> Option<Vec<String>> {
    let text = text?;
    let lines = text
        .lines()
        .map(str::trim_end)
        .skip_while(|line| line.trim().is_empty())
        .take(max_lines + 1)
        .map(|line| truncate_visible_end(line, max_width))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    let overflow = lines.len() > max_lines;
    let mut lines = lines.into_iter().take(max_lines).collect::<Vec<_>>();
    if overflow {
        lines.push("…".into());
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
            &[format!("└ {}", self.summary.label())],
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
        let lower = text.to_ascii_lowercase();
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
        if lower.contains("error") || lower.contains("failed") {
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
    let mut code_language = None;
    let mut code_lines = Vec::new();

    for line in text.lines() {
        if let Some(language) = line.strip_prefix("```") {
            if code_language.is_some() {
                blocks.push(MarkdownBlockView::Code(MarkdownCodeBlockView {
                    language: code_language.take(),
                    code: code_lines.join("\n"),
                    line_count: code_lines.len(),
                }));
                code_lines.clear();
            } else {
                flush_paragraph(&mut blocks, &mut paragraph);
                let language = language.trim();
                code_language = (!language.is_empty()).then(|| language.to_string());
            }
            continue;
        }

        if code_language.is_some() {
            code_lines.push(line.to_string());
            continue;
        }

        if line.trim().is_empty() {
            flush_paragraph(&mut blocks, &mut paragraph);
            continue;
        }

        paragraph.push(normalize_markdown_line(line));
    }

    flush_paragraph(&mut blocks, &mut paragraph);
    if code_language.is_some() || !code_lines.is_empty() {
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

fn normalize_markdown_line(line: &str) -> String {
    let trimmed = line.trim();
    let trimmed = trimmed
        .strip_prefix("# ")
        .or_else(|| trimmed.strip_prefix("## "))
        .or_else(|| trimmed.strip_prefix("### "))
        .unwrap_or(trimmed);
    let trimmed = trimmed
        .strip_prefix("- ")
        .map(|value| format!("• {value}"))
        .or_else(|| trimmed.strip_prefix("* ").map(|value| format!("• {value}")))
        .or_else(|| {
            trimmed
                .strip_prefix("> ")
                .map(|value| format!("quote: {value}"))
        })
        .unwrap_or_else(|| trimmed.to_string());

    trimmed.replace('`', "")
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
            output.push(MessageLineView::new(
                format!("{segment_prefix}{segment}"),
                role,
            ));
        }
        used_prefix = true;
    }
}

fn push_unprefixed_block(
    output: &mut Vec<MessageLineView>,
    source_lines: &[String],
    role: MessageRole,
    max_width: usize,
) {
    for source in source_lines {
        let wrapped = wrap_text_hard(source, max_width.max(1));
        if wrapped.is_empty() {
            output.push(MessageLineView::new(String::new(), role));
        } else {
            output.extend(
                wrapped
                    .into_iter()
                    .map(|segment| MessageLineView::new(segment, role)),
            );
        }
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
        let wrapped = wrap_text_hard(&line.text, width);
        if wrapped.is_empty() {
            out.push(MessageLineView::new(String::new(), line.role));
        } else {
            out.extend(
                wrapped
                    .into_iter()
                    .map(|segment| MessageLineView::new(segment, line.role)),
            );
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
    let normalized = strip_ansi(text).replace('\n', " ");
    let source = normalized.trim();
    if source.is_empty() {
        return vec![String::new()];
    }
    let width = max_width.max(1);
    let mut lines = wrap_text_hard(source, width);
    let was_truncated = lines.len() > max_lines;
    if summarize && was_truncated {
        lines.truncate(max_lines);
    }
    if was_truncated {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            *last = append_ellipsis(last, width);
        }
    }
    lines
}

fn role_prefix(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User | MessageRole::Assistant | MessageRole::Tool => "",
        MessageRole::System => "system> ",
        MessageRole::Progress => "progress> ",
        MessageRole::Error => "error> ",
    }
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
    use wonder_of_u_core::{MessagePayload, SessionId};

    use super::*;

    fn use_id(value: &str) -> ToolUseId {
        ToolUseId::parse(value).expect("valid tool use id")
    }

    #[test]
    fn markdown_summary_renders_text_and_code_fallbacks() {
        let view = MarkdownSummaryView::new(
            MessageRole::Assistant,
            "# Heading\n- first item\n```rust\nfn main() {}\nprintln!(\"hi\");\n```",
        );

        assert_eq!(
            view.display_lines(80),
            vec![
                MessageLineView::new("Heading • first item", MessageRole::Assistant,),
                MessageLineView::new("code[rust]> fn main() {}", MessageRole::Assistant),
                MessageLineView::new("            +1 more line", MessageRole::Assistant),
            ]
        );
    }

    #[test]
    fn markdown_summary_keeps_user_text_flush_left_without_prefix() {
        let view = MarkdownSummaryView::new(MessageRole::User, "review the diff and continue");

        assert_eq!(
            view.display_lines(80),
            vec![MessageLineView::new(
                "review the diff and continue",
                MessageRole::User,
            )]
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
                MessageLineView::new("  step one step two", MessageRole::Progress),
            ]
        );
    }

    #[test]
    fn rich_message_views_group_tool_uses_and_results() {
        let session_id = SessionId::new();
        let messages = vec![
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

        assert_eq!(
            rich_message_views(&messages),
            vec![RichMessageView::ToolGroup(GroupedToolCallView {
                tool: "bash".into(),
                calls: vec![
                    ToolCallView {
                        use_id: use_id("00000000-0000-0000-0000-000000000001"),
                        input: Some(
                            serde_json::json!({ "command": "cargo test -p wonder-of-u-tui" })
                        ),
                        result: Some(RejectedToolMessageView {
                            kind: RejectedToolMessageKind::Success,
                            status: ToolResultStatus::Success,
                            detail: "tests passed".into(),
                        }),
                    },
                    ToolCallView {
                        use_id: use_id("00000000-0000-0000-0000-000000000002"),
                        input: Some(serde_json::json!({ "command": "rm -rf /tmp/build" })),
                        result: Some(RejectedToolMessageView {
                            kind: RejectedToolMessageKind::Rejected,
                            status: ToolResultStatus::Rejected,
                            detail: "Tool use rejected by the user".into(),
                        }),
                    },
                ],
            })]
        );

        let lines = rich_message_views(&messages)
            .into_iter()
            .flat_map(|view| view.display_lines(120))
            .collect::<Vec<_>>();
        assert_eq!(
            lines,
            vec![
                MessageLineView::new("● Run(Tests)", MessageRole::Tool,),
                MessageLineView::new("  └ cargo test -p wonder-of-u-tui", MessageRole::System,),
                MessageLineView::new("  └ tests passed", MessageRole::System),
                MessageLineView::new("", MessageRole::System),
                MessageLineView::new("● Bash(rm -rf /tmp/build)", MessageRole::Error,),
                MessageLineView::new("  └ rm -rf /tmp/build", MessageRole::System,),
                MessageLineView::new("  └ Tool use rejected by the user", MessageRole::Error,),
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
        };
        assert_eq!(
            read.display_lines("file_read", 120),
            vec![
                MessageLineView::new("● Read(lib.rs)", MessageRole::Tool),
                MessageLineView::new("  └ src/lib.rs", MessageRole::System),
                MessageLineView::new("  └ pub fn demo() {", MessageRole::System),
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
        };
        assert_eq!(
            write.display_lines("file_write", 120),
            vec![
                MessageLineView::new("● Edit(lib.rs)", MessageRole::Tool),
                MessageLineView::new("  └ src/lib.rs", MessageRole::System),
                MessageLineView::new("  └ pub fn demo() {", MessageRole::System),
                MessageLineView::new("        println!(\"updated\");", MessageRole::System),
                MessageLineView::new("    }", MessageRole::System),
                MessageLineView::new("  └ wrote src/lib.rs", MessageRole::System),
            ]
        );
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
                MessageLineView::new("  └ +12 -1 ~4", MessageRole::System),
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
        let lines = super::super::message_lines(&messages);
        assert!(
            lines.iter().all(|l| !l.text.starts_with("user>")),
            "UserText must not render with 'user>' prefix; lines: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.text.contains("hi there")),
            "UserText content must appear in transcript; lines: {lines:?}"
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
        let lines = super::super::message_lines(&messages);
        assert!(
            lines.iter().all(|l| !l.text.starts_with("assistant>")),
            "AssistantText must not render with 'assistant>' prefix; lines: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.text.contains("Hello!")),
            "AssistantText content must appear in transcript; lines: {lines:?}"
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
        let lines = super::super::message_lines(&messages);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert!(
            texts.iter().any(|t| t.starts_with('●')),
            "Tool headline must start with ● bullet; lines: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains('└')),
            "Tool result must contain └ subordinate marker; lines: {texts:?}"
        );
        assert!(
            texts.iter().all(|t| !t.starts_with("tool[")),
            "Old tool[name] format must not appear; lines: {texts:?}"
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
        let lines = super::super::message_lines(&messages);
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
    }
}
