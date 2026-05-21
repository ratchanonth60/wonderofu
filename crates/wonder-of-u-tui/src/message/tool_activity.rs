use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use wonder_of_u_core::TaskStatus;

use crate::{
    catalog::{
        CatalogAction, CatalogEntry, CatalogHeader, CatalogModel, CatalogSection, DetailPreview,
        EmptyState, PanelDescriptor, StatusBadge,
    },
    diff::{FileEditHunkSummary, HighlightedCodeView, PathLinkView, StructuredDiffView},
    measure::{line_width, strip_ansi, wrap_text_hard},
    permission::{PermissionAccessKind, PermissionDetailView, PermissionSummaryView},
};

use super::{
    FileEditReferenceView, GroupedToolCallView, MessageLineView, MessageRole,
    RejectedToolMessageView, ToolResultStatus,
};

const MAX_ACTIVITY_DETAIL_LINES: usize = 3;
const MAX_ACTIVITY_PREVIEW_LINES: usize = 3;

/// Counts grouped tool-call outcomes by terminal transcript status.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ToolResultCounts {
    /// Stores the pending
    pub pending: usize,
    /// Stores the success
    pub success: usize,
    /// Stores the error
    pub error: usize,
    /// Stores the rejected
    pub rejected: usize,
    /// Stores the cancelled
    pub cancelled: usize,
}

impl ToolResultCounts {
    fn push(&mut self, status: ToolResultStatus) {
        match status {
            ToolResultStatus::Pending => self.pending += 1,
            ToolResultStatus::Success => self.success += 1,
            ToolResultStatus::Error => self.error += 1,
            ToolResultStatus::Rejected => self.rejected += 1,
            ToolResultStatus::Cancelled => self.cancelled += 1,
        }
    }
    /// Handles total
    #[must_use]
    pub fn total(self) -> usize {
        self.pending + self.success + self.error + self.rejected + self.cancelled
    }
    /// Handles summary text
    #[must_use]
    pub fn summary_text(self) -> String {
        let mut parts = vec![format!(
            "{} {}",
            self.total(),
            if self.total() == 1 { "call" } else { "calls" }
        )];
        if self.success > 0 {
            parts.push(format!("{} ok", self.success));
        }
        if self.rejected > 0 {
            parts.push(format!("{} rejected", self.rejected));
        }
        if self.cancelled > 0 {
            parts.push(format!("{} cancelled", self.cancelled));
        }
        if self.error > 0 {
            parts.push(format!("{} error", self.error));
        }
        if self.pending > 0 {
            parts.push(format!("{} pending", self.pending));
        }
        parts.join(" · ")
    }
}

impl GroupedToolCallView {
    /// Handles result counts
    #[must_use]
    pub fn result_counts(&self) -> ToolResultCounts {
        let mut counts = ToolResultCounts::default();
        for call in &self.calls {
            counts.push(
                call.result
                    .as_ref()
                    .map_or(ToolResultStatus::Pending, |result| result.status),
            );
        }
        counts
    }
    /// Handles summary text
    #[must_use]
    pub fn summary_text(&self) -> String {
        self.result_counts().summary_text()
    }
}

impl FileEditReferenceView {
    /// Builds a file-edit reference from a structured diff by aggregating all hunk totals.
    #[must_use]
    pub fn from_diff(path: PathLinkView, diff: &StructuredDiffView) -> Self {
        let summary = diff
            .hunks
            .iter()
            .fold(FileEditHunkSummary::default(), |mut total, hunk| {
                let summary = hunk.summary();
                total.additions += summary.additions;
                total.removals += summary.removals;
                total.context += summary.context;
                total
            });

        Self::new(path, summary)
    }
}

/// Describes how a notebook cell edit was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotebookEditMode {
    /// Represents replace
    Replace,
    /// Represents insert
    Insert,
    /// Represents delete
    Delete,
}

impl NotebookEditMode {
    fn summary_label(self) -> &'static str {
        match self {
            Self::Replace => "replace cell in",
            Self::Insert => "insert cell in",
            Self::Delete => "delete cell in",
        }
    }
}

