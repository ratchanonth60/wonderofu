// ── Fleet Panel TUI controller tests ────────────────────────────────────────

fn seed_fleet_run(
    storage_dir: &Path,
    fleet_id: &str,
    description: &str,
    status: FleetRunStatus,
) {
    let id: FleetId = std::str::FromStr::from_str(fleet_id).expect("valid fleet id");
    let mut run = FleetRunState::new(description, PermissionMode::Default, Some(storage_dir.to_path_buf()));
    run.id = id;
    run.status = status;
    FleetStore::new(storage_dir)
        .write_run(&run)
        .expect("write fleet run");
}

// ── Open / close ──────────────────────────────────────────────────────────────

#[test]
fn fleet_panel_opens_via_slash_fleet() {
    let dir = unique_test_dir("tui-fleet-open-slash");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/fleet")).expect("open fleet panel");

    assert!(controller.fleet_panel.is_some(), "fleet panel should open");
    assert_eq!(
        controller.status_note.as_deref(),
        Some("fleet panel opened")
    );
}

#[test]
fn fleet_panel_closes_via_esc() {
    let dir = unique_test_dir("tui-fleet-esc");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/fleet")).expect("open fleet panel");
    assert!(controller.fleet_panel.is_some());

    send_prompt_key(&mut controller, picker_key(KeyCode::Esc));
    assert!(controller.fleet_panel.is_none(), "Esc should close fleet panel");
}

#[test]
fn fleet_panel_closes_via_q() {
    let dir = unique_test_dir("tui-fleet-q");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/fleet")).expect("open fleet panel");
    send_prompt_key(&mut controller, picker_key(KeyCode::Char('q')));
    assert!(controller.fleet_panel.is_none(), "'q' should close fleet panel");
}

// ── Ctrl+F toggle ─────────────────────────────────────────────────────────────

#[test]
fn ctrl_f_toggles_fleet_panel() {
    let dir = unique_test_dir("tui-fleet-ctrl-f");
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

    let ctrl_f = KeyEvent {
        code: KeyCode::Char('f'),
        modifiers: KeyModifiers {
            control: true,
            ..std::default::Default::default()
        },
    };

    send_prompt_key(&mut controller, ctrl_f);
    assert!(controller.fleet_panel.is_some(), "Ctrl+F should open panel");

    send_prompt_key(&mut controller, ctrl_f);
    assert!(
        controller.fleet_panel.is_none(),
        "Ctrl+F again should close panel"
    );
}

// ── Navigation ────────────────────────────────────────────────────────────────

#[test]
fn fleet_panel_navigate_up_down_wraps() {
    let dir = unique_test_dir("tui-fleet-nav-wrap");
    seed_fleet_run(
        &dir,
        "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        "first run",
        FleetRunStatus::Running,
    );
    seed_fleet_run(
        &dir,
        "11111111-2222-3333-4444-555555555555",
        "second run",
        FleetRunStatus::Pending,
    );

    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/fleet")).expect("open fleet panel");
    let panel = controller.fleet_panel.as_ref().unwrap();
    assert_eq!(panel.fleet_views.len(), 2, "should have 2 fleet runs");

    send_prompt_key(&mut controller, picker_key(KeyCode::Down));
    assert_eq!(
        controller.fleet_panel.as_ref().unwrap().selected_index,
        1
    );

    send_prompt_key(&mut controller, picker_key(KeyCode::Down));
    assert_eq!(
        controller.fleet_panel.as_ref().unwrap().selected_index,
        0
    );

    send_prompt_key(&mut controller, picker_key(KeyCode::Up));
    assert_eq!(
        controller.fleet_panel.as_ref().unwrap().selected_index,
        1
    );
}

// ── Panel stays open on non-fleet keys ────────────────────────────────────────

#[test]
fn fleet_panel_ignores_non_navigation_keys() {
    let dir = unique_test_dir("tui-fleet-ignore");
    seed_fleet_run(
        &dir,
        "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        "test run",
        FleetRunStatus::Running,
    );
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/fleet")).expect("open fleet panel");
    let before = controller.fleet_panel.as_ref().unwrap().selected_index;

    send_prompt_key(&mut controller, picker_key(KeyCode::Char('x')));
    assert!(controller.fleet_panel.is_some(), "panel should stay open");
    assert_eq!(
        controller.fleet_panel.as_ref().unwrap().selected_index,
        before,
        "selection should not change"
    );
}

