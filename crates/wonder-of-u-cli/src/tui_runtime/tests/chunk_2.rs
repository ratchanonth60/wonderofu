#[test]
fn controller_enters_history_search_with_ctrl_r() {
    let dir = unique_test_dir("tui-history-search-enter");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.prompt = TextBuffer::from_text("draft", false);
    seed_prompt_history(&mut controller, &["first prompt", "second prompt"]);

    send_prompt_key(&mut controller, ctrl_r_key());

    let history = controller
        .history_search
        .as_ref()
        .expect("history search active");
    let view = controller.view();
    let overlay = view.history_search.as_ref().expect("history overlay");
    assert!(history.query.is_empty());
    assert_eq!(history.saved_buffer.text(), "draft");
    assert_eq!(controller.prompt.text(), "draft");
    assert_eq!(view.prompt, "second prompt");
    assert_eq!(overlay.match_text.as_deref(), Some("second prompt"));
    assert_eq!(overlay.match_total, 2);
    let expected_note = history_search_status_note(true);
    assert_eq!(
        controller.status_note.as_deref(),
        Some(expected_note.as_str())
    );
}

#[test]
fn controller_filters_history_search_with_substring_query() {
    let dir = unique_test_dir("tui-history-search-filter");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.prompt = TextBuffer::from_text("draft", false);
    seed_prompt_history(
        &mut controller,
        &["Ship docs", "Review parity closeout", "Ship checklist"],
    );

    send_prompt_key(&mut controller, ctrl_r_key());
    send_prompt_key(&mut controller, picker_key(KeyCode::Char('s')));
    send_prompt_key(&mut controller, picker_key(KeyCode::Char('H')));
    send_prompt_key(&mut controller, picker_key(KeyCode::Char('I')));

    let view = controller.view();
    let overlay = view.history_search.as_ref().expect("history overlay");
    assert_eq!(controller.prompt.text(), "draft");
    assert_eq!(overlay.query, "sHI");
    assert_eq!(overlay.match_total, 2);
    assert_eq!(overlay.match_text.as_deref(), Some("Ship docs"));
    assert_eq!(view.prompt, "Ship docs");
}

#[test]
fn controller_history_search_cycles_through_matches() {
    let dir = unique_test_dir("tui-history-search-cycle");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    seed_prompt_history(
        &mut controller,
        &["deploy plan", "draft plan", "plan update"],
    );

    send_prompt_key(&mut controller, ctrl_r_key());
    send_prompt_key(&mut controller, picker_key(KeyCode::Char('p')));
    send_prompt_key(&mut controller, picker_key(KeyCode::Char('l')));
    send_prompt_key(&mut controller, picker_key(KeyCode::Char('a')));
    send_prompt_key(&mut controller, picker_key(KeyCode::Char('n')));
    assert_eq!(
        controller
            .view()
            .history_search
            .and_then(|overlay| overlay.match_text)
            .as_deref(),
        Some("plan update")
    );

    send_prompt_key(&mut controller, ctrl_r_key());
    assert_eq!(
        controller
            .view()
            .history_search
            .and_then(|overlay| overlay.match_text)
            .as_deref(),
        Some("draft plan")
    );

    send_prompt_key(&mut controller, picker_key(KeyCode::Up));
    assert_eq!(
        controller
            .view()
            .history_search
            .and_then(|overlay| overlay.match_text)
            .as_deref(),
        Some("deploy plan")
    );

    send_prompt_key(&mut controller, picker_key(KeyCode::Down));
    assert_eq!(
        controller
            .view()
            .history_search
            .and_then(|overlay| overlay.match_text)
            .as_deref(),
        Some("draft plan")
    );
}

#[test]
fn controller_history_search_enter_accepts_match() {
    let dir = unique_test_dir("tui-history-search-accept");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.prompt = TextBuffer::from_text("draft", false);
    seed_prompt_history(&mut controller, &["first prompt", "second prompt"]);

    send_prompt_key(&mut controller, ctrl_r_key());
    send_prompt_key(&mut controller, picker_key(KeyCode::Char('f')));
    send_prompt_key(&mut controller, picker_key(KeyCode::Enter));

    assert!(controller.history_search.is_none());
    assert_eq!(controller.prompt.text(), "first prompt");
    assert_eq!(controller.prompt.cursor(), "first prompt".chars().count());
    assert!(controller.view().history_search.is_none());
}

#[test]
fn controller_history_search_esc_restores_prior_buffer() {
    let dir = unique_test_dir("tui-history-search-esc");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.prompt = TextBuffer::from_text("draft note", false);
    seed_prompt_history(&mut controller, &["first prompt", "second prompt"]);

    send_prompt_key(&mut controller, ctrl_r_key());
    send_prompt_key(&mut controller, picker_key(KeyCode::Char('f')));
    send_prompt_key(&mut controller, picker_key(KeyCode::Esc));

    assert!(controller.history_search.is_none());
    assert_eq!(controller.prompt.text(), "draft note");
    assert!(controller.view().history_search.is_none());
}

#[test]
fn controller_history_search_reports_no_matches() {
    let dir = unique_test_dir("tui-history-search-no-match");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.prompt = TextBuffer::from_text("draft", false);
    seed_prompt_history(&mut controller, &["first prompt", "second prompt"]);

    send_prompt_key(&mut controller, ctrl_r_key());
    for ch in ['z', 'z', 'z'] {
        send_prompt_key(&mut controller, picker_key(KeyCode::Char(ch)));
    }

    let view = controller.view();
    let overlay = view.history_search.as_ref().expect("history overlay");
    assert_eq!(overlay.match_total, 0);
    assert_eq!(overlay.match_text, None);
    assert_eq!(view.prompt, "draft");
    let expected_note = history_search_status_note(false);
    assert_eq!(
        controller.status_note.as_deref(),
        Some(expected_note.as_str())
    );
}

