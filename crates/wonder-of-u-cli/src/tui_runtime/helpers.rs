use super::*;

pub(super) fn command_output_text(output: CommandOutput) -> (Option<String>, bool) {
    match output {
        CommandOutput::Text(text)
        | CommandOutput::EnqueuePrompt(text)
        | CommandOutput::OpenUi(text) => (Some(text), false),
        CommandOutput::ExitRequested => (Some("exit requested".into()), true),
        CommandOutput::Noop => (None, false),
    }
}

pub(super) fn launch_external_editor(request: &ExternalEditorRequest) -> Result<()> {
    let Some((editor, args)) = tui_editor_command() else {
        return Err(WonderError::validation(
            "set VISUAL or EDITOR to enable `/plan open`",
        ));
    };
    let status = ProcessCommand::new(&editor)
        .args(args)
        .arg(&request.path)
        .current_dir(&request.cwd)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(WonderError::internal(format!(
            "editor exited unsuccessfully: {editor}"
        )))
    }
}

pub(super) fn tui_editor_command() -> Option<(String, Vec<String>)> {
    let raw = std::env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })?;
    let mut tokens = shell_words::split(&raw).ok()?;
    let command = tokens.first()?.clone();
    Some((command, tokens.drain(1..).collect()))
}

/// Renders the current controller state using ratatui's `Terminal::draw`, which
/// handles buffer diffing and cursor positioning.
pub(super) fn render_tui<W: Write>(
    term: &mut Terminal<CrosstermBackend<W>>,
    controller: &TuiController<'_>,
) -> Result<()> {
    let view = controller.view();
    let theme = theme_for_state(
        controller.state.theme.as_deref(),
        controller.state.session_color.as_deref(),
    );
    let (cursor_x, cursor_y) = {
        let (width, height) = terminal::size().unwrap_or((80, 24));
        controller.prompt_cursor(width.max(20), height.max(6))
    };
    term.draw(|frame| {
        render_to_ratatui_frame(frame, &view, &theme);
        // Place the input cursor at the prompt position.
        frame.set_cursor_position((cursor_x, cursor_y));
    })
    .map_err(|e| WonderError::internal(e.to_string()))?;
    Ok(())
}

pub(super) fn render_to_ratatui_frame(frame: &mut Frame<'_>, view: &ShellView, theme: &Theme) {
    let area = frame.area();
    let snapshot = wonder_of_u_tui::render_snapshot(area.width, area.height, view, theme);
    let buffer = frame.buffer_mut();

    for y in 0..area.height {
        for x in 0..area.width {
            let Some(cell) = snapshot.cell(x, y) else {
                continue;
            };
            let target = &mut buffer[(area.x.saturating_add(x), area.y.saturating_add(y))];
            target.set_char(cell.symbol);
            target.set_style(ratatui_style(cell.style));
        }
    }
}

pub(super) fn ratatui_style(style: wonder_of_u_tui::TextStyle) -> RatatuiStyle {
    let mut rendered = RatatuiStyle::default();
    if let Some(fg) = style.fg {
        rendered = rendered.fg(ratatui_color(fg));
    }
    if let Some(bg) = style.bg {
        rendered = rendered.bg(ratatui_color(bg));
    }

    let mut modifiers = Modifier::empty();
    if style.bold {
        modifiers |= Modifier::BOLD;
    }
    if style.dim {
        modifiers |= Modifier::DIM;
    }
    if style.italic {
        modifiers |= Modifier::ITALIC;
    }
    if style.underlined {
        modifiers |= Modifier::UNDERLINED;
    }
    if style.reversed {
        modifiers |= Modifier::REVERSED;
    }

    rendered.add_modifier(modifiers)
}

pub(super) fn ratatui_color(color: wonder_of_u_tui::Color) -> RatatuiColor {
    match color {
        wonder_of_u_tui::Color::Reset => RatatuiColor::Reset,
        wonder_of_u_tui::Color::Black => RatatuiColor::Black,
        wonder_of_u_tui::Color::DarkGrey => RatatuiColor::DarkGray,
        wonder_of_u_tui::Color::Red => RatatuiColor::Red,
        wonder_of_u_tui::Color::DarkRed => RatatuiColor::LightRed,
        wonder_of_u_tui::Color::Green => RatatuiColor::Green,
        wonder_of_u_tui::Color::DarkGreen => RatatuiColor::LightGreen,
        wonder_of_u_tui::Color::Yellow => RatatuiColor::Yellow,
        wonder_of_u_tui::Color::DarkYellow => RatatuiColor::LightYellow,
        wonder_of_u_tui::Color::Blue => RatatuiColor::Blue,
        wonder_of_u_tui::Color::DarkBlue => RatatuiColor::LightBlue,
        wonder_of_u_tui::Color::Magenta => RatatuiColor::Magenta,
        wonder_of_u_tui::Color::DarkMagenta => RatatuiColor::LightMagenta,
        wonder_of_u_tui::Color::Cyan => RatatuiColor::Cyan,
        wonder_of_u_tui::Color::DarkCyan => RatatuiColor::LightCyan,
        wonder_of_u_tui::Color::Grey => RatatuiColor::Gray,
        wonder_of_u_tui::Color::White => RatatuiColor::White,
        wonder_of_u_tui::Color::Rgb(r, g, b) => RatatuiColor::Rgb(r, g, b),
    }
}