// ── View output ───────────────────────────────────────────────────────────────

#[test]
fn fleet_panel_view_includes_run_data() {
    let dir = unique_test_dir("tui-fleet-view");
    seed_fleet_run(
        &dir,
        "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        "integration test",
        FleetRunStatus::Running,
    );
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/fleet")).expect("open fleet panel");

    let view = controller.view();
    let overlay = view.fleet_panel.as_ref().expect("fleet_panel in view");
    assert!(overlay.title.contains("Fleet Runs"));
    assert!(
        overlay.lines.iter().any(|l| l.contains("integration test")),
        "should show fleet description"
    );
    assert!(
        overlay.lines.iter().any(|l| l.contains("running")),
        "should show fleet status"
    );
}

#[test]
fn fleet_panel_view_empty_shows_placeholder() {
    let dir = unique_test_dir("tui-fleet-empty");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/fleet")).expect("open fleet panel");

    let view = controller.view();
    let overlay = view.fleet_panel.as_ref().expect("fleet_panel in view");
    assert!(
        overlay
            .lines
            .iter()
            .any(|l| l.contains("No fleet runs found")),
        "empty panel should show placeholder"
    );
}

// ── Tick refresh ──────────────────────────────────────────────────────────────

#[test]
fn fleet_panel_refreshes_on_tick() {
    let dir = unique_test_dir("tui-fleet-tick-refresh");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/fleet")).expect("open fleet panel");
    assert!(controller
        .fleet_panel
        .as_ref()
        .unwrap()
        .fleet_views
        .is_empty());

    seed_fleet_run(
        &dir,
        "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        "new run",
        FleetRunStatus::Pending,
    );
    let changed = controller.refresh_runtime_state().expect("refresh");
    assert!(changed);

    assert_eq!(
        controller.fleet_panel.as_ref().unwrap().fleet_views.len(),
        1
    );
    assert_eq!(
        controller.fleet_panel.as_ref().unwrap().fleet_views[0].description,
        "new run"
    );
}

// ── Completion toast ───────────────────────────────────────────────────────────

#[test]
fn fleet_completion_notification_fires_once() {
    let dir = unique_test_dir("tui-fleet-complete");
    let fleet_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    // Controller construction refreshes runtime state once. Now seed fleet data
    // and tick again to trigger the fleet sync.
    seed_fleet_run(&dir, fleet_id, "done run", FleetRunStatus::Completed);

    let initial_notifications = controller.notifications.len();
    let changed = controller.refresh_runtime_state().expect("refresh");
    assert!(changed);

    let new_count = controller.notifications.len();
    assert!(
        new_count > initial_notifications,
        "should have fired at least one notification"
    );

    let key = format!("fleet:{fleet_id}:completed");
    assert!(controller.fleet_completion_keys.contains(&key));

    let count_after_first = controller.notifications.len();
    controller.refresh_runtime_state().expect("refresh again");
    assert_eq!(
        controller.notifications.len(),
        count_after_first,
        "second tick should not duplicate notification"
    );
}

// ── /fleet with args still delegates to CLI ────────────────────────────────────

#[test]
fn fleet_with_args_still_delegates_to_command() {
    let dir = unique_test_dir("tui-fleet-args");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    block_on(controller.execute_slash_command("/fleet status")).expect("execute /fleet status");

    // Fleet panel should NOT be open (only bare /fleet opens it)
    assert!(controller.fleet_panel.is_none());
    // Slash command output is NOT stored in messages (it goes to a dialog or
    // produces side-effects); same pattern as /model and /memory tests.
    // The key assertion: panel stayed closed.
}

// ── has_modal_overlay includes fleet panel ─────────────────────────────────────

#[test]
fn fleet_panel_is_modal_overlay() {
    let dir = unique_test_dir("tui-fleet-modal");
    let registry = commands::registry(Some(dir.clone())).expect("registry");
    let mut controller = TuiController::new(
        test_context(&dir),
        &registry,
        Some(dir.as_path()),
        TuiLaunchOptions { session_id: None },
    )
    .expect("controller");

    assert!(!controller.has_modal_overlay());

    block_on(controller.execute_slash_command("/fleet")).expect("open fleet panel");
    assert!(controller.has_modal_overlay());

    send_prompt_key(&mut controller, picker_key(KeyCode::Esc));
    assert!(!controller.has_modal_overlay());
}
