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
    // wrap_text_hard is a strict character-boundary splitter: at width=120 with
    // sidebar active, the main area is 83 cols and the "error[provider]> " prefix
    // consumes 17, leaving 66 chars for the body.  The word "from" straddles that
    // boundary ("fr" ends line 1, "om remote endpoint" starts line 2).  We check
    // for "remote endpoint" — which is entirely on the continuation line — rather
    // than "from remote endpoint", so the assertion does not depend on word-boundary
    // alignment.  The intent is to verify the message wraps rather than truncates.
    assert!(
        rows.iter().any(|row| row.contains("remote endpoint")),
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

    block_on(controller.execute_slash_command("/login")).expect("/login should not error");

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
    let result = block_on(controller.execute_slash_command("/login copilot"));

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

fn write_todo_task_list(
    dir: &std::path::Path,
    session_id: SessionId,
    tasks: impl IntoIterator<Item = TodoTaskEntry>,
) {
    let mut list = TodoTaskList::empty(session_id);
    for task in tasks {
        list.tasks.insert(task.task_id.clone(), task);
    }
    TodoTaskStore::new(dir)
        .write(&list)
        .expect("write todo list");
}

fn todo_task(id: &str, subject: &str, status: TodoTaskStatus) -> TodoTaskEntry {
    let mut task = TodoTaskEntry::new(id, subject, subject);
    task.status = status;
    task
}

#[test]
fn todo_task_store_sidebar_lines_empty_returns_none() {
    let dir = unique_test_dir("todo-store-sidebar-empty");
    let session_id = SessionId::new();

    assert!(todo_task_store_sidebar_lines(&dir, session_id).is_none());
}

#[test]
fn todo_task_store_sidebar_lines_all_deleted_returns_none() {
    let dir = unique_test_dir("todo-store-sidebar-deleted");
    let session_id = SessionId::new();
    write_todo_task_list(
        &dir,
        session_id,
        [todo_task("todo-1", "deleted", TodoTaskStatus::Deleted)],
    );

    assert!(todo_task_store_sidebar_lines(&dir, session_id).is_none());
}

#[test]
fn todo_task_store_sidebar_lines_formats_statuses() {
    let dir = unique_test_dir("todo-store-sidebar-statuses");
    let session_id = SessionId::new();
    write_todo_task_list(
        &dir,
        session_id,
        [
            todo_task("todo-1", "write tests", TodoTaskStatus::Pending),
            todo_task("todo-2", "deploy", TodoTaskStatus::InProgress),
            todo_task("todo-3", "done", TodoTaskStatus::Completed),
            todo_task("todo-4", "hidden", TodoTaskStatus::Deleted),
        ],
    );

    let lines = todo_task_store_sidebar_lines(&dir, session_id).expect("sidebar lines");
    assert_eq!(lines, vec!["  write tests", "▷ deploy", "✓ done"]);
}

#[test]
fn todo_task_store_sidebar_lines_caps_and_truncates() {
    let dir = unique_test_dir("todo-store-sidebar-cap");
    let session_id = SessionId::new();
    let tasks = (1..=8).map(|index| {
        todo_task(
            &format!("todo-{index}"),
            &format!("task {index} with a very long subject"),
            TodoTaskStatus::Pending,
        )
    });
    write_todo_task_list(&dir, session_id, tasks);

    let lines = todo_task_store_sidebar_lines(&dir, session_id).expect("sidebar lines");
    assert_eq!(lines.len(), 7, "expected 6 items + '+2 more' trailer");
    assert_eq!(lines[6], "  +2 more");
    let label = lines[0].trim_start();
    assert!(label.chars().count() <= 22, "label too long: {label}");
}

#[test]
fn todo_merged_sidebar_lines_prefers_store_over_markdown() {
    let dir = unique_test_dir("todo-merged-prefers-store");
    let session_id = SessionId::new();
    std::fs::write(dir.join("todos.md"), "- [ ] markdown item\n").expect("write todos.md");
    write_todo_task_list(
        &dir,
        session_id,
        [todo_task("todo-1", "store item", TodoTaskStatus::Pending)],
    );

    let lines = todo_merged_sidebar_lines(&dir, Some(&dir), session_id);
    assert_eq!(lines, vec!["  store item"]);
}

#[test]
fn todo_merged_sidebar_lines_falls_back_to_markdown() {
    let dir = unique_test_dir("todo-merged-fallback");
    let session_id = SessionId::new();
    std::fs::write(dir.join("todos.md"), "- [ ] markdown item\n").expect("write todos.md");

    assert_eq!(
        todo_merged_sidebar_lines(&dir, Some(&dir), session_id),
        vec!["  markdown item"]
    );
    assert_eq!(
        todo_merged_sidebar_lines(&dir, None, session_id),
        vec!["  markdown item"]
    );
}

#[test]
fn todo_merged_sidebar_lines_all_deleted_falls_back_to_markdown() {
    let dir = unique_test_dir("todo-merged-deleted-fallback");
    let session_id = SessionId::new();
    std::fs::write(dir.join("todos.md"), "- [ ] markdown item\n").expect("write todos.md");
    write_todo_task_list(
        &dir,
        session_id,
        [todo_task("todo-1", "deleted", TodoTaskStatus::Deleted)],
    );

    assert_eq!(
        todo_merged_sidebar_lines(&dir, Some(&dir), session_id),
        vec!["  markdown item"]
    );
}

#[test]
fn todo_merged_sidebar_lines_both_empty_returns_empty() {
    let dir = unique_test_dir("todo-merged-empty");
    let session_id = SessionId::new();

    assert!(todo_merged_sidebar_lines(&dir, Some(&dir), session_id).is_empty());
}

#[test]
fn mcp_sidebar_lines_no_storage_dir() {
    let dir = unique_test_dir("mcp-no-storage-cwd");
    let lines = mcp_sidebar_lines(None, &dir);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("no storage dir"));
}

