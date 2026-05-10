use super::*;

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    thread,
};

use serde_json::{Value, json};
use wonder_of_u_agent::{
    AgentSettings, AuthMaterial, CredentialStore, SettingsStore, StoredCredentials,
};
use wonder_of_u_core::{
    AuthState, InputMode, MessageEnvelope, MessagePayload, PendingLocalToolCall,
    PendingProviderToolCall, PendingToolApprovalState, PendingToolConversationRound, TokenUsage,
};
use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};
use wonder_of_u_tui::KeyModifiers;

use crate::commands;

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
    controller
        .handle_dialog_key(key, resolved, &mut |_| Ok(()))
        .expect("handle dialog key");
}

fn send_prompt_key(controller: &mut TuiController<'_>, key: KeyEvent) {
    controller
        .handle_key_event(key, &mut |_| Ok(()))
        .expect("handle prompt key");
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

    controller
        .execute_slash_command("/model openai:gpt-4.1")
        .expect("execute slash");

    assert_eq!(controller.state.provider.as_deref(), Some("openai"));
    assert_eq!(controller.state.model.as_deref(), Some("gpt-4.1"));
    assert_eq!(
        controller.state.auth,
        AuthState::missing(wonder_of_u_core::AuthMaterialKind::ApiKey)
    );
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, .. })
            if input == "/model openai:gpt-4.1"
    ));
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

    controller
        .execute_slash_command("/model")
        .expect("open model picker");

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

    controller
        .execute_slash_command("/model")
        .expect("open model picker");
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
    let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
    let _oai_key = EnvVarGuard::set("OPENAI_API_KEY", "");
    let dir = unique_test_dir("tui-model-picker-select");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller
        .execute_slash_command("/model")
        .expect("open picker");
    send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    assert!(controller.pending_model_picker.is_none());
    assert!(controller.dialog.is_none());
    assert_eq!(controller.state.provider.as_deref(), Some("anthropic"));
    assert_eq!(
        controller.state.model.as_deref(),
        Some("claude-3-5-haiku-latest")
    );
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/model"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("provider_selection=anthropic:claude-3-5-haiku-latest"))
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

    controller
        .execute_slash_command("/model")
        .expect("open picker");
    controller
        .handle_dialog_key(
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: wonder_of_u_tui::KeyModifiers::default(),
            },
            None,
            &mut |_| Ok(()),
        )
        .expect("cancel picker");

    assert!(controller.pending_model_picker.is_none());
    assert!(controller.dialog.is_none());
    assert_eq!(
        controller.status_note.as_deref(),
        Some("model picker cancelled")
    );
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/model"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("status=model picker cancelled"))
    ));
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

    controller
        .execute_slash_command("/model")
        .expect("open model picker");

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
        selected.description.contains("Anthropic")
            || selected.description.contains("Copilot")
            || selected.description.contains("OpenAI"),
        "selected entry should contain provider display name, got: {selected:?}"
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

    controller
        .execute_slash_command("/theme")
        .expect("open theme picker");
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
    assert!(status.contains("model:auto"));
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

    controller
        .handle_event(UiEvent::Tick, |_| Ok(()))
        .expect("tick should succeed");

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

    // Fire 4 ticks — each advances loading_frame by 1; elapsed_secs = frame / 2.
    for _ in 0..4 {
        controller
            .handle_event(UiEvent::Tick, |_| Ok(()))
            .expect("tick");
    }

    let view = controller.view();
    assert!(view.loading, "view must be in loading state");
    assert_eq!(
        view.loading_elapsed_secs, 2,
        "4 ticks at 500ms each → 2 elapsed seconds"
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

    controller
        .handle_event(
            UiEvent::Key(KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers {
                    shift: true,
                    control: false,
                    alt: false,
                },
            }),
            |_| Ok(()),
        )
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

    controller
        .execute_slash_command("/theme")
        .expect("open theme picker");

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

    controller
        .execute_slash_command("/theme")
        .expect("open theme picker");
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

    controller
        .execute_slash_command("/theme show")
        .expect("show theme");

    assert_eq!(controller.status_note.as_deref(), Some("theme"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Theme"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/theme show"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Theme"))
    ));
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

    controller
        .execute_slash_command("/theme show")
        .expect("show theme");
    controller
        .handle_dialog_key(
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: wonder_of_u_tui::KeyModifiers::default(),
            },
            None,
            &mut |_| Ok(()),
        )
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

    controller
        .execute_slash_command("/color purple")
        .expect("set color");

    assert_eq!(controller.state.session_color.as_deref(), Some("purple"));
    assert_eq!(controller.status_note.as_deref(), Some("color purple"));
    assert!(controller.view().footer.contains("shift+tab to cycle"));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/color purple"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("color=purple"))
    ));
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

    controller
        .execute_slash_command("/color")
        .expect("show color");

    assert_eq!(controller.status_note.as_deref(), Some("color"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Color"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/color"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Color"))
    ));
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

    controller
        .execute_slash_command("/brief")
        .expect("toggle brief");

    assert!(controller.state.brief_mode);
    assert_eq!(controller.status_note.as_deref(), Some("brief on"));
    assert!(controller.view().footer.contains("shift+tab to cycle"));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/brief"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("brief_mode=true"))
    ));
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

    controller
        .execute_slash_command("/brief show")
        .expect("show brief");

    assert_eq!(controller.status_note.as_deref(), Some("brief"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Brief"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/brief show"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Brief"))
    ));
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

    controller
        .execute_slash_command("/fast")
        .expect("toggle fast");

    assert!(controller.state.fast_mode);
    assert_eq!(controller.status_note.as_deref(), Some("fast on"));
    assert!(controller.view().footer.contains("shift+tab to cycle"));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/fast"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("fast_mode=true"))
    ));
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

    controller
        .execute_slash_command("/fast show")
        .expect("show fast");

    assert_eq!(controller.status_note.as_deref(), Some("fast"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Fast"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/fast show"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Fast"))
    ));
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

    controller
        .execute_slash_command("/effort high")
        .expect("set effort");

    assert_eq!(controller.state.effort_level.as_deref(), Some("high"));
    assert_eq!(controller.status_note.as_deref(), Some("effort high"));
    assert!(controller.view().footer.contains("shift+tab to cycle"));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/effort high"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("effort_level=high"))
    ));
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

    controller
        .execute_slash_command("/effort")
        .expect("show effort");

    assert_eq!(controller.status_note.as_deref(), Some("effort"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Effort"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/effort"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Effort"))
    ));
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

    controller
        .execute_slash_command("/bug parity gap")
        .expect("show feedback");

    assert_eq!(controller.status_note.as_deref(), Some("feedback"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Feedback"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/bug parity gap"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Feedback") && text.contains("parity gap"))
    ));
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

    controller
        .execute_slash_command("/release-notes")
        .expect("show release notes");

    assert_eq!(controller.status_note.as_deref(), Some("release notes"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Release Notes"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/release-notes"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Release Notes"))
    ));
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

    controller
        .execute_slash_command("/version")
        .expect("show version");

    assert_eq!(controller.status_note.as_deref(), Some("version"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Version"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/version"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Version"))
    ));
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

    controller
        .execute_slash_command("/desktop")
        .expect("show desktop");

    assert_eq!(controller.status_note.as_deref(), Some("desktop"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Desktop"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/desktop"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Desktop") && text.contains("desktop_docs_url="))
    ));
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

    controller
        .execute_slash_command("/ios")
        .expect("show mobile");

    assert_eq!(controller.status_note.as_deref(), Some("mobile"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Mobile"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/ios"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Mobile") && text.contains("ios_qr_text="))
    ));
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

    controller
        .execute_slash_command("/chrome")
        .expect("show chrome");

    assert_eq!(controller.status_note.as_deref(), Some("chrome"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Chrome"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/chrome"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Chrome") && text.contains("extension_url="))
    ));
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

    controller.execute_slash_command("/ide").expect("show ide");

    assert_eq!(controller.status_note.as_deref(), Some("ide integration"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "IDE Integration"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/ide"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## IDE Integration") && text.contains("ide_docs_url="))
    ));
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

    controller
        .execute_slash_command("/permissions accept-edits")
        .expect("set permissions mode");

    assert_eq!(
        controller.state.permission_mode,
        PermissionMode::AcceptEdits
    );
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/permissions accept-edits"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("status=permission mode updated"))
    ));
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

    controller
        .execute_slash_command("/add-dir extra")
        .expect("execute add-dir");

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
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/add-dir extra"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("status=added working directory"))
    ));
}

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

    controller
        .execute_slash_command("/memory")
        .expect("open memory picker");

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

    controller
        .execute_slash_command("/theme")
        .expect("open theme picker");
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

    controller
        .execute_slash_command("/theme")
        .expect("open theme picker");
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

    controller
        .execute_slash_command("/memory")
        .expect("open memory picker");
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

    controller
        .execute_slash_command("/memory")
        .expect("open memory picker");
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

    controller
        .execute_slash_command("/memory")
        .expect("open picker");
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

    controller
        .execute_slash_command("/memory")
        .expect("open picker");
    controller
        .handle_dialog_key(
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: wonder_of_u_tui::KeyModifiers::default(),
            },
            None,
            &mut |_| Ok(()),
        )
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

    controller
        .execute_slash_command("/tag bugfix")
        .expect("open tag removal dialog");

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
    controller
        .execute_slash_command("/tag bugfix")
        .expect("open tag removal dialog");

    controller
        .handle_dialog_key(
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: wonder_of_u_tui::KeyModifiers::default(),
            },
            Some(ResolvedKey::Edit(EditAction::InsertNewline)),
            &mut |_| Ok(()),
        )
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

    controller
        .execute_slash_command("/context")
        .expect("show context");

    assert_eq!(controller.status_note.as_deref(), Some("context usage"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Context Usage"
    ));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/context"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("## Context Usage"))
    ));
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

    controller
        .execute_slash_command("/stats")
        .expect("show stats");

    assert_eq!(controller.status_note.as_deref(), Some("session stats"));
    assert!(controller.dialog.is_none());
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/stats"
                && output
                    .as_deref()
                    .is_some_and(|text| {
                        text.contains("Session Statistics")
                            && text.contains("Provider:  openai")
                            && text.contains("Estimated cost:      $0.4200")
                    })
    ));
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

    controller
        .execute_slash_command("/help")
        .expect("show help");

    assert_eq!(controller.status_note.as_deref(), Some("help"));
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/help"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("Slash Commands") && text.contains("/search"))
    ));

    let lines = wonder_of_u_tui::message_lines(&controller.state.messages, false);
    assert!(lines.iter().any(|line| {
        line.text == "Slash Commands" && line.role == wonder_of_u_tui::MessageRole::System
    }));
    assert!(lines.iter().any(|line| {
        line.text.contains("/search")
            && line.text.contains("Search workspace files")
            && line.role == wonder_of_u_tui::MessageRole::System
    }));
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

    controller
        .execute_slash_command("/thinking")
        .expect("report thinking");
    assert!(!controller.state.thinking_enabled);
    assert!(
        controller
            .status_note
            .as_deref()
            .is_some_and(|s| s.starts_with("thinking off")),
        "expected status to start with 'thinking off', got {:?}",
        controller.status_note
    );
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/thinking"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("Thinking is currently disabled"))
    ));

    controller
        .execute_slash_command("/thinking on")
        .expect("enable thinking");
    assert!(controller.state.thinking_enabled);
    assert!(
        controller
            .status_note
            .as_deref()
            .is_some_and(|s| s.starts_with("thinking on")),
        "expected status to start with 'thinking on', got {:?}",
        controller.status_note
    );
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/thinking on"
                && output
                    .as_deref()
                    .is_some_and(|text| text.contains("Thinking enabled"))
    ));

    controller
        .execute_slash_command("/thinking off")
        .expect("disable thinking");
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

    controller
        .execute_slash_command("/usage")
        .expect("show usage");

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

    controller
        .execute_slash_command("/keybindings")
        .expect("show keybindings");

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

    controller
        .execute_slash_command("/hooks")
        .expect("show hooks");

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

    controller
        .execute_slash_command("/privacy-settings")
        .expect("show privacy settings");

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

    controller
        .execute_slash_command("/settings")
        .expect("show settings");

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

    controller
        .execute_slash_command("/terminal-setup")
        .expect("show terminal setup");

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

    controller
        .execute_slash_command("/vim")
        .expect("toggle vim");
    assert_eq!(controller.vim.mode(), VimMode::Normal);
    assert_eq!(controller.status_note.as_deref(), Some("vim normal"));

    controller
        .execute_slash_command("/vim insert")
        .expect("set vim insert");
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

    controller
        .execute_slash_command("/permissions")
        .expect("open permissions picker");

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

    controller
        .execute_slash_command("/permissions")
        .expect("open permissions picker");
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

    controller
        .execute_slash_command("/permissions")
        .expect("open permissions picker");
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

    controller
        .execute_slash_command("/plan")
        .expect("enter plan mode");
    assert_eq!(controller.state.permission_mode, PermissionMode::Plan);
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/plan"
                && output.as_deref().is_some_and(|text| text.contains("status=plan mode enabled"))
    ));

    controller
        .execute_slash_command("/plan")
        .expect("show current plan");
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

    controller
        .execute_slash_command("/plan exit")
        .expect("exit plan mode");
    assert_eq!(controller.state.permission_mode, PermissionMode::Default);
    assert!(matches!(
        controller.state.messages.last().map(|message| &message.payload),
        Some(MessagePayload::Command { input, output })
            if input == "/plan exit"
                && output.as_deref().is_some_and(|text| text.contains("status=plan mode disabled"))
    ));
}

