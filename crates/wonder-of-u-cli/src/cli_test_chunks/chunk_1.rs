    #[test]
    fn help_lists_expanded_command_catalog() {
        let mut output = Vec::new();
        run_from(["wonder-of-u", "help"], &mut output).expect("run help");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("config"));
        assert!(text.contains("resume"));
        assert!(text.contains("rename"));
        assert!(text.contains("files"));
        assert!(text.contains("permissions"));
        assert!(text.contains("mcp"));
        assert!(text.contains("plugin"));
        assert!(text.contains("reload-plugins"));
        assert!(text.contains("skills"));
        assert!(text.contains("agents"));
        assert!(text.contains("tui"));
        assert!(!text.contains("features"));
    }

    #[test]
    fn hidden_command_can_still_be_addressed_directly() {
        let mut output = Vec::new();
        run_from(["wonder-of-u", "help", "features"], &mut output).expect("run help features");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("Print enabled first-release feature gates"));
        assert!(text.contains("hidden from help listings"));
    }

    #[test]
    fn features_command_lists_session_persistence() {
        let mut output = Vec::new();
        run_from(["wonder-of-u", "features"], &mut output).expect("run features");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("SessionPersistence"));
    }

    #[test]
    fn memory_path_command_prints_global_memory_path() {
        let storage_dir = unique_test_dir("cli-memory-path");
        let mut output = Vec::new();

        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.display().to_string(),
                "memory".to_string(),
                "path".to_string(),
            ],
            &mut output,
        )
        .expect("run memory path");

        let text = String::from_utf8(output).expect("utf8");
        assert_eq!(
            text.trim(),
            storage_dir.join("CLAUDE.md").display().to_string()
        );
    }

    #[test]
    fn theme_list_command_prints_available_themes() {
        let mut output = Vec::new();

        run_from(["wonder-of-u", "theme", "list"], &mut output).expect("run theme list");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.lines().any(|theme| theme == "default"));
        assert!(text.lines().any(|theme| theme == "midnight"));
    }

    #[test]
    fn theme_set_command_persists_theme() {
        let dir = unique_test_dir("cli-theme-command-set");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "theme".to_string(),
                "set".to_string(),
                "midnight".to_string(),
            ],
            &mut output,
        )
        .expect("run theme set");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("theme=midnight"));
        assert_eq!(
            SettingsStore::new(&dir)
                .read()
                .expect("read settings")
                .theme
                .as_deref(),
            Some("midnight")
        );

        let mut show_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "theme".to_string(),
            ],
            &mut show_output,
        )
        .expect("run theme show");
        let show_text = String::from_utf8(show_output).expect("utf8");
        assert_eq!(show_text.trim(), "current_theme=midnight");
    }

    #[test]
    fn theme_set_command_rejects_unknown_theme() {
        let dir = unique_test_dir("cli-theme-command-invalid");
        let storage_dir = dir.to_string_lossy().into_owned();

        let error = run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "theme".to_string(),
                "set".to_string(),
                "aurora".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect_err("unknown theme should fail");

        assert!(error.to_string().contains("unknown theme: aurora"));
    }

    #[test]
    fn output_style_list_command_prints_available_styles() {
        let mut output = Vec::new();

        run_from(["wonder-of-u", "output-style", "list"], &mut output)
            .expect("run output-style list");

        assert_eq!(
            String::from_utf8(output).expect("utf8").trim(),
            "markdown\nplain\nraw"
        );
    }

    #[test]
    fn output_style_set_command_persists_style() {
        let dir = unique_test_dir("cli-output-style-command-set");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "output-style".to_string(),
                "set".to_string(),
                "raw".to_string(),
            ],
            &mut output,
        )
        .expect("run output-style set");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("output_style=raw"));
        assert_eq!(
            wonder_of_u_agent::SettingsStore::new(&dir)
                .read()
                .expect("read settings")
                .output_style
                .as_deref(),
            Some("raw")
        );

        let mut show_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "output-style".to_string(),
            ],
            &mut show_output,
        )
        .expect("run output-style show");
        let show_text = String::from_utf8(show_output).expect("utf8");
        assert_eq!(show_text.trim(), "current_output_style=raw");
    }

    #[test]
    fn output_style_set_command_rejects_unknown_style() {
        let dir = unique_test_dir("cli-output-style-command-invalid");
        let storage_dir = dir.to_string_lossy().into_owned();

        let error = run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "output-style".to_string(),
                "set".to_string(),
                "html".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect_err("unknown output style should fail");

        assert!(error.to_string().contains("unknown output style: html"));
    }

    #[test]
    fn env_show_command_displays_persisted_and_system_values() {
        let dir = unique_test_dir("cli-env-command-show");
        let storage_dir = dir.to_string_lossy().into_owned();
        let _provider = EnvVarGuard::set("WONDER_PROVIDER", "anthropic");
        wonder_of_u_agent::SettingsStore::new(&dir)
            .write(&wonder_of_u_agent::AgentSettings {
                env_vars: std::collections::HashMap::from([(
                    "WONDER_MODEL".into(),
                    "sonnet".into(),
                )]),
                ..wonder_of_u_agent::AgentSettings::default()
            })
            .expect("write settings");

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "env".to_string(),
            ],
            &mut output,
        )
        .expect("run env show");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("persisted.WONDER_MODEL=sonnet"));
        assert!(text.contains("system.WONDER_PROVIDER=anthropic"));
    }

    #[test]
    fn env_set_and_unset_commands_persist_changes() {
        let dir = unique_test_dir("cli-env-command-set-unset");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut set_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "env".to_string(),
                "set".to_string(),
                "FOO=bar".to_string(),
            ],
            &mut set_output,
        )
        .expect("run env set");

        assert_eq!(
            wonder_of_u_agent::SettingsStore::new(&dir)
                .read()
                .expect("read settings")
                .env_vars
                .get("FOO"),
            Some(&"bar".to_string())
        );

        let mut unset_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "env".to_string(),
                "unset".to_string(),
                "FOO".to_string(),
            ],
            &mut unset_output,
        )
        .expect("run env unset");

        assert!(
            !wonder_of_u_agent::SettingsStore::new(&dir)
                .read()
                .expect("read settings")
                .env_vars
                .contains_key("FOO")
        );
    }

    #[test]
    fn env_set_command_rejects_invalid_assignment() {
        let dir = unique_test_dir("cli-env-command-invalid-assignment");
        let storage_dir = dir.to_string_lossy().into_owned();

        let error = run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "env".to_string(),
                "set".to_string(),
                "BROKEN".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect_err("invalid assignment");

        assert!(
            error
                .to_string()
                .contains("environment variable assignment must use KEY=VALUE syntax")
        );
    }

    #[test]
    fn status_reports_storage_counts() {
        let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
        let _oai_key = EnvVarGuard::set("OPENAI_API_KEY", "");
        let dir = unique_test_dir("cli-status");
        let storage_dir = dir.to_string_lossy().into_owned();

        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "session".to_string(),
                "new".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("create session");

        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "tasks".to_string(),
                "start".to_string(),
                "shell".to_string(),
                "--description".to_string(),
                "status smoke".to_string(),
                "--command".to_string(),
                "printf status-smoke".to_string(),
                "--read-only".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("start status task");
        std::thread::sleep(std::time::Duration::from_millis(200));

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "status".to_string(),
            ],
            &mut output,
        )
        .expect("run status");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("transcripts=1"));
        assert!(text.contains("metadata=1"));
        assert!(text.contains("snapshots=1"));
        assert!(text.contains("session_memory_indexes=1"));
        assert!(text.contains("provider_readiness=unconfigured"));
        assert!(text.contains("settings_sync=local_only"));
        assert!(text.contains("settings_sync_cloud=unsupported"));
        assert!(text.contains("remote_managed_settings=deferred"));
        assert!(text.contains("team_memory_sync=unsupported"));
        assert!(text.contains("skills=14"));
        assert!(text.contains("mcp_servers=0"));
        assert!(text.contains("active_tasks=0"));
        assert!(text.contains("terminal_tasks=1"));
        assert!(text.contains("completed_tasks=1"));
        assert!(text.contains("tasks_reconciled_at="));
        assert!(text.contains("fresh_task_heartbeats=0"));
        assert!(text.contains("plugin_runtime=command_subprocess"));
    }

    #[test]
    fn cost_and_usage_commands_render_human_readable_cost_summary() {
        let dir = unique_test_dir("cli-cost-command");
        let storage_dir = dir.to_string_lossy().into_owned();
        let store = TranscriptStore::new(&dir);
        let cost_store = CostStore::new(&dir);

        let mut state = AppState::new(dir.join("workspace"));
        fs::create_dir_all(&state.session.cwd).expect("create workspace");
        state.record_cost_usage(
            TokenUsage {
                input_tokens: 12_345,
                output_tokens: 3_456,
                cache_creation_tokens: 567,
                cache_read_tokens: 1_234,
            },
            Some(0.0842),
        );
        store
            .write_metadata(&SessionMetadata::from_app_state(&state))
            .expect("write metadata");
        cost_store
            .write_costs(&SessionCostLedger::from_app_state(&state))
            .expect("write costs");

        let _cwd_lock = CWD_TEST_LOCK.lock().expect("cwd lock");
        let original_cwd = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(&state.session.cwd).expect("set current dir");

        let mut cost_output = Vec::new();
        let mut usage_output = Vec::new();
        let result = (|| -> Result<()> {
            run_from(
                vec![
                    "wonder-of-u".to_string(),
                    "--storage-dir".to_string(),
                    storage_dir.clone(),
                    "cost".to_string(),
                ],
                &mut cost_output,
            )?;
            run_from(
                vec![
                    "wonder-of-u".to_string(),
                    "--storage-dir".to_string(),
                    storage_dir,
                    "usage".to_string(),
                ],
                &mut usage_output,
            )?;
            Ok(())
        })();
        std::env::set_current_dir(&original_cwd).expect("restore current dir");
        result.expect("run cost commands");

        let cost_text = String::from_utf8(cost_output).expect("cost utf8");
        let usage_text = String::from_utf8(usage_output).expect("usage utf8");

        assert_eq!(cost_text, usage_text);
        assert!(cost_text.contains(&format!("Session: {}", state.session.id)));
        assert!(cost_text.contains("  Input tokens:   12,345"));
        assert!(cost_text.contains("  Output tokens:  3,456"));
        assert!(cost_text.contains("  Cache read:     1,234"));
        assert!(cost_text.contains("  Cache write:    567"));
        assert!(cost_text.contains("  Total cost:     $0.0842"));
        assert!(cost_text.contains("All-time total:   $0.0842"));
    }

