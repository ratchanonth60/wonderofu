use wonder_of_u_core::{
    AppState, MessageEnvelope, MessagePayload, QueuedCommand, TaskKind, TaskState, TaskStatus,
    session_footer_text, session_status_text,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
    Progress,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageLineView {
    pub text: String,
    pub role: MessageRole,
}

impl MessageLineView {
    #[must_use]
    pub fn new(text: impl Into<String>, role: MessageRole) -> Self {
        Self {
            text: text.into(),
            role,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskPanelView {
    pub title: String,
    pub lines: Vec<MessageLineView>,
}

/// Describes the incremental prompt history search overlay.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HistorySearchView {
    pub query: String,
    pub match_text: Option<String>,
    pub match_index: usize,
    pub match_total: usize,
}

/// Carries the computed preview for the currently highlighted picker option.
///
/// `preview` is `None` when the active filter yields no matches, so the
/// renderer can simply skip the preview block rather than show empty content.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PickerView {
    pub preview: Option<String>,
}

pub fn message_lines(messages: &[MessageEnvelope]) -> Vec<MessageLineView> {
    let mut lines = Vec::new();

    for message in messages {
        render_message(message, &mut lines);
    }

    lines
}

#[must_use]
pub fn status_text(app: &AppState) -> String {
    session_status_text(app)
}

#[must_use]
pub fn footer_text(app: &AppState) -> String {
    session_footer_text(app)
}

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
                }
            }

            MessageLineView::new(text, role)
        })
        .collect();

    Some(TaskPanelView {
        title: "Tasks".into(),
        lines,
    })
}

#[must_use]
pub fn queued_panel_view(app: &AppState) -> Option<TaskPanelView> {
    const MAX_VISIBLE_COMMANDS: usize = 3;

    if app.queued_commands.is_empty() {
        return None;
    }

    let hidden_commands = app
        .queued_commands
        .len()
        .saturating_sub(MAX_VISIBLE_COMMANDS);
    let mut lines = app
        .queued_commands
        .iter()
        .take(MAX_VISIBLE_COMMANDS)
        .enumerate()
        .map(|(index, queued)| {
            MessageLineView::new(
                format!("{}. {}", index + 1, queued_command_preview(queued)),
                MessageRole::Progress,
            )
        })
        .collect::<Vec<_>>();
    if hidden_commands > 0 {
        lines.push(MessageLineView::new(
            format!("+{hidden_commands} more queued"),
            MessageRole::System,
        ));
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
                push_prefixed_lines(output, "result> ", result, MessageRole::Assistant);
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
    }
}

fn queued_command_preview(queued: &QueuedCommand) -> String {
    const MAX_PREVIEW_CHARS: usize = 32;

    let normalized = queued
        .command
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return "(empty command)".into();
    }

    truncate_with_ellipsis(&normalized, MAX_PREVIEW_CHARS)
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

fn task_kind_label(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::LocalShell => "shell",
        TaskKind::LocalAgent => "agent",
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

    use wonder_of_u_core::{
        AppState, MessagePayload, QueuePlacement, SessionId, TaskState, TokenUsage,
    };

    use super::*;

    #[test]
    fn message_lines_expand_multiline_payloads() {
        let session_id = SessionId::new();
        let messages = vec![
            MessageEnvelope::new(
                session_id,
                MessagePayload::AssistantText {
                    content: "line one\nline two".into(),
                },
            ),
            MessageEnvelope::new(
                session_id,
                MessagePayload::ToolResult {
                    tool: "bash".into(),
                    use_id: Default::default(),
                    success: false,
                    content: "permission denied".into(),
                },
            ),
        ];

        assert_eq!(
            message_lines(&messages),
            vec![
                MessageLineView::new("assistant> line one", MessageRole::Assistant),
                MessageLineView::new("           line two", MessageRole::Assistant),
                MessageLineView::new("tool[bash] error> permission denied", MessageRole::Error),
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
}