pub(super) fn theme_for_state(name: Option<&str>, session_color: Option<&str>) -> Theme {
    let mut theme = match name {
        Some("midnight") => Theme {
            background: wonder_of_u_tui::TextStyle::default()
                .bg(wonder_of_u_tui::Color::Black)
                .fg(wonder_of_u_tui::Color::Grey),
            border: wonder_of_u_tui::TextStyle::default().fg(wonder_of_u_tui::Color::DarkMagenta),
            title: wonder_of_u_tui::TextStyle::default()
                .fg(wonder_of_u_tui::Color::Magenta)
                .bold(),
            messages: wonder_of_u_tui::TextStyle::default().fg(wonder_of_u_tui::Color::Grey),
            prompt: wonder_of_u_tui::TextStyle::default()
                .fg(wonder_of_u_tui::Color::Cyan)
                .bold(),
            status: wonder_of_u_tui::TextStyle::default()
                .fg(wonder_of_u_tui::Color::Yellow)
                .bold(),
            footer: wonder_of_u_tui::TextStyle::default().fg(wonder_of_u_tui::Color::DarkCyan),
        },
        Some("light") => Theme {
            background: wonder_of_u_tui::TextStyle::default()
                .bg(wonder_of_u_tui::Color::White)
                .fg(wonder_of_u_tui::Color::Black),
            border: wonder_of_u_tui::TextStyle::default().fg(wonder_of_u_tui::Color::Blue),
            title: wonder_of_u_tui::TextStyle::default()
                .fg(wonder_of_u_tui::Color::DarkBlue)
                .bold(),
            messages: wonder_of_u_tui::TextStyle::default().fg(wonder_of_u_tui::Color::Black),
            prompt: wonder_of_u_tui::TextStyle::default()
                .fg(wonder_of_u_tui::Color::DarkCyan)
                .bold(),
            status: wonder_of_u_tui::TextStyle::default()
                .fg(wonder_of_u_tui::Color::DarkBlue)
                .bold(),
            footer: wonder_of_u_tui::TextStyle::default().fg(wonder_of_u_tui::Color::DarkGrey),
        },
        _ => Theme::default(),
    };
    if let Some(color) = session_color_style(name, session_color) {
        theme.border = theme.border.fg(color);
        theme.title = theme.title.fg(color);
        theme.prompt = theme.prompt.fg(color).bold();
    }
    theme
}

pub(super) fn session_color_style(
    theme_name: Option<&str>,
    color: Option<&str>,
) -> Option<wonder_of_u_tui::Color> {
    Some(match (theme_name, color?) {
        (_, "red") => wonder_of_u_tui::Color::Red,
        (Some("light"), "blue") => wonder_of_u_tui::Color::DarkBlue,
        (_, "blue") => wonder_of_u_tui::Color::Blue,
        (Some("light"), "green") => wonder_of_u_tui::Color::DarkGreen,
        (_, "green") => wonder_of_u_tui::Color::Green,
        (Some("light"), "yellow") => wonder_of_u_tui::Color::DarkYellow,
        (_, "yellow") => wonder_of_u_tui::Color::Yellow,
        (_, "purple") => wonder_of_u_tui::Color::Magenta,
        (_, "orange") => wonder_of_u_tui::Color::Rgb(255, 165, 0),
        (_, "pink") => wonder_of_u_tui::Color::Rgb(255, 105, 180),
        (Some("light"), "cyan") => wonder_of_u_tui::Color::DarkCyan,
        (_, "cyan") => wonder_of_u_tui::Color::Cyan,
        _ => return None,
    })
}

