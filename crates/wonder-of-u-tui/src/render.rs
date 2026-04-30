use wonder_of_u_core::AppState;

use crate::{
    dialog::DialogView,
    frame::{FrameBuffer, Rect},
    layout::ShellLayout,
    message::{
        footer_text, message_lines, queued_panel_view, status_text, task_panel_view,
        HistorySearchView, MessageLineView, MessageRole, TaskPanelView,
    },
    style::{Color, TextStyle, Theme},
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShellView {
    pub title: String,
    pub messages: Vec<MessageLineView>,
    pub prompt: String,
    pub history_search: Option<HistorySearchView>,
    pub status: String,
    pub footer: String,
    pub queued_panel: Option<TaskPanelView>,
    pub task_panel: Option<TaskPanelView>,
    pub dialog: Option<DialogView>,
}

impl ShellView {
    #[must_use]
    pub fn prompt_height(&self) -> u16 {
        let line_count = match &self.history_search {
            Some(search) => history_search_line_count(search),
            None => text_line_count(&self.prompt),
        };
        u16::try_from(line_count)
            .unwrap_or(u16::MAX.saturating_sub(2))
            .saturating_add(2)
    }

    #[must_use]
    pub fn from_app_state(app: &AppState, prompt: impl Into<String>) -> Self {
        Self {
            title: format!("Session: {}", app.session.title),
            messages: message_lines(&app.messages),
            prompt: prompt.into(),
            history_search: None,
            status: status_text(app),
            footer: footer_text(app),
            queued_panel: queued_panel_view(app),
            task_panel: task_panel_view(app),
            dialog: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StyledLine {
    text: String,
    style: TextStyle,
}

pub fn render_shell(frame: &mut FrameBuffer, view: &ShellView, theme: &Theme) {
    let layout = ShellLayout::split(frame.area(), view.prompt_height());
    frame.fill_rect(frame.area(), ' ', theme.background);

    draw_panel(
        frame,
        layout.messages,
        Some(&view.title),
        &message_panel_lines(view, theme),
        theme,
    );
    draw_panel(
        frame,
        layout.prompt,
        Some("Prompt"),
        &prompt_panel_lines(view, theme),
        theme,
    );
    draw_status_line(frame, layout.status, &view.status, theme.status);
    draw_status_line(frame, layout.footer, &view.footer, theme.footer);

    if let Some(task_panel) = &view.task_panel {
        draw_task_panel(frame, layout.messages, task_panel, theme);
    }

    if let Some(queued_panel) = &view.queued_panel {
        draw_queue_panel(frame, layout.messages, queued_panel, theme);
    }

    if let Some(dialog) = &view.dialog {
        draw_dialog(frame, layout.messages.inset(1), dialog, theme);
    }
}

#[must_use]
pub fn render_snapshot(width: u16, height: u16, view: &ShellView, theme: &Theme) -> FrameBuffer {
    let mut frame = FrameBuffer::new(width, height);
    render_shell(&mut frame, view, theme);
    frame
}

fn draw_panel(
    frame: &mut FrameBuffer,
    area: Rect,
    title: Option<&str>,
    lines: &[StyledLine],
    theme: &Theme,
) {
    if area.is_empty() {
        return;
    }

    frame.fill_rect(area, ' ', theme.background);
    if area.width >= 2 && area.height >= 2 {
        frame.draw_border(area, theme.border);
        if let Some(title) = title.filter(|title| !title.is_empty()) {
            let max_title_width = area.width.saturating_sub(4);
            frame.write_str(
                area.x.saturating_add(2),
                area.y,
                title,
                theme.title,
                max_title_width,
            );
        }

        let inner = area.inset(1);
        for (offset, line) in lines.iter().take(usize::from(inner.height)).enumerate() {
            let y = inner
                .y
                .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
            frame.write_str(inner.x, y, &line.text, line.style, inner.width);
        }
        return;
    }

    if let Some(first_line) = lines.first() {
        frame.write_str(
            area.x,
            area.y,
            &first_line.text,
            first_line.style,
            area.width,
        );
    }
}

fn draw_task_panel(frame: &mut FrameBuffer, area: Rect, panel: &TaskPanelView, theme: &Theme) {
    let overlay = area.inset(1);
    if overlay.width < 16 || overlay.height < 3 {
        return;
    }

    let content_width = panel
        .lines
        .iter()
        .map(|line| line.text.chars().count())
        .chain(std::iter::once(panel.title.chars().count()))
        .max()
        .unwrap_or(0);
    let desired_width = u16::try_from(content_width.saturating_add(4)).unwrap_or(overlay.width);
    let width = desired_width.min(overlay.width.min(24)).max(16);
    let height = u16::try_from(panel.lines.len().saturating_add(2))
        .unwrap_or(overlay.height)
        .min(overlay.height);
    let rect = Rect::new(
        overlay.right().saturating_sub(width),
        overlay.y,
        width,
        height,
    );

    draw_panel(
        frame,
        rect,
        Some(&panel.title),
        &message_lines_to_styled(&panel.lines, theme),
        theme,
    );
}

fn draw_queue_panel(frame: &mut FrameBuffer, area: Rect, panel: &TaskPanelView, theme: &Theme) {
    let overlay = area.inset(1);
    if overlay.width < 20 || overlay.height < 4 {
        return;
    }

    let content_width = panel
        .lines
        .iter()
        .map(|line| line.text.chars().count())
        .chain(std::iter::once(panel.title.chars().count()))
        .max()
        .unwrap_or(0);
    let desired_width = u16::try_from(content_width.saturating_add(4)).unwrap_or(overlay.width);
    let width = desired_width.min(overlay.width.min(36)).max(20);
    let height = u16::try_from(panel.lines.len().saturating_add(2))
        .unwrap_or(overlay.height)
        .min(overlay.height);
    let rect = Rect::new(
        overlay.x,
        overlay.bottom().saturating_sub(height),
        width,
        height,
    );

    draw_panel(
        frame,
        rect,
        Some(&panel.title),
        &message_lines_to_styled(&panel.lines, theme),
        theme,
    );
}

fn draw_dialog(frame: &mut FrameBuffer, viewport: Rect, dialog: &DialogView, theme: &Theme) {
    if viewport.width < 12 || viewport.height < 5 {
        return;
    }

    frame.fill_rect(viewport, ' ', theme.background);

    let actions = dialog.action_hint();
    let content_width = dialog
        .body
        .iter()
        .map(|line| line.chars().count())
        .chain(std::iter::once(dialog.title.chars().count()))
        .chain(std::iter::once(actions.chars().count()))
        .max()
        .unwrap_or(0);
    let width = u16::try_from(content_width.saturating_add(4))
        .unwrap_or(viewport.width)
        .max(viewport.width.saturating_sub(2))
        .min(viewport.width);
    let height = u16::try_from(dialog.body.len().saturating_add(3))
        .unwrap_or(viewport.height)
        .min(viewport.height);
    let rect = Rect::new(
        viewport.x + viewport.width.saturating_sub(width) / 2,
        viewport.y + viewport.height.saturating_sub(height) / 2,
        width,
        height,
    );

    let mut body = plain_lines(&dialog.body, theme.messages);
    body.push(StyledLine {
        text: actions,
        style: theme.status,
    });
    draw_panel(frame, rect, Some(&dialog.title), &body, theme);
}

fn draw_status_line(frame: &mut FrameBuffer, area: Rect, text: &str, style: TextStyle) {
    if area.is_empty() {
        return;
    }

    frame.fill_rect(area, ' ', style);
    frame.write_str(area.x, area.y, text, style, area.width);
}

fn message_panel_lines(view: &ShellView, theme: &Theme) -> Vec<StyledLine> {
    if view.messages.is_empty() {
        return vec![StyledLine {
            text: "No messages yet.".into(),
            style: style_for_message(theme, MessageRole::System),
        }];
    }

    message_lines_to_styled(&view.messages, theme)
}

fn message_lines_to_styled(lines: &[MessageLineView], theme: &Theme) -> Vec<StyledLine> {
    lines
        .iter()
        .map(|line| StyledLine {
            text: line.text.clone(),
            style: style_for_message(theme, line.role),
        })
        .collect()
}

fn prompt_panel_lines(view: &ShellView, theme: &Theme) -> Vec<StyledLine> {
    let Some(search) = &view.history_search else {
        return plain_lines(&split_lines(&view.prompt), theme.prompt);
    };

    let mut lines = vec![
        StyledLine {
            text: format!("History search: {}", search.query),
            style: theme.status,
        },
        StyledLine {
            text: format!(
                "Match: {}/{}",
                if search.match_total == 0 {
                    0
                } else {
                    search.match_index.saturating_add(1)
                },
                search.match_total
            ),
            style: theme.footer,
        },
    ];
    lines.extend(plain_lines(
        &split_lines(search.match_text.as_deref().unwrap_or("")),
        theme.prompt,
    ));
    lines
}

fn plain_lines(lines: &[String], style: TextStyle) -> Vec<StyledLine> {
    lines
        .iter()
        .map(|line| StyledLine {
            text: line.clone(),
            style,
        })
        .collect()
}

fn style_for_message(theme: &Theme, role: MessageRole) -> TextStyle {
    match role {
        MessageRole::User => {
            let mut style = theme.prompt;
            style.bold = true;
            style
        }
        MessageRole::Assistant => theme.messages,
        MessageRole::System => theme.footer,
        MessageRole::Tool => {
            let mut style = theme.status;
            style.bold = false;
            style
        }
        MessageRole::Progress => {
            let mut style = theme.status;
            style.dim = true;
            style
        }
        MessageRole::Error => TextStyle::default().fg(Color::Red).bold(),
    }
}

fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }

    text.lines().map(ToString::to_string).collect()
}

fn text_line_count(text: &str) -> usize {
    text.lines().count().max(1)
}

fn history_search_line_count(search: &HistorySearchView) -> usize {
    2usize.saturating_add(text_line_count(search.match_text.as_deref().unwrap_or("")))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use wonder_of_u_core::{
        AppState, MessageEnvelope, MessagePayload, TaskState, TaskStatus, TokenUsage,
    };

    use super::*;
    use crate::{dialog::DialogView, message::MessageLineView};

    #[test]
    fn empty_shell_snapshot_renders_placeholder_message() {
        let view = ShellView {
            title: "Session: Empty".into(),
            messages: Vec::new(),
            prompt: String::new(),
            history_search: None,
            status: "prompt | 0 messages".into(),
            footer: "ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
        };

        let frame = render_snapshot(30, 9, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "+-Session: Empty-------------+",
                "|No messages yet.            |",
                "|                            |",
                "+----------------------------+",
                "+-Prompt---------------------+",
                "|                            |",
                "+----------------------------+",
                "prompt | 0 messages",
                "ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn prompt_shell_snapshot_renders_messages_and_tasks() {
        let view = ShellView {
            title: "Session: Demo".into(),
            messages: vec![
                MessageLineView::new("system> ready", MessageRole::System),
                MessageLineView::new("assistant> hello", MessageRole::Assistant),
            ],
            prompt: "/status".into(),
            history_search: None,
            status: "prompt | 2 messages".into(),
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: Some(TaskPanelView {
                title: "Tasks".into(),
                lines: vec![MessageLineView::new(
                    "[running] shell: index workspace",
                    MessageRole::Progress,
                )],
            }),
            dialog: None,
        };

        let frame = render_snapshot(48, 10, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "+-Session: Demo--------------------------------+",
                "|system> ready         +-Tasks----------------+|",
                "|assistant> hello      |[running] shell: index||",
                "|                      +----------------------+|",
                "+----------------------------------------------+",
                "+-Prompt---------------------------------------+",
                "|/status                                       |",
                "+----------------------------------------------+",
                "prompt | 2 messages",
                "cwd=/workspace | ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn shell_view_from_app_state_adapts_messages_and_chrome() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        app.session.title = "Demo".into();
        app.session.git_branch = None;
        app.provider = Some("copilot".into());
        app.model = Some("gpt-5".into());
        app.record_cost_usage(
            TokenUsage {
                input_tokens: 12,
                output_tokens: 3,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            None,
        );
        app.push_message(MessageEnvelope::system(app.session.id, "ready"))
            .expect("push system");
        app.push_message(MessageEnvelope::new(
            app.session.id,
            MessagePayload::AssistantText {
                content: "working".into(),
            },
        ))
        .expect("push assistant");
        app.background_tasks.insert(
            Default::default(),
            TaskState {
                description: "index workspace".into(),
                status: TaskStatus::Running,
                ..TaskState::pending("index workspace")
            },
        );
        app.queue_command("/status", wonder_of_u_core::QueuePlacement::Later);

        let view = ShellView::from_app_state(&app, "/help");

        assert_eq!(view.title, "Session: Demo");
        assert_eq!(view.messages[0].text, "system> ready");
        assert!(view.status.contains("copilot:gpt-5"));
        assert!(view.footer.contains("cwd=/workspace"));
        assert!(view.queued_panel.is_some());
        assert!(view.task_panel.is_some());
    }

    #[test]
    fn shell_snapshot_renders_queued_commands_overlay() {
        let view = ShellView {
            title: "Session: Demo".into(),
            messages: vec![MessageLineView::new(
                "assistant> ready",
                MessageRole::Assistant,
            )],
            prompt: "/plan".into(),
            history_search: None,
            status: "prompt | 1 messages".into(),
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: Some(TaskPanelView {
                title: "Queued".into(),
                lines: vec![
                    MessageLineView::new("1. /status", MessageRole::Progress),
                    MessageLineView::new("2. draft migration plan", MessageRole::Progress),
                    MessageLineView::new("+2 more queued", MessageRole::System),
                ],
            }),
            task_panel: None,
            dialog: None,
        };

        let frame = render_snapshot(42, 12, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "+-Session: Demo--------------------------+",
                "|+-Queued------------------+             |",
                "||1. /status               |             |",
                "||2. draft migration plan  |             |",
                "||+2 more queued           |             |",
                "|+-------------------------+             |",
                "+----------------------------------------+",
                "+-Prompt---------------------------------+",
                "|/plan                                   |",
                "+----------------------------------------+",
                "prompt | 1 messages",
                "cwd=/workspace | ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn dialog_snapshot_renders_confirm_overlay() {
        let view = ShellView {
            title: "Session: Dialog".into(),
            messages: vec![MessageLineView::new("system> ready", MessageRole::System)],
            prompt: "continue?".into(),
            history_search: None,
            status: "permission | 1 messages".into(),
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: Some(DialogView::confirm(
                "Confirm action",
                ["Approve command execution", "This cannot be undone"],
            )),
        };

        let frame = render_snapshot(42, 12, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "+-Session: Dialog------------------------+",
                "| +-Confirm action---------------------+ |",
                "| |Approve command execution           | |",
                "| |This cannot be undone               | |",
                "| |[Confirm]  Cancel                   | |",
                "| +------------------------------------+ |",
                "+----------------------------------------+",
                "+-Prompt---------------------------------+",
                "|continue?                               |",
                "+----------------------------------------+",
                "permission | 1 messages",
                "cwd=/workspace | ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn history_search_snapshot_renders_overlay_and_preview() {
        let view = ShellView {
            title: "Session: Search".into(),
            messages: vec![MessageLineView::new(
                "assistant> ready",
                MessageRole::Assistant,
            )],
            prompt: "draft".into(),
            history_search: Some(HistorySearchView {
                query: "pla".into(),
                match_text: Some("draft plan".into()),
                match_index: 1,
                match_total: 3,
            }),
            status: "prompt | 1 messages".into(),
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
        };

        let frame = render_snapshot(42, 14, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "+-Session: Search------------------------+",
                "|assistant> ready                        |",
                "|                                        |",
                "|                                        |",
                "|                                        |",
                "|                                        |",
                "+----------------------------------------+",
                "+-Prompt---------------------------------+",
                "|History search: pla                     |",
                "|Match: 2/3                              |",
                "|draft plan                              |",
                "+----------------------------------------+",
                "prompt | 1 messages",
                "cwd=/workspace | ctrl-c interrupt",
            ]
            .join("\n")
        );
    }
}
