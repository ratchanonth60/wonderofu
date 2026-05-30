    #[test]
    fn login_and_model_commands_drive_ready_status() {
        let _anthropic = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
        let _gemini = EnvVarGuard::set("GEMINI_API_KEY", "");
        let dir = unique_test_dir("cli-provider");
        let storage_dir = dir.to_string_lossy().into_owned();

        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "model".to_string(),
                "set".to_string(),
                "--provider".to_string(),
                "openai".to_string(),
                "--model".to_string(),
                "gpt-4.1".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("set model");
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "login".to_string(),
                "--provider".to_string(),
                "openai".to_string(),
                "--api-key".to_string(),
                "secret".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("login");

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
        assert!(text.contains("provider_selection=openai:gpt-4.1"));
        assert!(text.contains("provider_readiness=ready"));
        assert!(text.contains("auth_status=ready"));
    }

    #[test]
    fn prompt_command_executes_openai_and_persists_session() {
        let _anthropic = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
        let _openai = EnvVarGuard::set("OPENAI_API_KEY", "");
        let _gemini = EnvVarGuard::set("GEMINI_API_KEY", "");
        let dir = unique_test_dir("cli-prompt");
        let storage_dir = dir.to_string_lossy().into_owned();
        let (api_base, server) = spawn_json_server(
            |headers, body| {
                let headers = headers.to_ascii_lowercase();
                assert!(headers.contains("post /v1/chat/completions http/1.1"));
                assert!(headers.contains("authorization: bearer test-key"));
                assert_eq!(
                    body.pointer("/messages/0/content").and_then(Value::as_str),
                    Some("Be brief")
                );
                assert_eq!(
                    body.pointer("/messages/1/content").and_then(Value::as_str),
                    Some("Hello runtime")
                );
                assert_eq!(
                    body.pointer("/max_completion_tokens")
                        .and_then(Value::as_u64),
                    Some(32)
                );
            },
            json!({
                "choices": [{
                    "finish_reason": "stop",
                    "message": { "content": "Runtime reply" }
                }],
                "usage": {
                    "prompt_tokens": 9,
                    "completion_tokens": 4
                }
            }),
        );

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
                api_base,
            ],
            &mut Vec::new(),
        )
        .expect("set prompt api base");
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "login".to_string(),
                "--provider".to_string(),
                "openai".to_string(),
                "--api-key".to_string(),
                "test-key".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("login for prompt");

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "prompt".to_string(),
                "--system".to_string(),
                "Be brief".to_string(),
                "--max-output-tokens".to_string(),
                "32".to_string(),
                "Hello runtime".to_string(),
            ],
            &mut output,
        )
        .expect("run prompt");
        server.join().expect("prompt server finished");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("Runtime reply"));
        assert!(text.contains("provider_selection=openai:gpt-5.5"));
        assert!(text.contains("persisted=true"));
        assert!(text.contains("total_tokens=13"));

        let session_id = SessionId::parse(extract_value(&text, "session_id=")).expect("session id");
        let restored = TranscriptStore::new(&dir)
            .restore_session(session_id)
            .expect("restore prompt session");
        assert_eq!(restored.transcript.messages.len(), 2);
        assert!(matches!(
            &restored.transcript.messages[0].payload,
            MessagePayload::UserText { content } if content == "Hello runtime"
        ));
        assert!(matches!(
            &restored.transcript.messages[1].payload,
            MessagePayload::AssistantText { content } if content == "Runtime reply"
        ));

        let costs = CostStore::new(&dir)
            .read_costs(session_id)
            .expect("read prompt costs");
        assert_eq!(costs.costs.usage.input_tokens, 9);
        assert_eq!(costs.costs.usage.output_tokens, 4);
    }

    #[test]
    fn prompt_command_generates_local_suggestions_without_extra_network_requests() {
        let _anthropic = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
        let _openai = EnvVarGuard::set("OPENAI_API_KEY", "");
        let _gemini = EnvVarGuard::set("GEMINI_API_KEY", "");
        let dir = unique_test_dir("cli-prompt-suggestion");
        let storage_dir = dir.to_string_lossy().into_owned();
        let (api_base, server) = spawn_json_server(
            |headers, body| {
                let headers = headers.to_ascii_lowercase();
                assert!(headers.contains("post /v1/chat/completions http/1.1"));
                assert!(headers.contains("authorization: bearer suggest-key"));
                assert_eq!(
                    body.pointer("/messages/0/content").and_then(Value::as_str),
                    Some("Fix the bug in the failing tests")
                );
            },
            json!({
                "choices": [{
                    "finish_reason": "stop",
                    "message": {
                        "content": "I fixed the bug and updated the tests, but I haven't run them yet."
                    }
                }],
                "usage": {
                    "prompt_tokens": 11,
                    "completion_tokens": 8
                }
            }),
        );

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
                api_base,
            ],
            &mut Vec::new(),
        )
        .expect("set prompt suggestion api base");
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "login".to_string(),
                "--provider".to_string(),
                "openai".to_string(),
                "--api-key".to_string(),
                "suggest-key".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("login for prompt suggestion");

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                dir.to_string_lossy().into_owned(),
                "prompt".to_string(),
                "Fix the bug in the failing tests".to_string(),
            ],
            &mut output,
        )
        .expect("run prompt with local suggestion");
        server.join().expect("prompt suggestion server finished");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("prompt_suggestion=run the tests"));
        assert!(text.contains("prompt_suggestion_kind=verify"));
        assert!(text.contains("query_phase=completed"));
        assert!(text.contains("coordinator_mode=direct"));
        assert!(text.contains("coordinator_cloud_queries=unsupported"));
        assert!(text.contains("coordinator_external_backend=deferred"));
    }

    #[test]
    fn prompt_command_runs_tool_loop_and_persists_tool_messages() {
        let _anthropic = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
        let _openai = EnvVarGuard::set("OPENAI_API_KEY", "");
        let _gemini = EnvVarGuard::set("GEMINI_API_KEY", "");
        let dir = unique_test_dir("cli-prompt-tool-loop");
        let storage_dir = dir.to_string_lossy().into_owned();
        let (api_base, server) = spawn_json_server_sequence(
            |index, headers, body| {
                let headers = headers.to_ascii_lowercase();
                assert!(headers.contains("post /v1/chat/completions http/1.1"));
                assert!(headers.contains("authorization: bearer tool-key"));
                match index {
                    0 => {
                        assert_eq!(
                            body.pointer("/messages/0/content").and_then(Value::as_str),
                            Some("Inspect Cargo metadata")
                        );
                        assert_eq!(
                            body.pointer("/tool_choice").and_then(Value::as_str),
                            Some("auto")
                        );
                        let tools = body
                            .get("tools")
                            .and_then(Value::as_array)
                            .expect("tools array");
                        assert!(tools.iter().any(|tool| {
                            tool.pointer("/function/name").and_then(Value::as_str) == Some("glob")
                        }));
                    }
                    1 => {
                        assert_eq!(
                            body.pointer("/messages/0/content").and_then(Value::as_str),
                            Some("Inspect Cargo metadata")
                        );
                        assert_eq!(
                            body.pointer("/messages/1/role").and_then(Value::as_str),
                            Some("assistant")
                        );
                        assert_eq!(
                            body.pointer("/messages/1/tool_calls/0/function/name")
                                .and_then(Value::as_str),
                            Some("glob")
                        );
                        assert_eq!(
                            body.pointer("/messages/2/role").and_then(Value::as_str),
                            Some("tool")
                        );
                        assert!(
                            body.pointer("/messages/2/content")
                                .and_then(Value::as_str)
                                .is_some_and(|content| content.contains("Cargo.toml"))
                        );
                    }
                    other => panic!("unexpected request index {other}"),
                }
            },
            vec![
                json!({
                    "choices": [{
                        "finish_reason": "tool_calls",
                        "message": {
                            "content": "Checking workspace metadata.",
                            "tool_calls": [{
                                "id": "call_glob_1",
                                "type": "function",
                                "function": {
                                    "name": "glob",
                                    "arguments": "{\"pattern\":\"Cargo.toml\",\"path\":\".\"}"
                                }
                            }]
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 2
                    }
                }),
                json!({
                    "choices": [{
                        "finish_reason": "stop",
                        "message": { "content": "Tool loop reply" }
                    }],
                    "usage": {
                        "prompt_tokens": 7,
                        "completion_tokens": 5
                    }
                }),
            ],
        );

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
                api_base,
            ],
            &mut Vec::new(),
        )
        .expect("set prompt tool-loop api base");
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "login".to_string(),
                "--provider".to_string(),
                "openai".to_string(),
                "--api-key".to_string(),
                "tool-key".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("login for prompt tool loop");

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "prompt".to_string(),
                "--tools".to_string(),
                "Inspect Cargo metadata".to_string(),
            ],
            &mut output,
        )
        .expect("run prompt tool loop");
        server.join().expect("tool loop server finished");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("Tool loop reply"));
        assert!(text.contains("tool_use_requested=true"));
        assert!(text.contains("tool_calls=1"));

        let session_id = SessionId::parse(extract_value(&text, "session_id=")).expect("session id");
        let restored = TranscriptStore::new(&dir)
            .restore_session(session_id)
            .expect("restore prompt tool-loop session");
        assert_eq!(restored.transcript.messages.len(), 5);
        assert!(matches!(
            &restored.transcript.messages[0].payload,
            MessagePayload::UserText { content } if content == "Inspect Cargo metadata"
        ));
        assert!(matches!(
            &restored.transcript.messages[1].payload,
            MessagePayload::AssistantText { content } if content == "Checking workspace metadata."
        ));
        assert!(matches!(
            &restored.transcript.messages[2].payload,
            MessagePayload::AssistantToolUse { tool, .. } if tool == "glob"
        ));
        assert!(matches!(
            &restored.transcript.messages[3].payload,
            MessagePayload::ToolResult { tool, success, content, .. }
                if tool == "glob" && *success && content.contains("Cargo.toml")
        ));
        assert!(matches!(
            &restored.transcript.messages[4].payload,
            MessagePayload::AssistantText { content } if content == "Tool loop reply"
        ));

        let costs = CostStore::new(&dir)
            .read_costs(session_id)
            .expect("read prompt tool-loop costs");
        assert_eq!(costs.costs.usage.input_tokens, 17);
        assert_eq!(costs.costs.usage.output_tokens, 7);
    }

    #[test]
    fn skills_run_executes_bundled_skill_with_prompt_persistence() {
        let _anthropic = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
        let _openai = EnvVarGuard::set("OPENAI_API_KEY", "");
        let _gemini = EnvVarGuard::set("GEMINI_API_KEY", "");
        let dir = unique_test_dir("cli-skills-run");
        let storage_dir = dir.to_string_lossy().into_owned();
        let (api_base, server) = spawn_json_server(
            |headers, body| {
                let headers = headers.to_ascii_lowercase();
                assert!(headers.contains("post /v1/chat/completions http/1.1"));
                assert!(headers.contains("authorization: bearer skill-key"));
                assert_eq!(
                    body.pointer("/messages/0/content").and_then(Value::as_str),
                    Some("You are terse")
                );
                let prompt = body
                    .pointer("/messages/1/content")
                    .and_then(Value::as_str)
                    .expect("skill prompt");
                assert!(prompt.contains("Skill: workspace-audit"));
                assert!(prompt.contains("Skill instructions:"));
                assert!(prompt.contains("Inspect the current repository as a Rust workspace."));
                assert!(prompt.contains("User request:\nSummarize the repository quickly"));
            },
            json!({
                "choices": [{
                    "finish_reason": "stop",
                    "message": { "content": "Skill runtime reply" }
                }],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": 6
                }
            }),
        );

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
                api_base,
            ],
            &mut Vec::new(),
        )
        .expect("set skills api base");
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "login".to_string(),
                "--provider".to_string(),
                "openai".to_string(),
                "--api-key".to_string(),
                "skill-key".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("login for skills run");

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "skills".to_string(),
                "run".to_string(),
                "workspace-audit".to_string(),
                "--system".to_string(),
                "You are terse".to_string(),
                "--max-output-tokens".to_string(),
                "40".to_string(),
                "Summarize the repository quickly".to_string(),
            ],
            &mut output,
        )
        .expect("run skill");
        server.join().expect("skills server finished");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("Skill runtime reply"));
        assert!(text.contains("skill=workspace-audit"));
        assert!(text.contains("allowed_tools=glob,grep,file_read"));
        assert!(text.contains("provider_selection=openai:gpt-5.5"));
        assert!(text.contains("persisted=true"));

        let session_id = SessionId::parse(extract_value(&text, "session_id=")).expect("session id");
        let restored = TranscriptStore::new(&dir)
            .restore_session(session_id)
            .expect("restore skills session");
        assert_eq!(restored.transcript.messages.len(), 2);
        assert!(matches!(
            &restored.transcript.messages[0].payload,
            MessagePayload::UserText { content }
                if content.contains("Skill: workspace-audit")
                    && content.contains("User request:\nSummarize the repository quickly")
        ));
        assert!(matches!(
            &restored.transcript.messages[1].payload,
            MessagePayload::AssistantText { content } if content == "Skill runtime reply"
        ));
    }

    #[test]
    fn skills_run_tools_restricts_tool_registry_to_manifest_allowed_tools() {
        let _anthropic = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
        let _openai = EnvVarGuard::set("OPENAI_API_KEY", "");
        let _gemini = EnvVarGuard::set("GEMINI_API_KEY", "");
        let dir = unique_test_dir("cli-skills-run-tools");
        let storage_dir = dir.to_string_lossy().into_owned();
        let (api_base, server) = spawn_json_server_sequence(
            |index, headers, body| {
                let headers = headers.to_ascii_lowercase();
                assert!(headers.contains("post /v1/chat/completions http/1.1"));
                assert!(headers.contains("authorization: bearer skill-tools-key"));
                match index {
                    0 => {
                        let prompt = body
                            .pointer("/messages/0/content")
                            .and_then(Value::as_str)
                            .expect("skill prompt");
                        assert!(prompt.contains("Skill: workspace-audit"));
                        assert!(prompt.contains(
                            "Tool execution mode: manifest-restricted automatic tool execution enabled"
                        ));
                        let tools = body
                            .get("tools")
                            .and_then(Value::as_array)
                            .expect("tools array");
                        assert!(tools.iter().any(|tool| {
                            tool.pointer("/function/name").and_then(Value::as_str) == Some("glob")
                        }));
                        assert!(!tools.iter().any(|tool| {
                            tool.pointer("/function/name").and_then(Value::as_str) == Some("bash")
                        }));
                    }
                    1 => {
                        assert_eq!(
                            body.pointer("/messages/1/role").and_then(Value::as_str),
                            Some("assistant")
                        );
                        assert_eq!(
                            body.pointer("/messages/1/tool_calls/0/function/name")
                                .and_then(Value::as_str),
                            Some("glob")
                        );
                        assert_eq!(
                            body.pointer("/messages/2/role").and_then(Value::as_str),
                            Some("tool")
                        );
                    }
                    other => panic!("unexpected request index {other}"),
                }
            },
            vec![
                json!({
                    "choices": [{
                        "finish_reason": "tool_calls",
                        "message": {
                            "content": "Inspecting allowed files.",
                            "tool_calls": [{
                                "id": "skill_call_1",
                                "type": "function",
                                "function": {
                                    "name": "glob",
                                    "arguments": "{\"pattern\":\"Cargo.toml\",\"path\":\".\"}"
                                }
                            }]
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 14,
                        "completion_tokens": 3
                    }
                }),
                json!({
                    "choices": [{
                        "finish_reason": "stop",
                        "message": { "content": "Skill tool reply" }
                    }],
                    "usage": {
                        "prompt_tokens": 8,
                        "completion_tokens": 4
                    }
                }),
            ],
        );

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
                api_base,
            ],
            &mut Vec::new(),
        )
        .expect("set skills tools api base");
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "login".to_string(),
                "--provider".to_string(),
                "openai".to_string(),
                "--api-key".to_string(),
                "skill-tools-key".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("login for skills tools");

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "skills".to_string(),
                "run".to_string(),
                "workspace-audit".to_string(),
                "--tools".to_string(),
                "Summarize the workspace metadata".to_string(),
            ],
            &mut output,
        )
        .expect("run skill with tools");
        server.join().expect("skills tools server finished");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("Skill tool reply"));
        assert!(text.contains("tool_use_requested=true"));
        assert!(text.contains("tool_calls=1"));
        assert!(text.contains("allowed_tools=glob,grep,file_read"));
        assert!(text.contains("note=skill execution used the prompt tool loop"));
    }

    #[test]
    fn plugin_and_skills_commands_surface_catalog_metadata() {
        let dir = unique_test_dir("cli-plugin-skills");
        let storage_dir = dir.to_string_lossy().into_owned();
        seed_plugin_fixture(&dir);

        let mut plugin_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "plugin".to_string(),
                "status".to_string(),
            ],
            &mut plugin_output,
        )
        .expect("plugin status");
        let plugin_text = String::from_utf8(plugin_output).expect("utf8");
        assert!(plugin_text.contains("plugin[0].id=demo-plugin"));
        assert!(plugin_text.contains("plugin[0].readiness=needs_trust"));

        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "plugin".to_string(),
                "trust".to_string(),
                "demo-plugin".to_string(),
                "--state".to_string(),
                "trusted".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("trust plugin");

        let mut skills_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "skills".to_string(),
                "list".to_string(),
            ],
            &mut skills_output,
        )
        .expect("skills list");
        let skills_text = String::from_utf8(skills_output).expect("utf8");
        assert!(skills_text.contains("skill[0].name=workspace-audit"));
        assert!(skills_text.contains("release-check"));

        let mut reload_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "reload-plugins".to_string(),
            ],
            &mut reload_output,
        )
        .expect("reload plugins");
        let reload_text = String::from_utf8(reload_output).expect("utf8");
        assert!(reload_text.contains("plugins=1"));
        assert!(reload_text.contains("skills=15"));

        let mut slash_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "slash".to_string(),
                "/skills".to_string(),
                "show".to_string(),
                "release-check".to_string(),
            ],
            &mut slash_output,
        )
        .expect("slash skills show");
        let slash_text = String::from_utf8(slash_output).expect("utf8");
        assert!(slash_text.contains("skill=release-check"));
        assert!(slash_text.contains("source=plugin:demo-plugin"));
    }

    #[test]
    fn plugin_install_validate_and_list_accept_upstream_plugin_json() {
        let dir = unique_test_dir("cli-plugin-install-upstream");
        let storage_dir = dir.join("storage");
        let storage_arg = storage_dir.to_string_lossy().into_owned();
        let plugin_root = dir.join("upstream-plugin");
        fs::create_dir_all(plugin_root.join("commands")).expect("commands dir");
        fs::write(plugin_root.join("commands/run.txt"), "echo run").expect("command file");
        fs::write(
            plugin_root.join("plugin.json"),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "name": "upstream-plugin",
                "version": "0.1.0",
                "description": "upstream plugin",
                "commands": [{
                    "name": "upstream-run",
                    "description": "Run plugin",
                    "path": "commands/run.txt"
                }]
            }))
            .expect("plugin manifest"),
        )
        .expect("write plugin manifest");

        let mut validate_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "plugin".to_string(),
                "validate".to_string(),
                plugin_root.to_string_lossy().into_owned(),
            ],
            &mut validate_output,
        )
        .expect("validate plugin");
        let validate_text = String::from_utf8(validate_output).expect("utf8");
        assert!(validate_text.contains("valid=true"));
        assert!(validate_text.contains("path="));
        assert!(validate_text.contains("plugin.json"));
        assert!(validate_text.contains("name=upstream-plugin"));

        let mut install_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_arg.clone(),
                "plugin".to_string(),
                "install".to_string(),
                plugin_root.to_string_lossy().into_owned(),
            ],
            &mut install_output,
        )
        .expect("install plugin");
        let install_text = String::from_utf8(install_output).expect("utf8");
        assert!(install_text.contains("installed=true"));
        assert!(install_text.contains(&format!("path={}", plugin_root.display())));

        let mut list_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_arg,
                "plugin".to_string(),
                "list".to_string(),
            ],
            &mut list_output,
        )
        .expect("list plugins");
        let list_text = String::from_utf8(list_output).expect("utf8");
        assert!(list_text.contains("plugins=1"));
        assert!(list_text.contains("plugin[0]=upstream-plugin"));
        assert!(list_text.contains("source=configured"));
    }

    #[test]
    fn plugin_run_executes_trusted_registered_command() {
        let dir = unique_test_dir("cli-plugin-run");
        let storage_dir = dir.to_string_lossy().into_owned();
        let plugin_root = seed_runnable_plugin_fixture(
            &dir,
            "shell-plugin",
            json!([{
                "name": "echo",
                "aliases": ["run"],
                "description": "Echo arguments",
                "path": "commands/echo.sh"
            }]),
            Some(PluginTrustDecision::Trusted),
        );
        write_executable_script(
            &plugin_root.join("commands"),
            "echo.sh",
            "printf 'stdout args: %s\\n' \"$*\"\nprintf 'stderr args: %s\\n' \"$*\" >&2\n",
        );

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "plugin".to_string(),
                "run".to_string(),
                "shell-plugin".to_string(),
                "/run".to_string(),
                "--flag".to_string(),
                "value".to_string(),
            ],
            &mut output,
        )
        .expect("plugin run");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("plugin=shell-plugin"));
        assert!(text.contains("command=echo"));
        assert!(text.contains("success=true"));
        assert!(text.contains("exit_status=0"));
        assert!(text.contains("stdout args: --flag value"));
        assert!(text.contains("stderr args: --flag value"));
    }

    #[test]
    fn plugin_run_rejects_untrusted_plugins() {
        let dir = unique_test_dir("cli-plugin-run-untrusted");
        let storage_dir = dir.to_string_lossy().into_owned();
        let plugin_root = seed_runnable_plugin_fixture(
            &dir,
            "needs-trust-plugin",
            json!([{
                "name": "echo",
                "description": "Echo arguments",
                "path": "commands/echo.sh"
            }]),
            None,
        );
        write_executable_script(
            &plugin_root.join("commands"),
            "echo.sh",
            "printf 'should not run\\n'\n",
        );

        let error = run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "plugin".to_string(),
                "run".to_string(),
                "needs-trust-plugin".to_string(),
                "echo".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect_err("untrusted plugin should fail");
        assert!(error.to_string().contains("not trusted"));
    }

    #[test]
    fn plugin_run_rejects_interactive_and_auth_only_commands() {
        let dir = unique_test_dir("cli-plugin-run-constraints");
        let storage_dir = dir.to_string_lossy().into_owned();
        let _openai = EnvVarGuard::set("OPENAI_API_KEY", "");
        let plugin_root = seed_runnable_plugin_fixture(
            &dir,
            "restricted-plugin",
            json!([
                {
                    "name": "interactive",
                    "description": "Interactive command",
                    "path": "commands/interactive.sh",
                    "interactive_only": true
                },
                {
                    "name": "secure",
                    "description": "Auth command",
                    "path": "commands/secure.sh",
                    "requires_auth": true
                }
            ]),
            Some(PluginTrustDecision::Trusted),
        );
        write_executable_script(
            &plugin_root.join("commands"),
            "interactive.sh",
            "printf 'interactive\\n'\n",
        );
        write_executable_script(
            &plugin_root.join("commands"),
            "secure.sh",
            "printf 'secure\\n'\n",
        );
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "model".to_string(),
                "set".to_string(),
                "--provider".to_string(),
                "openai".to_string(),
                "--model".to_string(),
                "gpt-4.1".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("select openai provider");

        let interactive_error = run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "plugin".to_string(),
                "run".to_string(),
                "restricted-plugin".to_string(),
                "interactive".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect_err("interactive plugin command should fail");
        assert!(interactive_error.to_string().contains("interactive-only"));

        let auth_error = run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "plugin".to_string(),
                "run".to_string(),
                "restricted-plugin".to_string(),
                "secure".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect_err("auth plugin command should fail");
        assert!(auth_error.to_string().contains("requires provider auth"));
    }

    #[test]
    fn slash_transport_executes_registry_commands() {
        let mut output = Vec::new();
        run_from(["wonder-of-u", "slash", "/help", "status"], &mut output).expect("run slash help");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("Show runtime, storage, MCP, plugin, skill, and task status"));
    }

    #[test]
    fn registry_includes_commit_and_commit_push_pr_commands() {
        let mut registry = wonder_of_u_core::CommandRegistry::new();
        registry
            .register(std::sync::Arc::new(
                commands::review_workflow::CommitCommand::new(),
            ))
            .expect("register commit");
        registry
            .register(std::sync::Arc::new(
                commands::review_workflow::CommitPushPrCommand::new(),
            ))
            .expect("register commit-push-pr");

        let commit = registry.resolve_spec("commit").expect("commit spec");
        assert_eq!(commit.name, "commit");
        let commit_push_pr = registry
            .resolve_spec("commit-push-pr")
            .expect("commit-push-pr spec");
        assert_eq!(commit_push_pr.name, "commit-push-pr");
    }

    #[test]
    fn slash_transport_preserves_commit_prompt_arguments() {
        let invocation = invocation_from_slash_tokens(vec![
            "/commit".into(),
            "use".into(),
            "the README change".into(),
        ])
        .expect("slash invocation");

        assert_eq!(invocation.name, "commit");
        assert_eq!(invocation.args, "use 'the README change'");
        assert_eq!(invocation.raw, "/commit use 'the README change'");
    }

    #[test]
    fn slash_transport_executes_dynamic_plugin_commands() {
        let dir = unique_test_dir("cli-slash-plugin-command");
        let storage_dir = dir.to_string_lossy().into_owned();
        let plugin_root = seed_runnable_plugin_fixture(
            &dir,
            "slash-plugin",
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
            "printf 'plugin slash args: %s\\n' \"$*\"\n",
        );

        let mut output = Vec::new();
        run_from(
            [
                "wonder-of-u",
                "--storage-dir",
                storage_dir.as_str(),
                "slash",
                "/echo",
                "--flag",
                "value",
            ],
            &mut output,
        )
        .expect("run slash plugin command");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("plugin=slash-plugin"));
        assert!(text.contains("command=echo"));
        assert!(text.contains("plugin slash args: --flag value"));
    }