pub(super) fn prompt_cursor_position(
    width: u16,
    height: u16,
    prompt: &str,
    cursor: usize,
) -> (u16, u16) {
    let layout = ShellLayout::split(
        wonder_of_u_tui::Rect::new(0, 0, width.max(1), height.max(1)),
        ShellView {
            title: String::new(),
            messages: Vec::new(),
            prompt: prompt.into(),
            history_search: None,
            status: String::new(),
            loading: false,
            loading_verb: None,
            footer: String::new(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            scroll: wonder_of_u_tui::TranscriptScrollView::default(),
        }
        .prompt_height(),
    );
    let inner = wonder_of_u_tui::Rect::new(
        layout.prompt.x.saturating_add(2),
        layout.prompt.y.saturating_add(1),
        layout.prompt.width.saturating_sub(4),
        layout.prompt.height.saturating_sub(2),
    );
    let cursor_text = prompt.chars().take(cursor).collect::<String>();
    let mut line = 0u16;
    let mut column = 0u16;
    for ch in cursor_text.chars() {
        if ch == '\n' {
            line = line.saturating_add(1);
            column = 0;
        } else {
            column = column.saturating_add(1);
        }
    }
    (
        inner
            .x
            .saturating_add(2)
            .saturating_add(column.min(inner.width.saturating_sub(1))),
        inner
            .y
            .saturating_add(line.min(inner.height.saturating_sub(1))),
    )
}

pub(super) fn history_search_cursor_position(
    width: u16,
    height: u16,
    view: &HistorySearchView,
    query_cursor: usize,
) -> (u16, u16) {
    let layout = ShellLayout::split(
        wonder_of_u_tui::Rect::new(0, 0, width.max(1), height.max(1)),
        ShellView {
            title: String::new(),
            messages: Vec::new(),
            prompt: view.match_text.clone().unwrap_or_default(),
            history_search: Some(view.clone()),
            status: String::new(),
            loading: false,
            loading_verb: None,
            footer: String::new(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            scroll: wonder_of_u_tui::TranscriptScrollView::default(),
        }
        .prompt_height(),
    );
    let inner = wonder_of_u_tui::Rect::new(
        layout.prompt.x.saturating_add(2),
        layout.prompt.y.saturating_add(1),
        layout.prompt.width.saturating_sub(4),
        layout.prompt.height.saturating_sub(2),
    );
    let query_prefix = "search: ".chars().count();
    (
        inner
            .x
            .saturating_add(
                u16::try_from(query_prefix.saturating_add(query_cursor)).unwrap_or(u16::MAX),
            )
            .min(inner.right().saturating_sub(1)),
        inner.y,
    )
}

pub(super) fn default_session_title(cwd: &Path) -> String {
    let leaf = cwd
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace");
    format!("Interactive: {leaf}")
}

pub(super) fn compose_conversation_prompt(messages: &[MessageEnvelope], input: &str) -> String {
    let history = messages
        .iter()
        .filter_map(conversation_line)
        .collect::<Vec<_>>();
    if history.is_empty() {
        return input.to_string();
    }

    let mut prompt = String::from(
        "Continue the conversation below. Use the prior assistant and user context when it is relevant.\n\n",
    );
    for line in history
        .into_iter()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        prompt.push_str(&line);
        prompt.push('\n');
    }
    prompt.push_str("user: ");
    prompt.push_str(&normalize_inline(input));
    prompt.push_str("\nassistant:");
    prompt
}

pub(super) fn conversation_line(message: &MessageEnvelope) -> Option<String> {
    match &message.payload {
        MessagePayload::UserText { content } => {
            Some(format!("user: {}", normalize_inline(content)))
        }
        MessagePayload::AssistantText { content } => {
            Some(format!("assistant: {}", normalize_inline(content)))
        }
        MessagePayload::AssistantToolUse { tool, input, .. } => Some(format!(
            "assistant_tool[{tool}]: {}",
            normalize_inline(&serde_json::to_string(input).unwrap_or_default())
        )),
        MessagePayload::ToolResult {
            tool,
            success,
            content,
            ..
        } => Some(format!(
            "tool[{tool} {}]: {}",
            if *success { "ok" } else { "error" },
            normalize_inline(content)
        )),
        MessagePayload::Permission {
            tool,
            decision,
            reason,
        } => Some(format!(
            "permission[{tool} {decision}]: {}",
            normalize_inline(reason)
        )),
        MessagePayload::System { content } => {
            Some(format!("system: {}", normalize_inline(content)))
        }
        _ => None,
    }
}

pub(super) fn normalize_inline(text: &str) -> String {
    truncate_chars(
        &text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ⏎ "),
        320,
    )
}

pub(super) fn append_streamed_text(
    state: &mut AppState,
    assistant_index: usize,
    delta: &str,
) -> Result<()> {
    if delta.is_empty() {
        return Ok(());
    }
    let Some(message) = state.messages.get_mut(assistant_index) else {
        return Err(WonderError::internal("streamed assistant message missing"));
    };
    let MessagePayload::AssistantText { content } = &mut message.payload else {
        return Err(WonderError::internal(
            "streamed assistant placeholder was not an assistant message",
        ));
    };
    content.push_str(delta);
    Ok(())
}

pub(super) fn set_assistant_message_content(
    state: &mut AppState,
    assistant_index: usize,
    content: &str,
) -> Result<()> {
    let Some(message) = state.messages.get_mut(assistant_index) else {
        return Err(WonderError::internal("assistant message missing"));
    };
    let MessagePayload::AssistantText {
        content: assistant_content,
    } = &mut message.payload
    else {
        return Err(WonderError::internal(
            "assistant placeholder was not an assistant message",
        ));
    };
    assistant_content.clear();
    assistant_content.push_str(content);
    Ok(())
}

pub(super) fn tool_spec_to_provider_tool(spec: wonder_of_u_core::ToolSpec) -> ProviderToolSpec {
    ProviderToolSpec {
        name: spec.name,
        description: spec.description,
        input_schema: spec.input_schema,
    }
}

pub(super) fn render_provider_tool_result(result: &ToolResult) -> String {
    if result.success {
        result.content.clone()
    } else {
        format!("ERROR: {}", result.content)
    }
}

pub(super) fn permission_decision_label(decision: &PermissionDecision) -> &'static str {
    match decision {
        PermissionDecision::Allow { .. } => "allow",
        PermissionDecision::Ask { .. } => "ask",
        PermissionDecision::Deny { .. } => "deny",
    }
}

pub(super) fn task_status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::Running => "running",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Killed => "killed",
        TaskStatus::Cancelled => "cancelled",
    }
}

pub(super) fn task_notification_severity(status: TaskStatus) -> NotificationSeverity {
    match status {
        TaskStatus::Completed => NotificationSeverity::Success,
        TaskStatus::Failed | TaskStatus::Killed | TaskStatus::Cancelled => {
            NotificationSeverity::Warning
        }
        TaskStatus::Pending | TaskStatus::Running => NotificationSeverity::Info,
    }
}