/// Summarizes a rejected notebook edit without tying the model to a renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotebookRejectionSummaryView {
    /// Stores the notebook
    pub notebook: PathLinkView,
    /// Stores the cell identifier
    pub cell_id: Option<String>,
    /// Stores the cell type
    pub cell_type: Option<String>,
    /// Stores the edit mode
    pub edit_mode: NotebookEditMode,
    /// Stores the preview
    pub preview: Option<HighlightedCodeView>,
}

impl NotebookRejectionSummaryView {
    /// Creates a new value
    #[must_use]
    pub fn new(notebook: PathLinkView, edit_mode: NotebookEditMode) -> Self {
        Self {
            notebook,
            cell_id: None,
            cell_type: None,
            edit_mode,
            preview: None,
        }
    }
    /// Handles with cell id
    #[must_use]
    pub fn with_cell_id(mut self, cell_id: impl Into<String>) -> Self {
        self.cell_id = Some(cell_id.into());
        self
    }
    /// Handles with cell type
    #[must_use]
    pub fn with_cell_type(mut self, cell_type: impl Into<String>) -> Self {
        self.cell_type = Some(cell_type.into());
        self
    }
    /// Handles with preview
    #[must_use]
    pub fn with_preview(mut self, code: impl Into<String>) -> Self {
        let language = match self.cell_type.as_deref() {
            Some("markdown") => Some("markdown".to_string()),
            Some("code") => Some("python".to_string()),
            _ => None,
        };
        let preview_path = match self.cell_type.as_deref() {
            Some("markdown") => Some(PathLinkView::new("cell.md")),
            Some("code") => Some(PathLinkView::new("cell.py")),
            _ => None,
        };

        self.preview = Some(HighlightedCodeView {
            path: preview_path,
            language,
            code: code.into(),
            first_line_number: 1,
            truncated: false,
        });
        self
    }
    /// Handles display lines
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        let mut header = format!(
            "notebook> rejected {} {}",
            self.edit_mode.summary_label(),
            self.notebook.display_text(max_width).text
        );
        if let Some(cell_id) = self.cell_id.as_deref() {
            header.push_str(" · cell ");
            header.push_str(cell_id);
        }
        if let Some(cell_type) = self.cell_type.as_deref() {
            header.push_str(" · ");
            header.push_str(cell_type);
        }

        let mut lines = push_line(header, MessageRole::Error, max_width);
        if let Some(preview) = &self.preview {
            let source = preview
                .code
                .lines()
                .take(MAX_ACTIVITY_PREVIEW_LINES + 1)
                .map(str::to_string)
                .collect::<Vec<_>>();
            let overflow = source.len() > MAX_ACTIVITY_PREVIEW_LINES;
            let mut preview_lines = source
                .into_iter()
                .take(MAX_ACTIVITY_PREVIEW_LINES)
                .map(|line| truncate_visible_end(&line, max_width.saturating_sub(2).max(1)))
                .collect::<Vec<_>>();
            if overflow {
                preview_lines.push("…".into());
            }
            push_wrapped_block(
                &mut lines,
                "",
                &preview_lines,
                MessageRole::System,
                max_width,
            );
        }
        lines
    }
}

/// The type of MCP list/detail surface represented by a catalog summary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpCatalogKind {
    /// Represents resources
    Resources,
    /// Represents tools
    Tools,
}

impl McpCatalogKind {
    fn singular(self) -> &'static str {
        match self {
            Self::Resources => "resource",
            Self::Tools => "tool",
        }
    }

    fn plural(self) -> &'static str {
        match self {
            Self::Resources => "resources",
            Self::Tools => "tools",
        }
    }
}

/// A single MCP resource or tool entry summarized for list/detail rendering.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct McpCatalogItemView {
    /// Stores the id
    pub id: String,
    /// Stores the label
    pub label: String,
    /// Stores the description
    pub description: Option<String>,
    /// Stores the badges
    pub badges: Vec<StatusBadge>,
    /// Stores the detail lines
    pub detail_lines: Vec<String>,
    /// Stores the keywords
    pub keywords: Vec<String>,
    /// Stores the disabled reason
    pub disabled_reason: Option<String>,
    /// Stores the actions
    pub actions: Vec<CatalogAction>,
}

