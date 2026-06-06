#[test]
fn controller_opens_memory_picker_for_bare_memory_command() {
    let dir = unique_test_dir("tui-memory-picker-open");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/memory")).expect("open memory picker");

    assert_eq!(
        controller.status_note.as_deref(),
        Some("memory: type to filter, use Up/Down to choose, Tab/Enter to select, Esc to cancel")
    );
    assert!(controller.pending_memory_picker.is_some());
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Memory"
    ));
    assert!(!controller.state.messages.iter().any(|message| {
        matches!(
            &message.payload,
            MessagePayload::Command { input, .. } if input == "/memory"
        )
    }));
}

#[test]
fn controller_keeps_theme_picker_open_when_search_has_no_matches() {
    let dir = unique_test_dir("tui-theme-picker-no-matches");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/theme")).expect("open theme picker");
    for ch in ['z', 'z', 'z'] {
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Char(ch)),
            Some(ResolvedKey::InsertChar(ch)),
        );
    }
    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    let picker = controller
        .pending_theme_picker
        .as_ref()
        .expect("theme picker still open");
    let dialog = controller.dialog.as_ref().expect("dialog");
    let expected_matches = format!("Matches: 0/{}", picker.options.len());
    assert_eq!(dialog.body.first().map(String::as_str), Some("Search: zzz"));
    assert_eq!(
        dialog.body.get(1).map(String::as_str),
        Some(expected_matches.as_str())
    );
    assert_eq!(
        dialog.body.get(2).map(String::as_str),
        Some("No matching themes.")
    );
    assert_eq!(
        controller.status_note.as_deref(),
        Some("theme picker: no matching option to select")
    );
}

#[test]
fn controller_filters_theme_picker_with_fuzzy_query() {
    let dir = unique_test_dir("tui-theme-picker-fuzzy");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/theme")).expect("open theme picker");
    for ch in ['m', 'd', 'n', 'g', 'h', 't'] {
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Char(ch)),
            Some(ResolvedKey::InsertChar(ch)),
        );
    }

    let picker = controller
        .pending_theme_picker
        .as_ref()
        .expect("theme picker still open");
    let dialog = controller.dialog.as_ref().expect("dialog");
    let expected_matches = format!("Matches: 1/{}", picker.options.len());
    assert_eq!(
        dialog.body.first().map(String::as_str),
        Some("Search: mdnght")
    );
    assert_eq!(
        dialog.body.get(1).map(String::as_str),
        Some(expected_matches.as_str())
    );
    assert!(dialog.body.iter().any(|line| line.contains("midnight")));
}

#[test]
fn controller_filters_memory_picker_with_search_query() {
    let dir = unique_test_dir("tui-memory-picker-filter");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/memory")).expect("open memory picker");
    for ch in ['u', 's', 'e', 'r'] {
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Char(ch)),
            Some(ResolvedKey::InsertChar(ch)),
        );
    }

    let picker = controller
        .pending_memory_picker
        .as_ref()
        .expect("memory picker open");
    let dialog = controller.dialog.as_ref().expect("dialog");
    let expected_matches = format!("Matches: 2/{}", picker.options.len());
    assert_eq!(
        dialog.body.first().map(String::as_str),
        Some("Search: user")
    );
    assert_eq!(
        dialog.body.get(1).map(String::as_str),
        Some(expected_matches.as_str())
    );
    assert!(dialog.body.iter().any(|line| line.contains("User memory")));
    assert!(
        dialog
            .body
            .iter()
            .any(|line| line.contains("Project memory"))
    );
}

#[test]
fn controller_filters_memory_picker_with_fuzzy_query() {
    let dir = unique_test_dir("tui-memory-picker-fuzzy-filter");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/memory")).expect("open memory picker");
    for ch in ['u', 's', 'r'] {
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Char(ch)),
            Some(ResolvedKey::InsertChar(ch)),
        );
    }

    let picker = controller
        .pending_memory_picker
        .as_ref()
        .expect("memory picker open");
    let dialog = controller.dialog.as_ref().expect("dialog");
    let expected_matches = format!("Matches: {}/{}", picker.options.len(), picker.options.len());
    assert_eq!(dialog.body.first().map(String::as_str), Some("Search: usr"));
    assert_eq!(
        dialog.body.get(1).map(String::as_str),
        Some(expected_matches.as_str())
    );
    assert!(
        dialog
            .body
            .get(2)
            .is_some_and(|line| line.contains("User memory"))
    );
    assert!(
        dialog
            .body
            .iter()
            .any(|line| line.contains("Project memory"))
    );
}

#[test]
fn controller_selects_memory_target_from_picker() {
    let dir = unique_test_dir("tui-memory-picker-select");
    let _editor = EnvVarGuard::set("EDITOR", "vi");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/memory")).expect("open picker");
    send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    assert!(controller.pending_memory_picker.is_none());
    assert!(controller.dialog.is_none());
    assert_eq!(
        controller.pending_external_editor.as_ref(),
        Some(&ExternalEditorRequest {
            cwd: dir.clone(),
            path: dir.join("config/CLAUDE.md"),
        })
    );
    assert_eq!(
        controller.status_note.as_deref(),
        Some("opening file in editor")
    );
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/memory"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("memory_target=user"))
    ));
}

#[test]
fn controller_cancels_memory_picker() {
    let dir = unique_test_dir("tui-memory-picker-cancel");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/memory")).expect("open picker");
    block_on(controller.handle_dialog_key(
        KeyEvent {
            code: KeyCode::Esc,
            modifiers: wonder_of_u_tui::KeyModifiers::default(),
        },
        None,
    ))
    .expect("cancel picker");

    assert!(controller.pending_memory_picker.is_none());
    assert!(controller.dialog.is_none());
    assert_eq!(
        controller.status_note.as_deref(),
        Some("memory picker cancelled")
    );
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/memory"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("status=memory picker cancelled"))
    ));
}

#[test]
fn controller_opens_tag_removal_confirmation_for_matching_tag() {
    let dir = unique_test_dir("tui-tag-remove-confirm");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.state.set_session_tags(vec!["bugfix".into()]);
    controller
        .persist_state_snapshot()
        .expect("persist tagged state");

    block_on(controller.execute_slash_command("/tag bugfix")).expect("open tag removal dialog");

    assert!(matches!(
        controller.pending_tag_removal.as_ref(),
        Some(pending) if pending.tag == "bugfix"
    ));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Remove tag?"
    ));
    assert!(!controller.state.messages.iter().any(|message| {
        matches!(
            &message.payload,
            MessagePayload::Command { input, .. } if input == "/tag bugfix"
        )
    }));
}

#[test]
fn controller_confirms_tag_removal() {
    let dir = unique_test_dir("tui-tag-remove-complete");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.state.set_session_tags(vec!["bugfix".into()]);
    controller
        .persist_state_snapshot()
        .expect("persist tagged state");
    block_on(controller.execute_slash_command("/tag bugfix")).expect("open tag removal dialog");

    block_on(controller.handle_dialog_key(
        KeyEvent {
            code: KeyCode::Enter,
            modifiers: wonder_of_u_tui::KeyModifiers::default(),
        },
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    ))
    .expect("confirm tag removal");

    assert!(controller.pending_tag_removal.is_none());
    assert!(controller.state.session.tags.is_empty());
    assert_eq!(controller.status_note.as_deref(), Some("removed #bugfix"));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/tag bugfix"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("session_tags="))
    ));
}