#[test]
fn controller_history_search_with_empty_history_is_safe() {
    let dir = unique_test_dir("tui-history-search-empty");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.prompt = TextBuffer::from_text("draft", false);

    send_prompt_key(&mut controller, ctrl_r_key());
    send_prompt_key(&mut controller, picker_key(KeyCode::Up));
    send_prompt_key(&mut controller, picker_key(KeyCode::Enter));

    let view = controller.view();
    let overlay = view.history_search.as_ref().expect("history overlay");
    assert_eq!(overlay.match_total, 0);
    assert_eq!(overlay.match_text, None);
    assert_eq!(controller.prompt.text(), "draft");
    let expected_note = history_search_status_note(false);
    assert_eq!(
        controller.status_note.as_deref(),
        Some(expected_note.as_str())
    );
}

#[test]
fn controller_restores_permission_dialog_from_snapshot_resume() {
    let dir = unique_test_dir("tui-resume-permission-dialog");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut state = AppState::new(dir.clone());
    state.input_mode = InputMode::PermissionPending;
    state.pending_tool_approval = Some(PendingToolApprovalState {
        request_prompt: "write the note".into(),
        rounds: Vec::new(),
        current_round: PendingToolConversationRound {
            assistant_text: None,
            calls: vec![PendingProviderToolCall {
                call_id: "call_bash".into(),
                tool_name: "bash".into(),
                arguments: serde_json::json!({ "command": "echo hi" }),
                thought_signature: None,
            }],
            results: Vec::new(),
        },
        pending_call: PendingLocalToolCall {
            provider_call: PendingProviderToolCall {
                call_id: "call_bash".into(),
                tool_name: "bash".into(),
                arguments: serde_json::json!({ "command": "echo hi" }),
                thought_signature: None,
            },
            use_id: ToolUseId::new(),
        },
        remaining_calls: Vec::new(),
        reason: "workspace write requires approval".into(),
    });
    let permission = MessageEnvelope::new(
        state.session.id,
        MessagePayload::Permission {
            tool: "bash".into(),
            decision: "ask".into(),
            reason: "workspace write requires approval".into(),
        },
    );
    state
        .push_message(permission.clone())
        .expect("push permission");
    let store = TranscriptStore::new(&dir);
    store
        .write_metadata(
            &wonder_of_u_storage::SessionMetadata::from_state_with_transcript(&state, 1),
        )
        .expect("write metadata");
    store
        .append_message(&permission)
        .expect("append transcript");
    store
        .write_snapshot(&wonder_of_u_storage::SessionSnapshot::from_app_state(
            &state, 1, 0,
        ))
        .expect("write snapshot");

    let controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions {
            session_id: Some(state.session.id.to_string()),
        },
    )
    .expect("controller");

    assert_eq!(controller.state.input_mode, InputMode::PermissionPending);
    assert_eq!(controller.turn_state, TurnState::ToolPermissionPending);
    assert_eq!(
        controller.status_note.as_deref(),
        Some("approval required: bash")
    );
    let dialog = controller.view().dialog.expect("permission dialog");
    assert_eq!(dialog.title, "Permission: Run shell command");
    assert_eq!(dialog.body[0], "Allow Claude to run this shell command?");
    assert!(dialog.body.iter().any(|line| line == "Access: destructive"));
    assert!(
        dialog
            .body
            .iter()
            .any(|line| line == "Reason: workspace write requires approval")
    );
    assert!(dialog.body.iter().any(|line| line == "Command: echo hi"));
}

#[test]
fn controller_restores_task_notice_from_snapshot_resume() {
    let dir = unique_test_dir("tui-resume-task-dialog");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut state = AppState::new(dir.clone());
    state.input_mode = InputMode::TaskNotification;
    let mut task = TaskState::pending("run tests");
    task.mark_finished(
        TaskStatus::Completed,
        Some(0),
        Some("all tests passed".into()),
    );
    state.upsert_task(task.clone());
    let message = MessageEnvelope::system(state.session.id, "session resumed");
    state.push_message(message.clone()).expect("push message");
    let store = TranscriptStore::new(&dir);
    store
        .write_metadata(
            &wonder_of_u_storage::SessionMetadata::from_state_with_transcript(&state, 1),
        )
        .expect("write metadata");
    store.append_message(&message).expect("append transcript");
    store
        .write_snapshot(&wonder_of_u_storage::SessionSnapshot::from_app_state(
            &state, 1, 0,
        ))
        .expect("write snapshot");

    let controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions {
            session_id: Some(state.session.id.to_string()),
        },
    )
    .expect("controller");

    assert_eq!(controller.state.input_mode, InputMode::TaskNotification);
    assert_eq!(controller.turn_state, TurnState::Completed);
    assert_eq!(
        controller.status_note.as_deref(),
        Some("task completed: run tests")
    );
    let dialog = controller.view().dialog.expect("task dialog");
    assert_eq!(dialog.title, "Task update");
    assert!(dialog.body.iter().any(|line| line.contains("run tests")));
    assert!(
        dialog
            .body
            .iter()
            .any(|line| line.contains("all tests passed"))
    );
}

#[test]
fn controller_restores_notice_dialog_from_snapshot_resume() {
    let dir = unique_test_dir("tui-resume-notice-dialog");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut state = AppState::new(dir.clone());
    let message = MessageEnvelope::new(
        state.session.id,
        MessagePayload::Command {
            input: "/context".into(),
            output: Some("## Context Usage\nTokens: 42\nWindow: 8 messages\n".into()),
        },
    );
    state.push_message(message.clone()).expect("push command");
    let store = TranscriptStore::new(&dir);
    store
        .write_metadata(
            &wonder_of_u_storage::SessionMetadata::from_state_with_transcript(&state, 1),
        )
        .expect("write metadata");
    store.append_message(&message).expect("append transcript");
    store
        .write_snapshot(&wonder_of_u_storage::SessionSnapshot::from_app_state(
            &state, 1, 0,
        ))
        .expect("write snapshot");

    let controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions {
            session_id: Some(state.session.id.to_string()),
        },
    )
    .expect("controller");

    assert_eq!(controller.state.input_mode, InputMode::Prompt);
    assert_eq!(controller.turn_state, TurnState::Completed);
    assert_eq!(controller.status_note.as_deref(), Some("context usage"));
    let dialog = controller.view().dialog.expect("restored notice dialog");
    assert_eq!(dialog.title, "Context Usage");
    assert!(dialog.body.iter().any(|line| line.contains("Tokens: 42")));
}

