//! Provides message support
//!
use wonder_of_u_core::{
    AppState, MessageEnvelope, MessagePayload, TaskKind, TaskState, TaskStatus,
    session_footer_text, session_status_text,
};

use crate::{prompt::PromptQueueView, style::TextStyle};

mod rich;
mod tool_activity;

/// Re-exports items from `rich`
pub use rich::{
    AttachmentKind, AttachmentSummaryView, FileEditReferenceView, GroupedToolCallView,
    MarkdownBlockView, MarkdownCodeBlockView, MarkdownSummaryView, RejectedToolMessageKind,
    RejectedToolMessageView, RichMessageView, SystemErrorKind, SystemErrorView, ThinkingBlockView,
    ToolCallView, ToolResultStatus, TranscriptBoundaryView, highlight_code_block,
    rich_message_views,
};
/// Re-exports items from `tool_activity`
pub use tool_activity::{
    McpCatalogItemView, McpCatalogKind, McpCatalogSummaryView, NotebookEditMode,
    NotebookRejectionSummaryView, RejectedPermissionSummaryView, TaskActivityKind,
    TaskActivitySummaryView, ToolResultCounts, UnknownToolOutputView,
};

const DEFAULT_MESSAGE_SUMMARY_WIDTH: usize = 80;
/// Enumerates message role
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageRole {
    /// Represents user
    User,
    /// Represents assistant
    Assistant,
    /// Represents system
    System,
    /// Represents tool
    Tool,
    /// Represents progress
    Progress,
    /// Represents error
    Error,
}
/// Represents a styled message span.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageSpanView {
    /// Stores the text
    pub text: String,
    /// Stores the optional style override
    pub style: Option<TextStyle>,
}

impl MessageSpanView {
    /// Creates a new value
    #[must_use]
    pub fn new(text: impl Into<String>, style: Option<TextStyle>) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

/// Represents message line view
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageLineView {
    /// Stores the text
    pub text: String,
    /// Stores the role
    pub role: MessageRole,
    /// Stores the optional styled spans
    pub spans: Vec<MessageSpanView>,
}

impl MessageLineView {
    /// Creates a new value
    #[must_use]
    pub fn new(text: impl Into<String>, role: MessageRole) -> Self {
        Self {
            text: text.into(),
            role,
            spans: Vec::new(),
        }
    }

    /// Creates a line with styled spans.
    #[must_use]
    pub fn with_spans(role: MessageRole, spans: Vec<MessageSpanView>) -> Self {
        let text = spans.iter().fold(String::new(), |mut text, span| {
            text.push_str(&span.text);
            text
        });
        Self { text, role, spans }
    }
}
/// Represents task panel view
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskPanelView {
    /// Stores the title
    pub title: String,
    /// Stores the lines
    pub lines: Vec<MessageLineView>,
}

/// Describes the incremental prompt history search overlay.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HistorySearchView {
    /// Stores the query
    pub query: String,
    /// Stores the match text
    pub match_text: Option<String>,
    /// Stores the match index
    pub match_index: usize,
    /// Stores the match total
    pub match_total: usize,
}

/// Describes a single workspace search hit.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchMatch {
    /// Relative file path for the match.
    pub file: String,
    /// One-based line number.
    pub line: u32,
    /// Full matched line text.
    pub text: String,
}

/// Carries the computed preview for the currently highlighted picker option.
///
/// `preview` is `None` when the active filter yields no matches, so the
/// renderer can simply skip the preview block rather than show empty content.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PickerView {
    /// Stores the preview
    pub preview: Option<String>,
}
/// Represents picker list entry
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PickerListEntry {
    /// Stores the label
    pub label: String,
    /// Stores the description
    pub description: String,
    /// Stores the tag
    pub tag: Option<String>,
    /// Stores the selected
    pub selected: bool,
    /// Optional group header rendered above this entry.
    /// Only the first entry of each provider group should carry a header.
    pub group_header: Option<String>,
}
/// Represents picker list view
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PickerListView {
    /// Stores the title
    pub title: String,
    /// Stores the query
    pub query: String,
    /// Stores the entries
    pub entries: Vec<PickerListEntry>,
    /// Stores the hint
    pub hint: String,
}

/// Handles message lines
pub fn message_lines(messages: &[MessageEnvelope], expand_output: bool) -> Vec<MessageLineView> {
    message_lines_for_width(messages, DEFAULT_MESSAGE_SUMMARY_WIDTH, expand_output)
}