#[test]
fn controller_executes_queued_plan_prompt() {
    let dir = unique_test_dir("tui-slash-plan-prompt");
    let (api_base, handle) = spawn_json_sequence_server(
        |_index, _headers, body| {
            assert_eq!(body["model"], "claude-3-7-sonnet-latest");
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
    write_provider_config_for(&dir, "anthropic", "claude-3-7-sonnet-latest", &api_base);
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    let mut render_calls = 0usize;
    controller
        .execute_slash_command_with("/plan draft the migration plan", &mut |_| {
            render_calls += 1;
            Ok(())
        })
        .expect("execute plan prompt");

    handle.join().expect("server join");

    assert_eq!(controller.state.permission_mode, PermissionMode::Plan);
    assert!(render_calls >= 2);
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
    write_provider_config_for(&dir, "anthropic", "claude-3-7-sonnet-latest", &api_base);
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

    let mut queued_snapshots = Vec::new();
    controller
        .drain_queued_commands(&mut |controller| {
            queued_snapshots.push(controller.view().queued_panel.clone());
            Ok(())
        })
        .expect("drain queued prompts");

    handle.join().expect("server join");

    assert!(queued_snapshots.iter().any(|panel| {
        panel.as_ref().is_some_and(|panel| {
            panel.lines
                == vec![wonder_of_u_tui::message::MessageLineView::new(
                    "1. second queued prompt",
                    wonder_of_u_tui::message::MessageRole::Progress,
                )]
        })
    }));
    assert!(queued_snapshots.iter().any(Option::is_none));
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

    controller
        .execute_slash_command("/plan open")
        .expect("open plan");

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

    controller
        .execute_slash_command("/memory open project")
        .expect("open project memory");

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

    controller
        .execute_slash_command("/clear")
        .expect("clear view");

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

    controller
        .execute_slash_command("/compact --keep-last 2")
        .expect("compact view");

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
            assert_eq!(body["model"], "claude-3-7-sonnet-latest");
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
    write_provider_config_for(&dir, "anthropic", "claude-3-7-sonnet-latest", &api_base);
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.prompt.insert_text("hello from tui");
    let mut render_calls = 0usize;
    controller
        .submit_prompt(&mut |_| {
            render_calls += 1;
            Ok(())
        })
        .expect("submit prompt");

    handle.join().expect("server join");

    assert_eq!(controller.prompt.text(), "");
    assert_eq!(controller.turn_state, TurnState::Completed);
    assert_eq!(controller.state.messages.len(), 2);
    assert!(render_calls >= 2);
    assert!(matches!(
        &controller.state.messages[0].payload,
        MessagePayload::UserText { content } if content == "hello from tui"
    ));
    assert!(matches!(
        &controller.state.messages[1].payload,
        MessagePayload::AssistantText { content } if content == "hello back"
    ));
    assert_eq!(controller.state.provider.as_deref(), Some("anthropic"));
    assert_eq!(
        controller.state.model.as_deref(),
        Some("claude-3-7-sonnet-latest")
    );
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
                assert_eq!(body["messages"][0]["content"], "read the note");
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
                assert_eq!(body["messages"][0]["content"], "read the note");
                assert_eq!(
                    body["messages"][1]["tool_calls"][0]["function"]["name"],
                    "file_read"
                );
                assert_eq!(body["messages"][2]["role"], "tool");
                assert!(
                    body["messages"][2]["content"]
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
    let mut render_calls = 0usize;
    controller
        .submit_prompt(&mut |_| {
            render_calls += 1;
            Ok(())
        })
        .expect("submit prompt");

    handle.join().expect("server join");

    assert_eq!(controller.turn_state, TurnState::Completed);
    assert!(render_calls >= 4);
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
                    assert_eq!(body["messages"][0]["content"], "write the note");
                    assert!(
                        body["tools"]
                            .as_array()
                            .expect("tools array")
                            .iter()
                            .any(|tool| tool["function"]["name"] == "file_write")
                    );
                }
                1 => {
                    assert_eq!(body["messages"][0]["content"], "write the note");
                    assert_eq!(
                        body["messages"][1]["tool_calls"][0]["function"]["name"],
                        "file_write"
                    );
                    assert_eq!(body["messages"][2]["role"], "tool");
                    assert!(
                        body["messages"][2]["content"]
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
    let mut render_calls = 0usize;
    controller
        .submit_prompt(&mut |_| {
            render_calls += 1;
            Ok(())
        })
        .expect("submit prompt");

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

    controller
        .handle_key_event(
            wonder_of_u_tui::KeyEvent {
                code: wonder_of_u_tui::KeyCode::Enter,
                modifiers: wonder_of_u_tui::KeyModifiers::default(),
            },
            &mut |_| {
                render_calls += 1;
                Ok(())
            },
        )
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
    assert!(render_calls >= 5);
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
                    assert_eq!(body["messages"][0]["content"], "write the note");
                }
                1 => {
                    assert_eq!(
                        body["messages"][1]["tool_calls"][0]["function"]["name"],
                        "file_write"
                    );
                    assert_eq!(body["messages"][2]["role"], "tool");
                    let content = body["messages"][2]["content"]
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
    controller
        .submit_prompt(&mut |_| Ok(()))
        .expect("submit prompt");

    let dialog = controller.view().dialog.expect("permission dialog");
    assert_eq!(dialog.title, "Permission: Write file");
    assert_eq!(
        controller.status_note.as_deref(),
        Some("approval required: file_write")
    );

    controller
        .handle_key_event(
            wonder_of_u_tui::KeyEvent {
                code: wonder_of_u_tui::KeyCode::Esc,
                modifiers: wonder_of_u_tui::KeyModifiers::default(),
            },
            &mut |_| Ok(()),
        )
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
                    assert_eq!(body["messages"][0]["content"], "write the note");
                }
                1 => {
                    assert_eq!(
                        body["messages"][1]["tool_calls"][0]["function"]["name"],
                        "file_write"
                    );
                    assert_eq!(body["messages"][2]["role"], "tool");
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
    controller
        .submit_prompt(&mut |_| Ok(()))
        .expect("submit prompt");
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

    restored
        .handle_key_event(
            wonder_of_u_tui::KeyEvent {
                code: wonder_of_u_tui::KeyCode::Enter,
                modifiers: wonder_of_u_tui::KeyModifiers::default(),
            },
            &mut |_| Ok(()),
        )
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

    controller
        .handle_dialog_key(
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: wonder_of_u_tui::KeyModifiers::default(),
            },
            None,
            &mut |_| Ok(()),
        )
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
        controller
            .handle_event(UiEvent::Tick, |_| Ok(()))
            .expect("tick");
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

    controller
        .handle_dialog_key(
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: wonder_of_u_tui::KeyModifiers::default(),
            },
            None,
            &mut |_| Ok(()),
        )
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

    controller
        .handle_event(UiEvent::FocusLost, |_| Ok(()))
        .expect("lose focus");
    controller
        .handle_event(UiEvent::Tick, |_| Ok(()))
        .expect("tick");
    assert_eq!(
        controller.view().notifications.len(),
        1,
        "TTL should pause while the terminal is unfocused"
    );

    controller
        .handle_event(UiEvent::FocusGained, |_| Ok(()))
        .expect("gain focus");
    controller
        .handle_event(UiEvent::Tick, |_| Ok(()))
        .expect("tick");
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
    controller
        .execute_slash_command("/tasks")
        .expect("run /tasks with no tasks");

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

    controller
        .execute_slash_command("/model")
        .expect("open model picker");

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

    controller
        .execute_slash_command("/model")
        .expect("open model picker");

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
    controller
        .execute_slash_command("/model")
        .expect("open model picker");
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

    controller
        .handle_key_event(
            wonder_of_u_tui::KeyEvent {
                code: wonder_of_u_tui::KeyCode::Char('c'),
                modifiers: wonder_of_u_tui::KeyModifiers {
                    control: true,
                    ..wonder_of_u_tui::KeyModifiers::default()
                },
            },
            &mut |_| Ok(()),
        )
        .expect("interrupt");

    assert!(!controller.exit_requested);
    assert_eq!(controller.status_note.as_deref(), Some("confirm exit"));
    assert!(controller.dialog.is_some());

    controller
        .handle_key_event(
            wonder_of_u_tui::KeyEvent {
                code: wonder_of_u_tui::KeyCode::Enter,
                modifiers: wonder_of_u_tui::KeyModifiers::default(),
            },
            &mut |_| Ok(()),
        )
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

    controller
        .handle_key_event(
            wonder_of_u_tui::KeyEvent {
                code: wonder_of_u_tui::KeyCode::Esc,
                modifiers: wonder_of_u_tui::KeyModifiers::default(),
            },
            &mut |_| Ok(()),
        )
        .expect("enter normal mode");
    assert_eq!(controller.vim.mode(), VimMode::Normal);
    assert_eq!(controller.prompt.cursor(), 2);

    controller
        .handle_key_event(
            wonder_of_u_tui::KeyEvent {
                code: wonder_of_u_tui::KeyCode::Char('x'),
                modifiers: wonder_of_u_tui::KeyModifiers::default(),
            },
            &mut |_| Ok(()),
        )
        .expect("delete char");
    assert_eq!(controller.prompt.text(), "ab");

    controller
        .handle_key_event(
            wonder_of_u_tui::KeyEvent {
                code: wonder_of_u_tui::KeyCode::Char('a'),
                modifiers: wonder_of_u_tui::KeyModifiers::default(),
            },
            &mut |_| Ok(()),
        )
        .expect("append after cursor");
    assert_eq!(controller.vim.mode(), VimMode::Insert);

    controller
        .handle_key_event(
            wonder_of_u_tui::KeyEvent {
                code: wonder_of_u_tui::KeyCode::Char('z'),
                modifiers: wonder_of_u_tui::KeyModifiers::default(),
            },
            &mut |_| Ok(()),
        )
        .expect("insert after append");
    assert_eq!(controller.prompt.text(), "abz");
    assert_eq!(controller.status_note, None);
    assert_eq!(controller.vim.mode(), VimMode::Insert);
}

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
            }],
            results: Vec::new(),
        },
        pending_call: PendingLocalToolCall {
            provider_call: PendingProviderToolCall {
                call_id: "call_bash".into(),
                tool_name: "bash".into(),
                arguments: serde_json::json!({ "command": "echo hi" }),
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
        Some("claude-3-7-sonnet-latest".into()),
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
    assert_eq!(
        controller.state.model.as_deref(),
        Some("claude-3-7-sonnet-latest")
    );
    assert!(controller.state.auth.is_ready());
    // Provider/runtime details now live in state rather than the compact footer.
    assert!(controller.view().footer.contains("shift+tab to cycle"));
}

#[test]
fn prompt_cursor_tracks_edit_position_inside_prompt_panel() {
    // CHROME_HEIGHT=1: available=9, prompt at y=6, height=3 → content at x=1, y=7.
    // cursor=2 → after "ab" → line=0, col=2, x_offset=2 → (1+2+2, 7) = (5, 7).
    let (x, y) = prompt_cursor_position(40, 10, "abc", 2, false, false);
    assert_eq!((x, y), (5, 7));
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
    write_provider_config_for(&dir, "anthropic", "claude-3-7-sonnet-latest", &api_base);
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller
        .execute_slash_command("/brief")
        .expect("enable brief");
    assert!(controller.state.brief_mode, "brief mode should be enabled");

    controller
        .state
        .queue_command("hello", wonder_of_u_core::QueuePlacement::Now);
    controller
        .drain_queued_commands(&mut |_| Ok(()))
        .expect("drain prompt");

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

    controller
        .execute_slash_command("/brief")
        .expect("enable brief");
    assert!(controller.state.brief_mode, "brief mode should be on");

    controller
        .execute_slash_command("/clear")
        .expect("clear session");

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

    // Run a command that appends a message to the transcript.
    controller
        .execute_slash_command("/model openai:gpt-4.1")
        .expect("execute slash");

    // After the command a message was persisted → scroll state updated.
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
        controller.execute_slash_command(cmd).expect("slash cmd");
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
        controller
            .execute_slash_command("/model openai:gpt-4.1")
            .expect("slash cmd");
    }

    // Simulate a small viewport so max_offset is non-zero.
    controller.scroll_state.last_visible_lines = 1;

    let initial_total = controller.scroll_state.last_total_lines;
    if initial_total > 1 {
        controller.scroll_state.scroll_by(1);
        assert!(!controller.scroll_state.is_following_tail());
        let offset_before = controller.scroll_state.offset_from_bottom;

        // Add another message.
        controller
            .execute_slash_command("/model openai:gpt-4.1")
            .expect("slash cmd");

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

    controller
        .handle_event(
            UiEvent::Resize {
                width: 80,
                height: 40,
            },
            |_| Ok(()),
        )
        .expect("handle resize");

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

    controller
        .handle_event(
            UiEvent::Resize {
                width: 80,
                height: 24,
            },
            |_| Ok(()),
        )
        .expect("handle resize");

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
    controller
        .execute_slash_command("/setup")
        .expect("execute /setup");
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

fn scroll_mouse_event(kind: wonder_of_u_tui::MouseEventKind, col: u16, row: u16) -> UiEvent {
    use crossterm::event::{
        KeyModifiers as CrosstermMods, MouseEvent as CrosstermME, MouseEventKind as CrosstermMEK,
    };
    let ct_kind = match kind {
        wonder_of_u_tui::MouseEventKind::ScrollUp => CrosstermMEK::ScrollUp,
        wonder_of_u_tui::MouseEventKind::ScrollDown => CrosstermMEK::ScrollDown,
        wonder_of_u_tui::MouseEventKind::ScrollLeft => CrosstermMEK::ScrollLeft,
        wonder_of_u_tui::MouseEventKind::ScrollRight => CrosstermMEK::ScrollRight,
        _ => panic!("scroll_mouse_event: unsupported MouseEventKind"),
    };
    UiEvent::Mouse(CrosstermME {
        kind: ct_kind,
        column: col,
        row,
        modifiers: CrosstermMods::empty(),
    })
}

#[test]
fn controller_mouse_scroll_up_inside_transcript_scrolls_toward_older() {
    let dir = unique_test_dir("mouse-scroll-up");
    write_provider_config(&dir, "http://127.0.0.1:1/v1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.on_terminal_resize(80, 24);
    controller.scroll_state.on_resize(20, 100);

    assert_eq!(
        controller.scroll_state.offset_from_bottom, 0,
        "starts at tail"
    );
    controller.handle_mouse_event(scroll_mouse_event(
        wonder_of_u_tui::MouseEventKind::ScrollUp,
        40,
        5,
    ));
    assert_eq!(
        controller.scroll_state.offset_from_bottom, 3,
        "ScrollUp must advance 3 lines toward older content"
    );
    assert!(controller.needs_render);
}

#[test]
fn controller_mouse_scroll_down_inside_transcript_scrolls_toward_newer() {
    let dir = unique_test_dir("mouse-scroll-down");
    write_provider_config(&dir, "http://127.0.0.1:1/v1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.on_terminal_resize(80, 24);
    controller.scroll_state.on_resize(20, 100);

    controller.scroll_state.scroll_by(10);
    controller.handle_mouse_event(scroll_mouse_event(
        wonder_of_u_tui::MouseEventKind::ScrollDown,
        40,
        5,
    ));
    assert_eq!(
        controller.scroll_state.offset_from_bottom, 7,
        "ScrollDown must retreat 3 lines toward newer content"
    );
}

#[test]
fn controller_mouse_scroll_outside_transcript_area_ignored() {
    let dir = unique_test_dir("mouse-scroll-outside");
    write_provider_config(&dir, "http://127.0.0.1:1/v1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.on_terminal_resize(80, 24);
    controller.scroll_state.on_resize(20, 100);

    // row 22 falls in the prompt/chrome zone of an 80×24 terminal
    controller.handle_mouse_event(scroll_mouse_event(
        wonder_of_u_tui::MouseEventKind::ScrollUp,
        40,
        22,
    ));
    assert_eq!(
        controller.scroll_state.offset_from_bottom, 0,
        "wheel over the prompt zone must be ignored"
    );
}

#[test]
fn controller_mouse_scroll_ignored_when_overlay_active() {
    let dir = unique_test_dir("mouse-scroll-overlay");
    write_provider_config(&dir, "http://127.0.0.1:1/v1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.on_terminal_resize(80, 24);
    controller.scroll_state.on_resize(20, 100);

    controller.dialog = Some(wonder_of_u_tui::DialogView::notice(
        "Overlay active",
        ["This dialog prevents scrolling"],
    ));
    controller.handle_mouse_event(scroll_mouse_event(
        wonder_of_u_tui::MouseEventKind::ScrollUp,
        40,
        5,
    ));
    assert_eq!(
        controller.scroll_state.offset_from_bottom, 0,
        "scroll must be suppressed while a dialog is shown"
    );
}

#[test]
fn controller_horizontal_mouse_wheel_ignored() {
    let dir = unique_test_dir("mouse-scroll-horiz");
    write_provider_config(&dir, "http://127.0.0.1:1/v1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.on_terminal_resize(80, 24);
    controller.scroll_state.on_resize(20, 100);

    controller.handle_mouse_event(scroll_mouse_event(
        wonder_of_u_tui::MouseEventKind::ScrollLeft,
        40,
        5,
    ));
    controller.handle_mouse_event(scroll_mouse_event(
        wonder_of_u_tui::MouseEventKind::ScrollRight,
        40,
        5,
    ));
    assert_eq!(
        controller.scroll_state.offset_from_bottom, 0,
        "horizontal wheel events must not change the scroll position"
    );
}

// ── autostart tests ───────────────────────────────────────────────────────────

fn make_unconfigured_controller() -> (TuiController<'static>, PathBuf) {
    let dir = unique_test_dir("tui-autostart-unconfigured");
    let registry = Box::new(commands::registry(Some(dir.clone())).expect("registry"));
    let registry: &'static _ = Box::leak(registry);
    let controller = TuiController::new(
        test_context(&dir),
        registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    (controller, dir)
}

fn make_ready_controller() -> (TuiController<'static>, PathBuf) {
    let dir = unique_test_dir("tui-autostart-ready");
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = Box::new(commands::registry(Some(dir.clone())).expect("registry"));
    let registry: &'static _ = Box::leak(registry);
    let controller = TuiController::new(
        test_context(&dir),
        registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    (controller, dir)
}

#[test]
fn controller_auto_opens_setup_when_provider_not_configured() {
    let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
    let _oai_key = EnvVarGuard::set("OPENAI_API_KEY", "");
    let (controller, _dir) = make_unconfigured_controller();

    assert!(
        controller.pending_setup_overlay.is_some(),
        "setup overlay should be auto-opened when provider is not configured"
    );
    assert!(
        !controller.setup_cancelled_this_session,
        "cancelled flag must be false on first launch"
    );
}

#[test]
fn controller_does_not_auto_open_setup_when_provider_ready() {
    let (controller, _dir) = make_ready_controller();

    assert!(
        controller.pending_setup_overlay.is_none(),
        "setup overlay must NOT open when provider is already configured"
    );
}

#[test]
fn controller_cancel_setup_sets_session_flag_and_prevents_reopen() {
    let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
    let _oai_key = EnvVarGuard::set("OPENAI_API_KEY", "");
    let (mut controller, _dir) = make_unconfigured_controller();

    controller
        .cancel_setup_overlay()
        .expect("cancel_setup_overlay");

    assert!(
        controller.setup_cancelled_this_session,
        "cancel must set the session flag"
    );
    assert!(
        controller.pending_setup_overlay.is_none(),
        "overlay must be closed after cancel"
    );

    controller
        .maybe_auto_open_setup()
        .expect("maybe_auto_open_setup");
    assert!(
        controller.pending_setup_overlay.is_none(),
        "session flag must prevent autostart from re-opening the overlay"
    );
}

// -- Provider-form tests ------------------------------------------------------

#[test]
fn provider_form_opens_via_open_provider_form() {
    let (mut controller, _dir) = open_setup_overlay_controller();

    controller.open_provider_form(crate::tui_runtime::setup::ProviderFormKind::ApiKey);

    assert!(
        controller.pending_provider_form.is_some(),
        "pending_provider_form should be Some after open_provider_form"
    );
    assert!(
        controller.pending_setup_overlay.is_none(),
        "setup overlay should be cleared when provider form opens"
    );
    assert!(
        controller.has_picker_overlay(),
        "has_picker_overlay() should be true while provider form is open"
    );
}

#[test]
fn provider_form_opens_on_login_item() {
    let (mut controller, _dir) = open_setup_overlay_controller();

    let overlay = controller
        .pending_setup_overlay
        .as_ref()
        .expect("overlay open");
    let login_idx = overlay
        .items
        .iter()
        .position(|i| i.id == "login")
        .expect("login item");
    for _ in 0..login_idx {
        send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
    }
    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    assert!(
        controller.pending_provider_form.is_some(),
        "provider form should open after selecting login item"
    );
    assert!(
        controller.pending_setup_overlay.is_none(),
        "setup overlay should close when provider form opens"
    );
}

#[test]
fn provider_form_esc_cancels_from_stage1() {
    let (mut controller, _dir) = open_setup_overlay_controller();
    controller.open_provider_form(crate::tui_runtime::setup::ProviderFormKind::ApiKey);

    send_dialog_key(&mut controller, picker_key(KeyCode::Esc), None);

    assert!(
        controller.pending_provider_form.is_none(),
        "Esc in stage-1 should cancel the provider form"
    );
    assert_eq!(
        controller.status_note.as_deref(),
        Some("provider form cancelled"),
    );
}

#[test]
fn provider_form_esc_in_stage2_goes_back() {
    use crate::tui_runtime::setup::ProviderFormStage;

    let (mut controller, _dir) = open_setup_overlay_controller();
    controller.open_provider_form(crate::tui_runtime::setup::ProviderFormKind::ApiBase);

    // Tab advances to stage-2.
    send_dialog_key(&mut controller, picker_key(KeyCode::Tab), None);
    {
        let form = controller
            .pending_provider_form
            .as_ref()
            .expect("form open");
        assert_eq!(form.stage, ProviderFormStage::EnterValue);
    }

    // Esc goes back to stage-1.
    send_dialog_key(&mut controller, picker_key(KeyCode::Esc), None);
    {
        let form = controller
            .pending_provider_form
            .as_ref()
            .expect("form still open");
        assert_eq!(form.stage, ProviderFormStage::PickProvider);
    }
}

#[test]
fn provider_form_has_picker_list_view() {
    let (mut controller, _dir) = open_setup_overlay_controller();
    controller.open_provider_form(crate::tui_runtime::setup::ProviderFormKind::ApiKey);

    let view = controller.current_picker_list_view();
    assert!(
        view.is_some(),
        "picker list view should be present while form is open"
    );
    let picker = view.unwrap();
    assert!(
        !picker.entries.is_empty(),
        "provider list must have at least one entry"
    );
    assert!(
        picker.title.contains("API Key"),
        "title should mention 'API Key'; got {:?}",
        picker.title
    );
}

#[test]
fn provider_form_dismiss_clears_form() {
    let (mut controller, _dir) = open_setup_overlay_controller();
    controller.open_provider_form(crate::tui_runtime::setup::ProviderFormKind::ApiBase);
    assert!(controller.has_picker_overlay());

    controller.dismiss_dialog();

    assert!(!controller.has_picker_overlay());
    assert!(controller.pending_provider_form.is_none());
}

#[test]
fn provider_form_api_key_write_redacts_key_in_status_note() {
    let (mut controller, dir) = open_setup_overlay_controller();
    controller.open_provider_form(crate::tui_runtime::setup::ProviderFormKind::ApiKey);

    // Advance to stage-2 with Tab.
    send_dialog_key(&mut controller, picker_key(KeyCode::Tab), None);

    // Type a fake secret.
    let secret = "sk-testkey9999";
    for c in secret.chars() {
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Char(c)),
            Some(ResolvedKey::InsertChar(c)),
        );
    }

    // Submit.
    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    assert!(
        controller.pending_provider_form.is_none(),
        "form should close after submit"
    );

    let note = controller.status_note.clone().unwrap_or_default();
    assert!(
        !note.contains(secret),
        "status note must NOT contain the raw API key; got: {note:?}"
    );

    let creds = CredentialStore::new(dir.as_path())
        .read()
        .expect("read credentials");
    assert!(
        !creds.providers.is_empty(),
        "at least one credential should be stored after API-key form submission"
    );
}

// ── Copilot OAuth flow tests ──────────────────────────────────────────────────

#[test]
fn copilot_oauth_action_for_item_id_returns_copilot_oauth() {
    use crate::tui_runtime::setup::{SetupItemAction, action_for_item_id};

    let action = action_for_item_id("copilot-oauth", "/setup");
    assert!(
        matches!(action, SetupItemAction::CopilotOAuth),
        "copilot-oauth item should map to CopilotOAuth action, got {action:?}"
    );
}

/// Build a controller with `CopilotOAuthFlowState::AwaitingConfirmation` already set,
/// avoiding a real network call for unit tests.
fn make_controller_with_copilot_awaiting() -> (TuiController<'static>, tempfile::TempDir) {
    use wonder_of_u_agent::CopilotDeviceCode;
    use wonder_of_u_tui::DialogActionView;

    let dir_obj = tempfile::TempDir::new().expect("tempdir");
    let dir = dir_obj.path().to_path_buf();
    let registry = Box::new(commands::registry(Some(dir.clone())).expect("registry"));
    let registry: &'static _ = Box::leak(registry);
    let mut controller = TuiController::new(
        test_context(&dir),
        registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    let fake_device_code = CopilotDeviceCode {
        device_code: "fake-device-secret".into(),
        user_code: "ABCD-1234".into(),
        verification_uri: "https://github.com/login/device".into(),
        expires_in: 900,
        interval: 5,
    };
    let dialog = wonder_of_u_tui::DialogView {
        title: "GitHub Copilot Login".into(),
        body: vec![
            format!("1. Visit:      {}", fake_device_code.verification_uri),
            format!("2. Enter code: {}", fake_device_code.user_code),
            String::new(),
            "Press Enter to open the browser and wait for authorization.".into(),
            "Press Esc to cancel.".into(),
        ],
        actions: vec![
            DialogActionView::new("Open Browser", true),
            DialogActionView::new("Cancel", false),
        ],
    };
    controller.dialog = Some(dialog);
    controller.pending_setup_overlay = None; // real flow clears this; mirror that here
    controller.pending_copilot_oauth = Some(
        crate::tui_runtime::setup::CopilotOAuthFlowState::AwaitingConfirmation {
            device_code: fake_device_code,
        },
    );

    (controller, dir_obj)
}

#[test]
fn copilot_oauth_awaiting_shows_dialog_with_url_and_code() {
    let (controller, _dir) = make_controller_with_copilot_awaiting();

    let dialog = controller
        .dialog
        .as_ref()
        .expect("dialog should be set when oauth flow is AwaitingConfirmation");
    assert_eq!(dialog.title, "GitHub Copilot Login");
    let body_text = dialog.body.join("\n");
    assert!(
        body_text.contains("https://github.com/login/device"),
        "dialog body must show the verification URL; got: {body_text:?}"
    );
    assert!(
        body_text.contains("ABCD-1234"),
        "dialog body must show the user code; got: {body_text:?}"
    );
    // Raw device_code secret must never appear in the dialog.
    assert!(
        !body_text.contains("fake-device-secret"),
        "dialog body must NOT contain the raw device_code; got: {body_text:?}"
    );
}

#[test]
fn tui_browser_launch_is_disabled_under_tests() {
    assert!(
        browser_launch_disabled(),
        "TUI tests must never spawn a real system browser"
    );
}

#[test]
fn copilot_oauth_esc_cancels_awaiting_confirmation() {
    let (mut controller, _dir) = make_controller_with_copilot_awaiting();

    send_dialog_key(&mut controller, picker_key(KeyCode::Esc), None);

    assert!(
        controller.pending_copilot_oauth.is_none(),
        "AwaitingConfirmation should be cleared after Esc"
    );
    assert!(
        controller.dialog.is_none(),
        "dialog should be dismissed after Esc"
    );
    assert_eq!(
        controller.status_note.as_deref(),
        Some("copilot oauth cancelled"),
        "status note should report cancellation"
    );
}

#[test]
fn copilot_oauth_polling_esc_cancels() {
    use std::sync::mpsc;
    use wonder_of_u_agent::CopilotOAuthToken;

    let dir_obj = tempfile::TempDir::new().expect("tempdir");
    let dir = dir_obj.path().to_path_buf();
    let registry = Box::new(commands::registry(Some(dir.clone())).expect("registry"));
    let registry: &'static _ = Box::leak(registry);
    let mut controller = TuiController::new(
        test_context(&dir),
        registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // Channel with nothing sent yet — simulates an in-progress poll.
    let (_tx, rx) = mpsc::channel::<wonder_of_u_core::Result<CopilotOAuthToken>>();
    controller.dialog = Some(wonder_of_u_tui::DialogView::notice(
        "GitHub Copilot Login",
        ["Waiting for authorization…"],
    ));
    controller.pending_setup_overlay = None; // real flow clears this; mirror that here
    controller.pending_copilot_oauth =
        Some(crate::tui_runtime::setup::CopilotOAuthFlowState::Polling {
            user_code: "ABCD-1234".into(),
            result_rx: rx,
        });

    send_dialog_key(&mut controller, picker_key(KeyCode::Esc), None);

    assert!(
        controller.pending_copilot_oauth.is_none(),
        "Polling state should be cleared after Esc"
    );
    assert!(
        controller.dialog.is_none(),
        "dialog should be dismissed after Esc"
    );
    assert_eq!(
        controller.status_note.as_deref(),
        Some("copilot oauth polling cancelled"),
    );
}

#[test]
fn copilot_oauth_tick_stores_token_and_redacts_in_status_note() {
    use std::sync::mpsc;
    use wonder_of_u_agent::{AuthMaterial, CopilotOAuthToken};

    let dir_obj = tempfile::TempDir::new().expect("tempdir");
    let dir = dir_obj.path().to_path_buf();
    let registry = Box::new(commands::registry(Some(dir.clone())).expect("registry"));
    let registry: &'static _ = Box::leak(registry);
    let mut controller = TuiController::new(
        test_context(&dir),
        registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    let secret_token = "ghu_super_secret_access_token_xyz";
    let (tx, rx) = mpsc::channel::<wonder_of_u_core::Result<CopilotOAuthToken>>();
    tx.send(Ok(CopilotOAuthToken {
        access_token: secret_token.into(),
        refresh_token: Some("ghu_refresh_token".into()),
        expires_at: None,
    }))
    .expect("send token");

    controller.dialog = Some(wonder_of_u_tui::DialogView::notice(
        "GitHub Copilot Login",
        ["Waiting…"],
    ));
    controller.pending_setup_overlay = None; // real flow clears this; mirror that here
    controller.pending_copilot_oauth =
        Some(crate::tui_runtime::setup::CopilotOAuthFlowState::Polling {
            user_code: "ABCD-1234".into(),
            result_rx: rx,
        });

    controller
        .tick_copilot_oauth_poll()
        .expect("tick should succeed");

    assert!(
        controller.pending_copilot_oauth.is_none(),
        "flow state should be cleared after successful poll"
    );
    assert!(
        controller.dialog.is_none(),
        "dialog should be dismissed after successful poll"
    );

    let note = controller.status_note.clone().unwrap_or_default();
    // Must confirm success without leaking the token.
    assert!(
        !note.contains(secret_token),
        "status note must NOT contain the raw access token; got: {note:?}"
    );
    assert!(
        note.contains("authorized"),
        "status note should confirm successful authorization; got: {note:?}"
    );

    // Token must be persisted to CredentialStore.
    let creds = CredentialStore::new(dir.as_path())
        .read()
        .expect("read credentials");
    let copilot = creds
        .providers
        .get("copilot")
        .expect("copilot credential must be stored");
    assert!(
        matches!(
            copilot,
            AuthMaterial::OAuth {
                access_token: Some(_),
                ..
            }
        ),
        "copilot credential must be OAuth with an access_token; got: {copilot:?}"
    );
    if let AuthMaterial::OAuth {
        access_token: Some(stored),
        ..
    } = copilot
    {
        assert_eq!(stored, secret_token);
    }
}

#[test]
fn copilot_oauth_tick_on_error_sets_status_note() {
    use std::sync::mpsc;
    use wonder_of_u_agent::CopilotOAuthToken;
    use wonder_of_u_core::WonderError;

    let dir_obj = tempfile::TempDir::new().expect("tempdir");
    let dir = dir_obj.path().to_path_buf();
    let registry = Box::new(commands::registry(Some(dir.clone())).expect("registry"));
    let registry: &'static _ = Box::leak(registry);
    let mut controller = TuiController::new(
        test_context(&dir),
        registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    let (tx, rx) = mpsc::channel::<wonder_of_u_core::Result<CopilotOAuthToken>>();
    tx.send(Err(WonderError::validation("device code expired")))
        .expect("send error");

    controller.pending_setup_overlay = None; // real flow clears this; mirror that here
    controller.pending_copilot_oauth =
        Some(crate::tui_runtime::setup::CopilotOAuthFlowState::Polling {
            user_code: "ABCD-1234".into(),
            result_rx: rx,
        });

    controller
        .tick_copilot_oauth_poll()
        .expect("tick should not panic on poll error");

    assert!(
        controller.pending_copilot_oauth.is_none(),
        "flow state should be cleared after poll error"
    );
    let note = controller.status_note.clone().unwrap_or_default();
    assert!(
        note.contains("Copilot OAuth failed"),
        "status note should report failure; got: {note:?}"
    );
}

#[test]
fn copilot_oauth_tick_noop_when_no_state() {
    let dir_obj = tempfile::TempDir::new().expect("tempdir");
    let dir = dir_obj.path().to_path_buf();
    let registry = Box::new(commands::registry(Some(dir.clone())).expect("registry"));
    let registry: &'static _ = Box::leak(registry);
    let mut controller = TuiController::new(
        test_context(&dir),
        registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    assert!(controller.pending_copilot_oauth.is_none());
    let note_before = controller.status_note.clone();
    controller
        .tick_copilot_oauth_poll()
        .expect("tick noop should not error");
    // tick is a no-op when there is no pending oauth state; status note is unchanged.
    assert!(controller.pending_copilot_oauth.is_none());
    assert_eq!(controller.status_note, note_before);
}

#[test]
fn copilot_oauth_dismiss_dialog_clears_flow() {
    let (mut controller, _dir) = make_controller_with_copilot_awaiting();
    assert!(controller.pending_copilot_oauth.is_some());
    assert!(controller.dialog.is_some());

    controller.dismiss_dialog();

    assert!(
        controller.pending_copilot_oauth.is_none(),
        "dismiss_dialog should clear pending_copilot_oauth"
    );
    assert!(controller.dialog.is_none());
}

#[test]
fn copilot_oauth_setup_overlay_item_has_copilot_oauth_action() {
    use crate::tui_runtime::setup::SetupItemAction;

    let (controller, _dir) = open_setup_overlay_controller();
    let overlay = controller
        .pending_setup_overlay
        .as_ref()
        .expect("setup overlay open");
    let item = overlay
        .items
        .iter()
        .find(|i| i.id == "copilot-oauth")
        .expect("copilot-oauth item exists");
    assert!(
        matches!(item.action, SetupItemAction::CopilotOAuth),
        "copilot-oauth item must use CopilotOAuth action; got {:?}",
        item.action
    );
}

// ── sanitize_error_for_display ────────────────────────────────────────────────

#[test]
fn sanitize_error_bearer_token_is_redacted() {
    // Raw provider errors may contain Authorization headers or inline Bearer
    // tokens.  These must never reach the persistent transcript.
    let raw = "request failed: Bearer supersecrettoken123 trailing text";
    let sanitized = sanitize_error_for_display(raw);
    assert!(
        !sanitized.contains("supersecrettoken123"),
        "bearer token must be redacted; got: {sanitized:?}"
    );
    assert!(
        sanitized.contains("Bearer [REDACTED]"),
        "placeholder must be present; got: {sanitized:?}"
    );
    // Non-sensitive parts of the message must survive.
    assert!(
        sanitized.contains("request failed"),
        "non-sensitive prefix must survive; got: {sanitized:?}"
    );
}

#[test]
fn sanitize_error_sk_key_is_redacted() {
    // OpenAI-style `sk-...` keys embedded in error messages are redacted.
    let raw = r#"{"error":"invalid key","key":"sk-abc123XYZ"}"#;
    let sanitized = sanitize_error_for_display(raw);
    assert!(
        !sanitized.contains("sk-abc123XYZ"),
        "sk- key must be redacted; got: {sanitized:?}"
    );
    assert!(
        sanitized.contains("sk-[REDACTED]"),
        "redacted placeholder must be present; got: {sanitized:?}"
    );
}

#[test]
fn sanitize_error_authorization_header_is_redacted() {
    // Multi-line error dumps that include HTTP headers must have the
    // Authorization line fully replaced so the value is never persisted.
    let raw =
        "HTTP/1.1 401 Unauthorized\nAuthorization: Bearer tok999\nContent-Type: application/json";
    let sanitized = sanitize_error_for_display(raw);
    assert!(
        !sanitized.contains("tok999"),
        "auth value must be redacted; got: {sanitized:?}"
    );
    assert!(
        sanitized.contains("Authorization: [REDACTED]"),
        "placeholder must replace the header value; got: {sanitized:?}"
    );
    // Other headers must survive.
    assert!(
        sanitized.contains("Content-Type"),
        "non-sensitive headers must survive; got: {sanitized:?}"
    );
}

#[test]
fn sanitize_error_long_message_is_preserved() {
    // Provider errors can include useful multi-line diagnostics. The history
    // entry should preserve the full text after credential redaction so users
    // can inspect the complete failure.
    let long_msg = "x".repeat(500);
    let sanitized = sanitize_error_for_display(&long_msg);
    assert_eq!(sanitized.chars().count(), 500);
    assert_eq!(sanitized, long_msg);
}

#[test]
fn sanitize_error_plain_message_passes_through_unchanged() {
    let plain = "connection refused (os error 111)";
    let sanitized = sanitize_error_for_display(plain);
    assert_eq!(
        sanitized, plain,
        "plain messages without credentials must be unchanged"
    );
}

// ── prompt / history-search cursor at small terminal height ──────────────────

/// Box-layout cap sanity: at height=6 the renderer uses `(h/3).max(3) = 3`.
/// A 3-line prompt has uncapped box height = 5, capped to 3.  With
/// CHROME_HEIGHT=1 the prompt rect lands at y=2 so the first content row is y=3.
#[test]
fn prompt_cursor_position_small_terminal_respects_renderer_cap() {
    // 3-line prompt: uncapped box height = 5.  At height=6, cap = max(2, 3) = 3.
    let (x, y) = prompt_cursor_position(40, 6, "line1\nline2\nline3", 5, false, false);
    // cursor=5 → "line1" (no newline seen) → line=0, col=5, first-line x_offset=2.
    // CHROME_HEIGHT=1: available=5, messages=2, prompt=(0,2,40,3),
    // content_x=1, content_y=3, content_height=1.
    assert_eq!(
        (x, y),
        (8, 3),
        "cursor row must land inside the prompt area rendered at the correct cap; got ({x}, {y})"
    );
}

/// Same cap-formula check for `history_search_cursor_position`.  At height=6
/// (box cap=3, CHROME_HEIGHT=1) a history-search view with no match has uncapped
/// height=5 which gets capped to 3, placing content_y=3.
#[test]
fn history_search_cursor_position_small_terminal_respects_renderer_cap() {
    let view = HistorySearchView {
        query: "abc".into(),
        match_text: None,
        match_index: 0,
        match_total: 0,
    };
    // query_cursor=3 → x = content_x(1) + "search: ".len()(8) + 3 = 12
    // CHROME_HEIGHT=1: available=5, messages=2, prompt=(0,2,40,3), content_y=3.
    let (x, y) = history_search_cursor_position(40, 6, &view, 3, false, false);
    assert_eq!(
        (x, y),
        (12, 3),
        "history-search cursor must use the renderer's box cap; got ({x}, {y})"
    );
}

/// Continuation lines (line index > 0) must use `x_offset = 0` because the
/// `› ` marker only appears on the first prompt line.  This is the key
/// distinction from the first-line path and ensures the cursor lands at the
/// correct column inside the restored rounded-bordered box.
#[test]
fn prompt_cursor_position_continuation_line_has_no_marker_offset() {
    // "abc\ndef" with cursor=6 points to 'f' on the second line.
    // Processing "abc\nde" → a(col=1), b(col=2), c(col=3), \n(line=1, col=0),
    // d(col=1), e(col=2) → line=1, col=2.
    // On 40×16 (CHROME_HEIGHT=1): available=15, cap=max(5,3)=5.
    //   prompt_height("abc\ndef")=4 (2 lines + 2 borders); capped=4 (fits).
    //   message_height=15-4=11; prompt = Rect(0, 11, 40, 4).
    //   content_x=1, content_y=12, content_height=2.
    // line=1 is within content_height — the continuation path applies.
    //   x = content_x(1) + x_offset(0) + col(2) = 3
    //   y = content_y(12) + line(1) = 13
    let (x, y) = prompt_cursor_position(40, 16, "abc\ndef", 6, false, false);
    assert_eq!(
        (x, y),
        (3, 13),
        "continuation-line cursor must use x_offset=0 (no marker); got ({x}, {y})"
    );
}

// ── wide-terminal cursor regression (sidebar column deduction) ────────────────

/// On a 100-column terminal the renderer carves out SIDEBAR_WIDTH+1 columns,
/// leaving `effective_width = 61` for the prompt box.  Before the fix, both
/// cursor helpers used the raw terminal width (100) for layout, so a prompt/query
/// long enough to push the cursor past column 60 would land inside the sidebar.
///
/// Concrete geometry for width=100, height=24, CHROME_HEIGHT=1, sidebar_active=true:
///   effective_width = 61
///   prompt box = Rect(0, 20, 61, 3)  →  content_x=1, content_width=59
///   first-line max cursor x = content_x(1) + x_offset(2) + max_col(56) = 59
///   separator sits at column 61 (render_shell draws │ there)
///   → cursor x must be < 61

#[test]
fn prompt_cursor_position_wide_terminal_stays_inside_main_area() {
    // 80-char single-line prompt; cursor at the very end.  Before the fix the
    // unclamped x would be 1+2+80 = 83, well inside the sidebar (61..99).
    let prompt = "a".repeat(80);
    let (x, y) = prompt_cursor_position(100, 24, &prompt, 80, true, false);

    // main_w = 61; separator at column 61; valid main-area columns = 0..=60.
    assert!(
        x < 61,
        "cursor x ({x}) must be left of the │ separator at column 61"
    );
    // Max reachable: content_x(1) + x_offset(2) + max_col(56) = 59.
    // CHROME_HEIGHT=1: content_y = 20 + 1 = 21.
    assert_eq!(
        (x, y),
        (59, 21),
        "wide-terminal cursor must clamp to the rightmost content cell; got ({x}, {y})"
    );
}

/// Same regression gate for `history_search_cursor_position`.  A query whose
/// length (plus the 8-char "search: " prefix) exceeds the content width of the
/// main area must be clamped to the last valid content column.
///
/// Width=100, sidebar_active=true → effective_width=61 → content_width=59.
/// max x = content_x(1) + content_width(59) - 1 = 59.
#[test]
fn history_search_cursor_wide_terminal_stays_inside_main_area() {
    // query_cursor=90 → prefix(8)+cursor(90)=98.  Before the fix:
    //   content_width = 98 (layout over full 100 cols), x = min(99, 98) = 98 → sidebar!
    // After fix:
    //   content_width = 59, x = min(99, 59) = 59 → stays in main area.
    let view = HistorySearchView {
        query: "a".repeat(90),
        match_text: None,
        match_index: 0,
        match_total: 0,
    };
    let (x, y) = history_search_cursor_position(100, 24, &view, 90, true, false);

    assert!(
        x < 61,
        "history-search cursor x ({x}) must be left of the │ separator at column 61"
    );
    // CHROME_HEIGHT=1: history search (3-line box → 5 rows capped) places
    // prompt at y=18; content_y=19.
    assert_eq!(
        (x, y),
        (59, 19),
        "wide-terminal history-search cursor must clamp to rightmost content cell; got ({x}, {y})"
    );
}

/// Verify that a narrow terminal (width=99, one below the sidebar threshold)
/// is unaffected by the sidebar logic even when sidebar_active=true.  The
/// prompt layout still spans the full terminal width.
#[test]
fn prompt_cursor_position_just_below_sidebar_threshold_uses_full_width() {
    // At width=99 shell_main_area_width returns 99 regardless of sidebar_active.
    // 10-char prompt, cursor at end → line=0, col=10, x_offset=2.
    // effective_width=99; CHROME_HEIGHT=1: available=19, messages=16, prompt at y=16.
    // content_y=17, x=1+2+10=13.
    let (x, y) = prompt_cursor_position(99, 20, &"a".repeat(10), 10, true, false);
    assert_eq!(
        (x, y),
        (13, 17),
        "terminal just below sidebar threshold must use full width; got ({x}, {y})"
    );
}

// ── combined-pattern sanitization ─────────────────────────────────────────────

/// An error message that contains both an `sk-` API key *and* a Bearer token
/// must have both patterns independently redacted.
#[test]
fn sanitize_error_combined_credential_patterns_are_both_redacted() {
    let raw = "request failed: Authorization: Bearer eyJhbGciOiJSUzI1NiJ9.payload, \
               key=sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ12345678";
    let sanitized = sanitize_error_for_display(raw);
    assert!(
        !sanitized.contains("eyJhbGciOiJSUzI1NiJ9"),
        "Bearer token value must be redacted; got: {sanitized:?}"
    );
    assert!(
        !sanitized.contains("sk-proj-"),
        "sk- key must be redacted; got: {sanitized:?}"
    );
    assert!(
        sanitized.contains("[REDACTED]"),
        "redacted placeholder must appear; got: {sanitized:?}"
    );
}

// ── sidebar regression tests ──────────────────────────────────────────────────

/// After construction the sidebar panel must be enabled so first-time users see
/// the keybinding hints and session metadata without any extra setup.
#[test]
fn sidebar_default_visible() {
    let dir = unique_test_dir("tui-sidebar-default-visible");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    assert!(
        controller.sidebar_visible,
        "sidebar must default to visible after construction"
    );
}

/// Calling `toggle_sidebar()` twice must return to the original visible state.
/// The first call sets status_note to "sidebar off"; the second sets it to "sidebar on".
#[test]
fn sidebar_toggle_method_flips_and_restores() {
    let dir = unique_test_dir("tui-sidebar-toggle-method");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    assert!(controller.sidebar_visible, "precondition: starts visible");

    controller.toggle_sidebar();
    assert!(
        !controller.sidebar_visible,
        "sidebar must be hidden after first toggle"
    );
    assert_eq!(
        controller.status_note.as_deref(),
        Some("sidebar off"),
        "status note must read 'sidebar off' after hiding"
    );

    controller.toggle_sidebar();
    assert!(
        controller.sidebar_visible,
        "sidebar must be visible after second toggle"
    );
    assert_eq!(
        controller.status_note.as_deref(),
        Some("sidebar on"),
        "status note must read 'sidebar on' after restoring"
    );
}

/// Pressing Ctrl+B must flip `sidebar_visible` via the normal key-event path,
/// exercising the `is_ctrl_char('b')` intercept in `handle_key_event`.
///
/// Any auto-opened setup overlay is dismissed first so the key reaches the
/// sidebar intercept rather than being swallowed by `handle_dialog_key`.
#[test]
fn sidebar_ctrl_b_toggles_via_key_event() {
    let dir = unique_test_dir("tui-sidebar-ctrl-b");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // Dismiss any auto-opened setup overlay so Ctrl+B reaches the main key
    // handler; handle_key_event routes to handle_dialog_key when a picker
    // overlay is active, which would swallow the keystroke.
    controller.pending_setup_overlay = None;

    let was_visible = controller.sidebar_visible;

    let ctrl_b = KeyEvent {
        code: KeyCode::Char('b'),
        modifiers: wonder_of_u_tui::KeyModifiers {
            control: true,
            ..wonder_of_u_tui::KeyModifiers::default()
        },
    };
    send_prompt_key(&mut controller, ctrl_b);

    assert_eq!(
        controller.sidebar_visible, !was_visible,
        "Ctrl+B must flip sidebar_visible"
    );
}

/// The bare `/sidebar` command must behave as a toggle (same as `toggle_sidebar()`).
#[test]
fn sidebar_slash_command_bare_toggles() {
    let dir = unique_test_dir("tui-sidebar-slash-toggle");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    let original = controller.sidebar_visible;

    controller
        .execute_slash_command("/sidebar")
        .expect("/sidebar must not error");

    assert_eq!(
        controller.sidebar_visible, !original,
        "/sidebar bare must flip sidebar_visible"
    );
}

/// `/sidebar on` must unconditionally set visible; `/sidebar off` must hide;
/// `/sidebar toggle` must flip — verified in sequence to keep them independent.
#[test]
fn sidebar_slash_on_off_toggle_subcommands() {
    let dir = unique_test_dir("tui-sidebar-slash-on-off");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // `/sidebar off` must hide regardless of current state.
    controller
        .execute_slash_command("/sidebar off")
        .expect("/sidebar off must not error");
    assert!(
        !controller.sidebar_visible,
        "/sidebar off must set sidebar_visible=false"
    );
    assert_eq!(
        controller.status_note.as_deref(),
        Some("sidebar off"),
        "status note must read 'sidebar off'"
    );

    // `/sidebar on` must restore visibility.
    controller
        .execute_slash_command("/sidebar on")
        .expect("/sidebar on must not error");
    assert!(
        controller.sidebar_visible,
        "/sidebar on must set sidebar_visible=true"
    );
    assert_eq!(
        controller.status_note.as_deref(),
        Some("sidebar on"),
        "status note must read 'sidebar on'"
    );

    // `/sidebar toggle` must flip to hidden again.
    controller
        .execute_slash_command("/sidebar toggle")
        .expect("/sidebar toggle must not error");
    assert!(
        !controller.sidebar_visible,
        "/sidebar toggle must flip to false"
    );
}

/// When `sidebar_visible` is false, `view()` must return `sidebar: None` so the
/// renderer suppresses the column entirely.
#[test]
fn sidebar_view_is_none_when_hidden() {
    let dir = unique_test_dir("tui-sidebar-view-none");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.sidebar_visible = false;

    assert!(
        controller.view().sidebar.is_none(),
        "view().sidebar must be None when sidebar_visible=false"
    );
}

/// When `sidebar_visible` is true, `view()` must carry a populated `SidebarView`
/// so the renderer can display the companion panel.
#[test]
fn sidebar_view_is_some_when_visible() {
    let dir = unique_test_dir("tui-sidebar-view-some");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.sidebar_visible = true;

    assert!(
        controller.view().sidebar.is_some(),
        "view().sidebar must be Some when sidebar_visible=true"
    );
}

/// With the sidebar visible and a non-empty prompt, the status bar should still
/// expose the compact Claude-style context summary rather than keybinding hints.
#[test]
fn sidebar_compact_status_when_visible() {
    let dir = unique_test_dir("tui-sidebar-compact-status");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.sidebar_visible = true;
    controller.prompt.insert_text("hello");

    let status = controller.view().status;

    assert!(
        status.contains("model:auto"),
        "compact status must contain the model summary; got: {status:?}"
    );
    assert!(
        status.contains("0 tok"),
        "compact status must contain token usage; got: {status:?}"
    );
}

/// With the sidebar hidden and a non-empty prompt, the same context summary
/// remains visible so narrow layouts still keep Claude-style chrome.
#[test]
fn sidebar_full_status_when_hidden() {
    let dir = unique_test_dir("tui-sidebar-full-status");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.sidebar_visible = false;
    controller.prompt.insert_text("hello");

    let status = controller.view().status;

    assert!(
        status.contains("model:auto"),
        "full status must contain the model summary when sidebar is hidden; got: {status:?}"
    );
    assert!(
        status.contains("cost:--"),
        "full status must contain the cost summary when sidebar is hidden; got: {status:?}"
    );
}

// ── controller provider failure ───────────────────────────────────────────────

#[test]
fn controller_provider_failure_appends_error_message_and_clears_prompt() {
    // Point the provider at a port that refuses connections so submit_prompt
    // receives a network error.  The error must be persisted as a
    // ProviderError transcript entry and the prompt must be cleared.
    let dir = unique_test_dir("tui-provider-failure");
    // Port 1 reliably refuses connections on Linux (privileged port, never bound).
    write_provider_config(dir.as_path(), "http://127.0.0.1:1");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.prompt.insert_text("trigger provider failure");
    controller
        .submit_prompt(&mut |_| Ok(()))
        .expect("submit_prompt itself must not propagate the provider error");

    // The prompt must be cleared — error is visible in history.
    assert_eq!(
        controller.prompt.text(),
        "",
        "prompt must be cleared after provider failure"
    );
    // A ProviderError must have been appended to the transcript.
    let has_provider_error = controller.state.messages.iter().any(|msg| {
        matches!(&msg.payload, MessagePayload::ProviderError { kind, .. } if kind == "provider")
    });
    assert!(
        has_provider_error,
        "transcript must contain a ProviderError after submit failure; messages: {:?}",
        controller
            .state
            .messages
            .iter()
            .map(|m| &m.payload)
            .collect::<Vec<_>>()
    );
    // The status note must point the user to history.
    assert_eq!(
        controller.status_note.as_deref(),
        Some("provider error — see history"),
        "status note must direct user to history"
    );
    // The error message in history must not contain raw credentials.
    for msg in &controller.state.messages {
        if let MessagePayload::ProviderError { message, .. } = &msg.payload {
            assert!(
                !message.contains("Bearer ") || message.contains("[REDACTED]"),
                "persisted error must not contain unredacted Bearer tokens"
            );
        }
    }
}

#[test]
fn controller_view_sizes_provider_errors_to_main_pane_width() {
    let dir = unique_test_dir("tui-provider-error-width");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.last_terminal_size = (120, 30);
    controller.state.messages.push(MessageEnvelope::new(
        controller.state.session.id,
        MessagePayload::ProviderError {
            kind: "provider".into(),
            message:
                "validation failed: provider HTTP request failed with status 400 from remote endpoint"
                    .into(),
        },
    ));

    let view = controller.view();
    let rows = render_to_test_backend(120, 20, &view, &Theme::default());

    assert!(
        rows.iter().any(|row| row.contains("status 400 ")),
        "provider error headline should keep the wider main-column budget before wrapping; rows: {rows:?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("from remote endpoint")),
        "provider error should continue onto the next line instead of truncating; rows: {rows:?}"
    );
}

// ── /login TUI interception ────────────────────────────────────────────────

#[test]
fn login_without_args_opens_setup_overlay() {
    let dir = unique_test_dir("tui-login-no-args");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // Dismiss any auto-opened setup overlay first so we start clean.
    controller.pending_setup_overlay = None;

    controller
        .execute_slash_command("/login")
        .expect("/login should not error");

    assert!(
        controller.pending_setup_overlay.is_some(),
        "/login with no args must open the setup overlay"
    );
}

#[test]
fn login_copilot_opens_oauth_dialog() {
    let dir = unique_test_dir("tui-login-copilot");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // `open_copilot_oauth_flow()` makes a real HTTP request for a device code.
    // In a test environment without network we expect it to fail and show an
    // error dialog rather than panicking.  Either outcome proves the interceptor
    // fired (i.e. no clap-args parse error was returned to the caller).
    let result = controller.execute_slash_command("/login copilot");

    // The interceptor must not bubble up a "missing --provider" clap error.
    // It is fine if the OAuth HTTP call fails in CI (no credentials); the
    // important invariant is that the *error type* is a network/agent error,
    // not a validation-parse error from LoginCommand.
    match &result {
        Ok(()) => {
            // OAuth request succeeded or opened a dialog — both are fine.
            // Either pending_copilot_oauth or a notice dialog must be set.
            assert!(
                controller.pending_copilot_oauth.is_some() || controller.dialog.is_some(),
                "/login copilot should set OAuth state or an error dialog"
            );
        }
        Err(e) => {
            let msg = e.to_string();
            assert!(
                !msg.contains("--provider") && !msg.contains("required arguments"),
                "/login copilot must not fail with a clap arg-parse error; got: {msg}"
            );
        }
    }
}

// ── Sidebar helper tests ──────────────────────────────────────────────────────

#[test]
fn parse_todo_lines_happy_path() {
    let md = "# Tasks\n- [x] done task\n- [ ] pending task\n- [X] also done\n";
    let lines = parse_todo_lines(md);
    assert_eq!(lines[0], "✓ done task");
    assert_eq!(lines[1], "  pending task");
    assert_eq!(lines[2], "✓ also done");
}

#[test]
fn parse_todo_lines_empty_file_returns_empty() {
    assert!(parse_todo_lines("").is_empty());
    assert!(parse_todo_lines("# No todos here\nJust prose.\n").is_empty());
}

#[test]
fn parse_todo_lines_caps_at_six_and_shows_more() {
    // 8 items → 6 shown + "+2 more"
    let md = (1..=8)
        .map(|i| format!("- [ ] task {i}\n"))
        .collect::<String>();
    let lines = parse_todo_lines(&md);
    assert_eq!(lines.len(), 7, "expected 6 items + '+2 more' trailer");
    assert_eq!(lines[6], "  +2 more");
}

#[test]
fn parse_todo_lines_truncates_long_descriptions() {
    let long_task = "a".repeat(40);
    let md = format!("- [ ] {long_task}\n");
    let lines = parse_todo_lines(&md);
    assert_eq!(lines.len(), 1);
    // Label should be at most 22 chars (prefix "  " is separate).
    let label = lines[0].trim_start();
    assert!(
        label.chars().count() <= 22,
        "label too long: {label:?} ({} chars)",
        label.chars().count()
    );
}

#[test]
fn todo_sidebar_lines_missing_file_returns_empty() {
    let dir = wonder_of_u_test_support::unique_test_dir("todo-missing");
    let lines = todo_sidebar_lines(&dir);
    assert!(lines.is_empty(), "missing todos.md should yield empty vec");
}

#[test]
fn todo_sidebar_lines_reads_file() {
    let dir = wonder_of_u_test_support::unique_test_dir("todo-reads");
    std::fs::write(dir.join("todos.md"), "- [x] done\n- [ ] pending\n").expect("write todos.md");
    let lines = todo_sidebar_lines(&dir);
    assert_eq!(lines[0], "✓ done");
    assert_eq!(lines[1], "  pending");
}

#[test]
fn mcp_sidebar_lines_no_storage_dir() {
    let lines = mcp_sidebar_lines(None);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("no storage dir"));
}

#[test]
fn mcp_sidebar_lines_empty_config() {
    let dir = wonder_of_u_test_support::unique_test_dir("mcp-empty");
    let lines = mcp_sidebar_lines(Some(&dir));
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].contains("no servers configured"),
        "got: {:?}",
        lines
    );
}

#[test]
fn mcp_sidebar_lines_with_servers() {
    use std::collections::BTreeMap;
    use wonder_of_u_mcp::{McpConfigStore, McpServerConfig};

    let dir = wonder_of_u_test_support::unique_test_dir("mcp-servers");
    let store = McpConfigStore::new(&dir);
    let config = wonder_of_u_mcp::McpConfig {
        servers: vec![
            McpServerConfig {
                name: "demo".into(),
                command: "demo-server".into(),
                args: vec![],
                env: BTreeMap::new(),
                enabled: true,
                cwd: None,
                protocol_version: None,
            },
            McpServerConfig {
                name: "disabled-srv".into(),
                command: "other-server".into(),
                args: vec![],
                env: BTreeMap::new(),
                enabled: false,
                cwd: None,
                protocol_version: None,
            },
        ],
        ..wonder_of_u_mcp::McpConfig::default()
    };
    store.write(&config).expect("write mcp config");

    let lines = mcp_sidebar_lines(Some(&dir));
    // First line: "N/M servers enabled"
    assert!(lines[0].contains("1/2"), "summary line: {:?}", lines[0]);
    assert!(
        lines[0].contains("servers enabled"),
        "summary line: {:?}",
        lines[0]
    );
    // Enabled server has ✓ prefix
    let demo_line = lines
        .iter()
        .find(|l| l.contains("demo"))
        .expect("demo line");
    assert!(demo_line.starts_with('✓'), "enabled server: {demo_line:?}");
    // Disabled server has space prefix
    let dis_line = lines
        .iter()
        .find(|l| l.contains("disabled-srv"))
        .expect("disabled line");
    assert!(!dis_line.starts_with('✓'), "disabled server: {dis_line:?}");
}

#[test]
fn tool_sidebar_lines_produces_summary() {
    let dir = wonder_of_u_test_support::unique_test_dir("tool-sidebar-summary");
    let context = ToolContext {
        session_id: SessionId::new(),
        cwd: dir,
        permission_mode: PermissionMode::Default,
        additional_working_directories: Vec::new(),
        permission_rules: Vec::new(),
        features: FeatureSet::first_release(),
        bash_session_store: None,
    };
    let lines = tool_sidebar_lines(&context);
    // Must not be empty and must not be an error line.
    assert!(!lines.is_empty());
    assert!(
        !lines[0].starts_with('⚠'),
        "unexpected error line: {:?}",
        lines[0]
    );
    // First line should contain "enabled / registered".
    assert!(
        lines[0].contains("enabled") && lines[0].contains("registered"),
        "first line: {:?}",
        lines[0]
    );
}

#[test]
fn lsp_sidebar_lines_never_panics() {
    // Just ensure it runs without panicking; actual binary presence is env-dependent.
    let dir = wonder_of_u_test_support::unique_test_dir("lsp-check");
    let lines = lsp_sidebar_lines(&dir);
    assert_eq!(
        lines.len(),
        LSP_SERVERS.len(),
        "should produce one line per known LSP server"
    );
}

#[test]
fn binary_on_path_returns_false_for_nonexistent() {
    assert!(
        !binary_on_path("__wonder_of_u_definitely_not_a_real_binary__"),
        "nonexistent binary must not be found on PATH"
    );
}