pub(super) fn latest_terminal_task(
    tasks: &BTreeMap<wonder_of_u_core::TaskId, TaskState>,
) -> Option<&TaskState> {
    tasks
        .values()
        .filter(|task| task.status.is_terminal())
        .max_by(|left, right| {
            left.finished_at
                .unwrap_or(left.started_at)
                .cmp(&right.finished_at.unwrap_or(right.started_at))
                .then_with(|| left.started_at.cmp(&right.started_at))
        })
}

pub(super) fn next_task_notification<'a>(
    previous: &'a BTreeMap<wonder_of_u_core::TaskId, TaskState>,
    next: &'a BTreeMap<wonder_of_u_core::TaskId, TaskState>,
) -> Option<&'a TaskState> {
    next.values().find(|task| {
        let Some(previous_task) = previous.get(&task.id) else {
            return task.status.is_terminal();
        };
        previous_task.status != task.status && task.status.is_terminal()
    })
}

pub(super) fn task_notification_lines(task: &TaskState) -> Vec<String> {
    let mut lines = vec![format!(
        "[{}] {}",
        task_status_label(task.status),
        task.description
    )];
    if let Some(status_message) = task
        .status_message
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        lines.push(status_message.to_string());
    }
    if let Some(exit_code) = task.exit_code {
        lines.push(format!("exit code: {exit_code}"));
    }
    lines
}

pub(super) fn permission_required_status(tool: &str) -> String {
    format!("approval required: {tool}")
}

pub(super) fn permission_dialog_for_tool_call(
    tool_name: &str,
    input: &serde_json::Value,
    read_only: bool,
    destructive: bool,
    reason: String,
) -> DialogView {
    let request = PermissionRequest::new(tool_name)
        .read_only(read_only)
        .destructive(destructive);
    let summary = PermissionSummaryView::from_request(&request, input);
    let mut dialog = summary.to_dialog_view();
    dialog.title = format!("Permission: {}", summary.title);
    let reason = reason.trim();
    if !reason.is_empty() {
        dialog.body.insert(2, format!("Reason: {reason}"));
    }
    dialog
}

pub(super) fn tool_permission_flags(tool_name: &str) -> (bool, bool) {
    builtin_tool_registry()
        .ok()
        .and_then(|registry| registry.resolve(tool_name))
        .map(|tool| {
            let spec = tool.spec();
            (spec.read_only, spec.destructive)
        })
        .unwrap_or((false, false))
}

pub(super) fn restored_permission_dialog(
    messages: &[MessageEnvelope],
    pending_approval: Option<&PendingToolApprovalState>,
) -> Option<(DialogView, String)> {
    messages
        .iter()
        .rev()
        .find_map(|message| match &message.payload {
            MessagePayload::Permission {
                tool,
                decision,
                reason,
            } => {
                let (dialog, status_note) = match decision.as_str() {
                    "ask" if pending_approval.is_some() => {
                        let Some(pending) = pending_approval else {
                            unreachable!("pending approval checked above");
                        };
                        let call = &pending.pending_call.provider_call;
                        let (read_only, destructive) = tool_permission_flags(&call.tool_name);
                        (
                            permission_dialog_for_tool_call(
                                &call.tool_name,
                                &call.arguments,
                                read_only,
                                destructive,
                                reason.clone(),
                            ),
                            permission_required_status(&call.tool_name),
                        )
                    }
                    "ask" => (
                        DialogView::notice(
                            "Permission required",
                            [
                                format!("Tool `{tool}` requires approval."),
                                reason.clone(),
                                "Pending execution state is no longer available.".into(),
                            ],
                        ),
                        format!("pending approval unavailable: {tool}"),
                    ),
                    "deny" => (
                        DialogView::notice(
                            "Permission denied",
                            [format!("Tool `{tool}` was denied."), reason.clone()],
                        ),
                        format!("permission denied: {tool}"),
                    ),
                    other => (
                        DialogView::notice(
                            "Permission update",
                            [
                                format!("Tool `{tool}` permission is `{other}`."),
                                reason.clone(),
                            ],
                        ),
                        format!("permission update: {tool}"),
                    ),
                };
                Some((dialog, status_note))
            }
            _ => None,
        })
}

pub(super) fn restored_prompt_ui_state(
    messages: &[MessageEnvelope],
) -> Option<RestoredPromptUiState> {
    let MessagePayload::Command {
        input,
        output: Some(output),
    } = &messages.last()?.payload
    else {
        return None;
    };

    restored_command_ui_state(input, output)
}

