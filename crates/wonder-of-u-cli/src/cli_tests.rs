    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::{
        fs,
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        path::Path,
        process::Command as ProcessCommand,
        sync::{LazyLock, Mutex},
        thread,
    };

    use serde_json::{Value, json};
    use time::OffsetDateTime;
    use wonder_of_u_agent::SettingsStore;
    use wonder_of_u_core::{
        AppState, MessagePayload, SessionId, TaskState, TaskStatus, TokenUsage,
    };
    use wonder_of_u_plugins::{PluginConfig, PluginConfigStore, PluginTrustDecision};
    use wonder_of_u_storage::{
        CostStore, SessionCostLedger, SessionMetadata, TaskStore, TranscriptStore,
    };
    use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

    use super::*;

    static CWD_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn write_skill_fixture(dir: &Path, name: &str, command: &str) {
        fs::create_dir_all(dir).expect("create skill dir");
        fs::write(
            dir.join("skill.json"),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "name": name,
                "description": format!("{name} skill"),
                "prompt_path": "prompt.md",
                "allowed_tools": ["grep", "file_read"],
                "slash_command": command,
            }))
            .expect("skill manifest"),
        )
        .expect("write skill manifest");
        fs::write(dir.join("prompt.md"), format!("Prompt for {name}")).expect("write prompt");
    }

    fn seed_plugin_fixture(storage_dir: &Path) {
        let configured_root = storage_dir.join("seed-plugins");
        let plugin_root = configured_root.join("demo-plugin");
        fs::create_dir_all(plugin_root.join("commands")).expect("commands dir");
        write_skill_fixture(
            &plugin_root.join("skills/release-check"),
            "release-check",
            "release-check",
        );
        fs::write(plugin_root.join("commands/run.txt"), "echo run").expect("command file");
        fs::write(
            plugin_root.join("plugin.json"),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "name": "demo-plugin",
                "version": "0.1.0",
                "description": "demo plugin",
                "commands": [{
                    "name": "demo-plugin-run",
                    "description": "Run plugin",
                    "path": "commands/run.txt"
                }],
                "skills": [{
                    "path": "skills/release-check"
                }]
            }))
            .expect("plugin manifest"),
        )
        .expect("write plugin manifest");

        let config = PluginConfig {
            additional_plugin_dirs: vec![configured_root],
            ..PluginConfig::default()
        };
        PluginConfigStore::new(storage_dir)
            .write(&config)
            .expect("write plugin config");
    }

    fn seed_runnable_plugin_fixture(
        storage_dir: &Path,
        plugin_name: &str,
        commands: Value,
        trust: Option<PluginTrustDecision>,
    ) -> std::path::PathBuf {
        let configured_root = storage_dir.join(format!("{plugin_name}-plugins"));
        let plugin_root = configured_root.join(plugin_name);
        fs::create_dir_all(plugin_root.join("commands")).expect("commands dir");
        fs::write(
            plugin_root.join("plugin.json"),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "name": plugin_name,
                "version": "0.1.0",
                "description": format!("{plugin_name} plugin"),
                "commands": commands,
            }))
            .expect("plugin manifest"),
        )
        .expect("write plugin manifest");

        let mut config = PluginConfig {
            additional_plugin_dirs: vec![configured_root],
            ..PluginConfig::default()
        };
        if let Some(trust) = trust {
            config
                .set_trust(plugin_name, trust)
                .expect("plugin trust entry");
        }
        PluginConfigStore::new(storage_dir)
            .write(&config)
            .expect("write plugin config");
        plugin_root
    }

    fn extract_value<'a>(text: &'a str, prefix: &str) -> &'a str {
        text.lines()
            .find_map(|line| line.strip_prefix(prefix))
            .unwrap_or_else(|| panic!("missing prefix {prefix} in {text}"))
    }

    fn write_executable_script(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}")).expect("write script");
        #[cfg(unix)]
        {
            let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).expect("set script permissions");
        }
        path
    }

    fn read_http_request_parts(stream: &mut TcpStream) -> (String, Vec<u8>) {
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
        (
            headers,
            buffer[header_end..header_end + content_length].to_vec(),
        )
    }

    fn read_http_request(stream: &mut TcpStream) -> (String, Value) {
        let (headers, body) = read_http_request_parts(stream);
        let body = serde_json::from_slice(&body).expect("body json");
        (headers, body)
    }

    fn spawn_json_server(
        assert_request: impl FnOnce(String, Value) + Send + 'static,
        response_body: Value,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("server address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let (headers, body) = read_http_request(&mut stream);
            assert_request(headers, body);
            let response_body = serde_json::to_string(&response_body).expect("serialize response");
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
        });
        (format!("http://{address}/v1"), handle)
    }

    fn spawn_json_server_sequence(
        mut assert_request: impl FnMut(usize, String, Value) + Send + 'static,
        response_bodies: Vec<Value>,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("server address");
        let handle = thread::spawn(move || {
            for (index, response_body) in response_bodies.into_iter().enumerate() {
                let (mut stream, _) = listener.accept().expect("accept request");
                let (headers, body) = read_http_request(&mut stream);
                assert_request(index, headers, body);
                let response_body =
                    serde_json::to_string(&response_body).expect("serialize response");
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

    include!("cli_test_chunks/chunk_0.rs");
    include!("cli_test_chunks/chunk_1.rs");
    include!("cli_test_chunks/chunk_2.rs");
    include!("cli_test_chunks/chunk_3.rs");