#[test]
fn controller_shows_context_notice_dialog() {
    let dir = unique_test_dir("tui-context-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/context")).expect("show context");

    assert_eq!(controller.status_note.as_deref(), Some("context usage"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Context Usage"
    ));
    // /context output goes to dialog, not transcript.
    assert!(
        !matches!(
            controller.state.messages.last().map(|m| &m.payload),
            Some(MessagePayload::Command { input, .. }) if input == "/context"
        ),
        "/context must not record to transcript"
    );
}

#[test]
fn controller_records_session_stats_in_transcript() {
    let dir = unique_test_dir("tui-stats-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.state.provider = Some("openai".into());
    controller.state.model = Some("gpt-4.1".into());
    controller.state.record_cost_usage(
        TokenUsage {
            input_tokens: 128,
            output_tokens: 32,
            cache_creation_tokens: 16,
            cache_read_tokens: 8,
        },
        Some(0.42),
    );

    block_on(controller.execute_slash_command("/stats")).expect("show stats");

    assert_eq!(controller.status_note.as_deref(), Some("session stats"));
    // /stats output goes to dialog, not transcript.
    assert!(
        matches!(controller.dialog.as_ref(), Some(dialog) if dialog.title == "/stats"),
        "dialog must open for /stats; got: {:?}",
        controller.dialog
    );
    assert!(
        !matches!(
            controller.state.messages.last().map(|m| &m.payload),
            Some(MessagePayload::Command { input, .. }) if input == "/stats"
        ),
        "/stats must not record to transcript"
    );
}

#[test]
fn controller_records_help_with_system_styled_rows() {
    let dir = unique_test_dir("tui-help-command");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/help")).expect("show help");

    assert_eq!(controller.status_note.as_deref(), Some("help"));
    // /help output goes to dialog, not transcript.
    assert!(
        matches!(controller.dialog.as_ref(), Some(dialog) if dialog.title == "/help"),
        "dialog must open for /help; got: {:?}",
        controller.dialog
    );
    assert!(
        !matches!(
            controller.state.messages.last().map(|m| &m.payload),
            Some(MessagePayload::Command { input, .. }) if input == "/help"
        ),
        "/help must not record to transcript"
    );
}

#[test]
fn thinking_slash_command_toggles_and_reports_state() {
    let dir = unique_test_dir("tui-thinking-slash");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/thinking")).expect("report thinking");
    assert!(!controller.state.thinking_enabled);
    assert!(
        controller
            .status_note
            .as_deref()
            .is_some_and(|s| s.starts_with("thinking off")),
        "expected status to start with 'thinking off', got {:?}",
        controller.status_note
    );
    // /thinking is a toggle — output goes to status_note only, not transcript.
    assert!(
        controller.state.messages.is_empty(),
        "/thinking must not record to transcript"
    );

    block_on(controller.execute_slash_command("/thinking on")).expect("enable thinking");
    assert!(controller.state.thinking_enabled);
    assert!(
        controller
            .status_note
            .as_deref()
            .is_some_and(|s| s.starts_with("thinking on")),
        "expected status to start with 'thinking on', got {:?}",
        controller.status_note
    );

    block_on(controller.execute_slash_command("/thinking off")).expect("disable thinking");
    assert!(!controller.state.thinking_enabled);
    assert!(
        controller
            .status_note
            .as_deref()
            .is_some_and(|s| s.starts_with("thinking off")),
        "expected status to start with 'thinking off', got {:?}",
        controller.status_note
    );
}

#[test]
fn controller_meta_t_toggles_thinking_mode() {
    let dir = unique_test_dir("tui-thinking-meta-toggle");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.pending_setup_overlay = None;
    controller.dialog = None;

    send_prompt_key(&mut controller, alt_key('t'));
    assert!(controller.state.thinking_enabled);
    assert_eq!(controller.status_note.as_deref(), Some("thinking on"));
    // /thinking toggle goes to status_note only, not transcript.
    assert!(controller.state.messages.is_empty(), "/thinking must not record to transcript");

    send_prompt_key(&mut controller, alt_key('t'));
    assert!(!controller.state.thinking_enabled);
    assert_eq!(controller.status_note.as_deref(), Some("thinking off"));
}

#[test]
fn controller_meta_o_toggles_fast_mode() {
    let dir = unique_test_dir("tui-fast-meta-toggle");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.pending_setup_overlay = None;
    controller.dialog = None;

    send_prompt_key(&mut controller, alt_key('o'));
    assert!(controller.state.fast_mode);
    assert_eq!(controller.status_note.as_deref(), Some("fast on"));
    // /fast output is machine hints only; no dialog and no transcript entry.
    assert!(controller.dialog.is_none(), "/fast must not open dialog");
    assert!(
        controller.state.messages.is_empty(),
        "/fast must not record to transcript"
    );

    send_prompt_key(&mut controller, alt_key('o'));
    assert!(!controller.state.fast_mode);
    assert_eq!(controller.status_note.as_deref(), Some("fast off"));
}

#[test]
fn controller_meta_p_opens_model_picker() {
    let dir = unique_test_dir("tui-model-meta-picker");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.pending_setup_overlay = None;
    controller.dialog = None;

    send_prompt_key(&mut controller, alt_key('p'));

    assert!(controller.pending_model_picker.is_some());
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Model picker"
    ));
}

#[test]
fn controller_meta_e_toggles_tool_output_expansion() {
    let dir = unique_test_dir("tui-tool-output-meta-expand");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.pending_setup_overlay = None;
    controller.dialog = None;

    send_prompt_key(&mut controller, alt_key('e'));
    assert!(controller.expand_tool_output);
    assert_eq!(
        controller.status_note.as_deref(),
        Some("tool output expanded")
    );

    send_prompt_key(&mut controller, alt_key('e'));
    assert!(!controller.expand_tool_output);
    assert_eq!(
        controller.status_note.as_deref(),
        Some("tool output collapsed")
    );
}

#[test]
fn controller_shows_usage_notice_dialog() {
    let dir = unique_test_dir("tui-usage-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/usage")).expect("show usage");

    assert_eq!(controller.status_note.as_deref(), Some("usage"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Usage"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/usage"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Usage"))
    ));
}

#[test]
fn controller_shows_keybindings_notice_dialog() {
    let dir = unique_test_dir("tui-keybindings-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/keybindings")).expect("show keybindings");

    assert_eq!(controller.status_note.as_deref(), Some("keybindings"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Keybindings"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/keybindings"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Keybindings"))
    ));
}

#[test]
fn controller_shows_hooks_notice_dialog() {
    let dir = unique_test_dir("tui-hooks-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/hooks")).expect("show hooks");

    assert_eq!(controller.status_note.as_deref(), Some("hooks"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Hooks"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/hooks"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Hooks"))
    ));
}

#[test]
fn controller_shows_privacy_settings_notice_dialog() {
    let dir = unique_test_dir("tui-privacy-settings-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/privacy-settings")).expect("show privacy settings");

    assert_eq!(controller.status_note.as_deref(), Some("privacy settings"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Privacy Settings"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/privacy-settings"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Privacy Settings"))
    ));
}