impl McpCatalogItemView {
    /// Creates a new value
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            badges: Vec::new(),
            detail_lines: Vec::new(),
            keywords: Vec::new(),
            disabled_reason: None,
            actions: Vec::new(),
        }
    }
    /// Handles with description
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
    /// Handles with badge
    #[must_use]
    pub fn with_badge(mut self, badge: StatusBadge) -> Self {
        self.badges.push(badge);
        self
    }
    /// Handles with detail line
    #[must_use]
    pub fn with_detail_line(mut self, line: impl Into<String>) -> Self {
        self.detail_lines.push(line.into());
        self
    }
    /// Handles with keyword
    #[must_use]
    pub fn with_keyword(mut self, keyword: impl Into<String>) -> Self {
        self.keywords.push(keyword.into());
        self
    }
    /// Handles disabled
    #[must_use]
    pub fn disabled(mut self, reason: impl Into<String>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }

    fn into_catalog_entry(self) -> CatalogEntry {
        let panel = PanelDescriptor::new(self.label.clone())
            .with_subtitle(self.description.clone().unwrap_or_default());
        let mut entry = CatalogEntry::new(self.id, self.label);
        if let Some(description) = self.description {
            entry = entry.with_description(description);
        }
        for badge in self.badges.iter().cloned() {
            entry = entry.with_badge(badge);
        }
        for keyword in self.keywords {
            entry = entry.with_keyword(keyword);
        }
        for action in self.actions {
            entry.actions.push(action);
        }
        if !self.detail_lines.is_empty() || !self.badges.is_empty() {
            let mut preview = DetailPreview::new(panel);
            preview.badges = self.badges;
            preview.lines = self.detail_lines;
            entry = entry.with_preview(preview);
        }
        if let Some(reason) = self.disabled_reason {
            entry = entry.disabled(reason);
        }
        entry
    }
}

/// A catalog-backed MCP list/detail summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpCatalogSummaryView {
    /// Stores the server name
    pub server_name: String,
    /// Stores the kind
    pub kind: McpCatalogKind,
    /// Stores the catalog
    pub catalog: CatalogModel,
}

impl McpCatalogSummaryView {
    /// Handles resources
    #[must_use]
    pub fn resources(server_name: impl Into<String>, entries: Vec<McpCatalogItemView>) -> Self {
        Self::new(server_name.into(), McpCatalogKind::Resources, entries)
    }
    /// Handles tools
    #[must_use]
    pub fn tools(server_name: impl Into<String>, entries: Vec<McpCatalogItemView>) -> Self {
        Self::new(server_name.into(), McpCatalogKind::Tools, entries)
    }

    fn new(server_name: String, kind: McpCatalogKind, entries: Vec<McpCatalogItemView>) -> Self {
        let plural = kind.plural();
        let singular = kind.singular();
        let catalog = CatalogModel::new(
            CatalogHeader::new(format!("MCP {plural}"))
                .with_subtitle(format!("Server: {server_name}")),
            PanelDescriptor::new(plural.to_string()).with_subtitle("Visible entries"),
            PanelDescriptor::new(format!("{singular} details")).with_subtitle("Current selection"),
            EmptyState::new(format!("No MCP {plural}"))
                .with_body_line(format!("The server did not expose any {plural}.")),
            EmptyState::new(format!("No matching {plural}"))
                .with_body_line("Try a different filter."),
            vec![CatalogSection::new(
                PanelDescriptor::new(plural.to_string()),
                entries
                    .into_iter()
                    .map(McpCatalogItemView::into_catalog_entry)
                    .collect(),
            )],
        );

        Self {
            server_name,
            kind,
            catalog,
        }
    }
    /// Handles display lines
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        let count = self.catalog.visible_count();
        let mut lines = push_line(
            format!(
                "mcp[{}]> {} {}",
                self.server_name,
                count,
                if count == 1 {
                    self.kind.singular()
                } else {
                    self.kind.plural()
                }
            ),
            MessageRole::System,
            max_width,
        );

        if let Some(selection) = self.catalog.selection() {
            push_wrapped_block(
                &mut lines,
                "",
                &[format!("• {}", selection.entry.label)],
                MessageRole::Tool,
                max_width,
            );
            if let Some(preview) = selection.entry.preview.as_ref() {
                let detail_lines = preview
                    .lines
                    .iter()
                    .take(MAX_ACTIVITY_DETAIL_LINES)
                    .cloned()
                    .collect::<Vec<_>>();
                push_wrapped_block(
                    &mut lines,
                    "",
                    &detail_lines,
                    MessageRole::System,
                    max_width,
                );
            }
        }

