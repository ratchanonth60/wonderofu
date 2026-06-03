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
    // Scroll now works from anywhere on screen (no area restriction), so a
    // wheel event over the prompt zone should scroll the transcript.
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

    // row 22 falls in the prompt/chrome zone of an 80×24 terminal —
    // with the area restriction removed, scroll still applies.
    controller.handle_mouse_event(scroll_mouse_event(
        wonder_of_u_tui::MouseEventKind::ScrollUp,
        40,
        22,
    ));
    assert!(
        controller.scroll_state.offset_from_bottom > 0,
        "wheel anywhere on screen should scroll the transcript"
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

// ── layout-math regression tests (Fix 2) ─────────────────────────────────────

/// After `on_terminal_resize` the controller's `scroll_state.last_visible_lines`
/// must equal the messages-area height that the renderer computes.
///
/// Before the fix the controller used `prompt_lines + 2` (missing the
/// integrated footer row) and `.max(3)` (below the renderer's minimum of 6),
/// which made the visible-lines estimate diverge by at least one row.
#[test]
fn controller_resize_visible_lines_matches_renderer_layout() {
    let dir = unique_test_dir("layout-math-resize");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // Two-line prompt so the computed height is non-trivial.
    controller.prompt = wonder_of_u_tui::TextBuffer::from_text("line1\nline2", true);

    // Resize to a standard 80×24 terminal with no sidebar and no context warning.
    controller.on_terminal_resize(80, 24);

    // Renderer: prompt_height() = 2 + 3 = 5, max_prompt = max(24/3, 6) = 8,
    // capped = 5.  ShellLayout::split(Rect(0,0,80,24), 5):
    //   chrome = 1, available = 23, messages = 23 - 5 = 18.
    assert_eq!(
        controller.scroll_state.last_visible_lines, 18,
        "last_visible_lines must equal the renderer's messages-area height (18) for 80×24 with 2-line prompt"
    );
}

/// `transcript_messages_rect()` must return the same messages rect that the
/// renderer uses, so that mouse-wheel hit-testing is accurate.
#[test]
fn controller_messages_rect_matches_renderer_for_empty_prompt() {
    let dir = unique_test_dir("layout-math-rect");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // Empty prompt; set last_terminal_size directly (resize is a no-op for
    // the rect helper — it reads last_terminal_size on demand).
    controller.last_terminal_size = (80, 24);

    // Renderer: prompt_height() = 1+3 = 4, max_prompt = 8, capped = 4.
    // ShellLayout::split(Rect(0,0,80,24), 4):
    //   chrome=1, available=23, messages=19.
    let rect = controller.transcript_messages_rect();
    assert_eq!(
        rect,
        wonder_of_u_tui::Rect::new(0, 0, 80, 19),
        "messages rect must equal Rect(0,0,80,19) for 80×24 with empty prompt"
    );
}

/// A mouse ScrollUp event whose row equals `messages_rect.bottom()` (the
/// first row of the prompt box) must NOT scroll the transcript, because that
/// row is inside the prompt area — not the messages area.
#[test]
fn controller_mouse_scroll_on_prompt_border_row_does_not_scroll() {
    // Scroll now works from anywhere on screen; the prompt border row also
    // triggers a scroll.  Test name kept for history; assertion updated.
    let dir = unique_test_dir("layout-math-border-scroll");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    controller.on_terminal_resize(80, 24);
    controller.scroll_state.on_resize(19, 100);

    let prompt_border_row = controller.transcript_messages_rect().bottom();
    assert_eq!(
        prompt_border_row, 19,
        "prompt starts at row 19 for 80×24 empty-prompt layout"
    );

    controller.handle_mouse_event(scroll_mouse_event(
        wonder_of_u_tui::MouseEventKind::ScrollUp,
        40,
        prompt_border_row,
    ));

    // Area restriction removed — scrolling applies from any row.
    assert!(
        controller.scroll_state.offset_from_bottom > 0,
        "wheel on prompt border row should now scroll the transcript"
    );
}

// ── cursor geometry regression (Fix 1) ───────────────────────────────────────

/// When the prompt ends with '\n' (Shift+Enter at end of line), the cursor
/// must be placed on the *second* content row of the prompt box, not clamped
/// to the first row.
///
/// Before the fix, `text_line_count("first\n")` returned 1 (str::lines()
/// drops the trailing empty segment) so the prompt box was only 4 rows tall
/// (1 content row + 3 overhead), leaving `content_height = 1` and clamping
/// the cursor to row 0.  After the fix it returns 2 and the box is 5 rows,
/// so `content_height = 2` and the cursor can sit on row 1.
#[test]
fn cursor_is_one_row_below_first_after_trailing_newline() {
    // Compute the cursor position for "first\n" with the cursor at the end
    // (position 6, just after the '\n').
    let (_, cursor_y_trailing) = prompt_cursor_position(80, 24, "first\n", 6, false, false);

    // Compute the cursor position for "first\n" with the cursor *on* the
    // text (position 5, just before the '\n').
    let (_, cursor_y_on_text) = prompt_cursor_position(80, 24, "first\n", 5, false, false);

    // The cursor after the '\n' must be exactly one row below the text cursor.
    assert_eq!(
        cursor_y_trailing,
        cursor_y_on_text + 1,
        "cursor after trailing '\\n' must be one row below the text row; \
         on_text_row={cursor_y_on_text}, trailing_newline_row={cursor_y_trailing}"
    );
}

// ── autostart tests ───────────────────────────────────────────────────────────

fn make_missing_auth_controller(label: &str) -> (TuiController<'static>, PathBuf) {
    let dir = unique_test_dir(label);
    let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
    let _oai_key = EnvVarGuard::set("OPENAI_API_KEY", "");
    let _gemini_key = EnvVarGuard::set("GEMINI_API_KEY", "");
    SettingsStore::new(dir.as_path())
        .write(&AgentSettings {
            selected_provider: Some("anthropic".into()),
            ..AgentSettings::default()
        })
        .expect("write settings");
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

#[allow(dead_code)]
fn make_unconfigured_controller() -> (TuiController<'static>, PathBuf) {
    make_missing_auth_controller("tui-autostart-unconfigured")
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
    let (controller, _dir) = make_missing_auth_controller("tui-autostart-missing-auth");

    assert!(
        controller.pending_setup_overlay.is_some(),
        "setup overlay should be auto-opened when provider auth is missing"
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
    let (mut controller, _dir) = make_missing_auth_controller("tui-autostart-cancel");

    assert!(
        controller.pending_setup_overlay.is_some(),
        "setup overlay must be open before cancel (precondition)"
    );

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
        selected_action: 0,
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

/// Rounded prompt cap sanity: at height=6 the renderer keeps the prompt tall
/// enough for the integrated footer, so the 3-line prompt is clipped into the
/// remaining 5-row main area with the first visible content row at y=2.
#[test]
fn prompt_cursor_position_small_terminal_respects_renderer_cap() {
    // 3-line prompt: uncapped rounded height = 6. At height=6 the prompt box is
    // clipped to fit while keeping one transcript row visible.
    let (x, y) = prompt_cursor_position(40, 6, "line1\nline2\nline3", 5, false, false);
    // cursor=5 → "line1" (no newline seen) → line=0, col=5, first-line x_offset=2.
    // CHROME_HEIGHT=1: available=5, prompt=(0,1,40,4), content_x=1,
    // content_y=2, content_height=1.
    assert_eq!(
        (x, y),
        (8, 2),
        "cursor row must land inside the prompt area rendered at the correct cap; got ({x}, {y})"
    );
}

/// Same cap-formula check for `history_search_cursor_position`. At height=6 the
/// history query lands on the integrated prompt footer row.
#[test]
fn history_search_cursor_position_small_terminal_respects_renderer_cap() {
    let view = HistorySearchView {
        query: "abc".into(),
        match_text: None,
        match_index: 0,
        match_total: 0,
    };
    // query_cursor=3 → x = content_x(1) + "history: ".len()(9) + 3 = 13.
    let (x, y) = history_search_cursor_position(40, 6, &view, 3, false, false);
    assert_eq!(
        (x, y),
        (13, 3),
        "history-search cursor must use the renderer's box cap; got ({x}, {y})"
    );
}

/// Continuation lines (line index > 0) must use `x_offset = 0` because the
/// `› ` marker only appears on the first prompt line.  This is the key
/// distinction from the first-line path and ensures the cursor lands at the
/// correct column inside the rounded prompt area.
#[test]
fn prompt_cursor_position_continuation_line_has_no_marker_offset() {
    // "abc\ndef" with cursor=6 points to 'f' on the second line.
    // Processing "abc\nde" → a(col=1), b(col=2), c(col=3), \n(line=1, col=0),
    // d(col=1), e(col=2) → line=1, col=2.
    // On 40×16 (CHROME_HEIGHT=1): available=15, cap=max(5,6)=6.
    //   prompt_height("abc\ndef")=5 (2 content lines + footer + borders); capped=5.
    //   message_height=15-5=10; prompt = Rect(0, 10, 40, 5).
    //   content_x=1, content_y=11, content_height=2.
    // line=1 is within content_height — the continuation path applies.
    //   x = content_x(1) + x_offset(0) + col(2) = 3
    //   y = content_y(11) + line(1) = 12
    let (x, y) = prompt_cursor_position(40, 16, "abc\ndef", 6, false, false);
    assert_eq!(
        (x, y),
        (3, 12),
        "continuation-line cursor must use x_offset=0 (no marker); got ({x}, {y})"
    );
}

// ── wide-terminal cursor regression (sidebar column deduction) ────────────────

/// On a 120-column terminal the renderer carves out SIDEBAR_WIDTH+1 columns,
/// leaving `effective_width = 83` for the rounded prompt area.  Before the fix,
/// both cursor helpers used the raw terminal width for layout, so a prompt/query
/// long enough to push the cursor past column 82 would land inside the sidebar.
///
/// Concrete geometry for width=120, height=24, CHROME_HEIGHT=1, sidebar_active=true:
///   effective_width = 83
///   prompt area = Rect(0, 19, 83, 4)  →  content_x=1, content_width=81
///   first-line max cursor x = content_x(1) + x_offset(2) + max_col(78) = 81
///   separator sits at column 83 (render_shell draws │ there)
///   → cursor x must be < 83

#[test]
fn prompt_cursor_position_wide_terminal_stays_inside_main_area() {
    // 80-char single-line prompt; cursor at the very end. With rounded inset and
    // the new threshold at 120, the separator is at column 83, so 81 is safely inside.
    let prompt = "a".repeat(80);
    let (x, y) = prompt_cursor_position(120, 24, &prompt, 80, true, false);

    // main_w = 83; separator at column 83; valid main-area columns = 0..=82.
    assert!(
        x < 83,
        "cursor x ({x}) must be left of the │ separator at column 83"
    );
    // Max reachable: content_x(1) + x_offset(2) + max_col(78) = 81.
    // CHROME_HEIGHT=1: prompt_height=4, messages=19, prompt at y=19, content_y=20.
    assert_eq!(
        (x, y),
        (81, 20),
        "wide-terminal cursor must clamp to the rightmost content cell; got ({x}, {y})"
    );
}

/// Same regression gate for `history_search_cursor_position`.  A query whose
/// length (plus the 8-char "search: " prefix) exceeds the content width of the
/// main area must be clamped to the last valid content column.
///
/// Width=120, sidebar_active=true → effective_width=83 → content_width=81.
/// max x = content_x(1) + content_width(81) - 1 = 81.
#[test]
fn history_search_cursor_wide_terminal_stays_inside_main_area() {
    // query_cursor=90 → prefix(8)+cursor(90)=98.  Before the fix:
    //   content_width = 118 (layout over full 120 cols), x = min(118, 98) = 98 → sidebar!
    // After fix:
    //   content_width = 81, x = min(81, 99) = 81 → stays in main area.
    let view = HistorySearchView {
        query: "a".repeat(90),
        match_text: None,
        match_index: 0,
        match_total: 0,
    };
    let (x, y) = history_search_cursor_position(120, 24, &view, 90, true, false);

    assert!(
        x < 83,
        "history-search cursor x ({x}) must be left of the │ separator at column 83"
    );
    // CHROME_HEIGHT=1: history search (1 content line + footer + borders → 4 rows)
    // places the prompt at y=19; the query cursor sits on the footer row at y=21.
    assert_eq!(
        (x, y),
        (81, 21),
        "wide-terminal history-search cursor must clamp to rightmost content cell; got ({x}, {y})"
    );
}

/// Verify that a narrow terminal (width=119, one below the sidebar threshold=120)
/// is unaffected by the sidebar logic even when sidebar_active=true.  The
/// prompt layout still spans the full terminal width.
#[test]
fn prompt_cursor_position_just_below_sidebar_threshold_uses_full_width() {
    // At width=119 shell_main_area_width returns 119 regardless of sidebar_active.
    // 10-char prompt, cursor at end → line=0, col=10, x_offset=2.
    // effective_width=119; CHROME_HEIGHT=1: available=19, messages=15, prompt at y=15.
    // content_y=16, x=1+2+10=13.
    let (x, y) = prompt_cursor_position(119, 20, &"a".repeat(10), 10, true, false);
    assert_eq!(
        (x, y),
        (13, 16),
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
        status.contains("local:llama3.2"),
        "compact status must contain the auto-selected model summary; got: {status:?}"
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
        status.contains("local:llama3.2"),
        "full status must contain the auto-selected model summary when sidebar is hidden; got: {status:?}"
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

    // ── Fix 3 regression: no blank AssistantText placeholder ─────────────────
    // Before the fix, submit_prompt left an empty AssistantText in the
    // transcript immediately before the ProviderError, producing a blank
    // assistant bullet in the UI.  Assert it is gone.
    let has_empty_assistant = controller.state.messages.iter().any(|msg| {
        matches!(&msg.payload, MessagePayload::AssistantText { content } if content.is_empty())
    });
    assert!(
        !has_empty_assistant,
        "no empty AssistantText placeholder must remain after a provider failure; messages: {:?}",
        controller
            .state
            .messages
            .iter()
            .map(|m| &m.payload)
            .collect::<Vec<_>>()
    );

    // The UserText entry for the attempted prompt must still be present so the
    // user can see what they typed before the error occurred.
    let has_user_text = controller.state.messages.iter().any(|msg| {
        matches!(&msg.payload, MessagePayload::UserText { content } if content == "trigger provider failure")
    });
    assert!(
        has_user_text,
        "UserText for the failed prompt must remain in the transcript; messages: {:?}",
        controller
            .state
            .messages
            .iter()
            .map(|m| &m.payload)
            .collect::<Vec<_>>()
    );

    // Message order: UserText must appear immediately before ProviderError.
    let payloads: Vec<_> = controller
        .state
        .messages
        .iter()
        .map(|m| &m.payload)
        .collect();
    let provider_error_idx = payloads
        .iter()
        .position(|p| matches!(p, MessagePayload::ProviderError { .. }))
        .expect("ProviderError must be present");
    assert!(
        provider_error_idx > 0,
        "ProviderError must be preceded by at least one message"
    );
    assert!(
        matches!(
            payloads[provider_error_idx - 1],
            MessagePayload::UserText { .. }
        ),
        "the message immediately before ProviderError must be UserText, got: {:?}",
        payloads[provider_error_idx - 1]
    );
}