#[test]
fn controller_restores_status_note_from_snapshot_resume() {
    let dir = unique_test_dir("tui-resume-status-note");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut state = AppState::new(dir.clone());
    state.set_session_color(Some("purple".into()));
    let message = MessageEnvelope::new(
        state.session.id,
        MessagePayload::Command {
            input: "/color purple".into(),
            output: Some("color=purple\nstatus=color updated\n".into()),
        },
    );
    state.push_message(message.clone()).expect("push command");
    let store = TranscriptStore::new(&dir);
    store
        .write_metadata(
            &wonder_of_u_storage::SessionMetadata::from_state_with_transcript(&state, 1),
        )
        .expect("write metadata");
    store.append_message(&message).expect("append transcript");
    store
        .write_snapshot(&wonder_of_u_storage::SessionSnapshot::from_app_state(
            &state, 1, 0,
        ))
        .expect("write snapshot");

    let controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions {
            session_id: Some(state.session.id.to_string()),
        },
    )
    .expect("controller");

    assert_eq!(controller.state.session_color.as_deref(), Some("purple"));
    assert_eq!(controller.status_note.as_deref(), Some("color purple"));
    assert!(controller.view().dialog.is_none());
    assert!(controller.view().footer.contains("shift+tab to cycle"));
}

#[test]
fn controller_preserves_restored_permission_mode_on_resume() {
    let dir = unique_test_dir("tui-resume-permission-mode");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut state = AppState::new(dir.clone());
    state.permission_mode = PermissionMode::Plan;
    let store = TranscriptStore::new(&dir);
    store.ensure_layout().expect("ensure layout");
    std::fs::write(store.paths().transcript_path(state.session.id), "").expect("write transcript");
    store
        .write_metadata(
            &wonder_of_u_storage::SessionMetadata::from_state_with_transcript(&state, 0),
        )
        .expect("write metadata");
    store
        .write_snapshot(&wonder_of_u_storage::SessionSnapshot::from_app_state(
            &state, 0, 0,
        ))
        .expect("write snapshot");

    let controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions {
            session_id: Some(state.session.id.to_string()),
        },
    )
    .expect("controller");

    assert_eq!(controller.state.permission_mode, PermissionMode::Plan);
    // Compact footer always shows the active permission mode label.
    assert!(
        controller.view().footer.contains("plan"),
        "compact footer must contain permission mode label: {}",
        controller.view().footer,
    );
}

#[test]
fn controller_preserves_restored_provider_selection_on_resume() {
    let dir = unique_test_dir("tui-resume-provider-selection");
    SettingsStore::new(&dir)
        .write(&AgentSettings {
            selected_provider: Some("openai".into()),
            selected_model: Some("gpt-4.1".into()),
            ..AgentSettings::default()
        })
        .expect("write settings");
    CredentialStore::new(&dir)
        .write(&StoredCredentials {
            providers: [
                (
                    "openai".into(),
                    AuthMaterial::ApiKey {
                        key: "openai-key".into(),
                    },
                ),
                (
                    "anthropic".into(),
                    AuthMaterial::ApiKey {
                        key: "anthropic-key".into(),
                    },
                ),
            ]
            .into(),
        })
        .expect("write credentials");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut state = AppState::new(dir.clone());
    state.set_provider_context(
        Some("anthropic".into()),
        Some("claude-sonnet-4-6".into()),
        AuthState::default(),
    );
    let store = TranscriptStore::new(&dir);
    store.ensure_layout().expect("ensure layout");
    std::fs::write(store.paths().transcript_path(state.session.id), "").expect("write transcript");
    store
        .write_metadata(
            &wonder_of_u_storage::SessionMetadata::from_state_with_transcript(&state, 0),
        )
        .expect("write metadata");
    store
        .write_snapshot(&wonder_of_u_storage::SessionSnapshot::from_app_state(
            &state, 0, 0,
        ))
        .expect("write snapshot");

    let controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions {
            session_id: Some(state.session.id.to_string()),
        },
    )
    .expect("controller");

    assert_eq!(controller.state.provider.as_deref(), Some("anthropic"));
    assert_eq!(controller.state.model.as_deref(), Some("claude-sonnet-4-6"));
    assert!(controller.state.auth.is_ready());
    // Provider/runtime details now live in state rather than the compact footer.
    assert!(controller.view().footer.contains("shift+tab to cycle"));
}

#[test]
fn prompt_cursor_tracks_edit_position_inside_prompt_panel() {
    // CHROME_HEIGHT=1: available=9, prompt_height=4 (rounded single line plus
    // integrated footer). messages=5, prompt at y=5, content_x=1, content_y=6.
    // cursor=2 → after "ab" → line=0, col=2, x_offset=2 → (1+2+2, 6) = (5, 6).
    let (x, y) = prompt_cursor_position(40, 10, "abc", 2, false, false);
    assert_eq!((x, y), (5, 6));
}