pub(super) fn restored_command_ui_state(
    input: &str,
    output: &str,
) -> Option<RestoredPromptUiState> {
    if let Some(mut picker) = parse_permission_picker_state(output) {
        picker.original_input = input.to_string();
        return Some(RestoredPromptUiState::PermissionPicker(picker));
    }
    if let Some(mut picker) = parse_memory_picker_state(output) {
        picker.original_input = input.to_string();
        return Some(RestoredPromptUiState::MemoryPicker(picker));
    }
    if let Some(tag) = parse_tag_remove_confirmation(output) {
        return Some(RestoredPromptUiState::TagRemoval(TagRemovalState {
            original_input: input.to_string(),
            tag,
        }));
    }
    if let Some(mut picker) = parse_theme_picker_state(output) {
        picker.original_input = input.to_string();
        return Some(RestoredPromptUiState::ThemePicker(picker));
    }
    if let Some(mut picker) = parse_model_picker_state(output) {
        picker.original_input = input.to_string();
        return Some(RestoredPromptUiState::ModelPicker(picker));
    }
    if let Some((title, body, note)) = parse_known_notice(output) {
        return Some(RestoredPromptUiState::Notice {
            dialog: DialogView::notice(title, body),
            status_note: note.into(),
        });
    }
    if let Some(view_action) = parse_view_action_hint(Some(output)) {
        return Some(RestoredPromptUiState::Status(match view_action {
            ViewActionHint::Clear => "conversation cleared".into(),
            ViewActionHint::Compact => "conversation compacted".into(),
        }));
    }
    if parse_insights_hint(output) {
        return Some(RestoredPromptUiState::Status("insights queued".into()));
    }
    if let Some(enabled) = parse_fast_mode_hint(output) {
        return Some(RestoredPromptUiState::Status(format!(
            "fast {}",
            if enabled { "on" } else { "off" }
        )));
    }
    if let Some(enabled) = parse_brief_mode_hint(output) {
        return Some(RestoredPromptUiState::Status(format!(
            "brief {}",
            if enabled { "on" } else { "off" }
        )));
    }
    if let Some(effort) = parse_effort_hint(output) {
        return Some(RestoredPromptUiState::Status(format!("effort {effort}")));
    }
    if let Some(color) = parse_session_color_hint(output) {
        return Some(RestoredPromptUiState::Status(format!("color {color}")));
    }
    None
}

pub(super) fn pending_provider_call_from_runtime(
    call: &ProviderToolCall,
) -> PendingProviderToolCall {
    PendingProviderToolCall {
        call_id: call.call_id.clone(),
        tool_name: call.tool_name.clone(),
        arguments: call.arguments.clone(),
    }
}

pub(super) fn pending_provider_result_from_runtime(
    result: &ProviderToolResultMessage,
) -> PendingProviderToolResult {
    PendingProviderToolResult {
        call_id: result.call_id.clone(),
        content: result.content.clone(),
    }
}

pub(super) fn pending_round_from_runtime(
    round: &ToolConversationRound,
) -> PendingToolConversationRound {
    PendingToolConversationRound {
        assistant_text: round.assistant_text.clone(),
        calls: round
            .calls
            .iter()
            .map(pending_provider_call_from_runtime)
            .collect(),
        results: round
            .results
            .iter()
            .map(pending_provider_result_from_runtime)
            .collect(),
    }
}

pub(super) fn pending_local_call_from_runtime(call: &LocalToolCall) -> PendingLocalToolCall {
    PendingLocalToolCall {
        provider_call: pending_provider_call_from_runtime(&call.provider_call),
        use_id: call.use_id,
    }
}

pub(super) fn runtime_provider_call_from_pending(
    call: &PendingProviderToolCall,
) -> ProviderToolCall {
    ProviderToolCall {
        call_id: call.call_id.clone(),
        tool_name: call.tool_name.clone(),
        arguments: call.arguments.clone(),
    }
}

pub(super) fn runtime_provider_result_from_pending(
    result: &PendingProviderToolResult,
) -> ProviderToolResultMessage {
    ProviderToolResultMessage {
        call_id: result.call_id.clone(),
        content: result.content.clone(),
    }
}

pub(super) fn runtime_round_from_pending(
    round: &PendingToolConversationRound,
) -> ToolConversationRound {
    ToolConversationRound {
        assistant_text: round.assistant_text.clone(),
        calls: round
            .calls
            .iter()
            .map(runtime_provider_call_from_pending)
            .collect(),
        results: round
            .results
            .iter()
            .map(runtime_provider_result_from_pending)
            .collect(),
    }
}

pub(super) fn local_call_from_pending(call: &PendingLocalToolCall) -> LocalToolCall {
    LocalToolCall {
        provider_call: runtime_provider_call_from_pending(&call.provider_call),
        use_id: call.use_id,
    }
}

pub(super) fn prompt_history_entries(messages: &[MessageEnvelope]) -> Vec<String> {
    let mut entries = Vec::new();
    for message in messages.iter().rev() {
        if let MessagePayload::UserText { content } = &message.payload {
            let trimmed = content.trim();
            if !trimmed.is_empty() && !entries.iter().any(|entry| entry == trimmed) {
                entries.push(trimmed.to_string());
            }
        }
    }
    entries
}

pub(super) fn current_history_search_match<'a>(
    search: &HistorySearchState,
    entries: &'a [String],
) -> Option<&'a str> {
    search
        .matches
        .get(search.cursor)
        .and_then(|&index| entries.get(index))
        .map(String::as_str)
}

pub(super) fn runtime_label(provider: Option<&str>, model: Option<&str>) -> &'static str {
    let _ = model;
    match provider {
        Some("openai" | "anthropic" | "copilot") => "runtime=tool-loop",
        Some(_) => "runtime=non-streaming",
        None => "runtime=tool-loop-ready",
    }
}

pub(super) fn turn_state_label(state: TurnState) -> &'static str {
    match state {
        TurnState::Idle => "idle",
        TurnState::EditingInput => "editing",
        TurnState::CommandQueued => "slash",
        TurnState::ModelRequestActive => "model",
        TurnState::ToolPermissionPending => "permission",
        TurnState::ToolExecuting => "tool",
        TurnState::StreamingResponse => "streaming",
        TurnState::Interrupted => "interrupted",
        TurnState::Completed => "done",
    }
}

