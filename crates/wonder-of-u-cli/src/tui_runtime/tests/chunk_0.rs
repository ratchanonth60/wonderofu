fn write_provider_config_for(storage_dir: &Path, provider: &str, model: &str, api_base: &str) {
    let mut settings = AgentSettings {
        selected_provider: Some(provider.into()),
        selected_model: Some(model.into()),
        ..AgentSettings::default()
    };
    settings
        .providers
        .entry(provider.into())
        .or_default()
        .api_base = Some(api_base.into());
    SettingsStore::new(storage_dir)
        .write(&settings)
        .expect("write settings");
    CredentialStore::new(storage_dir)
        .write(&StoredCredentials {
            providers: [(
                provider.into(),
                AuthMaterial::ApiKey {
                    key: "test-key".into(),
                },
            )]
            .into(),
        })
        .expect("write credentials");
}

fn write_provider_config(storage_dir: &Path, api_base: &str) {
    write_provider_config_for(storage_dir, "openai", "gpt-4.1", api_base);
}

fn read_http_request(stream: &mut TcpStream) -> (String, Value) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).expect("read request");
        assert!(read > 0, "expected request bytes");
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = String::from_utf8(buffer[..header_end].to_vec()).expect("headers utf8");
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(str::trim)
                .map(|value| value.parse::<usize>().expect("content length"))
        })
        .unwrap_or(0);
    while buffer.len() < header_end + content_length {
        let read = stream.read(&mut chunk).expect("read body");
        assert!(read > 0, "expected request body bytes");
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = serde_json::from_slice(&buffer[header_end..header_end + content_length])
        .expect("body json");
    (headers, body)
}

fn spawn_json_sequence_server(
    mut assert_request: impl FnMut(usize, String, Value) + Send + 'static,
    response_bodies: Vec<String>,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let address = listener.local_addr().expect("server address");
    let handle = thread::spawn(move || {
        for (index, response_body) in response_bodies.into_iter().enumerate() {
            let (mut stream, _) = listener.accept().expect("accept request");
            let (headers, body) = read_http_request(&mut stream);
            assert_request(index, headers, body);
            write!(
                stream,
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: application/json\r\n",
                    "Content-Length: {}\r\n",
                    "Connection: close\r\n\r\n",
                    "{}"
                ),
                response_body.len(),
                response_body
            )
            .expect("write response");
            stream.flush().expect("flush response");
        }
    });
    (format!("http://{address}/v1"), handle)
}

fn test_context(cwd: &Path) -> CommandContext {
    CommandContext {
        session_id: SessionId::new(),
        cwd: cwd.to_path_buf(),
        features: FeatureSet::first_release(),
        authenticated: false,
        interactive: true,
        permission_mode: PermissionMode::Default,
        theme: None,
        session_color: None,
        effort_level: None,
        brief_mode: false,
        fast_mode: false,
        optimize_token_mode: false,
        session_tags: Vec::new(),
        additional_working_directories: Vec::new(),
    }
}

fn picker_key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: wonder_of_u_tui::KeyModifiers::default(),
    }
}

fn send_dialog_key(
    controller: &mut TuiController<'_>,
    key: KeyEvent,
    resolved: Option<ResolvedKey>,
) {
    block_on(controller.handle_dialog_key(key, resolved)).expect("handle dialog key");
}

fn send_prompt_key(controller: &mut TuiController<'_>, key: KeyEvent) {
    block_on(controller.handle_key_event(key)).expect("handle prompt key");
}

fn ctrl_r_key() -> KeyEvent {
    KeyEvent {
        code: KeyCode::Char('r'),
        modifiers: wonder_of_u_tui::KeyModifiers {
            control: true,
            ..wonder_of_u_tui::KeyModifiers::default()
        },
    }
}

fn alt_key(ch: char) -> KeyEvent {
    KeyEvent {
        code: KeyCode::Char(ch),
        modifiers: wonder_of_u_tui::KeyModifiers {
            alt: true,
            ..wonder_of_u_tui::KeyModifiers::default()
        },
    }
}