/// Verify that enabling brief mode injects the hint into the system prompt
/// field exactly once and never into the messages array.
#[test]
fn controller_brief_mode_injects_system_prompt_once() {
    let dir = unique_test_dir("tui-brief-system-prompt");
    let (api_base, handle) = spawn_json_sequence_server(
        |_index, _headers, body| {
            let system = body["system"].as_str().expect("system field present");
            let brief_hint = "Be brief.";
            assert!(
                system.contains(brief_hint),
                "system prompt should contain the brief hint"
            );
            assert_eq!(
                system.matches(brief_hint).count(),
                1,
                "brief hint should appear exactly once in system prompt"
            );
            let messages = body["messages"].as_array().expect("messages array");
            assert_eq!(messages.len(), 1, "exactly one user message");
            assert_eq!(messages[0]["role"].as_str(), Some("user"));
            // The brief hint must NOT appear in the user message content.
            let content_text = messages[0]["content"][0]["text"]
                .as_str()
                .unwrap_or_default();
            assert!(
                !content_text.contains(brief_hint),
                "brief hint must not appear in user message content"
            );
        },
        vec![
            json!({
                "id": "msg_brief_test_1",
                "type": "message",
                "role": "assistant",
                "content": [{"type": "text", "text": "brief reply"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 5, "output_tokens": 2}
            })
            .to_string(),
        ],
    );
    write_provider_config_for(&dir, "anthropic", "claude-sonnet-4-6", &api_base);
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/brief")).expect("enable brief");
    assert!(controller.state.brief_mode, "brief mode should be enabled");

    controller
        .state
        .queue_command("hello", wonder_of_u_core::QueuePlacement::Now);
    drain_and_pump(&mut controller);

    handle.join().expect("server join");
}

/// Verify that the brief flag survives a `/clear` command (i.e. it is
/// persisted in the session snapshot and restored on reload).
#[test]
fn controller_brief_flag_persists_across_clear() {
    let dir = unique_test_dir("tui-brief-persists-clear");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/brief")).expect("enable brief");
    assert!(controller.state.brief_mode, "brief mode should be on");

    block_on(controller.execute_slash_command("/clear")).expect("clear session");

    assert!(
        controller.state.brief_mode,
        "brief mode should persist across /clear"
    );
}

// ── ratatui TestBackend snapshot tests ──────────────────────────────────

/// Renders a `ShellView` into a ratatui `TestBackend` at the given size and
/// returns each row as a plain-text string (no ANSI codes) so tests can
/// make simple string assertions without depending on exact cell styles.
fn render_to_test_backend(
    width: u16,
    height: u16,
    view: &wonder_of_u_tui::ShellView,
    theme: &wonder_of_u_tui::Theme,
) -> Vec<String> {
    use ratatui::{Terminal, backend::TestBackend};

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| {
            render_to_ratatui_frame(frame, view, theme);
        })
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect()
        })
        .collect()
}

#[test]
fn ratatui_empty_repl_renders_header_and_status() {
    // An empty ShellView should still render a header and status line.
    let view = wonder_of_u_tui::ShellView {
        title: "Test Session".into(),
        status: "claude-3-5-sonnet · default".into(),
        footer: "Ctrl+C to quit".into(),
        ..wonder_of_u_tui::ShellView::default()
    };
    let theme = wonder_of_u_tui::Theme::default();
    let rows = render_to_test_backend(60, 20, &view, &theme);

    // Status row is removed (CHROME_HEIGHT=1); the compact footer row renders the footer field.
    // There must be at least one row containing the session title.
    let has_title = rows.iter().any(|r| r.contains("Test Session"));
    assert!(
        has_title,
        "title 'Test Session' not found in rendered output"
    );

    // The single compact footer row must contain the footer text set in the view.
    let has_footer = rows.iter().any(|r| r.contains("Ctrl+C"));
    assert!(has_footer, "footer text not found in rendered output");
}

#[test]
fn ratatui_prompt_text_appears_in_frame() {
    // A non-empty prompt string should appear verbatim in the rendered frame.
    let view = wonder_of_u_tui::ShellView {
        title: String::new(),
        prompt: "hello world".into(),
        status: "model · default".into(),
        footer: String::new(),
        ..wonder_of_u_tui::ShellView::default()
    };
    let theme = wonder_of_u_tui::Theme::default();
    let rows = render_to_test_backend(80, 24, &view, &theme);

    let has_prompt = rows.iter().any(|r| r.contains("hello world"));
    assert!(has_prompt, "prompt text 'hello world' not found in frame");
}

#[test]
fn ratatui_message_text_appears_in_frame() {
    use wonder_of_u_tui::{MessageLineView, MessageRole};

    // A user message line should be visible in the messages panel.
    let view = wonder_of_u_tui::ShellView {
        title: "S".into(),
        messages: vec![MessageLineView {
            role: MessageRole::User,
            text: "test user message".into(),
            spans: Vec::new(),
            highlight: false,
        }],
        status: "m".into(),
        footer: "f".into(),
        ..wonder_of_u_tui::ShellView::default()
    };
    let theme = wonder_of_u_tui::Theme::default();
    let rows = render_to_test_backend(80, 24, &view, &theme);

    let has_msg = rows.iter().any(|r| r.contains("test user message"));
    assert!(has_msg, "user message text not found in frame");
}

#[test]
fn ratatui_frame_fills_full_terminal_size() {
    // Every row must have exactly `width` characters — no short rows.
    let width: u16 = 72;
    let height: u16 = 18;
    let view = wonder_of_u_tui::ShellView::default();
    let theme = wonder_of_u_tui::Theme::default();
    let rows = render_to_test_backend(width, height, &view, &theme);

    assert_eq!(rows.len(), height as usize, "wrong number of rows");
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(
            row.chars().count(),
            width as usize,
            "row {i} has wrong width"
        );
    }
}

#[test]
fn ratatui_midnight_theme_renders_without_panic() {
    // Verify that the midnight theme variant flows through without panicking.
    let view = wonder_of_u_tui::ShellView {
        title: "midnight".into(),
        prompt: "type here".into(),
        status: "claude · default".into(),
        footer: "hints".into(),
        ..wonder_of_u_tui::ShellView::default()
    };
    let theme = theme_for_state(Some("midnight"), None);
    // Should not panic.
    let rows = render_to_test_backend(80, 24, &view, &theme);
    assert_eq!(rows.len(), 24);
}