pub(super) fn parse_permission_mode_hint(value: &str) -> Option<PermissionMode> {
    match value {
        "default" => Some(PermissionMode::Default),
        "accept-edits" | "acceptEdits" => Some(PermissionMode::AcceptEdits),
        "bypass" | "bypass-permissions" | "bypassPermissions" => {
            Some(PermissionMode::BypassPermissions)
        }
        "dont-ask" | "dontAsk" => Some(PermissionMode::DontAsk),
        "plan" => Some(PermissionMode::Plan),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ViewActionHint {
    Clear,
    Compact,
}

pub(super) fn parse_view_action_hint(text: Option<&str>) -> Option<ViewActionHint> {
    match text?
        .lines()
        .find_map(|line| line.strip_prefix("view_action="))?
        .trim()
    {
        "clear" => Some(ViewActionHint::Clear),
        "compact" => Some(ViewActionHint::Compact),
        _ => None,
    }
}

pub(super) fn permission_mode_output_label(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "accept-edits",
        PermissionMode::BypassPermissions => "bypass-permissions",
        PermissionMode::DontAsk => "dont-ask",
        PermissionMode::Plan => "plan",
    }
}

pub(super) fn parse_vim_toggle_hint(text: &str) -> bool {
    text.lines()
        .find_map(|line| line.strip_prefix("vim_toggle="))
        .map(str::trim)
        == Some("true")
}

pub(super) fn parse_vim_mode_hint(text: &str) -> Option<VimMode> {
    match text
        .lines()
        .find_map(|line| line.strip_prefix("vim_mode="))?
        .trim()
    {
        "insert" => Some(VimMode::Insert),
        "normal" => Some(VimMode::Normal),
        _ => None,
    }
}

pub(super) fn parse_additional_working_directory_hint(
    text: &str,
) -> Option<AdditionalWorkingDirectory> {
    let path = text
        .lines()
        .find_map(|line| line.strip_prefix("add_dir="))
        .map(str::trim)
        .filter(|line| !line.is_empty())?;
    let source = match text
        .lines()
        .find_map(|line| line.strip_prefix("add_dir_source="))
        .map(str::trim)
        .unwrap_or("session_runtime")
    {
        "cli_arg" => PermissionRuleSource::CliArg,
        "command" => PermissionRuleSource::Command,
        "local" => PermissionRuleSource::Local,
        "project" => PermissionRuleSource::Project,
        "user" => PermissionRuleSource::User,
        _ => PermissionRuleSource::SessionRuntime,
    };
    Some(AdditionalWorkingDirectory::new(path, source))
}

pub(super) fn parse_permission_picker_state(text: &str) -> Option<PermissionPickerState> {
    let enabled = text
        .lines()
        .find_map(|line| line.strip_prefix("permission_picker="))?
        .trim()
        == "true";
    if !enabled {
        return None;
    }
    let options = text
        .lines()
        .filter_map(|line| line.strip_prefix("permission_option="))
        .filter_map(parse_permission_picker_option)
        .collect::<Vec<_>>();
    if options.is_empty() {
        return None;
    }
    let selected_index = options
        .iter()
        .position(|option| option.selected)
        .unwrap_or(0);
    Some(PermissionPickerState {
        original_input: "/permissions".into(),
        options,
        selected_index,
        query: TextBuffer::new(false),
    })
}

pub(super) fn parse_permission_picker_option(line: &str) -> Option<PermissionPickerOption> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(PermissionPickerOption {
        mode: parse_permission_mode_hint(value.get("mode")?.as_str()?)?,
        label: value.get("label")?.as_str()?.to_string(),
        description: value.get("description")?.as_str()?.to_string(),
        selected: value.get("selected")?.as_bool().unwrap_or(false),
    })
}

pub(super) fn parse_memory_picker_state(text: &str) -> Option<MemoryPickerState> {
    let enabled = text
        .lines()
        .find_map(|line| line.strip_prefix("memory_picker="))?
        .trim()
        == "true";
    if !enabled {
        return None;
    }
    let options = text
        .lines()
        .filter_map(|line| line.strip_prefix("memory_option="))
        .filter_map(parse_memory_picker_option)
        .collect::<Vec<_>>();
    if options.is_empty() {
        return None;
    }
    let selected_index = options
        .iter()
        .position(|option| option.selected)
        .unwrap_or(0);
    Some(MemoryPickerState {
        original_input: "/memory".into(),
        options,
        selected_index,
        query: TextBuffer::new(false),
    })
}

pub(super) fn parse_memory_picker_option(line: &str) -> Option<MemoryPickerOption> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(MemoryPickerOption {
        target: match value.get("target")?.as_str()? {
            "project" => crate::commands::project::MemoryTarget::Project,
            "user" => crate::commands::project::MemoryTarget::User,
            _ => return None,
        },
        label: value.get("label")?.as_str()?.to_string(),
        description: value.get("description")?.as_str()?.to_string(),
        path: PathBuf::from(value.get("path")?.as_str()?),
        selected: value.get("selected")?.as_bool().unwrap_or(false),
    })
}

pub(super) fn parse_theme_picker_state(text: &str) -> Option<ThemePickerState> {
    let enabled = text
        .lines()
        .find_map(|line| line.strip_prefix("theme_picker="))?
        .trim()
        == "true";
    if !enabled {
        return None;
    }
    let options = text
        .lines()
        .filter_map(|line| line.strip_prefix("theme_option="))
        .filter_map(parse_theme_picker_option)
        .collect::<Vec<_>>();
    if options.is_empty() {
        return None;
    }
    let selected_index = options
        .iter()
        .position(|option| option.selected)
        .unwrap_or(0);
    Some(ThemePickerState {
        original_input: "/theme".into(),
        options,
        selected_index,
        query: TextBuffer::new(false),
    })
}