#[test]
fn settings_slash_command_records_configuration_and_usage() {
    let dir = unique_test_dir("tui-settings-slash");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.state.provider = Some("anthropic".into());
    controller.state.model = Some("claude-3-5-sonnet-20241022".into());
    controller.state.auth = AuthState::ready(
        wonder_of_u_core::AuthMaterialKind::ApiKey,
        wonder_of_u_core::AuthSource::Environment,
    );
    controller.state.set_context_window_size(Some(200_000));
    controller.state.record_cost_usage(
        TokenUsage {
            input_tokens: 12_450,
            output_tokens: 3_821,
            cache_creation_tokens: 1_200,
            cache_read_tokens: 8_100,
        },
        Some(0.0412),
    );

    block_on(controller.execute_slash_command("/settings")).expect("show settings");

    assert_eq!(controller.status_note.as_deref(), Some("settings"));
    assert!(controller.dialog.is_none());
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/settings"
                && output.as_deref().is_some_and(|text| {
                    text.contains("Configuration")
                        && text.contains("Session Usage")
                        && text.contains("Provider Status")
                        && text.contains("$0.0412")
                })
    ));
}

#[test]
fn controller_shows_terminal_setup_notice_dialog() {
    let dir = unique_test_dir("tui-terminal-setup-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/terminal-setup")).expect("show terminal setup");

    assert_eq!(controller.status_note.as_deref(), Some("terminal setup"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Terminal Setup"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/terminal-setup"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Terminal Setup"))
    ));
}

#[test]
fn controller_toggles_vim_mode_from_slash_command() {
    let dir = unique_test_dir("tui-vim-command");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    assert_eq!(controller.vim.mode(), VimMode::Insert);

    block_on(controller.execute_slash_command("/vim")).expect("toggle vim");
    assert_eq!(controller.vim.mode(), VimMode::Normal);
    assert_eq!(controller.status_note.as_deref(), Some("vim normal"));

    block_on(controller.execute_slash_command("/vim insert")).expect("set vim insert");
    assert_eq!(controller.vim.mode(), VimMode::Insert);
    assert_eq!(controller.status_note.as_deref(), Some("vim insert"));
}

#[test]
fn controller_opens_permissions_picker() {
    let dir = unique_test_dir("tui-permissions-picker-open");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/permissions")).expect("open permissions picker");

    assert_eq!(
        controller.status_note.as_deref(),
        Some(
            "permission mode: type to filter, use Up/Down to choose, Tab/Enter to select, Esc to cancel"
        )
    );
    assert!(controller.pending_permission_picker.is_some());
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Permission mode"
    ));
    assert!(!controller.state.messages.iter().any(|message| {
        matches!(
            &message.payload,
            MessagePayload::Command { input, .. } if input == "/permissions"
        )
    }));
}

#[test]
fn controller_clears_permission_picker_search_back_to_full_list() {
    let dir = unique_test_dir("tui-permissions-picker-clear-search");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/permissions")).expect("open permissions picker");
    for ch in ['p', 'l', 'a', 'n'] {
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Char(ch)),
            Some(ResolvedKey::InsertChar(ch)),
        );
    }
    for _ in 0..4 {
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Backspace),
            Some(ResolvedKey::Edit(EditAction::Backspace)),
        );
    }

    let picker = controller
        .pending_permission_picker
        .as_ref()
        .expect("permission picker open");
    let dialog = controller.dialog.as_ref().expect("dialog");
    let expected_matches = format!("Matches: {0}/{0}", picker.options.len());
    assert_eq!(
        dialog.body.first().map(String::as_str),
        Some("Search: (all)")
    );
    assert_eq!(
        dialog.body.get(1).map(String::as_str),
        Some(expected_matches.as_str())
    );
    assert!(dialog.body.iter().any(|line| line.contains("Default")));
    assert!(dialog.body.iter().any(|line| line.contains("Plan")));
}

#[test]
fn controller_selects_permission_mode_from_picker() {
    let dir = unique_test_dir("tui-permissions-picker-select");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/permissions")).expect("open permissions picker");
    send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    assert!(controller.pending_permission_picker.is_none());
    assert!(controller.dialog.is_none());
    assert_eq!(
        controller.state.permission_mode,
        PermissionMode::AcceptEdits
    );
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/permissions"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("permission_mode=accept-edits"))
    ));
}

#[test]
fn controller_routes_plan_mode_slash_commands() {
    let dir = unique_test_dir("tui-slash-plan");
    std::fs::write(dir.join("plan.md"), "# queued plan\n- keep parity\n").expect("write plan");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/plan")).expect("enter plan mode");
    assert_eq!(controller.state.permission_mode, PermissionMode::Plan);
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/plan"
                && output.as_deref().is_some_and(|text| text.contains("status=plan mode enabled"))
    ));

    block_on(controller.execute_slash_command("/plan")).expect("show current plan");
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/plan"
                && output.as_deref().is_some_and(|text| {
                    text.contains("plan_exists=true")
                        && text.contains("Current Plan")
                        && text.contains("- keep parity")
                })
    ));

    block_on(controller.execute_slash_command("/plan exit")).expect("exit plan mode");
    assert_eq!(controller.state.permission_mode, PermissionMode::Default);
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/plan exit"
                && output.as_deref().is_some_and(|text| text.contains("status=plan mode disabled"))
    ));
}