#[test]
fn mcp_sidebar_lines_empty_config() {
    let dir = wonder_of_u_test_support::unique_test_dir("mcp-empty");
    let lines = mcp_sidebar_lines(Some(&dir), &dir);
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

    let lines = mcp_sidebar_lines(Some(&dir), &dir);
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
        session_worktree: None,
        permission_mode: PermissionMode::Default,
        additional_working_directories: Vec::new(),
        provider: None,
        model: None,
        permission_rules: Vec::new(),
        features: FeatureSet::first_release(),
        bash_session_store: None,
        progress_tx: None,
        interaction_rx: None,
        fork_context: None,
            file_checkpointer: None,
            network_policy: None,
    };
    let lines = tool_sidebar_lines(&context, None);
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

#[test]
fn controller_toggles_optimize_token_mode_from_command() {
    let dir = unique_test_dir("tui-optimize-tonken-toggle");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/optimize-tonken")).expect("toggle optimize-tonken");

    assert!(controller.state.optimize_token_mode);
    assert_eq!(controller.status_note.as_deref(), Some("optimize token on"));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/optimize-tonken must not record to transcript"
    );
}

#[test]
fn controller_toggles_optimize_token_mode_via_alias() {
    let dir = unique_test_dir("tui-optimize-token-alias-toggle");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/optimize-token")).expect("toggle via alias");

    assert!(controller.state.optimize_token_mode);
    assert_eq!(controller.status_note.as_deref(), Some("optimize token on"));
}

#[test]
fn controller_shows_optimize_token_notice_dialog() {
    let dir = unique_test_dir("tui-optimize-tonken-notice");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/optimize-tonken show")).expect("show optimize-tonken");

    assert_eq!(controller.status_note.as_deref(), Some("optimize token"));
    assert!(matches!(
        controller.dialog.as_ref(),
        Some(dialog) if dialog.title == "Optimize Token"
    ));
    assert!(
        !controller.state.messages.iter().any(|message| {
            matches!(&message.payload, MessagePayload::Command { .. })
        }),
        "/optimize-tonken show must not record to transcript"
    );
}

#[test]
fn controller_optimize_token_flag_persists_across_clear() {
    let dir = unique_test_dir("tui-optimize-tonken-persist");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/optimize-tonken")).expect("enable");
    assert!(controller.state.optimize_token_mode);

    block_on(controller.execute_slash_command("/clear")).expect("clear");
    assert!(
        controller.state.optimize_token_mode,
        "optimize_token_mode should survive /clear"
    );
}

// ── Multi-provider & local model TUI tests ──────────────────────────────────

/// Gateway provider (Groq) appears in the model picker when its API key is
/// set in the environment.  The picker must show the `any model accepted`
/// label because Groq is a non-strict provider.
#[test]
fn controller_model_picker_shows_non_strict_gateway_provider() {
    let _groq = EnvVarGuard::set("GROQ_API_KEY", "groq-test-key");
    let dir = unique_test_dir("tui-picker-groq");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/model")).expect("open model picker");

    let picker = controller
        .pending_model_picker
        .as_ref()
        .expect("model picker must be open");

    // Groq option must appear in the picker.
    assert!(
        picker.options.iter().any(|o| o.provider == "groq"),
        "groq must be present when GROQ_API_KEY is set; options: {:?}",
        picker
            .options
            .iter()
            .map(|o| &o.provider)
            .collect::<Vec<_>>()
    );

    // The groq option's model_display must communicate pass-through semantics.
    let groq_opt = picker
        .options
        .iter()
        .find(|o| o.provider == "groq")
        .unwrap();
    assert!(
        groq_opt.model_display.contains("any model accepted"),
        "non-strict groq entry must mention arbitrary model acceptance; got: {:?}",
        groq_opt.model_display
    );
}

/// Local provider is always ready (no auth) and must appear in the model
/// picker even with no env vars or stored credentials.
#[test]
fn controller_model_picker_always_includes_local_provider() {
    let dir = unique_test_dir("tui-picker-local");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/model")).expect("open model picker");

    assert!(
        controller.pending_model_picker.is_some(),
        "model picker must open when local provider is ready"
    );
    let picker = controller.pending_model_picker.as_ref().unwrap();
    assert!(
        picker.options.iter().any(|o| o.provider == "local"),
        "local provider must always appear in picker (no auth required); options: {:?}",
        picker
            .options
            .iter()
            .map(|o| &o.provider)
            .collect::<Vec<_>>()
    );
}