pub(super) fn parse_theme_picker_option(line: &str) -> Option<ThemePickerOption> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(ThemePickerOption {
        theme: value.get("theme")?.as_str()?.to_string(),
        label: value.get("label")?.as_str()?.to_string(),
        description: value.get("description")?.as_str()?.to_string(),
        selected: value.get("selected")?.as_bool().unwrap_or(false),
    })
}

pub(super) fn parse_model_picker_state(text: &str) -> Option<ModelPickerState> {
    let enabled = text
        .lines()
        .find_map(|line| line.strip_prefix("model_picker="))?
        .trim()
        == "true";
    if !enabled {
        return None;
    }
    let mut options = text
        .lines()
        .filter_map(|line| line.strip_prefix("model_option="))
        .filter_map(parse_model_picker_option)
        .collect::<Vec<_>>();
    if options.is_empty() {
        return None;
    }
    let selected_index = options
        .iter()
        .position(|option| option.selected)
        .unwrap_or(0);
    if let Some(option) = options.get_mut(selected_index) {
        option.selected = true;
    }
    Some(ModelPickerState {
        original_input: "/model".into(),
        options,
        selected_index,
        query: TextBuffer::new(false),
    })
}

pub(super) fn parse_theme_hint(text: &str) -> Option<&str> {
    let value = text.lines().find_map(|line| line.strip_prefix("theme="))?;
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

pub(super) fn parse_session_color_hint(text: &str) -> Option<&str> {
    let value = text.lines().find_map(|line| line.strip_prefix("color="))?;
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

pub(super) fn parse_effort_hint(text: &str) -> Option<&str> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("effort_level="))?;
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

pub(super) fn parse_fast_mode_hint(text: &str) -> Option<bool> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("fast_mode="))?;
    match value.trim() {
        "true" | "on" | "enabled" => Some(true),
        "false" | "off" | "disabled" => Some(false),
        _ => None,
    }
}

pub(super) fn parse_brief_mode_hint(text: &str) -> Option<bool> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("brief_mode="))?;
    match value.trim() {
        "true" | "on" | "enabled" => Some(true),
        "false" | "off" | "disabled" => Some(false),
        _ => None,
    }
}

pub(super) fn parse_insights_hint(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim() == "insights_prompt_ready=true")
}

pub(super) fn parse_session_tags_hint(text: &str) -> Option<Vec<String>> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("session_tags="))?;
    Some(
        value
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(ToString::to_string)
            .collect(),
    )
}

pub(super) fn parse_tag_remove_confirmation(text: &str) -> Option<String> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("tag_remove_confirmation="))?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

pub(super) fn apply_picker_query_edit(query: &mut TextBuffer, resolved: ResolvedKey) -> bool {
    match resolved {
        ResolvedKey::InsertChar(ch) => {
            query.insert_char(ch);
            true
        }
        ResolvedKey::Edit(EditAction::InsertNewline) => false,
        ResolvedKey::Edit(action) => {
            query.apply_edit_action(action);
            true
        }
        ResolvedKey::System(_) | ResolvedKey::Vim(_) => false,
    }
}

pub(super) fn filtered_picker_indices<T>(
    query: &TextBuffer,
    options: &[T],
    searchable: impl Fn(&T) -> String,
) -> Vec<usize> {
    let Some(query) = normalized_picker_query(query) else {
        return (0..options.len()).collect();
    };
    options
        .iter()
        .enumerate()
        .filter_map(|(index, option)| {
            searchable(option)
                .to_ascii_lowercase()
                .contains(&query)
                .then_some(index)
        })
        .collect()
}

pub(super) fn normalized_picker_query(query: &TextBuffer) -> Option<String> {
    let query = query.text();
    let trimmed = query.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_ascii_lowercase())
}

pub(super) fn picker_query_label(query: &TextBuffer) -> String {
    let query = query.text();
    if query.trim().is_empty() {
        "(all)".into()
    } else {
        query
    }
}

pub(super) fn picker_dialog_header(
    label: &str,
    query: &TextBuffer,
    filtered_count: usize,
    total_count: usize,
) -> Vec<String> {
    vec![
        format!("{label}: {}", picker_query_label(query)),
        format!("Matches: {filtered_count}/{total_count}"),
    ]
}

pub(super) fn selected_picker_index(selected_index: usize, filtered: &[usize]) -> Option<usize> {
    filtered
        .iter()
        .find(|&&index| index == selected_index)
        .copied()
        .or_else(|| filtered.first().copied())
}

pub(super) fn sync_picker_selection(selected_index: &mut usize, filtered: &[usize]) {
    if let Some(index) = selected_picker_index(*selected_index, filtered) {
        *selected_index = index;
    }
}

pub(super) fn step_picker_selection(
    selected_index: usize,
    delta: isize,
    filtered: &[usize],
) -> usize {
    let Some(current_position) = filtered
        .iter()
        .position(|&index| index == selected_index)
        .or_else(|| (!filtered.is_empty()).then_some(0))
    else {
        return selected_index;
    };
    let next_position =
        (current_position as isize + delta).rem_euclid(filtered.len() as isize) as usize;
    filtered[next_position]
}

