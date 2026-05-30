    #[test]
    fn help_lists_dynamic_plugin_commands() {
        let dir = unique_test_dir("cli-help-plugin-command");
        let storage_dir = dir.to_string_lossy().into_owned();
        let plugin_root = seed_runnable_plugin_fixture(
            &dir,
            "help-plugin",
            json!([{
                "name": "echo",
                "description": "Echo arguments",
                "path": "commands/echo.sh"
            }]),
            Some(PluginTrustDecision::Trusted),
        );
        write_executable_script(
            &plugin_root.join("commands"),
            "echo.sh",
            "printf 'help plugin\\n'\n",
        );

        let mut output = Vec::new();
        run_from(
            ["wonder-of-u", "--storage-dir", storage_dir.as_str(), "help"],
            &mut output,
        )
        .expect("run help");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("echo"));
        assert!(text.contains("Echo arguments"));
    }

    #[test]
    fn slash_transport_preserves_quoted_arguments_for_rename() {
        let dir = unique_test_dir("cli-rename");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut created = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "session".to_string(),
                "new".to_string(),
            ],
            &mut created,
        )
        .expect("create session");
        let created = String::from_utf8(created).expect("utf8");
        let session_id = created
            .lines()
            .find_map(|line| line.strip_prefix("session_id="))
            .expect("session id")
            .to_string();

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "slash".to_string(),
                "/rename".to_string(),
                session_id.clone(),
                "Quarterly Review".to_string(),
            ],
            &mut output,
        )
        .expect("rename session");
        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("title=Quarterly Review"));

        let mut resumed = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "resume".to_string(),
                session_id,
            ],
            &mut resumed,
        )
        .expect("resume session");
        let text = String::from_utf8(resumed).expect("utf8");
        assert!(text.contains("title=Quarterly Review"));
    }

    #[test]
    fn session_commands_round_trip_listing_resume_and_export() {
        let dir = unique_test_dir("cli-session-roundtrip");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut created = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "session".to_string(),
                "new".to_string(),
                "--title".to_string(),
                "Stored Session".to_string(),
            ],
            &mut created,
        )
        .expect("create session");
        let created = String::from_utf8(created).expect("utf8");
        let session_id = created
            .lines()
            .find_map(|line| line.strip_prefix("session_id="))
            .expect("session id")
            .to_string();

        let mut listed = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "session".to_string(),
                "list".to_string(),
            ],
            &mut listed,
        )
        .expect("list sessions");
        let listed = String::from_utf8(listed).expect("utf8");
        assert!(listed.contains(&format!("session[0].id={session_id}")));
        assert!(listed.contains("session[0].title=Stored Session"));

        let mut resumed = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "resume".to_string(),
                session_id.clone(),
            ],
            &mut resumed,
        )
        .expect("resume session");
        let resumed = String::from_utf8(resumed).expect("utf8");
        assert!(resumed.contains("resume_state=snapshot"));
        assert!(resumed.contains("transcript_messages=1"));
        assert!(resumed.contains("view_messages=1"));
        assert!(listed.contains("session[0].resume_source=snapshot"));

        let mut exported = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "export".to_string(),
                session_id,
                "--format".to_string(),
                "json".to_string(),
            ],
            &mut exported,
        )
        .expect("export session");
        let exported = String::from_utf8(exported).expect("utf8");
        assert!(exported.contains("\"title\": \"Stored Session\""));
        assert!(exported.contains("\"transcript\""));
        assert!(exported.contains("\"resume\""));
        assert!(exported.contains("\"source\": \"snapshot\""));
    }

    #[test]
    fn tag_command_round_trips_through_cli() {
        let dir = unique_test_dir("cli-tag");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut created = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "session".to_string(),
                "new".to_string(),
                "--title".to_string(),
                "Tagged Session".to_string(),
            ],
            &mut created,
        )
        .expect("create session");
        let created = String::from_utf8(created).expect("utf8");
        let session_id = extract_value(&created, "session_id=").to_string();

        let mut tagged = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "tag".to_string(),
                "bugfix".to_string(),
            ],
            &mut tagged,
        )
        .expect("tag session");
        let tagged = String::from_utf8(tagged).expect("utf8");
        assert!(tagged.contains(&format!("session_id={session_id}")));
        assert!(tagged.contains("session_tags=bugfix"));

        let mut listed = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "session".to_string(),
                "list".to_string(),
            ],
            &mut listed,
        )
        .expect("list sessions");
        let listed = String::from_utf8(listed).expect("utf8");
        assert!(listed.contains("session[0].tags=#bugfix"));

        let mut tag_list = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "tag".to_string(),
                "--list".to_string(),
            ],
            &mut tag_list,
        )
        .expect("list tags");
        let tag_list = String::from_utf8(tag_list).expect("utf8");
        assert!(tag_list.contains("tag[0].name=bugfix"));
        assert!(tag_list.contains(&format!("tag[0].session[0].id={session_id}")));
    }

    #[test]
    fn clear_and_compact_persist_resume_views() {
        let dir = unique_test_dir("cli-session-view-state");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut created = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "session".to_string(),
                "new".to_string(),
                "--title".to_string(),
                "View State".to_string(),
            ],
            &mut created,
        )
        .expect("create session");
        let created = String::from_utf8(created).expect("utf8");
        let session_id = extract_value(&created, "session_id=").to_string();

        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "rename".to_string(),
                session_id.clone(),
                "View State Renamed".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("rename session");

        let mut compacted = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "compact".to_string(),
                session_id.clone(),
                "--keep-last".to_string(),
                "1".to_string(),
            ],
            &mut compacted,
        )
        .expect("compact session");
        let compacted = String::from_utf8(compacted).expect("utf8");
        assert!(compacted.contains("view_action=compact"));
        assert!(compacted.contains("compacted_messages=1"));
        assert!(compacted.contains("transcript_unchanged=true"));

        let mut resumed = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "resume".to_string(),
                session_id.clone(),
            ],
            &mut resumed,
        )
        .expect("resume compacted session");
        let resumed = String::from_utf8(resumed).expect("utf8");
        assert!(resumed.contains("resume_state=snapshot"));
        assert!(resumed.contains("transcript_messages=2"));
        assert!(resumed.contains("view_messages=2"));
        assert!(resumed.contains("compacted_messages=1"));

        let mut cleared = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "clear".to_string(),
                session_id.clone(),
            ],
            &mut cleared,
        )
        .expect("clear session");
        let cleared = String::from_utf8(cleared).expect("utf8");
        assert!(cleared.contains("view_action=clear"));
        assert!(cleared.contains("compacted_messages=2"));
        assert!(cleared.contains("boundary_summary=Cleared the visible transcript"));

        let mut exported = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "export".to_string(),
                session_id,
                "--format".to_string(),
                "json".to_string(),
            ],
            &mut exported,
        )
        .expect("export cleared session");
        let exported = String::from_utf8(exported).expect("utf8");
        assert!(exported.contains("\"compacted_messages\": 2"));
        assert!(exported.contains("\"state\""));
    }

    #[test]
    fn config_and_permissions_commands_surface_foundations() {
        let dir = unique_test_dir("cli-config-permissions");
        let storage_dir = dir.to_string_lossy().into_owned();
        fs::create_dir_all(dir.join("config")).expect("create config dir");
        fs::write(
            dir.join("config").join("CLAUDE.md"),
            "# local user memory\n",
        )
        .expect("write user memory");

        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "config".to_string(),
                "set-api-base".to_string(),
                "--provider".to_string(),
                "openai".to_string(),
                "--api-base".to_string(),
                "https://example.invalid/v1".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("set api base");

        let mut config_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "config".to_string(),
                "show".to_string(),
            ],
            &mut config_output,
        )
        .expect("show config");
        let config_text = String::from_utf8(config_output).expect("utf8");
        assert!(
            config_text.contains("provider_override[openai].api_base=https://example.invalid/v1")
        );
        assert!(config_text.contains("settings_sync=local_only"));
        assert!(config_text.contains("settings_sync_cloud=unsupported"));
        assert!(config_text.contains("settings_sync_settings_exists=true"));
        assert!(config_text.contains("settings_sync_user_memory_exists=true"));
        assert!(config_text.contains("remote_managed_settings=deferred"));
        assert!(config_text.contains("team_memory_sync=unsupported"));

        let mut permissions_output = Vec::new();
        run_from(
            [
                "wonder-of-u",
                "permissions",
                "check",
                "--tool",
                "bash",
                "--shell",
                "rm -rf target/test-workspaces/demo",
            ],
            &mut permissions_output,
        )
        .expect("check permissions");
        let permissions_text = String::from_utf8(permissions_output).expect("utf8");
        assert!(permissions_text.contains("decision=ask"));
        assert!(permissions_text.contains("removes files recursively"));
    }

    #[test]
    fn tasks_commands_manage_shell_task_lifecycle() {
        let dir = unique_test_dir("cli-tasks-shell");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut started = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "tasks".to_string(),
                "start".to_string(),
                "shell".to_string(),
                "--description".to_string(),
                "echo hello".to_string(),
                "--command".to_string(),
                "printf 'hello from task'".to_string(),
                "--read-only".to_string(),
            ],
            &mut started,
        )
        .expect("start task");
        let started = String::from_utf8(started).expect("utf8");
        let task_id = extract_value(&started, "task_id=").to_string();

        let mut listed = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "tasks".to_string(),
                "list".to_string(),
            ],
            &mut listed,
        )
        .expect("list tasks");
        let listed = String::from_utf8(listed).expect("utf8");
        assert!(listed.contains(&format!("task[0].id={task_id}")));

        let mut shown_text = String::new();
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let mut shown = Vec::new();
            run_from(
                vec![
                    "wonder-of-u".to_string(),
                    "--storage-dir".to_string(),
                    storage_dir.clone(),
                    "tasks".to_string(),
                    "show".to_string(),
                    task_id.clone(),
                    "--tail".to_string(),
                    "20".to_string(),
                ],
                &mut shown,
            )
            .expect("show task");
            shown_text = String::from_utf8(shown).expect("utf8");
            if shown_text.contains("status=completed") {
                break;
            }
        }

        assert!(shown_text.contains("status=completed"));
        assert!(shown_text.contains("hello from task"));
        assert!(shown_text.contains("last_heartbeat_at="));

        let mut status = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "tasks".to_string(),
            ],
            &mut status,
        )
        .expect("task status");
        let status = String::from_utf8(status).expect("utf8");
        assert!(status.contains("tasks=1"));
        assert!(status.contains("reconciled_at="));
    }

    #[test]
    fn tasks_commands_stop_background_processes() {
        let dir = unique_test_dir("cli-tasks-stop");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut started = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "tasks".to_string(),
                "start".to_string(),
                "shell".to_string(),
                "--description".to_string(),
                "sleep".to_string(),
                "--command".to_string(),
                "sleep 10".to_string(),
                "--read-only".to_string(),
            ],
            &mut started,
        )
        .expect("start task");
        let task_id =
            extract_value(&String::from_utf8(started).expect("utf8"), "task_id=").to_string();

        let mut stopped = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "tasks".to_string(),
                "stop".to_string(),
                task_id,
                "--force".to_string(),
            ],
            &mut stopped,
        )
        .expect("stop task");
        let stopped = String::from_utf8(stopped).expect("utf8");
        assert!(stopped.contains("status=killed"));
    }

    #[test]
    fn tasks_reconcile_command_surfaces_stale_heartbeat() {
        let dir = unique_test_dir("cli-tasks-reconcile");
        let storage_dir = dir.to_string_lossy().into_owned();
        let store = TaskStore::new(&dir);
        let mut task = TaskState::pending_shell("stale heartbeat", "sleep 1", &dir);
        task.status = TaskStatus::Running;
        task.pid = Some(std::process::id());
        task.output_log = Some(store.paths().task_log_path(task.id));
        store.write_task(&task).expect("write task");
        store
            .write_heartbeat_at(
                task.id,
                OffsetDateTime::now_utc() - time::Duration::seconds(10),
            )
            .expect("write stale heartbeat");

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "tasks".to_string(),
                "reconcile".to_string(),
            ],
            &mut output,
        )
        .expect("reconcile tasks");
        let output = String::from_utf8(output).expect("utf8");
        assert!(output.contains("tasks=1"));
        assert!(output.contains("reconcile_changed=1"));
        assert!(output.contains("stale_heartbeats=1"));
    }

    #[test]
    fn agents_commands_manage_prompt_subprocess_entries() {
        let dir = unique_test_dir("cli-agents");
        let storage_dir = dir.to_string_lossy().into_owned();
        let script = write_executable_script(
            &dir,
            "agent-cli.sh",
            "printf 'agent cli args: %s\\n' \"$*\"\nsleep 10\n",
        );
        let _env = EnvVarGuard::set("WONDER_OF_U_CLI_BIN", script.into_os_string());

        let mut started = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "agents".to_string(),
                "start".to_string(),
                "local".to_string(),
                "--name".to_string(),
                "planner".to_string(),
                "--prompt".to_string(),
                "Summarize release blockers".to_string(),
                "--provider".to_string(),
                "openai".to_string(),
                "--model".to_string(),
                "gpt-4.1".to_string(),
            ],
            &mut started,
        )
        .expect("start agent");
        let started = String::from_utf8(started).expect("utf8");
        let agent_id = extract_value(&started, "agent_id=").to_string();
        assert!(started.contains("runtime=prompt_subprocess"));
        assert!(started.contains("status=running"));
        assert!(started.contains("pid="));

        let mut shown = String::new();
        for _ in 0..20 {
            let mut output = Vec::new();
            run_from(
                vec![
                    "wonder-of-u".to_string(),
                    "--storage-dir".to_string(),
                    storage_dir.clone(),
                    "agents".to_string(),
                    "show".to_string(),
                    agent_id.clone(),
                ],
                &mut output,
            )
            .expect("show agent");
            shown = String::from_utf8(output).expect("utf8");
            if shown.contains("agent cli args:") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(shown.contains("agent.runtime=prompt_subprocess"));
        assert!(shown.contains("agent.provider=openai"));
        assert!(shown.contains("status=running"));
        assert!(shown.contains("agent cli args:"));

        let mut status = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "agents".to_string(),
            ],
            &mut status,
        )
        .expect("agent status");
        let status = String::from_utf8(status).expect("utf8");
        assert!(status.contains("agents=1"));
        assert!(status.contains("prompt_subprocess=1"));
        assert!(status.contains("running=1"));

        let mut stopped = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "agents".to_string(),
                "stop".to_string(),
                agent_id,
            ],
            &mut stopped,
        )
        .expect("stop agent");
        let stopped = String::from_utf8(stopped).expect("utf8");
        assert!(stopped.contains("status=cancelled"));
    }

    #[test]
    fn task_start_respects_permission_gate() {
        let dir = unique_test_dir("cli-task-permissions");
        let storage_dir = dir.to_string_lossy().into_owned();
        let mut output = Vec::new();

        let error = run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "tasks".to_string(),
                "start".to_string(),
                "shell".to_string(),
                "--description".to_string(),
                "write file".to_string(),
                "--command".to_string(),
                "printf hi > task.txt".to_string(),
            ],
            &mut output,
        )
        .expect_err("permission gate should block default write task");

        assert!(
            error
                .to_string()
                .contains("task launch requires an allowed permission decision")
        );
    }

    #[test]
    fn mcp_commands_surface_config_and_status() {
        let dir = unique_test_dir("cli-mcp");
        let storage_dir = dir.to_string_lossy().into_owned();
        let config_dir = dir.join("config").join("mcp");
        fs::create_dir_all(&config_dir).expect("create mcp config dir");
        fs::write(
            config_dir.join("servers.json"),
            r#"{
  "schema_version": 1,
  "protocol_version": "2024-11-05",
  "client": {
    "name": "wonder-of-u",
    "version": "0.1.0"
  },
  "servers": [
    {
      "name": "demo",
      "command": "/bin/sh",
      "args": ["-c", "exit 0"],
      "enabled": true
    }
  ]
}
"#,
        )
        .expect("write mcp config");

        let mut show_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "mcp".to_string(),
                "show".to_string(),
            ],
            &mut show_output,
        )
        .expect("show mcp config");
        let show_text = String::from_utf8(show_output).expect("utf8");
        assert!(show_text.contains("servers=1"));
        assert!(show_text.contains("server[0].name=demo"));

        let mut disable_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "mcp".to_string(),
                "disable".to_string(),
                "demo".to_string(),
            ],
            &mut disable_output,
        )
        .expect("disable mcp server");
        let disable_text = String::from_utf8(disable_output).expect("utf8");
        assert!(disable_text.contains("enabled=false"));

        let mut status_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "mcp".to_string(),
                "status".to_string(),
            ],
            &mut status_output,
        )
        .expect("mcp status");
        let status_text = String::from_utf8(status_output).expect("utf8");
        assert!(status_text.contains("ready_servers=0"));
        assert!(status_text.contains("server[demo].state=disabled"));
    }

    #[test]
    fn files_command_lists_workspace_entries() {
        let _cwd_lock = CWD_TEST_LOCK.lock().expect("cwd lock");
        let dir = unique_test_dir("cli-files");
        fs::create_dir_all(dir.join("src")).expect("create src");
        fs::write(dir.join("Cargo.toml"), "[package]\nname='demo'\n").expect("write manifest");
        fs::write(dir.join("src").join("main.rs"), "fn main() {}\n").expect("write main");

        let mut output = Vec::new();
        let current = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(&dir).expect("set cwd");
        let result = run_from(["wonder-of-u", "files", "--limit", "10"], &mut output);
        std::env::set_current_dir(current).expect("restore cwd");
        result.expect("run files");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("Cargo.toml"));
        assert!(text.contains("src/main.rs"));
    }

    #[test]
    fn branch_and_diff_commands_report_git_state() {
        let _cwd_lock = CWD_TEST_LOCK.lock().expect("cwd lock");
        let dir = unique_test_dir("cli-git");
        init_git_repo(&dir);
        fs::write(dir.join("README.md"), "hello\n").expect("write readme");
        ProcessCommand::new("git")
            .args(["add", "README.md"])
            .current_dir(&dir)
            .status()
            .expect("git add");
        let current = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(&dir).expect("set cwd");

        let mut branch_output = Vec::new();
        let branch_result = run_from(["wonder-of-u", "branch"], &mut branch_output);
        let mut diff_output = Vec::new();
        let diff_result = run_from(
            ["wonder-of-u", "diff", "--name-only", "--staged"],
            &mut diff_output,
        );

        std::env::set_current_dir(current).expect("restore cwd");
        branch_result.expect("run branch");
        diff_result.expect("run diff");

        let branch_text = String::from_utf8(branch_output).expect("utf8");
        assert!(branch_text.contains("branch="));
        let diff_text = String::from_utf8(diff_output).expect("utf8");
        assert!(diff_text.contains("README.md"));
    }

    fn init_git_repo(path: &Path) {
        ProcessCommand::new("git")
            .args(["init", "--quiet", "-b", "main"])
            .current_dir(path)
            .status()
            .expect("git init");
    }