fn shift_backtab_key() -> KeyEvent {
    KeyEvent {
        code: KeyCode::BackTab,
        modifiers: wonder_of_u_tui::KeyModifiers {
            shift: true,
            ..wonder_of_u_tui::KeyModifiers::default()
        },
    }
}

fn seed_prompt_history(controller: &mut TuiController<'_>, entries: &[&str]) {
    let session_id = controller.state.session.id;
    for entry in entries {
        controller
            .state
            .messages
            .push(MessageEnvelope::user_text(session_id, *entry));
    }
}

#[test]
fn compose_conversation_prompt_includes_recent_history() {
    let session_id = SessionId::new();
    let messages = vec![
        MessageEnvelope::user_text(session_id, "first question"),
        MessageEnvelope::new(
            session_id,
            MessagePayload::AssistantText {
                content: "first answer".into(),
            },
        ),
    ];

    let prompt = compose_conversation_prompt(&messages, "next question");

    assert!(prompt.contains("user: first question"));
    assert!(prompt.contains("assistant: first answer"));
    assert!(prompt.ends_with("user: next question\nassistant:"));
}

#[test]
fn controller_routes_slash_commands_and_updates_provider_context() {
    let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
    let _oai_key = EnvVarGuard::set("OPENAI_API_KEY", "");
    let dir = unique_test_dir("tui-slash-model");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/model openai:gpt-4.1")).expect("execute slash");

    assert_eq!(controller.state.provider.as_deref(), Some("openai"));
    assert_eq!(controller.state.model.as_deref(), Some("gpt-4.1"));
    assert_eq!(
        controller.state.auth,
        AuthState::missing(wonder_of_u_core::AuthMaterialKind::ApiKey)
    );
    assert!(
        controller.state.messages.is_empty(),
        "/model must not record to transcript"
    );
}

#[test]
fn controller_opens_model_picker_for_bare_model_command() {
    let dir = unique_test_dir("tui-model-picker-open");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/model")).expect("open model picker");

    assert_eq!(
        controller.status_note.as_deref(),
        Some(
            "model picker: type to filter, use Up/Down to choose, Tab/Enter to select, Esc to cancel"
        )
    );
    assert!(controller.pending_model_picker.is_some());
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Model picker"
    ));
    assert!(!controller.state.messages.iter().any(|message| {
        matches!(
            &message.payload,
            MessagePayload::Command { input, .. } if input == "/model"
        )
    }));
}

#[test]
fn controller_filters_model_picker_with_visible_query_and_match_count() {
    let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
    let _oai_key = EnvVarGuard::set("OPENAI_API_KEY", "");
    let dir = unique_test_dir("tui-model-picker-filter");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/model")).expect("open model picker");
    for ch in ['h', 'a', 'i', 'k', 'u'] {
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Char(ch)),
            Some(ResolvedKey::InsertChar(ch)),
        );
    }

    let picker = controller
        .pending_model_picker
        .as_ref()
        .expect("model picker open");
    let dialog = controller.dialog.as_ref().expect("dialog");
    // Count how many options actually contain "haiku" — this may be > 1 when
    // multiple providers (e.g. Anthropic + Bedrock) offer haiku-series models.
    let haiku_count = picker
        .options
        .iter()
        .filter(|opt| {
            format!(
                "{} {} {} {} {}",
                opt.provider, opt.provider_display, opt.model, opt.model_display, opt.auth
            )
            .to_lowercase()
            .contains("haiku")
        })
        .count();
    let expected_matches = format!("Matches: {haiku_count}/{}", picker.options.len());
    assert_eq!(
        dialog.body.first().map(String::as_str),
        Some("Search: haiku")
    );
    assert_eq!(
        dialog.body.get(1).map(String::as_str),
        Some(expected_matches.as_str())
    );
    assert!(dialog.body.iter().any(|line| line.contains("haiku")));
    assert!(!dialog.body.iter().any(|line| line.contains("sonnet")));
    assert_eq!(
        controller.status_note.as_deref(),
        Some(
            "model picker: type to filter, use Up/Down to choose, Tab/Enter to select, Esc to cancel"
        )
    );
}

