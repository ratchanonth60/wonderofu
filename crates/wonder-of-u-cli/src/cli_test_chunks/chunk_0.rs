    #[test]
    fn no_command_defaults_to_doctor_when_not_interactive() {
        let mut output = Vec::new();
        run_from(["wonder-of-u"], &mut output).expect("run doctor");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("foundation ok"));
    }

    #[test]
    fn no_command_prefers_tui_when_terminal_is_interactive() {
        let plan = launch_plan(None, true).expect("launch plan");

        assert_eq!(plan, LaunchPlan::Tui { session_id: None });
    }

    #[test]
    fn interactive_resume_routes_into_tui_with_session_id() {
        let plan = launch_plan(
            Some(Commands::Resume {
                session_id: "abc-123".into(),
            }),
            true,
        )
        .expect("launch plan");

        assert_eq!(
            plan,
            LaunchPlan::Tui {
                session_id: Some("abc-123".into())
            }
        );
    }

    #[test]
    fn cost_subcommand_uses_direct_launch_plan() {
        let plan = launch_plan(Some(Commands::Cost { all: true }), false).expect("launch plan");

        assert_eq!(plan, LaunchPlan::Cost { all: true });
    }

    #[test]
    fn vim_subcommand_uses_direct_launch_plan() {
        let plan = launch_plan(Some(Commands::Vim { command: None }), false).expect("launch plan");

        assert_eq!(plan, LaunchPlan::Vim { command: None });
    }

    #[test]
    fn output_style_subcommand_uses_direct_launch_plan() {
        let plan =
            launch_plan(Some(Commands::OutputStyle { command: None }), false).expect("launch plan");

        assert_eq!(plan, LaunchPlan::OutputStyle { command: None });
    }

    #[test]
    fn keybindings_subcommand_uses_direct_launch_plan() {
        let plan = launch_plan(Some(Commands::Keybindings), false).expect("launch plan");

        assert_eq!(plan, LaunchPlan::Keybindings);
    }

    #[test]
    fn env_subcommand_uses_direct_launch_plan() {
        let plan = launch_plan(Some(Commands::Env { command: None }), false).expect("launch plan");

        assert_eq!(plan, LaunchPlan::Env { command: None });
    }

    #[test]
    fn copy_subcommand_uses_direct_launch_plan() {
        let plan = launch_plan(
            Some(Commands::Copy {
                session_id: Some("session-123".into()),
            }),
            false,
        )
        .expect("launch plan");

        assert_eq!(
            plan,
            LaunchPlan::Copy {
                session_id: Some("session-123".into()),
            }
        );
    }

    #[test]
    fn noninteractive_resume_keeps_summary_invocation() {
        let plan = launch_plan(
            Some(Commands::Resume {
                session_id: "abc-123".into(),
            }),
            false,
        )
        .expect("launch plan");

        assert_eq!(
            plan,
            LaunchPlan::Invocation(commands::invocation_from_tokens("resume", ["abc-123"]))
        );
    }

    #[test]
    fn rewind_command_keeps_cli_flags_in_the_registry_invocation() {
        let invocation = to_invocation(Commands::Rewind {
            session: Some("abc-123".into()),
            n: 3,
            yes: true,
        })
        .expect("rewind invocation");

        assert_eq!(
            invocation,
            commands::invocation_from_tokens(
                "rewind",
                ["--session", "abc-123", "--n", "3", "--yes"]
            )
        );
    }

    #[test]
    fn memory_command_uses_direct_launch_plan() {
        let plan =
            launch_plan(Some(Commands::Memory { command: None }), false).expect("launch plan");

        assert_eq!(plan, LaunchPlan::Memory { command: None });
    }

    #[test]
    fn keybindings_command_prints_shortcut_reference() {
        let mut output = Vec::new();

        run_from(["wonder-of-u", "keybindings"], &mut output).expect("run keybindings");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("## Keybindings"));
        assert!(text.contains("Ctrl+C / Ctrl+D"));
        assert!(text.contains("Enter: send message"));
    }

    #[test]
    fn vim_command_reports_current_state() {
        let dir = unique_test_dir("cli-vim-show");
        let mut output = Vec::new();

        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                dir.display().to_string(),
                "vim".to_string(),
            ],
            &mut output,
        )
        .expect("run vim show");

        let text = String::from_utf8(output).expect("utf8");
        assert_eq!(text.trim(), "vim mode: on");
    }

    #[test]
    fn vim_command_toggle_persists_state() {
        let dir = unique_test_dir("cli-vim-toggle-command");
        let mut output = Vec::new();

        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                dir.display().to_string(),
                "vim".to_string(),
                "toggle".to_string(),
            ],
            &mut output,
        )
        .expect("run vim toggle");

        let text = String::from_utf8(output).expect("utf8");
        assert_eq!(text.trim(), "vim mode: off");
        assert_eq!(
            wonder_of_u_agent::SettingsStore::new(&dir)
                .read()
                .expect("read settings")
                .vim_mode,
            Some(false)
        );
    }