/// Handles message lines for a specific summary width.
#[must_use]
pub fn message_lines_for_width(
    messages: &[MessageEnvelope],
    summary_width: usize,
    expand_output: bool,
) -> Vec<MessageLineView> {
    rich_message_views(messages, expand_output)
        .into_iter()
        .flat_map(|view| view.display_lines(summary_width.max(1), expand_output))
        .collect()
}
/// Handles status text
#[must_use]
pub fn status_text(app: &AppState) -> String {
    session_status_text(app)
}
/// Handles footer text
#[must_use]
pub fn footer_text(app: &AppState) -> String {
    session_footer_text(app)
}
/// Returns the task panel view
#[must_use]
pub fn task_panel_view(app: &AppState) -> Option<TaskPanelView> {
    if app.background_tasks.is_empty() {
        return None;
    }

    let mut tasks: Vec<&TaskState> = app.background_tasks.values().collect();
    tasks.sort_by(|left, right| {
        status_rank(left.status)
            .cmp(&status_rank(right.status))
            .then_with(|| left.description.cmp(&right.description))
    });

    let lines = tasks
        .into_iter()
        .take(4)
        .map(|task| {
            let role = match task.status {
                TaskStatus::Running | TaskStatus::Pending => MessageRole::Progress,
                TaskStatus::Completed => MessageRole::Assistant,
                TaskStatus::Failed | TaskStatus::Killed | TaskStatus::Cancelled => {
                    MessageRole::Error
                }
            };

            let mut text = format!(
                "[{}] {}: {}",
                task_status_label(task.status),
                task_kind_label(task.kind),
                task.description
            );
            match task.kind {
                TaskKind::LocalShell => {
                    if let Some(pid) = task.pid {
                        text.push_str(&format!(" • pid {pid}"));
                    }
                }
                TaskKind::LocalAgent => {
                    if let Some(agent) = &task.agent {
                        text.push_str(&format!(" • {}", agent_runtime_label(agent.runtime)));
                    }
                    // Show live progress summary when the agent is actively running.
                    if task.status == TaskStatus::Running {
                        if let Some(summary) = &task.agent_summary {
                            text.push_str(&format!(" — {summary}"));
                        }
                    }
                }
                TaskKind::RemoteAgent => {}
            }

            MessageLineView::new(text, role)
        })
        .collect();

    Some(TaskPanelView {
        title: "Tasks".into(),
        lines,
    })
}
/// Handles queued panel view
#[must_use]
pub fn queued_panel_view(app: &AppState) -> Option<TaskPanelView> {
    let queued_commands = app.queued_commands.iter().cloned().collect::<Vec<_>>();
    let queue = PromptQueueView::from_commands(&queued_commands)?;
    let overflow = queue.overflow_label();
    let mut lines = queue
        .commands
        .into_iter()
        .map(|queued| {
            MessageLineView::new(
                format!("{}. {}", queued.index, queued.preview),
                MessageRole::Progress,
            )
        })
        .collect::<Vec<_>>();
    if let Some(overflow) = overflow {
        lines.push(MessageLineView::new(overflow, MessageRole::System));
    }

    Some(TaskPanelView {
        title: "Queued".into(),
        lines,
    })
}

