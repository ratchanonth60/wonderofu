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
    Markdown(MarkdownSummaryView),
    Thinking(ThinkingBlockView),
    ToolGroup(GroupedToolCallView),
    FileEditReference(FileEditReferenceView),
    Attachment(AttachmentSummaryView),
    SystemError(SystemErrorView),
    Boundary(TranscriptBoundaryView),
    Fallback(Vec<MessageLineView>),
}

impl RichMessageView {
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
    pub role: MessageRole,
    pub blocks: Vec<MarkdownBlockView>,
}

impl MarkdownSummaryView {
    #[must_use]
    pub fn new(role: MessageRole, text: &str) -> Self {
        Self {
            role,
            blocks: parse_markdown_blocks(text),
        }
    }

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
                    push_wrapped_block(
                        &mut lines,
                        display_prefix,
                        &block_lines,
                        self.role,
                        max_width,
                    );
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
    Paragraph(String),
    Code(MarkdownCodeBlockView),
}

/// A summarized fenced code block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownCodeBlockView {
    pub language: Option<String>,
    pub code: String,
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
    pub content: String,
    pub collapsed: bool,
}

impl ThinkingBlockView {
    #[must_use]
    pub fn new(content: impl Into<String>, collapsed: bool) -> Self {
        Self {
            content: content.into(),
            collapsed,
        }
    }

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
    pub tool: String,
    pub calls: Vec<ToolCallView>,
}

impl GroupedToolCallView {
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

    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        let count = self.calls.len();
        let noun = if count == 1 { "call" } else { "calls" };
        let mut lines = push_line(
            format!("tools[{}]> {count} {noun}", self.tool),
            MessageRole::Tool,
            max_width,
        );
        for call in &self.calls {
            push_wrapped_block(&mut lines, "", &[call.label()], call.role(), max_width);
        }
        lines
    }
}

/// A summarized tool invocation and optional result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCallView {
    pub use_id: ToolUseId,
    pub input: Option<Value>,
    pub result: Option<RejectedToolMessageView>,
}

impl ToolCallView {
    fn result_for(success: bool, content: String) -> RejectedToolMessageView {
        RejectedToolMessageView::from_result(content, success)
    }

    fn label(&self) -> String {
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

    fn role(&self) -> MessageRole {
        match self.result.as_ref().map(|result| result.status) {
            Some(
                ToolResultStatus::Error | ToolResultStatus::Rejected | ToolResultStatus::Cancelled,
            ) => MessageRole::Error,
            _ => MessageRole::Tool,
        }
    }
}

/// Summarizes a file edit reference using diff summaries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEditReferenceView {
    pub path: PathLinkView,
    pub summary: FileEditHunkSummary,
    pub note: Option<String>,
}

impl FileEditReferenceView {
    #[must_use]
    pub fn new(path: PathLinkView, summary: FileEditHunkSummary) -> Self {
        Self {
            path,
            summary,
            note: None,
        }
    }

    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        let path = self.path.display_text(max_width).text;
        let mut lines = push_line(
            format!("edit> {path} ({})", self.summary.label()),
            MessageRole::Tool,
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
    pub label: String,
    pub uri: String,
    pub kind: AttachmentKind,
}

impl AttachmentSummaryView {
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
    File,
    Image,
    Pdf,
    Link,
    Directory,
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
    pub role: MessageRole,
    pub kind: SystemErrorKind,
    pub detail: String,
}

impl SystemErrorView {
    #[must_use]
    pub fn detect(role: MessageRole, text: &str) -> Option<Self> {
        let kind = SystemErrorKind::detect(text)?;
        Some(Self {
            role,
            kind,
            detail: text.trim().to_string(),
        })
    }

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
    RateLimit,
    Timeout,
    Authentication,
    Api,
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
    pub summary: String,
}

impl TranscriptBoundaryView {
    #[must_use]
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
        }
    }

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
    pub kind: RejectedToolMessageKind,
    pub status: ToolResultStatus,
    pub detail: String,
}

impl RejectedToolMessageView {
    #[must_use]
    pub fn from_result(detail: impl Into<String>, success: bool) -> Self {
        let detail = detail.into();
        if success {
            return Self {
                kind: RejectedToolMessageKind::Success,
                status: ToolResultStatus::Success,
                detail: summarize_detail(&detail),
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
            detail: summarize_detail(&detail),
        }
    }
}

/// Tool result classification used by grouped tool summaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolResultStatus {
    Pending,
    Success,
    Error,
    Rejected,
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
    Success,
    Rejected,
    PlanRejected,
    Cancelled,
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

fn push_line(text: impl Into<String>, role: MessageRole, max_width: usize) -> Vec<MessageLineView> {
    vec![MessageLineView::new(
        truncate_visible_end(&text.into(), max_width.max(1)),
        role,
    )]
}

fn truncate_existing_lines(lines: &[MessageLineView], max_width: usize) -> Vec<MessageLineView> {
    lines
        .iter()
        .map(|line| MessageLineView::new(truncate_visible_end(&line.text, max_width), line.role))
        .collect()
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

fn summarize_detail(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or_default().trim();
    truncate_visible_end(first_line, 36)
}

fn role_prefix(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user> ",
        MessageRole::Assistant => "assistant> ",
        MessageRole::System => "system> ",
        MessageRole::Tool => "tool> ",
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
                MessageLineView::new("assistant> Heading • first item", MessageRole::Assistant,),
                MessageLineView::new("code[rust]> fn main() {}", MessageRole::Assistant),
                MessageLineView::new("            +1 more line", MessageRole::Assistant),
            ]
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
                MessageLineView::new("tools[bash]> 2 calls", MessageRole::Tool),
                MessageLineView::new(
                    "  • #00000000 ok · command=\"cargo test -p wonder-of…\" → tests passed",
                    MessageRole::Tool,
                ),
                MessageLineView::new(
                    "  • #00000000 rejected · command=\"rm -rf /tmp/build\" → Tool use rejected by the user",
                    MessageRole::Error,
                ),
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
                    "edit> crates/wonder-of-u-tui/src/message/mod.rs (+12 -1 ~4)",
                    MessageRole::Tool,
                ),
                MessageLineView::new("  Rich transcript summaries", MessageRole::System),
            ]
        );
    }
}