        lines
    }
}

/// The task action represented by a task activity summary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskActivityKind {
    /// Represents create
    Create,
    /// Represents output
    Output,
    /// Represents stop
    Stop,
}

impl TaskActivityKind {
    fn label(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Output => "output",
            Self::Stop => "stop",
        }
    }
}

/// Summarizes task tool create/output/stop activity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskActivitySummaryView {
    /// Stores the kind
    pub kind: TaskActivityKind,
    /// Stores the task identifier
    pub task_id: String,
    /// Stores the status
    pub status: TaskStatus,
    /// Stores the summary
    pub summary: String,
    /// Stores the detail
    pub detail: Option<String>,
}

impl TaskActivitySummaryView {
    /// Handles created
    #[must_use]
    pub fn created(task_id: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            kind: TaskActivityKind::Create,
            task_id: task_id.into(),
            status: TaskStatus::Completed,
            summary: summary.into(),
            detail: None,
        }
    }
    /// Handles output
    #[must_use]
    pub fn output(
        task_id: impl Into<String>,
        status: TaskStatus,
        summary: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind: TaskActivityKind::Output,
            task_id: task_id.into(),
            status,
            summary: summary.into(),
            detail: Some(detail.into()),
        }
    }
    /// Handles stopped
    #[must_use]
    pub fn stopped(
        task_id: impl Into<String>,
        status: TaskStatus,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            kind: TaskActivityKind::Stop,
            task_id: task_id.into(),
            status,
            summary: summary.into(),
            detail: None,
        }
    }
    /// Handles display lines
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        let mut lines = push_line(
            format!(
                "task[{}]> #{} {} · {}",
                self.kind.label(),
                self.task_id,
                task_status_label(self.status),
                self.summary
            ),
            role_for_task_status(self.status),
            max_width,
        );
        if let Some(detail) = self.detail.as_deref() {
            let wrapped = wrap_summary_lines(detail, max_width.saturating_sub(2).max(1), true);
            push_wrapped_block(&mut lines, "", &wrapped, MessageRole::System, max_width);
        }
        lines
    }
}

/// A fallback surface for unknown or unsupported tool output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownToolOutputView {
    /// Stores the tool
    pub tool: String,
    /// Stores the status
    pub status: ToolResultStatus,
    /// Stores the headline
    pub headline: String,
    /// Stores the detail lines
    pub detail_lines: Vec<String>,
}

impl UnknownToolOutputView {
    /// Handles from result
    #[must_use]
    pub fn from_result(tool: impl Into<String>, content: impl AsRef<str>, success: bool) -> Self {
        let tool = tool.into();
        let content = sanitize_tool_output(content.as_ref());
        let result = RejectedToolMessageView::from_result(content.clone(), success);
        let headline = match result.status {
            ToolResultStatus::Success => first_non_empty_line(&content)
                .unwrap_or("Tool completed")
                .into(),
            ToolResultStatus::Rejected => "Tool use rejected".into(),
            ToolResultStatus::Cancelled => "Interrupted by user".into(),
            ToolResultStatus::Error | ToolResultStatus::Pending => {
                fallback_error_headline(&content)
            }
        };
        let detail_lines = content
            .lines()
            .skip_while(|line| line.trim().is_empty())
            .skip(1)
            .take(MAX_ACTIVITY_DETAIL_LINES)
            .map(str::to_string)
            .collect();

        Self {
            tool,
            status: result.status,
            headline,
            detail_lines,
        }
    }
    /// Handles display lines
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        let mut lines = push_line(
            format!(
                "tool[{}] {}> {}",
                self.tool,
                tool_status_label(self.status),
                self.headline
            ),
            role_for_tool_status(self.status),
            max_width,
        );
        if !self.detail_lines.is_empty() {
            push_wrapped_block(
                &mut lines,
                "",
                &self.detail_lines,
                MessageRole::System,
                max_width,
            );
        }
        lines
    }
}