#[test]
fn controller_selects_model_from_picker() {
    let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "test-key");
    let _oai_key = EnvVarGuard::set("OPENAI_API_KEY", "");
    let dir = unique_test_dir("tui-model-picker-select");
    write_provider_config_for(&dir, "anthropic", "claude-sonnet-4-6", "http://127.0.0.1:1/v1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/model")).expect("open picker");
    send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    assert!(controller.pending_model_picker.is_none());
    assert!(controller.dialog.is_none());
    assert_eq!(controller.state.provider.as_deref(), Some("anthropic"));
    assert!(controller.state.model.is_some());
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/model"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("provider_selection=anthropic:"))
    ));
}

#[test]
fn controller_cancels_model_picker() {
    let dir = unique_test_dir("tui-model-picker-cancel");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/model")).expect("open picker");
    block_on(controller.handle_dialog_key(
        KeyEvent {
            code: KeyCode::Esc,
            modifiers: wonder_of_u_tui::KeyModifiers::default(),
        },
        None,
    ))
    .expect("cancel picker");

    assert!(controller.pending_model_picker.is_none());
    assert!(controller.dialog.is_none());
    assert_eq!(
        controller.status_note.as_deref(),
        Some("model picker cancelled")
    );
    // Cancellation goes to notification, not transcript.
    assert!(
        !matches!(
            controller.state.messages.last().map(|m| &m.payload),
            Some(MessagePayload::Command { input, .. }) if input == "/model"
        ),
        "cancel must not record to transcript"
    );
    assert!(
        controller.notifications.dismiss("model-picker-cancelled").is_some(),
        "cancel must push a notification"
    );
}

#[test]
fn controller_model_picker_view_lists_selected_description() {
    let dir = unique_test_dir("tui-model-picker-list");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/model")).expect("open model picker");

    let view = controller.view();
    let picker = view
        .picker_list
        .as_ref()
        .expect("picker_list present while model picker is open");
    let selected = picker
        .entries
        .iter()
        .find(|entry| entry.selected)
        .expect("selected entry");
    assert!(
        selected.group_header.as_deref().is_some_and(|h| {
            h.contains("Anthropic")
                || h.contains("Copilot")
                || h.contains("OpenAI")
                || h.contains("Local")
        }),
        "selected entry should have provider group header, got: {selected:?}"
    );
    assert!(
        !selected.label.is_empty(),
        "selected label should not be empty"
    );
}

#[test]
fn controller_picker_list_is_empty_when_no_matches() {
    let dir = unique_test_dir("tui-picker-list-no-matches");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/theme")).expect("open theme picker");
    // Type a query that matches nothing so the filter is empty.
    for ch in ['z', 'z', 'z'] {
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Char(ch)),
            Some(ResolvedKey::InsertChar(ch)),
        );
    }

    let view = controller.view();
    let picker = view
        .picker_list
        .as_ref()
        .expect("picker_list present while theme picker is open");
    assert!(
        picker.entries.is_empty(),
        "picker should be empty when filter has no matches"
    );
}

#[test]
fn controller_empty_prompt_status_shows_shortcut_hint() {
    let dir = unique_test_dir("tui-empty-prompt-status");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    let status = controller.view().status;
    assert!(
        status.contains("local:llama3.2"),
        "status must contain auto-selected model; got: {status:?}"
    );
    assert!(status.contains("0 tok"));
    assert!(status.contains("cost:--"));
}