pub(super) fn picker_status_note(title: &str) -> String {
    format!("{title}: {PICKER_CONTROLS_NOTE}")
}

pub(super) fn history_search_status_note(has_match: bool) -> String {
    if has_match {
        format!("history search: {HISTORY_SEARCH_CONTROLS_NOTE}")
    } else {
        format!("history search: no matches; {HISTORY_SEARCH_CONTROLS_NOTE}")
    }
}

pub(super) fn overlay_closed_status(title: &str) -> String {
    format!("{} closed", title.to_ascii_lowercase())
}

pub(super) fn parse_known_notice(text: &str) -> Option<(String, Vec<String>, &'static str)> {
    [
        ("## Context Usage", "Context Usage", "context usage"),
        ("## Activity Stats", "Activity Stats", "activity stats"),
        ("## Usage", "Usage", "usage"),
        ("## Theme", "Theme", "theme"),
        ("## Color", "Color", "color"),
        ("## Fast", "Fast", "fast"),
        ("## Brief", "Brief", "brief"),
        ("## Effort", "Effort", "effort"),
        ("## Feedback", "Feedback", "feedback"),
        ("## Insights", "Insights", "insights"),
        ("## Upgrade", "Upgrade", "upgrade"),
        ("## Desktop", "Desktop", "desktop"),
        ("## Mobile", "Mobile", "mobile"),
        ("## Chrome", "Chrome", "chrome"),
        ("## IDE Integration", "IDE Integration", "ide integration"),
        ("## Release Notes", "Release Notes", "release notes"),
        ("## Version", "Version", "version"),
        ("## Hooks", "Hooks", "hooks"),
        ("## Keybindings", "Keybindings", "keybindings"),
        (
            "## Privacy Settings",
            "Privacy Settings",
            "privacy settings",
        ),
        ("## Terminal Setup", "Terminal Setup", "terminal setup"),
    ]
    .into_iter()
    .find_map(|(heading, title, note)| {
        parse_notice(text, heading, title).map(|(title, body)| (title, body, note))
    })
}

pub(super) fn parse_notice(
    text: &str,
    heading: &str,
    title: &str,
) -> Option<(String, Vec<String>)> {
    let mut lines = text.lines();
    if lines.next()?.trim() != heading {
        return None;
    }
    Some((
        title.into(),
        lines
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect(),
    ))
}

pub(super) fn parse_model_picker_option(line: &str) -> Option<ModelPickerOption> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(ModelPickerOption {
        provider: value.get("provider")?.as_str()?.to_string(),
        provider_display: value.get("provider_display")?.as_str()?.to_string(),
        model: value.get("model")?.as_str()?.to_string(),
        model_display: value.get("model_display")?.as_str()?.to_string(),
        default: value.get("default")?.as_bool().unwrap_or(false),
        selected: value.get("selected")?.as_bool().unwrap_or(false),
        auth: value.get("auth")?.as_str()?.to_string(),
    })
}

pub(super) fn parse_external_editor_request(
    text: &str,
    cwd: &Path,
) -> Option<ExternalEditorRequest> {
    let should_open = text
        .lines()
        .find_map(|line| {
            line.strip_prefix("open_external=")
                .or_else(|| line.strip_prefix("plan_open_external="))
                .or_else(|| line.strip_prefix("memory_open_external="))
        })?
        .trim()
        == "true";
    if !should_open {
        return None;
    }
    let path = text
        .lines()
        .find_map(|line| {
            line.strip_prefix("external_path=")
                .or_else(|| line.strip_prefix("plan_path="))
                .or_else(|| line.strip_prefix("memory_path="))
        })
        .map(str::trim)
        .filter(|line| !line.is_empty())?;
    Some(ExternalEditorRequest {
        cwd: cwd.to_path_buf(),
        path: PathBuf::from(path),
    })
}

/// Parses the `setup_menu=true` / `setup_item=<json>` payload emitted by the
/// `/setup` command and returns a ready-to-open [`SetupOverlayState`], or
/// `None` if the payload is absent or `setup_menu=false`.
pub(super) fn parse_setup_overlay_state(text: &str) -> Option<SetupOverlayState> {
    let enabled = text
        .lines()
        .find_map(|line| line.strip_prefix("setup_menu="))?
        .trim()
        == "true";
    if !enabled {
        return None;
    }
    let provider_label = text
        .lines()
        .find_map(|line| line.strip_prefix("provider_selection="))
        .unwrap_or("unconfigured")
        .trim()
        .to_string();
    let readiness_label = text
        .lines()
        .find_map(|line| line.strip_prefix("provider_readiness="))
        .unwrap_or("unknown")
        .trim()
        .to_string();
    let items: Vec<SetupItem> = text
        .lines()
        .filter_map(|line| line.strip_prefix("setup_item="))
        .filter_map(parse_setup_item)
        .collect();
    if items.is_empty() {
        return None;
    }
    Some(SetupOverlayState::new(items, provider_label, readiness_label))
}

/// Parses a single `setup_item=<json>` value into a [`SetupItem`].
pub(super) fn parse_setup_item(line: &str) -> Option<SetupItem> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let id = value.get("id")?.as_str()?.to_string();
    let label = value.get("label")?.as_str()?.to_string();
    let description = value.get("description")?.as_str()?.to_string();
    let command = value
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let action = action_for_item_id(&id, &command);
    Some(SetupItem {
        id,
        label,
        description,
        action,
    })
}