fn render_message(message: &MessageEnvelope, output: &mut Vec<MessageLineView>) {
    match &message.payload {
        MessagePayload::UserText { content } => {
            push_prefixed_lines(output, "user> ", content, MessageRole::User)
        }
        MessagePayload::UserAttachment { label, uri } => output.push(MessageLineView::new(
            format!("attachment> {label} ({uri})"),
            MessageRole::User,
        )),
        MessagePayload::UserPasteReference { sha256, bytes } => output.push(MessageLineView::new(
            format!("paste> {sha256} ({bytes} bytes)"),
            MessageRole::User,
        )),
        MessagePayload::AssistantText { content } => {
            push_prefixed_lines(output, "assistant> ", content, MessageRole::Assistant)
        }
        MessagePayload::AssistantThinking { content, collapsed } => {
            let prefix = if *collapsed {
                "thinking> [collapsed] "
            } else {
                "thinking> "
            };
            push_prefixed_lines(output, prefix, content, MessageRole::Progress);
        }
        MessagePayload::AssistantToolUse { tool, input, .. } => output.push(MessageLineView::new(
            format!("tool[{tool}] input {input}"),
            MessageRole::Tool,
        )),
        MessagePayload::ToolResult {
            tool,
            success,
            content,
            ..
        } => {
            let status = if *success { "ok" } else { "error" };
            let role = if *success {
                MessageRole::Tool
            } else {
                MessageRole::Error
            };
            push_prefixed_lines(output, &format!("tool[{tool}] {status}> "), content, role);
        }
        MessagePayload::BashOutput {
            stdout,
            stderr,
            exit_code,
        } => {
            if !stdout.is_empty() {
                push_prefixed_lines(output, "stdout> ", stdout, MessageRole::Assistant);
            }
            if !stderr.is_empty() {
                push_prefixed_lines(output, "stderr> ", stderr, MessageRole::Error);
            }
            if let Some(code) = exit_code {
                output.push(MessageLineView::new(
                    format!("bash> exit code {code}"),
                    if *code == 0 {
                        MessageRole::Progress
                    } else {
                        MessageRole::Error
                    },
                ));
            }
        }
        MessagePayload::System { content } => {
            push_prefixed_lines(output, "system> ", content, MessageRole::System)
        }
        MessagePayload::Progress { label, detail } => {
            let mut text = format!("progress> {label}");
            if let Some(detail) = detail {
                text.push_str(": ");
                text.push_str(detail);
            }
            output.push(MessageLineView::new(text, MessageRole::Progress));
        }
        MessagePayload::Command {
            input,
            output: result,
        } => {
            output.push(MessageLineView::new(
                format!("command> {input}"),
                MessageRole::User,
            ));
            if let Some(result) = result {
                if input.trim() == "/help" {
                    push_prefixed_lines(output, "", result, MessageRole::System);
                } else {
                    push_prefixed_lines(output, "result> ", result, MessageRole::Assistant);
                }
            }
        }
        MessagePayload::HookResult {
            hook,
            success,
            output: result,
        } => {
            let role = if *success {
                MessageRole::Progress
            } else {
                MessageRole::Error
            };
            push_prefixed_lines(output, &format!("hook[{hook}]> "), result, role);
        }
        MessagePayload::HookProgress {
            event,
            tool_name,
            hook_count,
            success,
        } => {
            let icon = if *success { "⚙" } else { "⚠" };
            let noun = if *hook_count == 1 { "hook" } else { "hooks" };
            output.push(MessageLineView::new(
                format!("{icon} {hook_count} {event} {noun} ran for {tool_name}"),
                MessageRole::Progress,
            ));
        }
        MessagePayload::CompactBoundary { summary } => {
            push_prefixed_lines(output, "summary> ", summary, MessageRole::System)
        }
        MessagePayload::Task {
            task_id: _,
            status,
            message,
        } => output.push(MessageLineView::new(
            format!("task> [{}] {message}", task_status_label(*status)),
            match status {
                TaskStatus::Failed | TaskStatus::Killed | TaskStatus::Cancelled => {
                    MessageRole::Error
                }
                TaskStatus::Completed => MessageRole::Assistant,
                TaskStatus::Pending | TaskStatus::Running => MessageRole::Progress,
            },
        )),
        MessagePayload::Permission {
            tool,
            decision,
            reason,
        } => output.push(MessageLineView::new(
            format!("permission[{tool}] {decision}: {reason}"),
            MessageRole::Progress,
        )),
        MessagePayload::PlanApproval { summary, approved } => output.push(MessageLineView::new(
            format!(
                "plan> {}: {summary}",
                if *approved { "approved" } else { "rejected" }
            ),
            if *approved {
                MessageRole::Assistant
            } else {
                MessageRole::Error
            },
        )),
        MessagePayload::ProviderError { kind, message } => {
            push_prefixed_lines(
                output,
                &format!("error[{kind}]> "),
                message,
                MessageRole::Error,
            );
        }
        // Render the XML payload verbatim so the model transcript shows the
        // full structured notification.  The `task>` prefix keeps it visually
        // consistent with the existing Task variant.
        MessagePayload::TaskNotification { xml_payload, .. } => {
            push_prefixed_lines(output, "task> ", xml_payload, MessageRole::Progress);
        }
    }
}

fn push_prefixed_lines(
    output: &mut Vec<MessageLineView>,
    prefix: &str,
    content: &str,
    role: MessageRole,
) {
    let mut lines = content.lines();
    match lines.next() {
        Some(first) => output.push(MessageLineView::new(format!("{prefix}{first}"), role)),
        None => output.push(MessageLineView::new(prefix.to_string(), role)),
    }

    let continuation = " ".repeat(prefix.chars().count());
    for line in lines {
        output.push(MessageLineView::new(format!("{continuation}{line}"), role));
    }
}