#[test]
fn controller_sets_loading_status_for_active_turns() {
    let dir = unique_test_dir("tui-loading-status");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.turn_state = TurnState::ModelRequestActive;

    let view = controller.view();
    assert!(view.loading);
    assert!(
        view.loading_verb
            .as_deref()
            .is_some_and(|verb| verb.contains("thinking"))
    );
}

#[test]
fn controller_advances_loading_spinner_on_tick() {
    let dir = unique_test_dir("tui-loading-spinner-tick");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.turn_state = TurnState::ModelRequestActive;
    let before = controller.view().spinner_frame;

    block_on(controller.handle_event(UiEvent::Tick)).expect("tick should succeed");

    let after = controller.view().spinner_frame;
    assert_ne!(before, after);
}

#[test]
fn controller_view_footer_is_compact_with_permission_and_keybind_hint() {
    let dir = unique_test_dir("tui-compact-footer");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    let footer = controller.view().footer;

    // Compact format: "▸▸ {mode} (shift+tab to cycle) · ⌃C exit"
    assert!(
        footer.contains("shift+tab to cycle"),
        "compact footer must contain shift+tab hint: {footer}"
    );
    assert!(
        footer.contains("⌃C exit"),
        "compact footer must contain Ctrl-C exit hint: {footer}"
    );
    // Verbose fields must NOT appear in the compact footer.
    assert!(
        !footer.contains("storage="),
        "compact footer must not contain verbose storage field: {footer}"
    );
    assert!(
        !footer.contains("theme:"),
        "compact footer must not contain verbose theme field: {footer}"
    );
    assert!(
        !footer.contains("cwd="),
        "compact footer must not contain cwd field: {footer}"
    );
}

#[test]
fn controller_uses_resize_width_for_wide_message_wrapping() {
    let dir = unique_test_dir("tui-wide-wrap");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.on_terminal_resize(160, 30);
    controller.state.messages.push(MessageEnvelope::new(
        controller.state.session.id,
        MessagePayload::AssistantText {
            content: "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty"
                .into(),
        },
    ));

    let view = controller.view();
    let widest = view
        .messages
        .iter()
        .map(|line| line.text.chars().count())
        .max()
        .unwrap_or_default();

    assert!(
        widest > 90,
        "wide terminal should not use the 80-column fallback; widest={widest}, lines={:?}",
        view.messages
    );
}

#[test]
fn controller_view_loading_elapsed_secs_increases_with_ticks() {
    let dir = unique_test_dir("tui-loading-elapsed");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.turn_state = TurnState::ModelRequestActive;

    // Fire 40 ticks — each advances loading_frame by 1; elapsed_secs = frame / 20.
    for _ in 0..40 {
        block_on(controller.handle_event(UiEvent::Tick)).expect("tick");
    }

    let view = controller.view();
    assert!(view.loading, "view must be in loading state");
    assert_eq!(
        view.loading_elapsed_secs, 2,
        "40 ticks at 50ms each → 2 elapsed seconds"
    );
}

#[test]
fn controller_inserts_newline_on_shift_enter() {
    let dir = unique_test_dir("tui-shift-enter-newline");
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

    block_on(controller.handle_event(UiEvent::Key(KeyEvent {
        code: KeyCode::Enter,
        modifiers: KeyModifiers {
            shift: true,
            control: false,
            alt: false,
        },
    })))
    .expect("shift+enter should insert newline");

    assert_eq!(controller.prompt.text(), "\n");
    assert_eq!(controller.turn_state, TurnState::EditingInput);
}

#[test]
fn controller_opens_theme_picker_for_bare_theme_command() {
    let dir = unique_test_dir("tui-theme-picker-open");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/theme")).expect("open theme picker");

    assert_eq!(
        controller.status_note.as_deref(),
        Some(
            "theme picker: type to filter, use Up/Down to choose, Tab/Enter to select, Esc to cancel"
        )
    );
    assert!(controller.pending_theme_picker.is_some());
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Theme picker"
    ));
}