// ── TranscriptScrollState controller integration ──────────────────────────────

#[test]
fn controller_scroll_state_starts_in_follow_tail_mode() {
    let dir = unique_test_dir("tui-scroll-init");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    assert!(
        controller.scroll_state.is_following_tail(),
        "new controller should start in follow-tail mode"
    );
    assert_eq!(controller.scroll_state.offset_from_bottom, 0);
}

#[test]
fn controller_scroll_state_updates_total_lines_after_slash_command() {
    let dir = unique_test_dir("tui-scroll-msg-update");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // No messages yet; total_lines starts at 0.
    assert_eq!(controller.scroll_state.last_total_lines, 0);

    // Record a command message to the transcript (slash commands themselves
    // no longer write transcript entries; picker confirmations do).
    controller
        .record_command_message("/model", Some("provider_selection=openai:gpt-4.1"))
        .expect("record command message");

    // After the message is persisted the scroll state must update.
    assert!(
        controller.scroll_state.last_total_lines > 0,
        "total_lines should be > 0 after a message is added"
    );
}

#[test]
fn controller_scroll_state_stays_following_tail_after_messages() {
    let dir = unique_test_dir("tui-scroll-follow-tail");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // Send several commands to accumulate messages.
    for cmd in &[
        "/model openai:gpt-4.1",
        "/theme default",
        "/model openai:gpt-4.1",
    ] {
        block_on(controller.execute_slash_command(cmd)).expect("slash cmd");
    }

    // Never scrolled up → still following tail.
    assert!(
        controller.scroll_state.is_following_tail(),
        "should remain in follow-tail mode when no manual scroll has occurred"
    );
}

#[test]
fn controller_scroll_state_preserves_scrolled_offset_after_new_messages() {
    let dir = unique_test_dir("tui-scroll-preserve");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // Generate some messages.
    for _ in 0..3 {
        block_on(controller.execute_slash_command("/model openai:gpt-4.1")).expect("slash cmd");
    }

    // Simulate a small viewport so max_offset is non-zero.
    controller.scroll_state.last_visible_lines = 1;

    let initial_total = controller.scroll_state.last_total_lines;
    if initial_total > 1 {
        controller.scroll_state.scroll_by(1);
        assert!(!controller.scroll_state.is_following_tail());
        let offset_before = controller.scroll_state.offset_from_bottom;

        // Add another message.
        block_on(controller.execute_slash_command("/model openai:gpt-4.1")).expect("slash cmd");

        // Offset is preserved (or clamped to new max), not reset to 0.
        assert_eq!(
            controller.scroll_state.offset_from_bottom, offset_before,
            "scrolled-up offset should be preserved when new messages arrive"
        );
    }
}

#[test]
fn controller_resize_event_updates_scroll_state_visible_lines() {
    let dir = unique_test_dir("tui-scroll-resize");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.handle_event(            UiEvent::Resize {
                width: 80,
                height: 40,
            },)).expect("handle resize");

    assert!(
        controller.scroll_state.last_visible_lines > 0,
        "resize should set last_visible_lines to a positive value (got {})",
        controller.scroll_state.last_visible_lines
    );
}

#[test]
fn controller_resize_event_preserves_follow_tail_mode() {
    let dir = unique_test_dir("tui-scroll-resize-tail");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.handle_event(            UiEvent::Resize {
                width: 80,
                height: 24,
            },)).expect("handle resize");

    assert!(
        controller.scroll_state.is_following_tail(),
        "resize should not exit follow-tail mode when no scroll has occurred"
    );
}

// ---------------------------------------------------------------------------
// Setup overlay tests
// ---------------------------------------------------------------------------