/// Selecting the local provider from the picker with a custom model id (set
/// via `/model set local:phi3:mini`) must succeed and update the session state.
#[test]
fn controller_set_local_arbitrary_model_via_slash_model() {
    let dir = unique_test_dir("tui-local-custom-model");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/model local:phi3:mini")).expect("set local model");

    // The selection should have been applied; picker should be closed.
    assert!(
        controller.pending_model_picker.is_none(),
        "picker must be closed after explicit /model set"
    );
    assert_eq!(
        controller.state.provider.as_deref(),
        Some("local"),
        "provider must be local"
    );
    assert_eq!(
        controller.state.model.as_deref(),
        Some("phi3:mini"),
        "arbitrary local model must be accepted"
    );
}

/// The sidebar provider_lines must include at least one entry for the active
/// provider and show the active model.  The "+N more" missing-provider line
/// appears only when not all providers are authenticated—this test verifies
/// the sidebar populates correctly regardless of environment.
#[test]
fn controller_sidebar_shows_missing_provider_count() {
    let _groq = EnvVarGuard::set("GROQ_API_KEY", "groq-test-key");
    let dir = unique_test_dir("tui-sidebar-missing");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    // Inject a provider selection so the session has an active provider.
    controller.state.set_provider_context(
        Some("groq".into()),
        Some("llama-3.3-70b-versatile".into()),
        wonder_of_u_core::AuthState::not_required(),
    );

    let view = controller.view();
    let sidebar = view.sidebar.expect("sidebar must be present");

    // The active provider must appear with the ◈ marker.
    assert!(
        sidebar
            .provider_lines
            .iter()
            .any(|l| l.contains("◈") && l.contains("groq")),
        "active groq provider must appear with ◈ marker; provider_lines: {:?}",
        sidebar.provider_lines
    );

    // The active model must appear somewhere in provider_lines.
    assert!(
        sidebar
            .provider_lines
            .iter()
            .any(|l| l.contains("llama-3.3-70b-versatile")),
        "active model must appear in provider_lines; got: {:?}",
        sidebar.provider_lines
    );

    // If some providers are not ready (possible depending on env), the
    // "+N more (/setup to configure)" hint must appear.  When all providers
    // happen to be ready in the environment, the line is correctly absent.
    let total = wonder_of_u_agent::ProviderRegistry::builtin()
        .providers()
        .count();
    let ready = sidebar
        .provider_lines
        .iter()
        .filter(|l| !l.trim_start().starts_with('+'))
        .count();
    let has_missing_line = sidebar
        .provider_lines
        .iter()
        .any(|l| l.contains("more") && l.contains("setup"));
    if ready < total {
        assert!(
            has_missing_line,
            "must show missing-count hint when {ready}/{total} providers are ready; \
             provider_lines: {:?}",
            sidebar.provider_lines
        );
    }
}

/// Doctor command output must list all builtin providers with env var hints.
#[test]
fn doctor_output_includes_all_provider_env_hints() {
    let dir = unique_test_dir("doctor-env-hints");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/doctor")).expect("run doctor");

    // Doctor output goes to a dialog, not the transcript.
    let dialog = controller.dialog.as_ref().expect("doctor dialog");
    assert_eq!(dialog.title, "/doctor");
    let output = dialog.body.join("\n");
    let output = output.as_str();

    assert!(
        output.contains("providers_registered="),
        "must include provider count; got:\n{output}"
    );
    assert!(
        output.contains("GROQ_API_KEY"),
        "must include GROQ_API_KEY hint; got:\n{output}"
    );
    assert!(
        output.contains("HF_TOKEN") || output.contains("huggingface"),
        "must include HuggingFace hint; got:\n{output}"
    );
    assert!(
        output.contains("AZURE_OPENAI_API_KEY"),
        "must include Azure api-key hint; got:\n{output}"
    );
    assert!(
        output.contains("AZURE_OPENAI_API_ENDPOINT"),
        "must include Azure endpoint hint; got:\n{output}"
    );
    assert!(
        output.contains("no_auth_required"),
        "local provider must show no_auth_required hint; got:\n{output}"
    );
}

// ── Setup parity: new local-first item routing ────────────────────────────────