#[test]
fn controller_selects_theme_from_picker() {
    let dir = unique_test_dir("tui-theme-picker-select");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/theme")).expect("open theme picker");
    send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    assert!(controller.pending_theme_picker.is_none());
    assert!(controller.dialog.is_none());
    assert_eq!(controller.state.theme.as_deref(), Some("midnight"));
    assert!(controller.view().footer.contains("shift+tab to cycle"));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/theme"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("theme=midnight"))
    ));
}

#[test]
fn controller_shows_theme_notice_dialog() {
    let dir = unique_test_dir("tui-theme-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/theme show")).expect("show theme");

    assert_eq!(controller.status_note.as_deref(), Some("theme"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Theme"
    ));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/theme show must not record to transcript"
    );
}

#[test]
fn controller_dismisses_theme_notice_dialog_cleanly() {
    let dir = unique_test_dir("tui-theme-notice-dismiss");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/theme show")).expect("show theme");
    block_on(controller.handle_dialog_key(
        KeyEvent {
            code: KeyCode::Esc,
            modifiers: wonder_of_u_tui::KeyModifiers::default(),
        },
        None,
    ))
    .expect("dismiss theme notice");

    assert!(controller.dialog.is_none());
    assert_eq!(controller.state.input_mode, InputMode::Prompt);
    assert_eq!(controller.status_note.as_deref(), Some("theme closed"));
}

#[test]
fn apply_command_output_hints_clears_stale_notice_dialog() {
    let dir = unique_test_dir("tui-notice-clear");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.apply_command_output_hints(Some("## Theme\nCurrent theme: default\n"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Theme"
    ));

    controller.apply_command_output_hints(Some("status=theme updated\n"));

    assert!(controller.dialog.is_none());
}

#[test]
fn controller_hydrates_persisted_fast_and_effort_on_launch() {
    let dir = unique_test_dir("tui-hydrate-settings");
    SettingsStore::new(&dir)
        .write(&AgentSettings {
            selected_provider: Some("openai".into()),
            theme: Some("midnight".into()),
            effort_level: Some("high".into()),
            fast_mode: true,
            ..AgentSettings::default()
        })
        .expect("write settings");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    assert_eq!(controller.state.theme.as_deref(), Some("midnight"));
    assert_eq!(controller.state.effort_level.as_deref(), Some("high"));
    assert!(controller.state.fast_mode);
    // State already verified above; confirm compact footer is in use.
    assert!(controller.view().footer.contains("shift+tab to cycle"));
}

#[test]
fn controller_hydrates_persisted_vim_mode_setting() {
    let dir = unique_test_dir("tui-hydrate-vim-mode");
    SettingsStore::new(&dir)
        .write(&AgentSettings {
            vim_mode: Some(false),
            ..AgentSettings::default()
        })
        .expect("write settings");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    assert!(!controller.vim_enabled);
    // Compact footer is always active; vim mode details live in AppState/vim field.
    assert!(controller.view().footer.contains("shift+tab to cycle"));

    send_prompt_key(&mut controller, picker_key(KeyCode::Esc));
    assert_eq!(controller.vim.mode(), VimMode::Insert);
    assert!(controller.view().footer.contains("shift+tab to cycle"));
}

#[test]
fn controller_sets_session_color_from_command() {
    let dir = unique_test_dir("tui-color-set");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/color purple")).expect("set color");

    assert_eq!(controller.state.session_color.as_deref(), Some("purple"));
    assert_eq!(controller.status_note.as_deref(), Some("color purple"));
    assert!(controller.view().footer.contains("shift+tab to cycle"));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/color purple must not record to transcript"
    );
}

#[test]
fn controller_shows_color_notice_dialog() {
    let dir = unique_test_dir("tui-color-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/color")).expect("show color");

    assert_eq!(controller.status_note.as_deref(), Some("color"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Color"
    ));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/color must not record to transcript"
    );
}