fn task_kind_label(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::LocalShell => "shell",
        TaskKind::LocalAgent => "agent",
        TaskKind::RemoteAgent => "remote agent",
    }
}

fn agent_runtime_label(runtime: wonder_of_u_core::AgentRuntime) -> &'static str {
    match runtime {
        wonder_of_u_core::AgentRuntime::MetadataOnly => "metadata-only",
        wonder_of_u_core::AgentRuntime::PromptSubprocess => "prompt-subprocess",
        wonder_of_u_core::AgentRuntime::Deferred => "legacy-relaunch-required",
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

const fn status_rank(status: TaskStatus) -> u8 {
    match status {
        TaskStatus::Running => 0,
        TaskStatus::Pending => 1,
        TaskStatus::Failed => 2,
        TaskStatus::Killed => 3,
        TaskStatus::Cancelled => 4,
        TaskStatus::Completed => 5,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ::time::{Duration, OffsetDateTime};
    use wonder_of_u_core::{
        AppState, MessagePayload, QueuePlacement, SessionId, TaskState, TokenUsage, ToolUseId,
    };

    use super::*;

    #[test]
    fn message_lines_expand_multiline_payloads() {
        let session_id = SessionId::new();
        let use_id =
            ToolUseId::parse("00000000-0000-0000-0000-000000000001").expect("valid tool use id");
        let mut assistant = MessageEnvelope::new(
            session_id,
            MessagePayload::AssistantText {
                content: "line one\nline two".into(),
            },
        );
        assistant.timestamp =
            OffsetDateTime::UNIX_EPOCH + Duration::hours(12) + Duration::minutes(45);
        let messages = vec![
            MessageEnvelope::new(
                session_id,
                MessagePayload::UserText {
                    content: "review changes".into(),
                },
            ),
            assistant,
            MessageEnvelope::new(
                session_id,
                MessagePayload::ToolResult {
                    tool: "bash".into(),
                    use_id,
                    success: false,
                    content: "permission denied".into(),
                },
            ),
        ];

        // Claude visual parity: user and assistant messages open with a "● " bullet.
        // The timestamp line follows the first assistant content line.
        // Tool/error rows use their own "● Tool(args)" + "  ⎿  detail" chrome.
        assert_eq!(
            message_lines(&messages, false),
            vec![
                MessageLineView::new("● review changes", MessageRole::User),
                MessageLineView::new("● line one line two", MessageRole::Assistant),
                MessageLineView::with_spans(
                    MessageRole::System,
                    vec![
                        MessageSpanView::new(" ".repeat(72), None),
                        MessageSpanView::new(
                            "12:45 PM",
                            Some(
                                crate::style::TextStyle::default()
                                    .fg(crate::style::Color::DarkGrey)
                                    .dim(),
                            ),
                        ),
                    ],
                ),
                MessageLineView::new("● Bash", MessageRole::Error,),
                MessageLineView::new("  ⎿  permission denied", MessageRole::Error,),
            ]
        );
    }

    #[test]
    fn status_and_task_panel_reflect_app_state() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        app.provider = Some("copilot".into());
        app.model = Some("gpt-5".into());
        app.record_cost_usage(
            TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            None,
        );
        app.background_tasks.insert(
            Default::default(),
            TaskState {
                description: "run tests".into(),
                status: TaskStatus::Running,
                ..TaskState::pending("run tests")
            },
        );

        assert!(status_text(&app).contains("copilot:gpt-5"));
        assert!(status_text(&app).contains("auth=not_required"));
        assert!(footer_text(&app).contains("cwd=/workspace"));
        let panel = task_panel_view(&app).expect("task panel");
        assert_eq!(panel.lines[0].text, "[running] shell: run tests");
    }

    #[test]
    fn task_panel_shows_agent_summary_for_running_local_agent() {
        use wonder_of_u_core::{AgentTaskState, TaskId};

        let mut app = AppState::new(PathBuf::from("/workspace"));
        let mut task = TaskState::pending_agent(
            "fix bug",
            AgentTaskState::prompt_subprocess("fix bug", "Fix the null check", None, None),
        );
        task.status = TaskStatus::Running;
        task.agent_summary = Some("Fixing null check in validate.ts".into());
        app.background_tasks.insert(TaskId::new(), task);

        let panel = task_panel_view(&app).expect("task panel present");
        assert!(
            panel.lines[0]
                .text
                .contains("Fixing null check in validate.ts"),
            "agent_summary must appear in running LocalAgent line; got: {}",
            panel.lines[0].text
        );
    }

    #[test]
    fn task_panel_omits_agent_summary_when_not_running() {
        use wonder_of_u_core::{AgentTaskState, TaskId};

        let mut app = AppState::new(PathBuf::from("/workspace"));
        let mut task = TaskState::pending_agent(
            "fix bug",
            AgentTaskState::prompt_subprocess("fix bug", "Fix the null check", None, None),
        );
        task.status = TaskStatus::Completed;
        task.agent_summary = Some("Fixing null check in validate.ts".into());
        app.background_tasks.insert(TaskId::new(), task);

        let panel = task_panel_view(&app).expect("task panel present");
        assert!(
            !panel.lines[0]
                .text
                .contains("Fixing null check in validate.ts"),
            "agent_summary must NOT appear for Completed task; got: {}",
            panel.lines[0].text
        );
    }

    #[test]
    fn queued_panel_reflects_visible_commands_and_overflow() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        app.queue_command("/status", QueuePlacement::Now);
        app.queue_command("draft the migration plan", QueuePlacement::Next);
        app.queue_command("/theme midnight", QueuePlacement::Later);
        app.queue_command("summarize the open tasks in detail", QueuePlacement::Later);

        let panel = queued_panel_view(&app).expect("queue panel");

        assert_eq!(panel.title, "Queued");
        assert_eq!(
            panel.lines,
            vec![
                MessageLineView::new("1. /status", MessageRole::Progress),
                MessageLineView::new("2. draft the migration plan", MessageRole::Progress),
                MessageLineView::new("3. /theme midnight", MessageRole::Progress),
                MessageLineView::new("+1 more queued", MessageRole::System),
            ]
        );
    }

    #[test]
    fn message_lines_preserve_legacy_attachment_thinking_and_boundary_output() {
        let session_id = SessionId::new();
        let messages = vec![
            MessageEnvelope::new(
                session_id,
                MessagePayload::UserAttachment {
                    label: "diagram.png".into(),
                    uri: "file:///workspace/assets/diagram.png".into(),
                },
            ),
            MessageEnvelope::new(
                session_id,
                MessagePayload::AssistantThinking {
                    content: "step one\nstep two".into(),
                    collapsed: true,
                },
            ),
            MessageEnvelope::new(
                session_id,
                MessagePayload::CompactBoundary {
                    summary: "Conversation compacted".into(),
                },
            ),
        ];

        assert_eq!(
            message_lines(&messages, false),
            vec![
                MessageLineView::new("attachment> image diagram.png", MessageRole::User,),
                MessageLineView::new("  file:///workspace/assets/diagram.png", MessageRole::User),
                MessageLineView::new(
                    "thinking> ∴ Thinking · collapsed · 2 lines hidden",
                    MessageRole::Progress,
                ),
                MessageLineView::new("summary> ✻ Conversation compacted", MessageRole::System),
            ]
        );
    }

    #[test]
    fn message_lines_group_tool_calls_into_compact_summaries() {
        let session_id = SessionId::new();
        let use_id = wonder_of_u_core::ToolUseId::parse("00000000-0000-0000-0000-000000000001")
            .expect("valid tool use id");
        let messages = vec![
            MessageEnvelope::new(
                session_id,
                MessagePayload::AssistantToolUse {
                    tool: "bash".into(),
                    use_id,
                    input: serde_json::json!({ "command": "echo hi" }),
                },
            ),
            MessageEnvelope::new(
                session_id,
                MessagePayload::ToolResult {
                    tool: "bash".into(),
                    use_id,
                    success: true,
                    content: "done".into(),
                },
            ),
        ];

        assert_eq!(
            message_lines(&messages, false),
            vec![
                MessageLineView::new("● Bash(echo hi)", MessageRole::Tool,),
                MessageLineView::new("  ⎿  echo hi", MessageRole::System),
            ]
        );
    }

    #[test]
    fn provider_error_renders_with_error_role_and_kind_prefix() {
        let session_id = SessionId::new();
        let messages = vec![MessageEnvelope::new(
            session_id,
            MessagePayload::ProviderError {
                kind: "provider".into(),
                message: "connection refused\nextra detail".into(),
            },
        )];

        let lines = message_lines(&messages, false);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].role, MessageRole::Error);
        assert!(lines[0].text.starts_with("error[provider]> "));
        assert_eq!(lines[1].role, MessageRole::Error);
    }
}