/// Helper: create a `SetupOverlayState` containing exactly one item with the
/// given id/action, then open it on a fresh controller.
fn controller_with_synthetic_setup_item(
    item_id: &str,
    item_label: &str,
    action: crate::tui_runtime::setup::SetupItemAction,
) -> (TuiController<'static>, PathBuf) {
    use crate::tui_runtime::setup::{SetupItem, SetupOverlayState};

    let dir = unique_test_dir("tui-setup-synthetic");
    let registry = Box::new(commands::registry(Some(dir.clone())).expect("registry"));
    let registry: &'static _ = Box::leak(registry);
    let mut controller = TuiController::new(
        test_context(&dir),
        registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");
    controller.pending_setup_overlay = Some(SetupOverlayState::new(
        vec![SetupItem {
            id: item_id.into(),
            label: item_label.into(),
            description: String::new(),
            action,
        }],
        "test-provider".into(),
        "ready".into(),
    ));
    (controller, dir)
}

/// `"api-key"` (upstream alias) must open the `ProviderForm(ApiKey)` flow, not a
/// placeholder — verifies parity with the `"login"` alias.
#[test]
fn setup_overlay_action_for_api_key_alias_opens_provider_form() {
    use crate::tui_runtime::setup::{ProviderFormKind, SetupItemAction};

    let (mut controller, _dir) = controller_with_synthetic_setup_item(
        "api-key",
        "API Key",
        SetupItemAction::ProviderForm(ProviderFormKind::ApiKey),
    );

    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    assert!(
        controller.pending_setup_overlay.is_none(),
        "setup overlay should close after Enter"
    );
    assert!(
        controller.pending_provider_form.is_some(),
        "'api-key' item must open provider form"
    );
    let form = controller.pending_provider_form.as_ref().unwrap();
    assert_eq!(form.kind, ProviderFormKind::ApiKey);
}

/// `"local-provider"` (Ollama / Llama.cpp) must open the `ProviderForm(ApiBase)`
/// flow — the only TUI-configurable knob for local providers is the base URL.
#[test]
fn setup_overlay_action_for_local_provider_opens_api_base_form() {
    use crate::tui_runtime::setup::{ProviderFormKind, SetupItemAction};

    let (mut controller, _dir) = controller_with_synthetic_setup_item(
        "local-provider",
        "Local Provider",
        SetupItemAction::ProviderForm(ProviderFormKind::ApiBase),
    );

    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    assert!(
        controller.pending_setup_overlay.is_none(),
        "setup overlay should close after Enter"
    );
    assert!(
        controller.pending_provider_form.is_some(),
        "'local-provider' item must open provider form"
    );
    let form = controller.pending_provider_form.as_ref().unwrap();
    assert_eq!(form.kind, ProviderFormKind::ApiBase);
}

/// Deferred items (cloud/remote-only) must carry the `"deferred"` badge in the
/// picker list and show a notice dialog (not a provider form) when confirmed.
#[test]
fn setup_overlay_deferred_item_shows_deferred_tag_and_notice() {
    use crate::tui_runtime::setup::SetupItemAction;

    let deferred_msg =
        "Grove (cloud sync) is a remote feature not applicable to the local-first TUI.";
    let (mut controller, _dir) = controller_with_synthetic_setup_item(
        "grove",
        "Grove",
        SetupItemAction::Deferred(deferred_msg.into()),
    );

    // Confirm the tag is "deferred" in the picker view before confirming.
    {
        let picker = controller
            .current_picker_list_view()
            .expect("picker list must be present while setup overlay is open");
        let entry = picker.entries.first().expect("at least one entry");
        assert_eq!(
            entry.tag.as_deref(),
            Some("deferred"),
            "deferred action must produce 'deferred' tag; got {:?}",
            entry.tag
        );
    }

    // Confirm: setup overlay closes and a notice dialog appears.
    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    assert!(
        controller.pending_setup_overlay.is_none(),
        "setup overlay must close after confirming a deferred item"
    );
    assert!(
        controller.dialog.is_some(),
        "a notice dialog must appear for a deferred item"
    );
    assert!(
        controller.pending_provider_form.is_none(),
        "provider form must NOT open for a deferred item"
    );

    // Status note must mention "deferred".
    let note = controller.status_note.clone().unwrap_or_default();
    assert!(
        note.contains("deferred"),
        "status note must mention 'deferred'; got: {note:?}"
    );
}

/// `"trust"` resolves to a `Placeholder` (not `Deferred`) and shows a notice
/// explaining that trust is implicit in the local-first TUI.
#[test]
fn setup_overlay_trust_item_shows_coming_soon_tag_and_notice() {
    use crate::tui_runtime::setup::SetupItemAction;

    let (mut controller, _dir) = controller_with_synthetic_setup_item(
        "trust",
        "Workspace Trust",
        SetupItemAction::Placeholder(
            "Workspace trust is accepted implicitly in the local-first TUI. \
             No action required."
                .into(),
        ),
    );

    // Placeholder items show "coming soon" (local, not yet built — unlike cloud deferred).
    {
        let picker = controller
            .current_picker_list_view()
            .expect("picker list present");
        let entry = picker.entries.first().expect("entry");
        assert_eq!(
            entry.tag.as_deref(),
            Some("coming soon"),
            "trust Placeholder must show 'coming soon' tag; got {:?}",
            entry.tag
        );
    }

    // Confirm: a notice dialog appears (not a form).
    send_dialog_key(
        &mut controller,
        picker_key(KeyCode::Enter),
        Some(ResolvedKey::Edit(EditAction::InsertNewline)),
    );

    assert!(
        controller.dialog.is_some(),
        "notice dialog must appear for trust item"
    );
    assert!(
        controller.pending_provider_form.is_none(),
        "no form for trust item"
    );
}

/// `action_for_item_id("onboarding", …)` must return `Dispatch("/setup")`.
/// This is verified via the unit module but also sanity-checked here at the
/// integration level.
#[test]
fn setup_item_action_for_onboarding_is_dispatch_setup() {
    use crate::tui_runtime::setup::{SetupItemAction, action_for_item_id};

    let action = action_for_item_id("onboarding", "/setup");
    assert_eq!(
        action,
        SetupItemAction::Dispatch("/setup".into()),
        "'onboarding' must dispatch to /setup; got {action:?}"
    );
}

/// `action_for_item_id("api-key", …)` must equal `ProviderForm(ApiKey)`.
#[test]
fn setup_item_action_for_api_key_alias_is_provider_form() {
    use crate::tui_runtime::setup::{ProviderFormKind, SetupItemAction, action_for_item_id};

    let action = action_for_item_id("api-key", "/setup");
    assert_eq!(
        action,
        SetupItemAction::ProviderForm(ProviderFormKind::ApiKey),
        "'api-key' must be ProviderForm(ApiKey); got {action:?}"
    );
}

/// `action_for_item_id("local-provider", …)` must equal `ProviderForm(ApiBase)`.
#[test]
fn setup_item_action_for_local_provider_is_api_base_form() {
    use crate::tui_runtime::setup::{ProviderFormKind, SetupItemAction, action_for_item_id};

    let action = action_for_item_id("local-provider", "/setup");
    assert_eq!(
        action,
        SetupItemAction::ProviderForm(ProviderFormKind::ApiBase),
        "'local-provider' must be ProviderForm(ApiBase); got {action:?}"
    );
}

/// All six intentionally-deferred cloud IDs must resolve to `Deferred`, not
/// `Placeholder` or anything else.
#[test]
fn setup_item_action_deferred_cloud_ids_return_deferred_variant() {
    use crate::tui_runtime::setup::{SetupItemAction, action_for_item_id};

    for id in &[
        "grove",
        "telemetry",
        "bypass-permissions",
        "auto-mode",
        "channels",
        "chrome-onboarding",
    ] {
        let action = action_for_item_id(id, "/setup");
        assert!(
            matches!(action, SetupItemAction::Deferred(_)),
            "'{id}' must be Deferred; got {action:?}"
        );
    }
}

// ── provider_form_picker_view: stage rendering ────────────────────────────────

/// Stage 1 (`PickProvider`) must render a titled picker listing at least one
/// provider entry; none should be tagged.
#[test]
fn provider_form_picker_view_stage1_shows_provider_list() {
    let (mut controller, _dir) = open_setup_overlay_controller();
    controller.open_provider_form(crate::tui_runtime::setup::ProviderFormKind::ApiKey);

    let view = controller
        .current_picker_list_view()
        .expect("picker list present in stage-1");

    assert!(
        view.title.contains("API Key"),
        "stage-1 title must include 'API Key'; got {:?}",
        view.title
    );
    assert!(
        !view.entries.is_empty(),
        "stage-1 must list at least one provider"
    );
    // In stage-1 no entry should carry a tag (tag is only set on stage-2 selected row).
    for entry in &view.entries {
        assert!(
            entry.tag.is_none(),
            "stage-1 entries must have no tag; {:?} has tag {:?}",
            entry.label,
            entry.tag
        );
    }
}

/// Stage 2 (`EnterValue`) for `ApiKey` must mask input as bullets and show
/// the selected provider with a `"selected"` tag.
#[test]
fn provider_form_picker_view_stage2_api_key_masked_and_tagged() {
    use crate::tui_runtime::setup::ProviderFormStage;

    let (mut controller, _dir) = open_setup_overlay_controller();
    controller.open_provider_form(crate::tui_runtime::setup::ProviderFormKind::ApiKey);

    // Advance to stage-2 with Tab.
    send_dialog_key(&mut controller, picker_key(KeyCode::Tab), None);
    {
        let form = controller
            .pending_provider_form
            .as_ref()
            .expect("form open");
        assert_eq!(
            form.stage,
            ProviderFormStage::EnterValue,
            "must be in stage-2"
        );
    }

    // Type a fake key.
    for c in "sk-testXYZ".chars() {
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Char(c)),
            Some(ResolvedKey::InsertChar(c)),
        );
    }

    let view = controller
        .current_picker_list_view()
        .expect("picker list present in stage-2");

    // query (displayed in input box) must be fully masked bullets.
    assert!(
        !view.query.contains("sk-testXYZ"),
        "stage-2 query must not contain raw key; got {:?}",
        view.query
    );
    assert!(
        view.query.chars().all(|c| c == '\u{2022}'),
        "stage-2 query must be all bullet characters; got {:?}",
        view.query
    );

    // The selected-provider row must carry a "selected" tag.
    let tagged = view.entries.iter().find(|e| e.tag.is_some());
    assert!(
        tagged.is_some(),
        "stage-2 must have a 'selected' tagged entry"
    );
    assert_eq!(
        tagged.unwrap().tag.as_deref(),
        Some("selected"),
        "tagged entry must have tag 'selected'"
    );
}