#[test]
fn controller_plan_exit_restores_accept_edits_origin() {
    // Entering plan mode from AcceptEdits and exiting should restore AcceptEdits,
    // not fall back to Default.
    let dir = unique_test_dir("tui-plan-restore-accept-edits");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let ctx = CommandContext {
        permission_mode: PermissionMode::AcceptEdits,
        ..test_context(&dir)
    };
    let mut controller = TuiController::new(
        ctx,
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    assert_eq!(
        controller.state.permission_mode,
        PermissionMode::AcceptEdits
    );

    block_on(controller.execute_slash_command("/plan")).expect("enter plan mode");
    assert_eq!(controller.state.permission_mode, PermissionMode::Plan);
    // Controller should have saved the pre-plan origin.
    assert_eq!(
        controller.pre_plan_permission_mode,
        Some(PermissionMode::AcceptEdits)
    );

    block_on(controller.execute_slash_command("/plan exit")).expect("exit plan mode");
    // Must restore AcceptEdits, not Default.
    assert_eq!(
        controller.state.permission_mode,
        PermissionMode::AcceptEdits,
        "exiting plan mode should restore the pre-plan AcceptEdits mode"
    );
    // Slot must be cleared after restoration.
    assert_eq!(controller.pre_plan_permission_mode, None);
}

#[test]
fn controller_plan_exit_restores_bypass_permissions_origin() {
    // BypassPermissions is a coordinator/passthrough mode — must survive the
    // plan-mode round-trip without downgrading to Default.
    let dir = unique_test_dir("tui-plan-restore-bypass");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let ctx = CommandContext {
        permission_mode: PermissionMode::BypassPermissions,
        ..test_context(&dir)
    };
    let mut controller = TuiController::new(
        ctx,
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/plan")).expect("enter plan mode");
    assert_eq!(controller.state.permission_mode, PermissionMode::Plan);
    assert_eq!(
        controller.pre_plan_permission_mode,
        Some(PermissionMode::BypassPermissions)
    );

    block_on(controller.execute_slash_command("/plan exit")).expect("exit plan mode");
    assert_eq!(
        controller.state.permission_mode,
        PermissionMode::BypassPermissions,
        "exiting plan mode should restore BypassPermissions, not Default"
    );
    assert_eq!(controller.pre_plan_permission_mode, None);
}

#[test]
fn controller_plan_exit_from_default_restores_default() {
    // Entering plan from Default and exiting must restore Default (the common
    // case; also validates no regression).
    let dir = unique_test_dir("tui-plan-restore-default");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/plan")).expect("enter plan mode");
    assert_eq!(controller.state.permission_mode, PermissionMode::Plan);

    block_on(controller.execute_slash_command("/plan exit")).expect("exit plan mode");
    assert_eq!(
        controller.state.permission_mode,
        PermissionMode::Default,
        "exiting plan mode entered from Default should restore Default"
    );
    assert_eq!(controller.pre_plan_permission_mode, None);
}

#[test]
fn controller_pre_plan_slot_cleared_on_unrelated_mode_change() {
    // If the user changes permission mode via a non-plan command while in plan
    // mode the pre-plan slot should be cleared so we don't carry stale state.
    let dir = unique_test_dir("tui-plan-slot-clear");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/plan")).expect("enter plan mode");
    assert_eq!(
        controller.pre_plan_permission_mode,
        Some(PermissionMode::Default)
    );

    // Directly set a non-plan permission mode (simulates `/permissions` command
    // changing mode outside the plan flow).
    block_on(controller.execute_slash_command("/permissions accept-edits")).expect("change mode mid-session");
    // AcceptEdits is not Plan, so the pre-plan slot should have been cleared.
    assert_eq!(
        controller.pre_plan_permission_mode, None,
        "pre_plan slot must be cleared when leaving plan mode via an unrelated mode change"
    );
}

#[test]
fn controller_executes_queued_plan_prompt() {
    let dir = unique_test_dir("tui-slash-plan-prompt");
    let (api_base, handle) = spawn_json_sequence_server(
        |_index, _headers, body| {
            assert_eq!(body["model"], "claude-sonnet-4-6");
            assert_eq!(
                body.pointer("/messages/0/content/0/text")
                    .and_then(Value::as_str),
                Some("draft the migration plan")
            );
        },
        vec![
            json!({
                "id": "msg_plan_prompt_1",
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "text",
                    "text": "queued plan reply"
                }],
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": 8,
                    "output_tokens": 4
                }
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

    let render_calls = 0usize;
    block_on(controller.execute_slash_command_with("/plan draft the migration plan"))
        .expect("execute plan prompt");

    handle.join().expect("server join");

    assert_eq!(controller.state.permission_mode, PermissionMode::Plan);
    let _ = render_calls;
    assert!(controller.state.messages.iter().any(|message| {
        matches!(
            &message.payload,
            MessagePayload::Command { input, output }
                if input == "/plan draft the migration plan"
                    && output.as_deref().is_some_and(|text| {
                        text.contains("status=plan mode enabled")
                            && text.contains("enqueue_prompt=draft the migration plan")
                    })
        )
    }));
    assert!(controller.state.messages.iter().any(|message| {
        matches!(
            &message.payload,
            MessagePayload::UserText { content } if content == "draft the migration plan"
        )
    }));
    assert!(controller.state.messages.iter().any(|message| {
        matches!(
            &message.payload,
            MessagePayload::AssistantText { content } if content == "queued plan reply"
        )
    }));
}

#[test]
fn queued_commands_view_updates_as_prompts_drain() {
    let dir = unique_test_dir("tui-queued-visibility");
    let (api_base, handle) = spawn_json_sequence_server(
        |index, _headers, body| {
            let prompt = body
                .pointer("/messages/0/content/0/text")
                .and_then(Value::as_str)
                .expect("prompt text");
            match index {
                0 => assert_eq!(prompt, "first queued prompt"),
                1 => assert!(prompt.contains("user: second queued prompt")),
                _ => panic!("unexpected request index {index}"),
            }
        },
        vec![
            json!({
                "id": "msg_queue_prompt_1",
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "text",
                    "text": "first queued reply"
                }],
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": 8,
                    "output_tokens": 4
                }
            })
            .to_string(),
            json!({
                "id": "msg_queue_prompt_2",
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "text",
                    "text": "second queued reply"
                }],
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": 8,
                    "output_tokens": 4
                }
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
    controller
        .state
        .queue_command("first queued prompt", wonder_of_u_core::QueuePlacement::Now);
    controller.state.queue_command(
        "second queued prompt",
        wonder_of_u_core::QueuePlacement::Later,
    );

    block_on(controller.drain_queued_commands()).expect("drain queued prompts");

    handle.join().expect("server join");

    assert!(controller.state.queued_commands.is_empty());
    assert!(controller.state.messages.iter().any(|message| {
        matches!(
            &message.payload,
            MessagePayload::AssistantText { content } if content == "second queued reply"
        )
    }));
}

#[test]
fn controller_queues_external_editor_for_plan_open() {
    let dir = unique_test_dir("tui-slash-plan-open");
    std::fs::write(dir.join("plan.md"), "# plan\n").expect("write plan");
    let _editor = EnvVarGuard::set("EDITOR", "vi");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/plan open")).expect("open plan");

    assert_eq!(
        controller.status_note.as_deref(),
        Some("opening file in editor")
    );
    assert_eq!(
        controller.pending_external_editor.as_ref(),
        Some(&ExternalEditorRequest {
            cwd: dir.clone(),
            path: dir.join("plan.md"),
        })
    );
    assert!(!controller.state.messages.iter().any(|message| {
        matches!(
            &message.payload,
            MessagePayload::Command { input, .. } if input == "/plan open"
        )
    }));
}

#[test]
fn controller_queues_external_editor_for_memory_open() {
    let dir = unique_test_dir("tui-slash-memory-open");
    let _editor = EnvVarGuard::set("EDITOR", "vi");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/memory open project")).expect("open project memory");

    assert_eq!(
        controller.status_note.as_deref(),
        Some("opening file in editor")
    );
    assert_eq!(
        controller.pending_external_editor.as_ref(),
        Some(&ExternalEditorRequest {
            cwd: dir.clone(),
            path: dir.join("CLAUDE.md"),
        })
    );
    assert!(!controller.state.messages.iter().any(|message| {
        matches!(
            &message.payload,
            MessagePayload::Command { input, .. } if input == "/memory open project"
        )
    }));
}

#[test]
fn memory_file_selector_confirm_sets_external_editor() {
    let dir = unique_test_dir("tui-memory-file-selector");
    let _editor = EnvVarGuard::set("EDITOR", "vi");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.open_memory_file_selector();
    assert!(controller.pending_memory_file_selector.is_some());

    // Confirm (Enter) selects first entry (project CLAUDE.md)
    block_on(controller.handle_key_event(wonder_of_u_tui::KeyEvent {
        code: wonder_of_u_tui::KeyCode::Enter,
        modifiers: wonder_of_u_tui::KeyModifiers::default(),
    }))
    .expect("confirm");

    assert!(controller.pending_memory_file_selector.is_none());
    assert_eq!(
        controller.pending_external_editor.as_ref(),
        Some(&ExternalEditorRequest {
            cwd: dir.clone(),
            path: dir.join("CLAUDE.md"),
        })
    );
    assert_eq!(
        controller.status_note.as_deref(),
        Some("opening file in editor")
    );
}

#[test]
fn clear_reloads_live_view_without_recording_command_message() {
    let dir = unique_test_dir("tui-slash-clear");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    let session_id = controller.state.session.id;
    let user = MessageEnvelope::user_text(session_id, "first message");
    let assistant = MessageEnvelope::new(
        session_id,
        MessagePayload::AssistantText {
            content: "second message".into(),
        },
    );
    controller
        .state
        .push_message(user.clone())
        .expect("push user");
    controller
        .state
        .push_message(assistant.clone())
        .expect("push assistant");
    controller
        .persist_messages(&[user, assistant])
        .expect("persist messages");

    block_on(controller.execute_slash_command("/clear")).expect("clear view");

    assert_eq!(
        controller.status_note.as_deref(),
        Some("conversation cleared")
    );
    assert_eq!(controller.state.messages.len(), 1);
    assert!(matches!(
        &controller.state.messages[0].payload,
        MessagePayload::CompactBoundary { summary }
            if summary.contains("Cleared the visible transcript")
    ));
    assert!(!controller.state.messages.iter().any(|message| {
        matches!(
            &message.payload,
            MessagePayload::Command { input, .. } if input == "/clear"
        )
    }));
}

#[test]
fn compact_reloads_live_view_and_preserves_tail_messages() {
    let dir = unique_test_dir("tui-slash-compact");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    let session_id = controller.state.session.id;
    let user = MessageEnvelope::user_text(session_id, "older user");
    let assistant = MessageEnvelope::new(
        session_id,
        MessagePayload::AssistantText {
            content: "older assistant".into(),
        },
    );
    let recent_user = MessageEnvelope::user_text(session_id, "recent user");
    let recent_assistant = MessageEnvelope::new(
        session_id,
        MessagePayload::AssistantText {
            content: "recent assistant".into(),
        },
    );
    for message in [
        user.clone(),
        assistant.clone(),
        recent_user.clone(),
        recent_assistant.clone(),
    ] {
        controller
            .state
            .push_message(message)
            .expect("push seeded message");
    }
    controller
        .persist_messages(&[
            user,
            assistant,
            recent_user.clone(),
            recent_assistant.clone(),
        ])
        .expect("persist messages");

    block_on(controller.execute_slash_command("/compact --keep-last 2")).expect("compact view");

    assert_eq!(
        controller.status_note.as_deref(),
        Some("conversation compacted")
    );
    assert_eq!(controller.state.messages.len(), 3);
    assert!(matches!(
        &controller.state.messages[0].payload,
        MessagePayload::CompactBoundary { summary }
            if summary.contains("Compacted")
    ));
    assert!(matches!(
        &controller.state.messages[1].payload,
        MessagePayload::UserText { content } if content == "recent user"
    ));
    assert!(matches!(
        &controller.state.messages[2].payload,
        MessagePayload::AssistantText { content } if content == "recent assistant"
    ));
    assert!(!controller.state.messages.iter().any(|message| {
        matches!(
            &message.payload,
            MessagePayload::Command { input, .. } if input == "/compact --keep-last 2"
        )
    }));
}

#[test]
fn controller_submits_prompt_and_persists_session() {
    let dir = unique_test_dir("tui-prompt-submit");
    let (api_base, handle) = spawn_json_sequence_server(
        |_index, _headers, body| {
            assert_eq!(body["model"], "claude-sonnet-4-6");
            assert_eq!(
                body.pointer("/messages/0/content/0/text")
                    .and_then(Value::as_str),
                Some("hello from tui")
            );
            assert_eq!(body["max_tokens"], 1024);
            assert_eq!(
                body.pointer("/tool_choice/type").and_then(Value::as_str),
                Some("auto")
            );
        },
        vec![
            json!({
                "id": "msg_tui_1",
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "text",
                    "text": "hello back"
                }],
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": 4,
                    "output_tokens": 2
                }
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

    controller.prompt.insert_text("hello from tui");
    let render_calls = 0usize;
    block_on(controller.submit_prompt()).expect("submit prompt");

    handle.join().expect("server join");

    assert_eq!(controller.prompt.text(), "");
    assert_eq!(controller.turn_state, TurnState::Completed);
    assert_eq!(controller.state.messages.len(), 2);
    let _ = render_calls;
    assert!(matches!(
        &controller.state.messages[0].payload,
        MessagePayload::UserText { content } if content == "hello from tui"
    ));
    assert!(matches!(
        &controller.state.messages[1].payload,
        MessagePayload::AssistantText { content } if content == "hello back"
    ));
    assert_eq!(controller.state.provider.as_deref(), Some("anthropic"));
    assert_eq!(controller.state.model.as_deref(), Some("claude-sonnet-4-6"));
    assert_eq!(
        controller.status_note.as_deref(),
        Some("model response recorded")
    );

    let restored = TranscriptStore::new(&dir)
        .restore_session(controller.state.session.id)
        .expect("restore session");
    assert_eq!(restored.state.messages.len(), 2);
    assert_eq!(restored.metadata.entrypoint.as_deref(), Some("tui"));
}

#[test]
fn controller_executes_tool_loop_and_persists_tool_messages() {
    let dir = unique_test_dir("tui-tool-loop");
    std::fs::write(dir.join("note.txt"), "hello from file\n").expect("write note");
    let (api_base, handle) = spawn_json_sequence_server(
        |request_index, _headers, body| match request_index {
            0 => {
                assert_eq!(body["model"], "gpt-4.1");
                assert_eq!(body["messages"][1]["content"], "read the note");
                assert_eq!(body["tool_choice"], "auto");
                assert!(
                    body["tools"]
                        .as_array()
                        .expect("tools array")
                        .iter()
                        .any(|tool| tool["function"]["name"] == "file_read")
                );
            }
            1 => {
                assert_eq!(body["messages"][1]["content"], "read the note");
                assert_eq!(
                    body["messages"][2]["tool_calls"][0]["function"]["name"],
                    "file_read"
                );
                assert_eq!(body["messages"][3]["role"], "tool");
                assert!(
                    body["messages"][3]["content"]
                        .as_str()
                        .expect("tool content")
                        .contains("hello from file")
                );
            }
            other => panic!("unexpected request index {other}"),
        },
        vec![
            serde_json::to_string(&serde_json::json!({
                "choices": [{
                    "finish_reason": "tool_calls",
                    "message": {
                        "content": null,
                        "tool_calls": [{
                            "id": "call_note",
                            "type": "function",
                            "function": {
                                "name": "file_read",
                                "arguments": "{\"path\":\"note.txt\"}"
                            }
                        }]
                    }
                }],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": 3
                }
            }))
            .expect("serialize tool response"),
            serde_json::to_string(&serde_json::json!({
                "choices": [{
                    "finish_reason": "stop",
                    "message": {
                        "content": "The note says hello from file."
                    }
                }],
                "usage": {
                    "prompt_tokens": 18,
                    "completion_tokens": 6
                }
            }))
            .expect("serialize final response"),
        ],
    );
    write_provider_config(&dir, &api_base);
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.prompt.insert_text("read the note");
    let render_calls = 0usize;
    block_on(controller.submit_prompt()).expect("submit prompt");

    handle.join().expect("server join");

    assert_eq!(controller.turn_state, TurnState::Completed);
    let _ = render_calls;
    assert_eq!(controller.state.messages.len(), 4);
    assert!(matches!(
        &controller.state.messages[0].payload,
        MessagePayload::UserText { content } if content == "read the note"
    ));
    assert!(matches!(
        &controller.state.messages[1].payload,
        MessagePayload::AssistantToolUse { tool, input, .. }
            if tool == "file_read" && input["path"] == "note.txt"
    ));
    assert!(matches!(
        &controller.state.messages[2].payload,
        MessagePayload::ToolResult { tool, success, content, .. }
            if tool == "file_read" && *success && content.contains("hello from file")
    ));
    assert!(matches!(
        &controller.state.messages[3].payload,
        MessagePayload::AssistantText { content }
            if content == "The note says hello from file."
    ));
    assert_eq!(
        controller.status_note.as_deref(),
        Some("tool loop response recorded")
    );

    let restored = TranscriptStore::new(&dir)
        .restore_session(controller.state.session.id)
        .expect("restore session");
    assert_eq!(restored.state.messages.len(), 4);
    assert!(matches!(
        &restored.state.messages[1].payload,
        MessagePayload::AssistantToolUse { .. }
    ));
    assert!(matches!(
        &restored.state.messages[2].payload,
        MessagePayload::ToolResult { .. }
    ));
}

#[test]
fn controller_approves_permission_and_resumes_tool_loop() {
    let dir = unique_test_dir("tui-tool-permission-approve");
    let (api_base, handle) = spawn_json_sequence_server(
            |request_index, _headers, body| match request_index {
                0 => {
                    assert_eq!(body["messages"][1]["content"], "write the note");
                    assert!(
                        body["tools"]
                            .as_array()
                            .expect("tools array")
                            .iter()
                            .any(|tool| tool["function"]["name"] == "file_write")
                    );
                }
                1 => {
                    assert_eq!(body["messages"][1]["content"], "write the note");
                    assert_eq!(
                        body["messages"][2]["tool_calls"][0]["function"]["name"],
                        "file_write"
                    );
                    assert_eq!(body["messages"][3]["role"], "tool");
                    assert!(
                        body["messages"][3]["content"]
                            .as_str()
                            .expect("tool content")
                            .contains("note.txt")
                    );
                }
                other => panic!("unexpected request index {other}"),
            },
            vec![
                serde_json::to_string(&serde_json::json!({
                    "choices": [{
                        "finish_reason": "tool_calls",
                        "message": {
                            "content": null,
                            "tool_calls": [{
                                "id": "call_write",
                                "type": "function",
                                "function": {
                                    "name": "file_write",
                                    "arguments": "{\"path\":\"note.txt\",\"content\":\"hello after approval\\n\"}"
                                }
                            }]
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 3
                    }
                }))
                .expect("serialize tool response"),
                serde_json::to_string(&serde_json::json!({
                    "choices": [{
                        "finish_reason": "stop",
                        "message": {
                            "content": "The note has been written."
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 16,
                        "completion_tokens": 5
                    }
                }))
                .expect("serialize final response"),
            ],
        );
    write_provider_config(&dir, &api_base);
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.prompt.insert_text("write the note");
    let render_calls = 0usize;
    block_on(controller.submit_prompt()).expect("submit prompt");

    assert_eq!(controller.turn_state, TurnState::ToolPermissionPending);
    assert_eq!(controller.state.input_mode, InputMode::PermissionPending);
    assert!(controller.state.pending_tool_approval.is_some());
    let dialog = controller.view().dialog.expect("permission dialog");
    assert_eq!(dialog.title, "Permission: Write file");
    assert_eq!(
        controller.status_note.as_deref(),
        Some("approval required: file_write")
    );
    assert_eq!(dialog.body[0], "Allow Claude to write this file?");
    assert!(
        dialog
            .body
            .iter()
            .any(|line| line == "Access: changes allowed")
    );
    assert!(dialog.body.iter().any(|line| {
        line.strip_prefix("Reason: ")
            .is_some_and(|value| !value.trim().is_empty())
    }));
    assert!(dialog.body.iter().any(|line| line.starts_with("Path: ")));

    block_on(controller.handle_key_event(wonder_of_u_tui::KeyEvent {
        code: wonder_of_u_tui::KeyCode::Enter,
        modifiers: wonder_of_u_tui::KeyModifiers::default(),
    }))
    .expect("approve permission");

    handle.join().expect("server join");

    assert_eq!(
        std::fs::read_to_string(dir.join("note.txt")).expect("read note"),
        "hello after approval\n"
    );
    assert_eq!(controller.turn_state, TurnState::Completed);
    assert_eq!(controller.state.input_mode, InputMode::Prompt);
    assert!(controller.state.pending_tool_approval.is_none());
    assert!(controller.view().dialog.is_none());
    let _ = render_calls;
    assert!(matches!(
        &controller.state.messages[2].payload,
        MessagePayload::Permission { tool, decision, .. }
            if tool == "file_write" && decision == "ask"
    ));
    assert!(matches!(
        &controller.state.messages[3].payload,
        MessagePayload::Permission { tool, decision, .. }
            if tool == "file_write" && decision == "allow"
    ));
    assert!(matches!(
        &controller.state.messages[4].payload,
        MessagePayload::ToolResult { tool, success, .. }
            if tool == "file_write" && *success
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::AssistantText { content })
            if content == "The note has been written."
    ));
    assert_eq!(
        controller.status_note.as_deref(),
        Some("tool loop response recorded")
    );
}

#[test]
fn controller_denies_permission_and_resumes_tool_loop() {
    let dir = unique_test_dir("tui-tool-permission-deny");
    let (api_base, handle) = spawn_json_sequence_server(
            |request_index, _headers, body| match request_index {
                0 => {
                    assert_eq!(body["messages"][1]["content"], "write the note");
                }
                1 => {
                    assert_eq!(
                        body["messages"][2]["tool_calls"][0]["function"]["name"],
                        "file_write"
                    );
                    assert_eq!(body["messages"][3]["role"], "tool");
                    let content = body["messages"][3]["content"]
                        .as_str()
                        .expect("tool content");
                    assert!(content.contains("ERROR:"));
                    assert!(content.contains("denied by user"));
                }
                other => panic!("unexpected request index {other}"),
            },
            vec![
                serde_json::to_string(&serde_json::json!({
                    "choices": [{
                        "finish_reason": "tool_calls",
                        "message": {
                            "content": null,
                            "tool_calls": [{
                                "id": "call_write_deny",
                                "type": "function",
                                "function": {
                                    "name": "file_write",
                                    "arguments": "{\"path\":\"note.txt\",\"content\":\"should not be written\\n\"}"
                                }
                            }]
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 3
                    }
                }))
                .expect("serialize tool response"),
                serde_json::to_string(&serde_json::json!({
                    "choices": [{
                        "finish_reason": "stop",
                        "message": {
                            "content": "Okay, I did not write the note."
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 16,
                        "completion_tokens": 5
                    }
                }))
                .expect("serialize final response"),
            ],
        );
    write_provider_config(&dir, &api_base);
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.prompt.insert_text("write the note");
    block_on(controller.submit_prompt()).expect("submit prompt");

    let dialog = controller.view().dialog.expect("permission dialog");
    assert_eq!(dialog.title, "Permission: Write file");
    assert_eq!(
        controller.status_note.as_deref(),
        Some("approval required: file_write")
    );

    block_on(controller.handle_key_event(wonder_of_u_tui::KeyEvent {
        code: wonder_of_u_tui::KeyCode::Esc,
        modifiers: wonder_of_u_tui::KeyModifiers::default(),
    }))
    .expect("deny permission");

    handle.join().expect("server join");

    assert!(!dir.join("note.txt").exists());
    assert_eq!(controller.turn_state, TurnState::Completed);
    assert_eq!(controller.state.input_mode, InputMode::Prompt);
    assert!(controller.state.pending_tool_approval.is_none());
    assert!(controller.view().dialog.is_none());
    assert!(matches!(
        &controller.state.messages[2].payload,
        MessagePayload::Permission { tool, decision, .. }
            if tool == "file_write" && decision == "ask"
    ));
    assert!(matches!(
        &controller.state.messages[3].payload,
        MessagePayload::Permission { tool, decision, .. }
            if tool == "file_write" && decision == "deny"
    ));
    assert!(matches!(
        &controller.state.messages[4].payload,
        MessagePayload::ToolResult { tool, success, content, .. }
            if tool == "file_write"
                && !success
                && content.contains("tool execution denied by user")
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::AssistantText { content })
            if content == "Okay, I did not write the note."
    ));
    assert_eq!(
        controller.status_note.as_deref(),
        Some("tool loop response recorded")
    );
}

#[test]
fn controller_restores_pending_permission_and_resumes_tool_loop() {
    let dir = unique_test_dir("tui-tool-permission-resume");
    let (api_base, handle) = spawn_json_sequence_server(
            |request_index, _headers, body| match request_index {
                0 => {
                    assert_eq!(body["messages"][1]["content"], "write the note");
                }
                1 => {
                    assert_eq!(
                        body["messages"][2]["tool_calls"][0]["function"]["name"],
                        "file_write"
                    );
                    assert_eq!(body["messages"][3]["role"], "tool");
                }
                other => panic!("unexpected request index {other}"),
            },
            vec![
                serde_json::to_string(&serde_json::json!({
                    "choices": [{
                        "finish_reason": "tool_calls",
                        "message": {
                            "content": null,
                            "tool_calls": [{
                                "id": "call_write_resume",
                                "type": "function",
                                "function": {
                                    "name": "file_write",
                                    "arguments": "{\"path\":\"note.txt\",\"content\":\"hello after resume\\n\"}"
                                }
                            }]
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 3
                    }
                }))
                .expect("serialize tool response"),
                serde_json::to_string(&serde_json::json!({
                    "choices": [{
                        "finish_reason": "stop",
                        "message": {
                            "content": "The note has been written after resume."
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 16,
                        "completion_tokens": 5
                    }
                }))
                .expect("serialize final response"),
            ],
        );
    write_provider_config(&dir, &api_base);
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.prompt.insert_text("write the note");
    block_on(controller.submit_prompt()).expect("submit prompt");
    assert!(controller.state.pending_tool_approval.is_some());
    let session_id = controller.state.session.id.to_string();
    drop(controller);

    let mut restored = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions {
            session_id: Some(session_id),
        },
    )
    .expect("restored controller");

    assert_eq!(restored.turn_state, TurnState::ToolPermissionPending);
    assert_eq!(restored.state.input_mode, InputMode::PermissionPending);
    assert!(restored.state.pending_tool_approval.is_some());
    let dialog = restored.view().dialog.expect("permission dialog");
    assert_eq!(dialog.title, "Permission: Write file");
    assert_eq!(
        restored.status_note.as_deref(),
        Some("approval required: file_write")
    );
    assert_eq!(dialog.body[0], "Allow Claude to write this file?");
    assert!(
        dialog
            .body
            .iter()
            .any(|line| line == "Access: changes allowed")
    );
    assert!(dialog.body.iter().any(|line| {
        line.strip_prefix("Reason: ")
            .is_some_and(|value| !value.trim().is_empty())
    }));
    assert!(dialog.body.iter().any(|line| line.starts_with("Path: ")));

    block_on(restored.handle_key_event(wonder_of_u_tui::KeyEvent {
        code: wonder_of_u_tui::KeyCode::Enter,
        modifiers: wonder_of_u_tui::KeyModifiers::default(),
    }))
    .expect("approve restored permission");

    handle.join().expect("server join");

    assert_eq!(
        std::fs::read_to_string(dir.join("note.txt")).expect("read note"),
        "hello after resume\n"
    );
    assert_eq!(restored.turn_state, TurnState::Completed);
    assert_eq!(restored.state.input_mode, InputMode::Prompt);
    assert!(restored.state.pending_tool_approval.is_none());
    assert!(restored.view().dialog.is_none());
    assert!(matches!(
        restored.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::AssistantText { content })
            if content == "The note has been written after resume."
    ));
}

#[test]
fn controller_opens_task_notice_when_task_finishes() {
    let dir = unique_test_dir("tui-task-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    let store = TaskStore::new(&dir);
    let mut task = TaskState::pending("run tests");
    task.status = TaskStatus::Running;
    store.write_task(&task).expect("write running task");

    controller
        .refresh_runtime_state()
        .expect("load running task");
    assert!(controller.dialog.is_none());

    task.mark_finished(
        TaskStatus::Completed,
        Some(0),
        Some("all tests passed".into()),
    );
    store.write_task(&task).expect("write completed task");

    controller
        .refresh_runtime_state()
        .expect("load completed task");

    assert_eq!(controller.state.input_mode, InputMode::TaskNotification);
    assert_eq!(
        controller.status_note.as_deref(),
        Some("task completed: run tests")
    );
    let dialog = controller.view().dialog.expect("task notification dialog");
    assert_eq!(dialog.title, "Task update");
    assert!(dialog.body.iter().any(|line| line.contains("run tests")));
    assert!(
        dialog
            .body
            .iter()
            .any(|line| line.contains("all tests passed"))
    );

    block_on(controller.handle_dialog_key(
        KeyEvent {
            code: KeyCode::Esc,
            modifiers: wonder_of_u_tui::KeyModifiers::default(),
        },
        None,
    ))
    .expect("dismiss task notice");

    assert!(controller.dialog.is_none());
    assert_eq!(controller.state.input_mode, InputMode::Prompt);
    assert_eq!(
        controller.status_note.as_deref(),
        Some("task update closed")
    );
}

#[test]
fn controller_task_notice_clears_after_ttl_ticks() {
    let dir = unique_test_dir("tui-task-notice-ttl");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    let store = TaskStore::new(&dir);
    let mut task = TaskState::pending("run tests");
    task.status = TaskStatus::Running;
    store.write_task(&task).expect("write running task");
    controller
        .refresh_runtime_state()
        .expect("load running task");

    task.mark_finished(TaskStatus::Completed, Some(0), Some("done".into()));
    store.write_task(&task).expect("write completed task");
    controller
        .refresh_runtime_state()
        .expect("load completed task");

    assert!(
        controller.task_notice_ttl.is_some(),
        "TTL should be set after task notification"
    );
    assert!(controller.dialog.is_some(), "dialog should be open");

    // Tick until TTL expires (TASK_NOTICE_TTL ticks to decrement to 0, then one more to dismiss)
    for _ in 0..=TASK_NOTICE_TTL {
        block_on(controller.handle_event(UiEvent::Tick)).expect("tick");
    }

    assert!(
        controller.dialog.is_none(),
        "dialog should be dismissed after TTL ticks"
    );
    assert_eq!(controller.task_notice_ttl, None);
    assert!(
        controller.view().notifications.is_empty(),
        "overlay toast should expire with the dialog TTL"
    );
}

#[test]
fn controller_task_notice_esc_dismisses_immediately() {
    let dir = unique_test_dir("tui-task-notice-esc");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    let store = TaskStore::new(&dir);
    let mut task = TaskState::pending("build project");
    task.status = TaskStatus::Running;
    store.write_task(&task).expect("write running task");
    controller
        .refresh_runtime_state()
        .expect("load running task");

    task.mark_finished(TaskStatus::Completed, Some(0), None);
    store.write_task(&task).expect("write completed task");
    controller
        .refresh_runtime_state()
        .expect("load completed task");

    assert!(controller.dialog.is_some(), "dialog should be open");

    block_on(controller.handle_dialog_key(
        KeyEvent {
            code: KeyCode::Esc,
            modifiers: wonder_of_u_tui::KeyModifiers::default(),
        },
        None,
    ))
    .expect("dismiss via Esc");

    assert!(
        controller.dialog.is_none(),
        "dialog should be gone immediately after Esc"
    );
    assert_eq!(controller.state.input_mode, InputMode::Prompt);
}

#[test]
fn controller_task_notice_populates_notification_overlay() {
    let dir = unique_test_dir("tui-task-overlay");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    let store = TaskStore::new(&dir);
    let mut task = TaskState::pending("index workspace");
    task.status = TaskStatus::Running;
    store.write_task(&task).expect("write running task");
    controller
        .refresh_runtime_state()
        .expect("load running task");

    task.mark_finished(TaskStatus::Completed, Some(0), Some("all done".into()));
    store.write_task(&task).expect("write completed task");
    controller
        .refresh_runtime_state()
        .expect("load completed task");

    let notifications = controller.view().notifications;
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].title, "Task update");
    assert_eq!(notifications[0].severity, NotificationSeverity::Success);
    assert!(notifications[0].focused);
    assert!(
        notifications[0]
            .lines
            .iter()
            .any(|line| line.contains("all done"))
    );
}

#[test]
fn controller_notification_overlay_pauses_while_terminal_is_unfocused() {
    let dir = unique_test_dir("tui-task-overlay-focus");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.push_notification(
        "source-status",
        NotificationSeverity::Info,
        "Source status",
        ["workspace refreshed"],
        Some(1),
        true,
    );

    block_on(controller.handle_event(UiEvent::FocusLost)).expect("lose focus");
    block_on(controller.handle_event(UiEvent::Tick)).expect("tick");
    assert_eq!(
        controller.view().notifications.len(),
        1,
        "TTL should pause while the terminal is unfocused"
    );

    block_on(controller.handle_event(UiEvent::FocusGained)).expect("gain focus");
    block_on(controller.handle_event(UiEvent::Tick)).expect("tick");
    assert!(
        controller.view().notifications.is_empty(),
        "notification should expire once focus returns and ticks resume"
    );
}

#[test]
fn controller_empty_task_panel_shows_notice_when_opened() {
    let dir = unique_test_dir("tui-empty-tasks-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // No tasks exist — run bare /tasks command
    block_on(controller.execute_slash_command("/tasks")).expect("run /tasks with no tasks");

    assert_eq!(
        controller.status_note.as_deref(),
        Some("no background tasks running"),
        "status note should indicate no tasks"
    );
    assert!(
        controller.task_notice_ttl.is_some(),
        "TTL should be set for empty-tasks notice"
    );
    let dialog = controller
        .view()
        .dialog
        .expect("notice dialog should be shown");
    assert_eq!(dialog.title, "Background Tasks");
    assert!(
        dialog
            .body
            .iter()
            .any(|l| l.contains("No background tasks")),
        "dialog body should mention no background tasks"
    );
}

#[test]
fn controller_active_overlay_is_none_when_idle() {
    let dir = unique_test_dir("tui-overlay-none");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    assert_eq!(controller.active_overlay(), ActiveOverlay::None);
}

#[test]
fn controller_active_overlay_is_picker_when_picker_open() {
    let dir = unique_test_dir("tui-overlay-picker");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/model")).expect("open model picker");

    assert_eq!(controller.active_overlay(), ActiveOverlay::Picker);
}

#[test]
fn controller_opening_picker_clears_stale_notice() {
    let dir = unique_test_dir("tui-picker-clears-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // Simulate a stale status_note from a previous interaction.
    controller.status_note = Some("stale: task update closed".into());

    block_on(controller.execute_slash_command("/model")).expect("open model picker");

    let note = controller
        .status_note
        .as_deref()
        .expect("status note should be set");
    assert!(
        note.contains("model picker"),
        "status note should be from the picker, not stale: {note:?}"
    );
}

#[test]
fn controller_setting_notice_clears_picker() {
    let dir = unique_test_dir("tui-notice-clears-picker");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // Open model picker.
    block_on(controller.execute_slash_command("/model")).expect("open model picker");
    assert!(controller.pending_model_picker.is_some(), "picker open");

    // Simulate a task finishing which triggers a notice.
    let store = TaskStore::new(&dir);
    let mut task = TaskState::pending("index workspace");
    task.status = TaskStatus::Running;
    store.write_task(&task).expect("write running task");
    controller
        .refresh_runtime_state()
        .expect("load running task");

    task.mark_finished(TaskStatus::Completed, Some(0), None);
    store.write_task(&task).expect("write completed task");
    controller
        .refresh_runtime_state()
        .expect("load completed task — triggers notice");

    // The task notice should be showing and the picker should be gone.
    assert_eq!(controller.state.input_mode, InputMode::TaskNotification);
    assert!(
        controller.pending_model_picker.is_none(),
        "picker should be cleared when a notice is shown"
    );
    assert!(controller.dialog.is_some(), "notice dialog should be open");
}

#[test]
fn controller_confirms_exit_when_session_has_activity() {
    let dir = unique_test_dir("tui-exit-confirm");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.prompt.insert_text("unsent prompt");

    block_on(controller.handle_key_event(wonder_of_u_tui::KeyEvent {
        code: wonder_of_u_tui::KeyCode::Char('c'),
        modifiers: wonder_of_u_tui::KeyModifiers {
            control: true,
            ..wonder_of_u_tui::KeyModifiers::default()
        },
    }))
    .expect("interrupt");

    assert!(!controller.exit_requested);
    assert_eq!(controller.status_note.as_deref(), Some("confirm exit"));
    assert!(controller.dialog.is_some());

    block_on(controller.handle_key_event(wonder_of_u_tui::KeyEvent {
        code: wonder_of_u_tui::KeyCode::Enter,
        modifiers: wonder_of_u_tui::KeyModifiers::default(),
    }))
    .expect("confirm exit");

    assert!(controller.exit_requested);
    assert_eq!(controller.turn_state, TurnState::Interrupted);
}

#[test]
fn controller_executes_vim_normal_mode_edits() {
    let dir = unique_test_dir("tui-vim-mode");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.prompt.insert_text("abc");

    block_on(controller.handle_key_event(wonder_of_u_tui::KeyEvent {
        code: wonder_of_u_tui::KeyCode::Esc,
        modifiers: wonder_of_u_tui::KeyModifiers::default(),
    }))
    .expect("enter normal mode");
    assert_eq!(controller.vim.mode(), VimMode::Normal);
    assert_eq!(controller.prompt.cursor(), 2);

    block_on(controller.handle_key_event(wonder_of_u_tui::KeyEvent {
        code: wonder_of_u_tui::KeyCode::Char('x'),
        modifiers: wonder_of_u_tui::KeyModifiers::default(),
    }))
    .expect("delete char");
    assert_eq!(controller.prompt.text(), "ab");

    block_on(controller.handle_key_event(wonder_of_u_tui::KeyEvent {
        code: wonder_of_u_tui::KeyCode::Char('a'),
        modifiers: wonder_of_u_tui::KeyModifiers::default(),
    }))
    .expect("append after cursor");
    assert_eq!(controller.vim.mode(), VimMode::Insert);

    block_on(controller.handle_key_event(wonder_of_u_tui::KeyEvent {
        code: wonder_of_u_tui::KeyCode::Char('z'),
        modifiers: wonder_of_u_tui::KeyModifiers::default(),
    }))
    .expect("insert after append");
    assert_eq!(controller.prompt.text(), "abz");
    assert_eq!(controller.status_note, None);
    assert_eq!(controller.vim.mode(), VimMode::Insert);
}