/// Summarizes a permission request that was explicitly denied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectedPermissionSummaryView {
    /// Stores the tool
    pub tool: String,
    /// Stores the title
    pub title: String,
    /// Stores the access
    pub access: Option<PermissionAccessKind>,
    /// Stores the details
    pub details: Vec<PermissionDetailView>,
    /// Stores the reason
    pub reason: String,
}

impl RejectedPermissionSummaryView {
    /// Handles from summary
    #[must_use]
    pub fn from_summary(
        tool: impl Into<String>,
        summary: &PermissionSummaryView,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            tool: tool.into(),
            title: summary.title.clone(),
            access: Some(summary.access),
            details: summary.details.clone(),
            reason: reason.into(),
        }
    }
    /// Handles from decision
    #[must_use]
    pub fn from_decision(tool: impl Into<String>, reason: impl Into<String>) -> Self {
        let tool = tool.into();
        Self {
            title: format!("Use tool `{tool}`"),
            tool,
            access: None,
            details: Vec::new(),
            reason: reason.into(),
        }
    }
    /// Handles display lines
    #[must_use]
    pub fn display_lines(&self, max_width: usize) -> Vec<MessageLineView> {
        let mut header = format!("permission[{}]> denied", self.tool);
        if let Some(access) = self.access {
            header.push_str(" · ");
            header.push_str(access.label());
        }

        let mut lines = push_line(header, MessageRole::Error, max_width);
        let mut detail_lines = vec![self.title.clone(), self.reason.clone()];
        detail_lines.extend(
            self.details
                .iter()
                .map(|detail| format!("{}: {}", detail.label, detail.value)),
        );
        push_wrapped_block(
            &mut lines,
            "",
            &detail_lines,
            MessageRole::System,
            max_width,
        );
        lines
    }
}

fn sanitize_tool_output(content: &str) -> String {
    let stripped = strip_ansi(content).into_owned();
    strip_xmlish_tags(&stripped).trim().to_string()
}

fn fallback_error_headline(content: &str) -> String {
    if content.contains("InputValidationError:") {
        return "Invalid tool parameters".into();
    }

    let first = first_non_empty_line(content).unwrap_or("Tool execution failed");
    if first.starts_with("Error: ") || first.starts_with("Cancelled: ") {
        first.to_string()
    } else {
        format!("Error: {first}")
    }
}

fn strip_xmlish_tags(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    output
}

fn first_non_empty_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

fn role_for_tool_status(status: ToolResultStatus) -> MessageRole {
    match status {
        ToolResultStatus::Success | ToolResultStatus::Pending => MessageRole::Tool,
        ToolResultStatus::Error | ToolResultStatus::Rejected | ToolResultStatus::Cancelled => {
            MessageRole::Error
        }
    }
}

fn tool_status_label(status: ToolResultStatus) -> &'static str {
    match status {
        ToolResultStatus::Pending => "pending",
        ToolResultStatus::Success => "ok",
        ToolResultStatus::Error => "error",
        ToolResultStatus::Rejected => "rejected",
        ToolResultStatus::Cancelled => "cancelled",
    }
}

fn role_for_task_status(status: TaskStatus) -> MessageRole {
    match status {
        TaskStatus::Pending | TaskStatus::Running => MessageRole::Progress,
        TaskStatus::Completed => MessageRole::Assistant,
        TaskStatus::Failed | TaskStatus::Killed | TaskStatus::Cancelled => MessageRole::Error,
    }
}

fn task_status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::Running => "running",
        TaskStatus::Completed => "done",
        TaskStatus::Failed => "failed",
        TaskStatus::Killed => "killed",
        TaskStatus::Cancelled => "cancelled",
    }
}