/// Stage 2 for `ApiBase` must show the URL in plain text (not masked).
#[test]
fn provider_form_picker_view_stage2_api_base_shows_plain_url() {
    use crate::tui_runtime::setup::ProviderFormStage;

    let (mut controller, _dir) = open_setup_overlay_controller();
    controller.open_provider_form(crate::tui_runtime::setup::ProviderFormKind::ApiBase);

    // Advance to stage-2 with Tab.
    send_dialog_key(&mut controller, picker_key(KeyCode::Tab), None);
    {
        let form = controller
            .pending_provider_form
            .as_ref()
            .expect("form open");
        assert_eq!(form.stage, ProviderFormStage::EnterValue);
    }

    let url = "http://localhost:11434/v1";
    for c in url.chars() {
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Char(c)),
            Some(ResolvedKey::InsertChar(c)),
        );
    }

    let view = controller
        .current_picker_list_view()
        .expect("picker list present");

    assert!(
        view.query.contains("localhost:11434"),
        "ApiBase stage-2 query must show URL in plain text; got {:?}",
        view.query
    );
}

// ── immediate-flag tests ──────────────────────────────────────────────────────

/// All six commands that must bypass queued-prompt draining carry
/// `immediate = true` in the registry spec.
#[test]
fn immediate_commands_have_immediate_flag_set_in_registry() {
    let dir = unique_test_dir("tui-immediate-specs");
    let registry = commands::registry(Some(dir.clone())).expect("registry");

    // /clear requires SessionPersistence but resolve_spec bypasses availability
    // checking, so all six can be looked up unconditionally.
    for name in ["exit", "clear", "color", "effort", "fast", "hooks"] {
        let spec = registry
            .resolve_spec(name)
            .unwrap_or_else(|| panic!("/{name} not registered"));
        assert!(
            spec.immediate,
            "/{name} must have immediate=true so queued-prompt draining is skipped"
        );
    }
}