#[test]
fn controller_toggles_brief_mode_from_command() {
    let dir = unique_test_dir("tui-brief-toggle");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/brief")).expect("toggle brief");

    assert!(controller.state.brief_mode);
    assert_eq!(controller.status_note.as_deref(), Some("brief on"));
    assert!(controller.view().footer.contains("shift+tab to cycle"));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/brief must not record to transcript"
    );
}

#[test]
fn controller_shows_brief_notice_dialog() {
    let dir = unique_test_dir("tui-brief-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/brief show")).expect("show brief");

    assert_eq!(controller.status_note.as_deref(), Some("brief"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Brief"
    ));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/brief show must not record to transcript"
    );
}

#[test]
fn controller_toggles_fast_mode_from_command() {
    let dir = unique_test_dir("tui-fast-toggle");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/fast")).expect("toggle fast");

    assert!(controller.state.fast_mode);
    assert_eq!(controller.status_note.as_deref(), Some("fast on"));
    assert!(controller.view().footer.contains("shift+tab to cycle"));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/fast must not record to transcript"
    );
}

#[test]
fn controller_shows_fast_notice_dialog() {
    let dir = unique_test_dir("tui-fast-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/fast show")).expect("show fast");

    assert_eq!(controller.status_note.as_deref(), Some("fast"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Fast"
    ));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/fast show must not record to transcript"
    );
}

#[test]
fn controller_sets_effort_from_command() {
    let dir = unique_test_dir("tui-effort-set");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/effort high")).expect("set effort");

    assert_eq!(controller.state.effort_level.as_deref(), Some("high"));
    assert_eq!(controller.status_note.as_deref(), Some("effort high"));
    assert!(controller.view().footer.contains("shift+tab to cycle"));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/effort high must not record to transcript"
    );
}

#[test]
fn controller_shows_effort_notice_dialog() {
    let dir = unique_test_dir("tui-effort-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/effort")).expect("show effort");

    assert_eq!(controller.status_note.as_deref(), Some("effort"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Effort"
    ));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/effort must not record to transcript"
    );
}

#[test]
fn controller_shows_feedback_notice_dialog_from_alias() {
    let dir = unique_test_dir("tui-feedback-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/bug parity gap")).expect("show feedback");

    assert_eq!(controller.status_note.as_deref(), Some("feedback"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Feedback"
    ));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/bug parity gap must not record to transcript"
    );
}

#[test]
fn controller_shows_release_notes_notice_dialog() {
    let dir = unique_test_dir("tui-release-notes-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/release-notes")).expect("show release notes");

    assert_eq!(controller.status_note.as_deref(), Some("release notes"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Release Notes"
    ));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/release-notes must not record to transcript"
    );
}

#[test]
fn controller_shows_version_notice_dialog() {
    let dir = unique_test_dir("tui-version-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/version")).expect("show version");

    assert_eq!(controller.status_note.as_deref(), Some("version"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Version"
    ));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/version must not record to transcript"
    );
}

#[test]
fn controller_shows_desktop_notice_dialog() {
    let dir = unique_test_dir("tui-desktop-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/desktop")).expect("show desktop");

    assert_eq!(controller.status_note.as_deref(), Some("desktop"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Desktop"
    ));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/desktop must not record to transcript"
    );
}

#[test]
fn controller_shows_mobile_notice_dialog_from_alias() {
    let dir = unique_test_dir("tui-mobile-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/ios")).expect("show mobile");

    assert_eq!(controller.status_note.as_deref(), Some("mobile"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Mobile"
    ));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/ios must not record to transcript"
    );
}

#[test]
fn controller_shows_chrome_notice_dialog() {
    let dir = unique_test_dir("tui-chrome-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/chrome")).expect("show chrome");

    assert_eq!(controller.status_note.as_deref(), Some("chrome"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Chrome"
    ));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/chrome must not record to transcript"
    );
}

#[test]
fn controller_shows_ide_notice_dialog() {
    let dir = unique_test_dir("tui-ide-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/ide")).expect("show ide");

    assert_eq!(controller.status_note.as_deref(), Some("ide integration"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "IDE Integration"
    ));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/ide must not record to transcript"
    );
}