fn push_line(text: impl Into<String>, role: MessageRole, max_width: usize) -> Vec<MessageLineView> {
    vec![MessageLineView::new(
        truncate_visible_end(&text.into(), max_width.max(1)),
        role,
    )]
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

fn wrap_summary_lines(text: &str, max_width: usize, summarize: bool) -> Vec<String> {
    let normalized = strip_ansi(text).replace('\n', " ");
    let source = normalized.trim();
    if source.is_empty() {
        return vec![String::new()];
    }

    let width = max_width.max(1);
    let mut lines = wrap_text_hard(source, width);
    let was_truncated = summarize && lines.len() > MAX_ACTIVITY_DETAIL_LINES;
    if was_truncated {
        lines.truncate(MAX_ACTIVITY_DETAIL_LINES);
        if let Some(last) = lines.last_mut() {
            *last = append_ellipsis(last, width);
        }
    }
    lines
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
    use serde_json::json;
    use wonder_of_u_core::{PermissionRequest, ToolUseId};

    use super::*;
    use crate::catalog::CatalogTone;
    use crate::message::ToolCallView;

    fn use_id(value: &str) -> ToolUseId {
        ToolUseId::parse(value).expect("valid tool use id")
    }

    #[test]
    fn grouped_tool_summary_counts_each_result_kind() {
        let view = GroupedToolCallView {
            tool: "bash".into(),
            calls: vec![
                ToolCallView {
                    use_id: use_id("00000000-0000-0000-0000-000000000001"),
                    input: Some(json!({ "command": "cargo test" })),
                    result: Some(RejectedToolMessageView::from_result("done", true)),
                    elapsed_secs: None,
                },
                ToolCallView {
                    use_id: use_id("00000000-0000-0000-0000-000000000002"),
                    input: Some(json!({ "command": "rm -rf /workspace" })),
                    result: Some(RejectedToolMessageView::from_result(
                        "Tool use rejected by the user",
                        false,
                    )),
                    elapsed_secs: None,
                },
                ToolCallView {
                    use_id: use_id("00000000-0000-0000-0000-000000000003"),
                    input: Some(json!({ "command": "sleep 10" })),
                    result: None,
                    elapsed_secs: None,
                },
            ],
        };

        assert_eq!(
            view.result_counts(),
            ToolResultCounts {
                pending: 1,
                success: 1,
                error: 0,
                rejected: 1,
                cancelled: 0,
            }
        );
        assert_eq!(
            view.summary_text(),
            "3 calls · 1 ok · 1 rejected · 1 pending"
        );
    }

    #[test]
    fn file_edit_reference_can_be_built_from_structured_diff() {
        let diff = StructuredDiffView {
            path: Some(PathLinkView::new("src/lib.rs")),
            hunks: vec![
                crate::diff::StructuredDiffHunk {
                    header: "@@ -1,3 +1,4 @@".into(),
                    lines: vec![
                        crate::diff::StructuredDiffLine::context(Some(1), Some(1), "mod a;"),
                        crate::diff::StructuredDiffLine::added(Some(2), "mod b;"),
                    ],
                },
                crate::diff::StructuredDiffHunk {
                    header: "@@ -8,2 +9,3 @@".into(),
                    lines: vec![
                        crate::diff::StructuredDiffLine::removed(Some(8), "old();"),
                        crate::diff::StructuredDiffLine::added(Some(9), "new();"),
                        crate::diff::StructuredDiffLine::context(Some(10), Some(10), "}"),
                    ],
                },
            ],
            ..StructuredDiffView::default()
        };

        let view = FileEditReferenceView::from_diff(PathLinkView::new("src/lib.rs"), &diff);

        assert_eq!(
            view.summary,
            FileEditHunkSummary {
                additions: 2,
                removals: 1,
                context: 2,
            }
        );
    }

    #[test]
    fn notebook_rejection_includes_cell_and_preview() {
        let view = NotebookRejectionSummaryView::new(
            PathLinkView::new("notebooks/demo.ipynb"),
            NotebookEditMode::Replace,
        )
        .with_cell_id("cell-7")
        .with_cell_type("markdown")
        .with_preview("# Heading\nbody\nmore");

        assert_eq!(
            view.display_lines(80),
            vec![
                MessageLineView::new(
                    "notebook> rejected replace cell in notebooks/demo.ipynb · cell cell-7 · markdown",
                    MessageRole::Error,
                ),
                MessageLineView::new("  # Heading", MessageRole::System),
                MessageLineView::new("  body", MessageRole::System),
                MessageLineView::new("  more", MessageRole::System),
            ]
        );
    }

    #[test]
    fn mcp_catalog_builds_list_and_detail_views() {
        let summary = McpCatalogSummaryView::resources(
            "filesystem",
            vec![
                McpCatalogItemView::new("readme", "README")
                    .with_description("Workspace readme")
                    .with_badge(StatusBadge::new("text", CatalogTone::Muted))
                    .with_detail_line("uri: file:///workspace/README.md")
                    .with_detail_line("mime: text/markdown"),
                McpCatalogItemView::new("logs", "Logs")
                    .with_badge(StatusBadge::connected())
                    .with_detail_line("uri: file:///workspace/logs/"),
            ],
        );

        assert_eq!(summary.catalog.header.title, "MCP resources");
        assert_eq!(
            summary.catalog.header.subtitle.as_deref(),
            Some("Server: filesystem")
        );
        assert_eq!(
            summary
                .catalog
                .selected_entry()
                .map(|entry| entry.label.as_str()),
            Some("README")
        );
        assert_eq!(
            summary
                .catalog
                .detail_preview()
                .expect("detail preview")
                .lines,
            vec![
                "uri: file:///workspace/README.md".to_string(),
                "mime: text/markdown".to_string(),
            ]
        );
        assert_eq!(
            summary.display_lines(80),
            vec![
                MessageLineView::new("mcp[filesystem]> 2 resources", MessageRole::System),
                MessageLineView::new("  • README", MessageRole::Tool),
                MessageLineView::new("  uri: file:///workspace/README.md", MessageRole::System),
                MessageLineView::new("  mime: text/markdown", MessageRole::System),
            ]
        );
    }

    #[test]
    fn task_activity_summaries_cover_create_output_and_stop() {
        let created = TaskActivitySummaryView::created("42", "Index workspace");
        assert_eq!(
            created.display_lines(80),
            vec![MessageLineView::new(
                "task[create]> #42 done · Index workspace",
                MessageRole::Assistant,
            )]
        );

        let output = TaskActivitySummaryView::output(
            "42",
            TaskStatus::Running,
            "Streaming logs",
            "line one\nline two\nline three\nline four",
        );
        assert_eq!(
            output.display_lines(80),
            vec![
                MessageLineView::new(
                    "task[output]> #42 running · Streaming logs",
                    MessageRole::Progress,
                ),
                MessageLineView::new(
                    "  line one line two line three line four",
                    MessageRole::System,
                ),
            ]
        );

        let stopped =
            TaskActivitySummaryView::stopped("42", TaskStatus::Cancelled, "Stopped by user");
        assert_eq!(
            stopped.display_lines(80),
            vec![MessageLineView::new(
                "task[stop]> #42 cancelled · Stopped by user",
                MessageRole::Error,
            )]
        );
    }

    #[test]
    fn unknown_tool_output_falls_back_to_sanitized_error_summary() {
        let view = UnknownToolOutputView::from_result(
            "demo_tool",
            "<tool_use_error>InputValidationError: missing field</tool_use_error>\ntrace line",
            false,
        );

        assert_eq!(view.status, ToolResultStatus::Error);
        assert_eq!(view.headline, "Invalid tool parameters");
        assert_eq!(
            view.display_lines(80),
            vec![
                MessageLineView::new(
                    "tool[demo_tool] error> Invalid tool parameters",
                    MessageRole::Error,
                ),
                MessageLineView::new("  trace line", MessageRole::System),
            ]
        );
    }

    #[test]
    fn rejected_permission_summary_reuses_permission_details() {
        let summary = PermissionSummaryView::from_request(
            &PermissionRequest::new("file_edit").destructive(true),
            &json!({ "path": "/workspace/src/main.rs" }),
        );

        let rejected = RejectedPermissionSummaryView::from_summary(
            "file_edit",
            &summary,
            "User denied the request",
        );

        assert_eq!(rejected.access, Some(PermissionAccessKind::Destructive));
        assert_eq!(
            rejected.details,
            vec![PermissionDetailView::new("Path", "/workspace/src/main.rs")]
        );
        assert_eq!(
            rejected.display_lines(120),
            vec![
                MessageLineView::new(
                    "permission[file_edit]> denied · destructive",
                    MessageRole::Error,
                ),
                MessageLineView::new("  Edit file", MessageRole::System),
                MessageLineView::new("  User denied the request", MessageRole::System),
                MessageLineView::new("  Path: /workspace/src/main.rs", MessageRole::System),
            ]
        );
    }
}