/// `/exit` is an immediate command: executing it must not drain any queued
/// prompts that were enqueued before the command ran.
///
/// We use `storage_dir=None` so that `persistence.persisted` stays `false` and
/// `restore_current_session` is never called — which would otherwise wipe the
/// in-memory queue by replacing the entire `AppState` from storage.
#[test]
fn exit_command_does_not_drain_queued_prompts() {
    let dir = unique_test_dir("tui-exit-no-drain");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    // storage_dir=None → persistence.persisted=false → restore_current_session
    // is never triggered, preserving our manually-seeded queued_commands.
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        None,
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // Pre-seed the queue with a prompt that would normally be drained.
    controller
        .state
        .queue_command("hello world", QueuePlacement::Later);
    assert_eq!(controller.state.queued_commands.len(), 1);

    block_on(controller.execute_slash_command("/exit")).expect("execute /exit");

    assert_eq!(
        controller.state.queued_commands.len(),
        1,
        "/exit (immediate) must not drain queued prompts"
    );
    // exit_requested must still be honoured.
    assert!(
        controller.exit_requested,
        "/exit must set exit_requested regardless of immediate flag"
    );
}

/// A non-immediate command (e.g. `/model`) must drain queued commands after
/// execution.  We seed the queue with an empty string, which
/// `drain_queued_commands` will pop and discard, so the queue ends up empty.
///
/// `storage_dir=None` is used for the same reason as the exit test: to keep
/// `persistence.persisted=false` and avoid `restore_current_session` wiping
/// the queue before drain runs.
#[test]
fn non_immediate_command_drains_queued_empty_prompt() {
    let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
    let _oai_key = EnvVarGuard::set("OPENAI_API_KEY", "");
    let dir = unique_test_dir("tui-non-immediate-drain");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        None,
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // An empty-string queued command is popped and skipped by drain, so it
    // exercises the drain path without attempting a model call.
    controller.state.queue_command("", QueuePlacement::Later);
    assert_eq!(controller.state.queued_commands.len(), 1);

    // /model is not immediate — drain runs after execution.
    block_on(controller.execute_slash_command("/model openai:gpt-4.1")).expect("execute /model");

    assert_eq!(
        controller.state.queued_commands.len(),
        0,
        "non-immediate command (/model) must drain queued commands"
    );
}

// ── task-notification XML injection ─────────────────────────────────────────

/// Helper: build a controller, drive a task from Running → terminal, and
/// return the controller so callers can inspect state.
fn make_controller_with_terminal_task(
    dir: &std::path::Path,
    final_status: TaskStatus,
) -> TuiController<'static> {
    // Static registry leak is fine in tests – the registry is tiny and tests
    // are short-lived processes.
    let registry = commands::registry(Some(dir.to_path_buf())).expect("registry");
    let registry: &'static _ = Box::leak(Box::new(registry));

    let mut controller = TuiController::new(
        test_context(dir),
        registry,
        Some(dir),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    let store = TaskStore::new(dir);
    let mut task = TaskState::pending("build workspace");
    task.status = TaskStatus::Running;
    store.write_task(&task).expect("write running task");
    controller
        .refresh_runtime_state()
        .expect("first refresh (running)");

    task.mark_finished(final_status, Some(0), Some("done".into()));
    store.write_task(&task).expect("write terminal task");
    controller
        .refresh_runtime_state()
        .expect("second refresh (terminal)");

    controller
}

#[test]
fn task_completion_injects_xml_notification_into_transcript() {
    let dir = unique_test_dir("tui-inject-notif-completed");
    let controller = make_controller_with_terminal_task(&dir, TaskStatus::Completed);

    let notif_count = controller
        .state
        .messages
        .iter()
        .filter(|m| matches!(&m.payload, MessagePayload::TaskNotification { .. }))
        .count();
    assert_eq!(
        notif_count, 1,
        "expected exactly one TaskNotification message"
    );

    let xml = controller
        .state
        .messages
        .iter()
        .find_map(|m| {
            if let MessagePayload::TaskNotification { xml_payload, .. } = &m.payload {
                Some(xml_payload.clone())
            } else {
                None
            }
        })
        .expect("TaskNotification payload");

    assert!(
        xml.starts_with("<task-notification>"),
        "XML must start with <task-notification>: {xml}"
    );
    assert!(
        xml.contains("<status>completed</status>"),
        "XML must contain completed status: {xml}"
    );
    assert!(xml.contains("<summary>"), "XML must contain summary: {xml}");
}