/// Runs `/setup` on a controller and returns the controller.  The test helper
/// registers a storage dir so that `SetupCommand` can load a provider report.
fn open_setup_overlay_controller() -> (TuiController<'static>, PathBuf) {
    // leak the registry so it has a 'static lifetime for the controller.
    let dir = unique_test_dir("tui-setup-overlay");
    let registry = Box::new(commands::registry(Some(dir.clone())).expect("registry"));
    let registry: &'static _ = Box::leak(registry);
    let mut controller = TuiController::new(
        test_context(&dir),
        registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    block_on(controller.execute_slash_command("/setup")).expect("execute /setup");
    (controller, dir)
}

#[test]
fn controller_setup_command_opens_setup_overlay() {
    let (controller, _dir) = open_setup_overlay_controller();

    assert!(
        controller.pending_setup_overlay.is_some(),
        "expected setup overlay to be open after /setup"
    );
    assert!(
        controller.pending_model_picker.is_none(),
        "model picker should not be open while setup overlay is open"
    );
}

#[test]
fn controller_setup_overlay_view_has_picker_list() {
    let (controller, _dir) = open_setup_overlay_controller();

    let view = controller.view();
    let picker = view
        .picker_list
        .as_ref()
        .expect("picker_list should be present while setup overlay is open");
    assert_eq!(picker.title, "Setup");
    assert!(
        !picker.entries.is_empty(),
        "setup overlay must have at least one entry"
    );
    // Exactly one entry should be selected.
    let selected_count = picker.entries.iter().filter(|e| e.selected).count();
    assert_eq!(
        selected_count, 1,
        "exactly one setup entry should be selected"
    );
}

#[test]
fn controller_setup_overlay_has_nine_items() {
    let (controller, _dir) = open_setup_overlay_controller();

    let overlay = controller
        .pending_setup_overlay
        .as_ref()
        .expect("setup overlay open");
    assert_eq!(
        overlay.items.len(),
        9,
        "setup overlay should have the 9 first-slice entries, got {}",
        overlay.items.len()
    );
}

#[test]
fn controller_setup_overlay_item_ids() {
    let (controller, _dir) = open_setup_overlay_controller();

    let overlay = controller
        .pending_setup_overlay
        .as_ref()
        .expect("setup overlay open");
    let ids: Vec<&str> = overlay.items.iter().map(|i| i.id.as_str()).collect();
    assert!(ids.contains(&"login"), "missing 'login' item");
    assert!(
        ids.contains(&"copilot-oauth"),
        "missing 'copilot-oauth' item"
    );
    assert!(ids.contains(&"model"), "missing 'model' item");
    assert!(ids.contains(&"api-base"), "missing 'api-base' item");
    assert!(ids.contains(&"theme"), "missing 'theme' item");
    assert!(ids.contains(&"permissions"), "missing 'permissions' item");
    assert!(
        ids.contains(&"terminal-setup"),
        "missing 'terminal-setup' item"
    );
    assert!(ids.contains(&"memory"), "missing 'memory' item");
    assert!(ids.contains(&"keybindings"), "missing 'keybindings' item");
}

#[test]
fn controller_setup_overlay_known_items_have_dispatch_action() {
    let (controller, _dir) = open_setup_overlay_controller();

    use crate::tui_runtime::setup::{ProviderFormKind, SetupItemAction};

    let overlay = controller
        .pending_setup_overlay
        .as_ref()
        .expect("setup overlay open");

    let dispatch_ids = [
        "model",
        "theme",
        "permissions",
        "memory",
        "terminal-setup",
        "keybindings",
    ];
    for id in &dispatch_ids {
        let item = overlay
            .items
            .iter()
            .find(|i| i.id == *id)
            .unwrap_or_else(|| panic!("missing item '{id}'"));
        assert!(
            matches!(&item.action, SetupItemAction::Dispatch(_)),
            "item '{id}' should have a Dispatch action, got {:?}",
            item.action
        );
    }

    // "login" and "api-base" now open the provider form (not a placeholder).
    let login = overlay
        .items
        .iter()
        .find(|i| i.id == "login")
        .expect("login item");
    assert!(
        matches!(
            &login.action,
            SetupItemAction::ProviderForm(ProviderFormKind::ApiKey)
        ),
        "login should be ProviderForm(ApiKey), got {:?}",
        login.action
    );
    let api_base = overlay
        .items
        .iter()
        .find(|i| i.id == "api-base")
        .expect("api-base item");
    assert!(
        matches!(
            &api_base.action,
            SetupItemAction::ProviderForm(ProviderFormKind::ApiBase)
        ),
        "api-base should be ProviderForm(ApiBase), got {:?}",
        api_base.action
    );
    // Copilot OAuth now has its own TUI flow.
    let oauth = overlay
        .items
        .iter()
        .find(|i| i.id == "copilot-oauth")
        .expect("copilot-oauth item");
    assert!(
        matches!(&oauth.action, SetupItemAction::CopilotOAuth),
        "copilot-oauth should be CopilotOAuth action, got {:?}",
        oauth.action
    );
}

#[test]
fn controller_setup_overlay_navigate_down_and_up_wraps() {
    let (mut controller, _dir) = open_setup_overlay_controller();

    // Navigate down through all items and back up; selection should wrap.
    let count = controller
        .pending_setup_overlay
        .as_ref()
        .expect("overlay open")
        .items
        .len();
    for _ in 0..count {
        send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
    }
    // After `count` downs from index 0, we should be back at 0.
    let idx = controller
        .pending_setup_overlay
        .as_ref()
        .expect("overlay open")
        .selected_index;
    assert_eq!(
        idx, 0,
        "selection should wrap back to 0 after {count} downs"
    );

    // Navigate up once: should wrap to the last item.
    send_dialog_key(&mut controller, picker_key(KeyCode::Up), None);
    let idx = controller
        .pending_setup_overlay
        .as_ref()
        .expect("overlay open")
        .selected_index;
    assert_eq!(
        idx,
        count - 1,
        "one Up from index 0 should wrap to last item ({count}-1)"
    );
}

#[test]
fn controller_setup_overlay_esc_cancels() {
    let (mut controller, _dir) = open_setup_overlay_controller();

    send_dialog_key(
        &mut controller,
        KeyEvent {
            code: KeyCode::Esc,
            modifiers: wonder_of_u_tui::KeyModifiers::default(),
        },
        None,
    );

    assert!(
        controller.pending_setup_overlay.is_none(),
        "setup overlay should be dismissed after Esc"
    );
    assert_eq!(
        controller.status_note.as_deref(),
        Some("setup cancelled"),
        "status note should indicate cancellation"
    );
}

#[test]
fn controller_setup_overlay_enter_on_model_dispatches_model_picker() {
    let (mut controller, _dir) = open_setup_overlay_controller();

    // Find the "model" item index.
    let model_idx = {
        let overlay = controller
            .pending_setup_overlay
            .as_ref()
            .expect("overlay open");
        overlay
            .items
            .iter()
            .position(|i| i.id == "model")
            .expect("model item present")
    };

    // Navigate to the model item.
    for _ in 0..model_idx {
        send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
    }

    // Confirm with Enter.
    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    // After confirming model, the setup overlay should be gone and the model picker should open.
    assert!(
        controller.pending_setup_overlay.is_none(),
        "setup overlay should close after selecting an item"
    );
    assert!(
        controller.pending_model_picker.is_some(),
        "model picker should open after selecting the model item"
    );
}

#[test]
fn controller_setup_overlay_enter_on_theme_dispatches_theme_picker() {
    let (mut controller, _dir) = open_setup_overlay_controller();

    let theme_idx = {
        let overlay = controller
            .pending_setup_overlay
            .as_ref()
            .expect("overlay open");
        overlay
            .items
            .iter()
            .position(|i| i.id == "theme")
            .expect("theme item present")
    };
    for _ in 0..theme_idx {
        send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
    }
    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    assert!(controller.pending_setup_overlay.is_none());
    assert!(
        controller.pending_theme_picker.is_some(),
        "theme picker should open"
    );
}

#[test]
fn controller_setup_overlay_enter_on_permissions_dispatches_permission_picker() {
    let (mut controller, _dir) = open_setup_overlay_controller();

    let perm_idx = {
        let overlay = controller
            .pending_setup_overlay
            .as_ref()
            .expect("overlay open");
        overlay
            .items
            .iter()
            .position(|i| i.id == "permissions")
            .expect("permissions item present")
    };
    for _ in 0..perm_idx {
        send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
    }
    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    assert!(controller.pending_setup_overlay.is_none());
    assert!(
        controller.pending_permission_picker.is_some(),
        "permission picker should open"
    );
}

#[test]
fn controller_setup_overlay_enter_on_memory_dispatches_memory_picker() {
    let (mut controller, _dir) = open_setup_overlay_controller();

    let mem_idx = {
        let overlay = controller
            .pending_setup_overlay
            .as_ref()
            .expect("overlay open");
        overlay
            .items
            .iter()
            .position(|i| i.id == "memory")
            .expect("memory item present")
    };
    for _ in 0..mem_idx {
        send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
    }
    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    assert!(controller.pending_setup_overlay.is_none());
    assert!(
        controller.pending_memory_picker.is_some(),
        "memory picker should open"
    );
}

#[test]
fn controller_setup_overlay_enter_on_placeholder_item_shows_notice() {
    let (mut controller, _dir) = open_setup_overlay_controller();

    // "copilot-oauth" now opens the real OAuth device-code flow (no longer a Placeholder).
    // Navigating to that item and pressing Enter triggers `open_copilot_oauth_flow()`.
    // In a test environment without network access the device-code request fails and
    // a "Copilot Login Failed" notice is shown; on a live network the confirmation
    // dialog is shown instead.  Either way, the setup overlay is closed and a dialog
    // (not a provider form) is displayed.
    let copilot_idx = {
        let overlay = controller
            .pending_setup_overlay
            .as_ref()
            .expect("overlay open");
        overlay
            .items
            .iter()
            .position(|i| i.id == "copilot-oauth")
            .expect("copilot-oauth item present")
    };
    for _ in 0..copilot_idx {
        send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
    }
    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    // Setup overlay should be gone — `open_copilot_oauth_flow` clears it.
    assert!(
        controller.pending_setup_overlay.is_none(),
        "setup overlay should close after selecting copilot-oauth item"
    );
    // A dialog should be shown (either the confirmation or the error).
    assert!(
        controller.dialog.is_some(),
        "a dialog should appear when copilot-oauth is selected"
    );
    assert!(
        controller.pending_provider_form.is_none(),
        "provider form should NOT open for a CopilotOAuth item"
    );
}

#[test]
fn controller_setup_overlay_does_not_record_command_message_while_open() {
    let (controller, _dir) = open_setup_overlay_controller();

    // No message for "/setup" should appear while the overlay is open
    // (it's deferred until the overlay is dismissed).
    let has_setup_command = controller.state.messages.iter().any(|msg| {
        matches!(
            &msg.payload,
            MessagePayload::Command { input, .. } if input.trim() == "/setup"
        )
    });
    assert!(
        !has_setup_command,
        "/setup command message should not be recorded while the overlay is open"
    );
}

#[test]
fn controller_setup_overlay_tab_key_selects_item() {
    let (mut controller, _dir) = open_setup_overlay_controller();

    // Tab on first item (index 0: "login" → placeholder).
    send_dialog_key(&mut controller, picker_key(KeyCode::Tab), None);

    // Overlay should be dismissed and a dialog shown (placeholder case).
    assert!(
        controller.pending_setup_overlay.is_none(),
        "Tab should confirm and close the setup overlay"
    );
}

#[test]
fn controller_setup_overlay_handles_prompt_keys_in_vim_normal_mode() {
    let (mut controller, _dir) = open_setup_overlay_controller();
    controller.vim = VimState::new(VimMode::Normal);

    send_prompt_key(&mut controller, picker_key(KeyCode::Down));
    assert_eq!(
        controller
            .pending_setup_overlay
            .as_ref()
            .expect("setup overlay remains open")
            .selected_index,
        1,
        "setup overlay should receive navigation before Vim normal mode"
    );

    send_prompt_key(&mut controller, picker_key(KeyCode::Up));
    assert_eq!(
        controller
            .pending_setup_overlay
            .as_ref()
            .expect("setup overlay remains open")
            .selected_index,
        0
    );

    send_prompt_key(&mut controller, picker_key(KeyCode::Enter));

    assert!(
        controller.pending_setup_overlay.is_none(),
        "Enter should confirm the selected setup item in Vim normal mode"
    );
    assert!(
        controller.pending_provider_form.is_some(),
        "the login setup item should open the provider form"
    );
}

// ── Scroll-key routing tests ──────────────────────────────────────────────────

/// Build a controller that has had an initial resize so `last_visible_lines` is
/// set and page-scroll math works (40-line transcript, 10-line viewport).
fn controller_with_scroll_dims() -> (TuiController<'static>, PathBuf) {
    let dir = unique_test_dir("tui-scroll-keys");
    write_provider_config(&dir, "http://127.0.0.1:1/v1");
    let registry = Box::new(commands::registry(Some(dir.clone())).expect("registry"));
    let registry: &'static _ = Box::leak(registry);
    let mut ctrl = TuiController::new(
        test_context(&dir),
        registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    ctrl.scroll_state.on_resize(10, 40);
    ctrl.scroll_state.scroll_to_top();
    (ctrl, dir)
}

/// Build a `KeyEvent` with `Ctrl` held.
fn ctrl_key(code: KeyCode) -> KeyEvent {
    use wonder_of_u_tui::KeyModifiers;
    KeyEvent {
        code,
        modifiers: KeyModifiers {
            control: true,
            shift: false,
            alt: false,
        },
    }
}

#[test]
fn controller_page_up_scrolls_one_page_toward_top() {
    let (mut ctrl, _dir) = controller_with_scroll_dims();
    ctrl.scroll_state.scroll_to_bottom();
    let before = ctrl.scroll_state.offset_from_bottom;

    send_prompt_key(
        &mut ctrl,
        KeyEvent {
            code: KeyCode::PageUp,
            modifiers: Default::default(),
        },
    );

    assert!(
        ctrl.scroll_state.offset_from_bottom > before,
        "PageUp should increase offset_from_bottom (scroll toward older content)"
    );
}

#[test]
fn controller_page_down_scrolls_one_page_toward_bottom() {
    let (mut ctrl, _dir) = controller_with_scroll_dims();
    ctrl.scroll_state.scroll_to_top();
    let before = ctrl.scroll_state.offset_from_bottom;

    send_prompt_key(
        &mut ctrl,
        KeyEvent {
            code: KeyCode::PageDown,
            modifiers: Default::default(),
        },
    );

    assert!(
        ctrl.scroll_state.offset_from_bottom < before,
        "PageDown should decrease offset_from_bottom (scroll toward newer content)"
    );
}

#[test]
fn controller_page_down_from_tail_stays_at_zero() {
    let (mut ctrl, _dir) = controller_with_scroll_dims();
    ctrl.scroll_state.scroll_to_bottom();

    send_prompt_key(
        &mut ctrl,
        KeyEvent {
            code: KeyCode::PageDown,
            modifiers: Default::default(),
        },
    );

    assert_eq!(
        ctrl.scroll_state.offset_from_bottom, 0,
        "PageDown from tail should stay at 0 (saturating sub)"
    );
}

#[test]
fn controller_ctrl_home_jumps_to_top() {
    let (mut ctrl, _dir) = controller_with_scroll_dims();
    ctrl.scroll_state.scroll_to_bottom();
    let expected = ctrl
        .scroll_state
        .last_total_lines
        .saturating_sub(ctrl.scroll_state.last_visible_lines);

    send_prompt_key(&mut ctrl, ctrl_key(KeyCode::Home));

    assert_eq!(
        ctrl.scroll_state.offset_from_bottom, expected,
        "Ctrl+Home should jump to max offset (oldest content)"
    );
}

#[test]
fn controller_ctrl_end_returns_to_tail() {
    let (mut ctrl, _dir) = controller_with_scroll_dims();
    ctrl.scroll_state.scroll_to_top();

    send_prompt_key(&mut ctrl, ctrl_key(KeyCode::End));

    assert_eq!(
        ctrl.scroll_state.offset_from_bottom, 0,
        "Ctrl+End should return to follow-tail (offset = 0)"
    );
}

#[test]
fn controller_scroll_keys_no_op_while_dialog_overlay_active() {
    let (mut ctrl, _dir) = controller_with_scroll_dims();
    // Open the model picker — sets self.dialog = Some(...)
    for ch in ['/', 'm', 'o', 'd', 'e', 'l'] {
        send_prompt_key(
            &mut ctrl,
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: Default::default(),
            },
        );
    }
    send_prompt_key(
        &mut ctrl,
        KeyEvent {
            code: KeyCode::Enter,
            modifiers: Default::default(),
        },
    );

    ctrl.scroll_state.scroll_to_bottom();
    let before = ctrl.scroll_state.offset_from_bottom;

    send_prompt_key(
        &mut ctrl,
        KeyEvent {
            code: KeyCode::PageUp,
            modifiers: Default::default(),
        },
    );

    if ctrl.dialog.is_some() {
        assert_eq!(
            ctrl.scroll_state.offset_from_bottom, before,
            "PageUp must be a no-op while a dialog overlay is active"
        );
    }
}

#[test]
fn controller_scroll_keys_no_op_while_setup_overlay_active() {
    let (mut ctrl, _dir) = controller_with_scroll_dims();
    // Open the setup overlay via /setup.
    for ch in ['/', 's', 'e', 't', 'u', 'p'] {
        send_prompt_key(
            &mut ctrl,
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: Default::default(),
            },
        );
    }
    send_prompt_key(
        &mut ctrl,
        KeyEvent {
            code: KeyCode::Enter,
            modifiers: Default::default(),
        },
    );

    ctrl.scroll_state.scroll_to_bottom();
    let before = ctrl.scroll_state.offset_from_bottom;

    send_prompt_key(
        &mut ctrl,
        KeyEvent {
            code: KeyCode::PageUp,
            modifiers: Default::default(),
        },
    );

    if ctrl.pending_setup_overlay.is_some() {
        assert_eq!(
            ctrl.scroll_state.offset_from_bottom, before,
            "PageUp must be a no-op while the setup overlay is active"
        );
    }
}