#[test]
fn controller_routes_permissions_shorthand() {
    let dir = unique_test_dir("tui-permissions-shorthand");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/permissions accept-edits")).expect("set permissions mode");

    assert_eq!(
        controller.state.permission_mode,
        PermissionMode::AcceptEdits
    );
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/permissions accept-edits must not record to transcript"
    );
}

#[test]
fn controller_shift_backtab_cycles_permission_mode_in_prompt() {
    let dir = unique_test_dir("tui-shift-backtab-permission-cycle");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    send_prompt_key(&mut controller, shift_backtab_key());

    assert_eq!(controller.state.permission_mode, PermissionMode::Plan);
    assert_eq!(controller.state.input_mode, InputMode::Prompt);
    assert_eq!(
        controller.status_note.as_deref(),
        Some("permission mode plan mode")
    );
    assert!(controller.view().footer.contains("plan"));
    assert!(controller.pending_permission_picker.is_none());
    assert!(controller.dialog.is_none());
}

#[test]
fn controller_shift_backtab_keeps_slash_suggestions_open() {
    let dir = unique_test_dir("tui-shift-backtab-suggestions");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    send_prompt_key(
        &mut controller,
        KeyEvent {
            code: KeyCode::Char('/'),
            modifiers: KeyModifiers::default(),
        },
    );
    assert!(controller.active_suggestions.is_some());

    send_prompt_key(&mut controller, shift_backtab_key());

    assert_eq!(controller.state.permission_mode, PermissionMode::Plan);
    assert_eq!(controller.prompt.text(), "/");
    assert!(controller.active_suggestions.is_some());
    assert!(controller.state.messages.is_empty());
}

#[test]
fn controller_shift_backtab_is_ignored_while_picker_overlay_is_active() {
    let dir = unique_test_dir("tui-shift-backtab-picker-overlay");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/permissions")).expect("open permissions picker");

    send_dialog_key(&mut controller, shift_backtab_key(), None);

    assert_eq!(controller.state.permission_mode, PermissionMode::Default);
    assert!(controller.pending_permission_picker.is_some());
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Permission mode"
    ));
}

#[test]
fn controller_shift_backtab_is_ignored_while_setup_overlay_is_active() {
    let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
    let _oai_key = EnvVarGuard::set("OPENAI_API_KEY", "");
    let _gemini_key = EnvVarGuard::set("GEMINI_API_KEY", "");
    let dir = unique_test_dir("tui-shift-backtab-setup-overlay");
    SettingsStore::new(dir.as_path())
        .write(&AgentSettings {
            selected_provider: Some("anthropic".into()),
            ..AgentSettings::default()
        })
        .expect("write settings");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    assert!(controller.pending_setup_overlay.is_some());

    send_prompt_key(&mut controller, shift_backtab_key());

    assert_eq!(controller.state.permission_mode, PermissionMode::Default);
    assert!(controller.pending_setup_overlay.is_some());
}

#[test]
fn controller_adds_additional_working_directory_from_slash_command() {
    let dir = unique_test_dir("tui-add-dir");
    let extra = dir.join("extra");
    std::fs::create_dir_all(&extra).expect("create extra dir");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/add-dir extra")).expect("execute add-dir");

    assert_eq!(controller.state.additional_working_directories.len(), 1);
    assert_eq!(
        controller.state.additional_working_directories[0].path,
        extra.canonicalize().expect("canonical extra dir")
    );
    assert_eq!(
        controller
            .tool_context()
            .additional_working_directories
            .len(),
        1
    );
    // Machine-hint output (`status=…`) is a state-change side effect: it must
    // open no dialog and record nothing to the transcript.
    assert!(controller.dialog.is_none());
    assert!(
        controller.state.messages.is_empty(),
        "/add-dir must not record to transcript"
    );
}