#[test]
fn task_failure_injects_xml_notification_into_transcript() {
    let dir = unique_test_dir("tui-inject-notif-failed");
    let controller = make_controller_with_terminal_task(&dir, TaskStatus::Failed);

    let xml = controller
        .state
        .messages
        .iter()
        .find_map(|m| {
            if let MessagePayload::TaskNotification { xml_payload, .. } = &m.payload {
                Some(xml_payload.clone())
            } else {
                None
            }
        })
        .expect("TaskNotification payload for failed task");

    assert!(
        xml.contains("<status>failed</status>"),
        "failed task must have failed status in XML: {xml}"
    );
}

#[test]
fn tui_notification_overlay_preserved_alongside_xml_injection() {
    let dir = unique_test_dir("tui-inject-notif-overlay-preserved");
    let controller = make_controller_with_terminal_task(&dir, TaskStatus::Completed);

    let notifications = controller.view().notifications;
    assert_eq!(
        notifications.len(),
        1,
        "human TUI notification must still exist"
    );
    assert_eq!(notifications[0].title, "Task update");
    assert_eq!(notifications[0].severity, NotificationSeverity::Success);

    assert!(
        controller
            .state
            .messages
            .iter()
            .any(|m| matches!(&m.payload, MessagePayload::TaskNotification { .. })),
        "XML injection must also be present"
    );
}

#[test]
fn repeated_polling_does_not_duplicate_xml_notification() {
    let dir = unique_test_dir("tui-inject-notif-idempotent");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let registry: &'static _ = Box::leak(Box::new(registry));

    let mut controller = TuiController::new(
        test_context(&dir),
        registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    let store = TaskStore::new(&dir);
    let mut task = TaskState::pending("long running job");
    task.status = TaskStatus::Running;
    store.write_task(&task).expect("write running");
    controller.refresh_runtime_state().expect("refresh 1");

    task.mark_finished(TaskStatus::Completed, Some(0), Some("job done".into()));
    store.write_task(&task).expect("write completed");
    controller
        .refresh_runtime_state()
        .expect("refresh 2 – fires injection");

    controller
        .refresh_runtime_state()
        .expect("refresh 3 – no change");
    controller
        .refresh_runtime_state()
        .expect("refresh 4 – no change");
    controller
        .refresh_runtime_state()
        .expect("refresh 5 – no change");

    let notif_count = controller
        .state
        .messages
        .iter()
        .filter(|m| matches!(&m.payload, MessagePayload::TaskNotification { .. }))
        .count();

    assert_eq!(
        notif_count, 1,
        "repeated polling must not produce duplicate TaskNotification messages"
    );
}

#[test]
fn injected_task_id_tracked_in_state_set() {
    let dir = unique_test_dir("tui-inject-notif-state-set");
    let controller = make_controller_with_terminal_task(&dir, TaskStatus::Completed);

    let msg = controller
        .state
        .messages
        .iter()
        .find(|m| matches!(&m.payload, MessagePayload::TaskNotification { .. }))
        .expect("TaskNotification message");

    let task_id = match &msg.payload {
        MessagePayload::TaskNotification { task_id, .. } => *task_id,
        _ => unreachable!(),
    };

    assert!(
        controller
            .state
            .injected_task_notifications
            .contains(&task_id),
        "task id {task_id} must be in injected_task_notifications set"
    );
}

// --- token management: hybrid estimation, warning threshold, microcompact ---

fn make_plain_controller<'a>(
    dir: &std::path::Path,
    registry: &'a wonder_of_u_core::CommandRegistry,
) -> TuiController<'a> {
    TuiController::new(
        test_context(dir),
        registry,
        Some(dir),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller")
}

#[test]
fn estimated_context_tokens_anchors_on_api_usage() {
    let dir = unique_test_dir("tui-ctx-hybrid");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = make_plain_controller(&dir, &registry);
    let session_id = controller.state.session.id;

    controller
        .state
        .messages
        .push(MessageEnvelope::user_text(session_id, "a".repeat(400)));
    controller.note_context_usage(TokenUsage {
        input_tokens: 1_000,
        output_tokens: 200,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
    });

    // Message covered by the anchor must not be re-estimated.
    assert_eq!(controller.estimated_context_tokens(), 1_200);

    // A message appended after the anchor is estimated at ~4 chars/token.
    controller
        .state
        .messages
        .push(MessageEnvelope::user_text(session_id, "b".repeat(400)));
    assert_eq!(controller.estimated_context_tokens(), 1_300);
}

#[test]
fn estimated_context_tokens_falls_back_when_anchor_is_stale() {
    let dir = unique_test_dir("tui-ctx-stale-anchor");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = make_plain_controller(&dir, &registry);
    let session_id = controller.state.session.id;

    controller
        .state
        .messages
        .push(MessageEnvelope::user_text(session_id, "a".repeat(400)));
    controller
        .state
        .messages
        .push(MessageEnvelope::user_text(session_id, "b".repeat(400)));
    controller.note_context_usage(TokenUsage {
        input_tokens: 50_000,
        output_tokens: 0,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
    });

    // Simulate a transcript rewrite that shrinks messages below the anchor.
    controller.state.messages.clear();
    controller
        .state
        .messages
        .push(MessageEnvelope::user_text(session_id, "c".repeat(40)));
    assert_eq!(
        controller.estimated_context_tokens(),
        10,
        "stale anchor must fall back to the full character estimate"
    );

    controller.reset_context_usage_tracking();
    assert_eq!(controller.last_context_usage, None);
}