#[test]
fn controller_plain_home_still_edits_prompt_not_scroll() {
    let (mut ctrl, _dir) = controller_with_scroll_dims();
    for ch in ['h', 'e', 'l', 'l', 'o'] {
        send_prompt_key(
            &mut ctrl,
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: Default::default(),
            },
        );
    }
    ctrl.scroll_state.scroll_to_bottom();

    send_prompt_key(
        &mut ctrl,
        KeyEvent {
            code: KeyCode::Home,
            modifiers: Default::default(),
        },
    );

    assert_eq!(
        ctrl.scroll_state.offset_from_bottom, 0,
        "Plain Home should not alter scroll offset"
    );
}

#[test]
fn controller_plain_end_still_edits_prompt_not_scroll() {
    let (mut ctrl, _dir) = controller_with_scroll_dims();
    ctrl.scroll_state.scroll_to_top();
    let top_offset = ctrl.scroll_state.offset_from_bottom;

    send_prompt_key(
        &mut ctrl,
        KeyEvent {
            code: KeyCode::End,
            modifiers: Default::default(),
        },
    );

    assert_eq!(
        ctrl.scroll_state.offset_from_bottom, top_offset,
        "Plain End should not alter scroll offset"
    );
}

// ── tui-scroll-mouse: mouse-wheel transcript navigation ──────────────────────