#[test]
fn context_warning_uses_absolute_token_threshold() {
    let dir = unique_test_dir("tui-ctx-warning");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = make_plain_controller(&dir, &registry);
    // Raw window 200k → effective 180k → autocompact threshold 167k →
    // warning threshold 147k.
    controller.state.context_window_size = Some(200_000);
    controller.context_usage_anchor = 0;

    controller.last_context_usage = Some(140_000);
    assert!(!controller.context_warning_active());

    controller.last_context_usage = Some(150_000);
    assert!(controller.context_warning_active());

    // With autocompact disabled the threshold is the full effective window
    // (180k) so the warning moves to 160k.
    controller.state.auto_compact_enabled = false;
    assert!(!controller.context_warning_active());
    controller.last_context_usage = Some(165_000);
    assert!(controller.context_warning_active());
}

#[test]
fn maybe_autocompact_respects_disabled_toggle() {
    let dir = unique_test_dir("tui-autocompact-toggle");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = make_plain_controller(&dir, &registry);
    controller.state.context_window_size = Some(200_000);
    controller.last_context_usage = Some(190_000);
    controller.context_usage_anchor = 0;

    controller.state.auto_compact_enabled = false;
    controller.maybe_autocompact();
    assert!(
        controller.state.queued_commands.is_empty(),
        "disabled autocompact must not queue /compact"
    );

    controller.state.auto_compact_enabled = true;
    controller.maybe_autocompact();
    assert!(
        controller
            .state
            .queued_commands
            .iter()
            .any(|cmd| cmd.command == "/compact"),
        "enabled autocompact must queue /compact above the threshold"
    );
}

#[test]
fn autocompact_slash_command_toggles_state() {
    let dir = unique_test_dir("tui-autocompact-cmd");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = make_plain_controller(&dir, &registry);
    assert!(controller.state.auto_compact_enabled);

    block_on(controller.execute_slash_command("/autocompact off")).expect("toggle off");
    assert!(!controller.state.auto_compact_enabled);
    assert_eq!(controller.status_note.as_deref(), Some("autocompact off"));

    block_on(controller.execute_slash_command("/autocompact on")).expect("toggle on");
    assert!(controller.state.auto_compact_enabled);
}

fn push_tool_round(
    controller: &mut TuiController<'_>,
    tool: &str,
    content: &str,
    age: time::Duration,
) -> wonder_of_u_core::ToolUseId {
    let session_id = controller.state.session.id;
    let use_id = wonder_of_u_core::ToolUseId::new();
    let stamp = time::OffsetDateTime::now_utc() - age;
    let mut tool_use = MessageEnvelope::new(
        session_id,
        MessagePayload::AssistantToolUse {
            tool: tool.into(),
            use_id,
            input: serde_json::json!({}),
        },
    );
    tool_use.timestamp = stamp;
    let mut result = MessageEnvelope::new(
        session_id,
        MessagePayload::ToolResult {
            tool: tool.into(),
            use_id,
            success: true,
            content: content.into(),
        },
    );
    result.timestamp = stamp;
    controller.state.messages.push(tool_use);
    controller.state.messages.push(result);
    use_id
}

#[test]
fn time_based_microcompact_clears_old_tool_results() {
    let dir = unique_test_dir("tui-microcompact-clear");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = make_plain_controller(&dir, &registry);

    // 7 compactable rounds, all older than the 60-minute gap threshold.
    let ids: Vec<_> = (0..7)
        .map(|_| push_tool_round(&mut controller, "bash", &"x".repeat(400), time::Duration::hours(2)))
        .collect();
    controller.last_context_usage = Some(10_000);
    controller.context_usage_anchor = controller.state.messages.len();

    controller.maybe_time_based_microcompact();

    let cleared: Vec<_> = controller
        .state
        .messages
        .iter()
        .filter_map(|msg| match &msg.payload {
            MessagePayload::ToolResult {
                use_id, content, ..
            } if content == "[Old tool result content cleared]" => Some(*use_id),
            _ => None,
        })
        .collect();
    // Keep the newest 5, clear the oldest 2.
    assert_eq!(cleared, ids[..2].to_vec());
    // The anchored usage must drop by the freed estimate (2 × 400 chars / 4).
    assert_eq!(controller.last_context_usage, Some(10_000 - 200));
}

#[test]
fn time_based_microcompact_skips_recent_sessions() {
    let dir = unique_test_dir("tui-microcompact-recent");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = make_plain_controller(&dir, &registry);

    for _ in 0..7 {
        push_tool_round(&mut controller, "bash", &"x".repeat(400), time::Duration::minutes(5));
    }

    controller.maybe_time_based_microcompact();

    assert!(
        controller.state.messages.iter().all(|msg| !matches!(
            &msg.payload,
            MessagePayload::ToolResult { content, .. }
                if content == "[Old tool result content cleared]"
        )),
        "a recent assistant message must suppress the time-based microcompact"
    );
}
