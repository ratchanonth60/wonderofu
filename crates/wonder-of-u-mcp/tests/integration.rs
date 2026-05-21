use std::{collections::BTreeMap, time::Duration};

use serde_json::json;
use tokio::time::timeout;
use wonder_of_u_core::tool::Tool;
use wonder_of_u_mcp::{
    McpClient, McpClientIdentity, McpConfig, McpContent, McpServerConfig, discover_catalog_tools,
};

fn fake_server_path() -> String {
    env!("CARGO_BIN_EXE_fake_mcp_server").to_string()
}

fn fake_server_config(exit_after: Option<&str>) -> McpServerConfig {
    let mut env = BTreeMap::new();
    if let Some(exit_after) = exit_after {
        env.insert("WONDER_OF_U_FAKE_MCP_EXIT_AFTER".into(), exit_after.into());
    }
    McpServerConfig {
        name: "demo".into(),
        command: fake_server_path(),
        args: Vec::new(),
        env,
        enabled: true,
        cwd: None,
        protocol_version: None,
    }
}

fn fake_server_config_with_env(env: BTreeMap<String, String>) -> McpServerConfig {
    McpServerConfig {
        name: "demo".into(),
        command: fake_server_path(),
        args: Vec::new(),
        env,
        enabled: true,
        cwd: None,
        protocol_version: None,
    }
}

#[tokio::test]
async fn integration_mcp_fake_server_tool_call() {
    let mut client = McpClient::connect(
        &fake_server_config(None),
        &McpClientIdentity::default(),
        "2024-11-05",
    )
    .expect("connect fake server");

    assert_eq!(client.initialize_result().server_info.name, "fake-mcp");

    let tools = client.list_tools().expect("list tools");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "Echo Text");
    assert_eq!(
        tools[0]
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.read_only_hint),
        Some(true)
    );
    assert_eq!(
        tools[0].meta.as_ref().expect("meta")["origin"],
        json!("fake-server")
    );

    let result = client
        .call_tool("Echo Text", json!({ "text": "hello integration" }))
        .expect("call tool");
    assert!(!result.is_error);
    assert_eq!(
        result.content,
        vec![McpContent {
            kind: "text".into(),
            text: Some("echoed: hello integration".into()),
        }]
    );
}

#[tokio::test]
async fn integration_mcp_fake_server_resource_list_and_read() {
    let mut client = McpClient::connect(
        &fake_server_config(None),
        &McpClientIdentity::default(),
        "2024-11-05",
    )
    .expect("connect fake server");

    let resources = client.list_resources().expect("list resources");
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].uri, "file:///workspace/Cargo.toml");

    let result = client
        .read_resource(&resources[0].uri)
        .expect("read resource");
    assert_eq!(result.contents.len(), 1);
    assert_eq!(result.contents[0].uri, resources[0].uri);
    assert_eq!(result.contents[0].mime_type.as_deref(), Some("text/toml"));
    assert_eq!(
        result.contents[0].text.as_deref(),
        Some("[workspace]\nmembers = [\"crates/wonder-of-u-mcp\"]\n")
    );
}

#[tokio::test]
async fn integration_mcp_client_reconnects_after_server_restart() {
    let client = McpClient::connect(
        &fake_server_config(Some("initialized")),
        &McpClientIdentity::default(),
        "2024-11-05",
    )
    .expect("connect short-lived fake server");

    let disconnected = timeout(
        Duration::from_secs(1),
        tokio::task::spawn_blocking(move || {
            let mut client = client;
            client.list_tools()
        }),
    )
    .await
    .expect("disconnection should not hang")
    .expect("join blocking task")
    .expect_err("list_tools should fail after server exits");
    let message = disconnected.to_string();
    assert!(
        message.contains("unexpected EOF") || message.contains("Broken pipe"),
        "unexpected disconnect error: {message}"
    );

    let mut restarted = McpClient::connect(
        &fake_server_config(None),
        &McpClientIdentity::default(),
        "2024-11-05",
    )
    .expect("reconnect fake server");
    let tools = restarted.list_tools().expect("list tools after restart");
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Echo Text"]
    );
}

#[tokio::test]
async fn integration_mcp_rejects_oversized_message_before_allocation() {
    let config = fake_server_config_with_env(BTreeMap::from([(
        "WONDER_OF_U_FAKE_MCP_OVERSIZE_ON".into(),
        "tools/list".into(),
    )]));
    let mut client = McpClient::connect(&config, &McpClientIdentity::default(), "2024-11-05")
        .expect("connect fake server");

    let error = timeout(
        Duration::from_secs(1),
        tokio::task::spawn_blocking(move || client.list_tools()),
    )
    .await
    .expect("oversized response should not hang")
    .expect("join blocking task")
    .expect_err("oversized content length should fail");
    assert!(
        error.to_string().contains("exceeds maximum"),
        "unexpected error: {error}"
    );
}

// ── discover_catalog_tools + DynamicMcpTool ─────────────────────────────────

/// Build a [`McpConfig`] that points to the fake server binary.
fn fake_mcp_config() -> McpConfig {
    McpConfig {
        servers: vec![fake_server_config(None)],
        ..McpConfig::default()
    }
}

#[test]
fn discover_catalog_tools_returns_one_tool_from_fake_server() {
    let config = fake_mcp_config();
    let tools = discover_catalog_tools(&config);

    assert_eq!(tools.len(), 1, "expected exactly one catalog tool");
    let tool = &tools[0];
    // Qualified name must be namespaced.
    assert_eq!(
        tool.spec().name,
        "mcp__demo__echo_text",
        "qualified name should be namespaced with server prefix"
    );
    assert!(
        tool.spec().read_only,
        "readOnlyHint should propagate to ToolSpec"
    );
    assert_eq!(
        tool.spec().input_schema["x-mcp-search-like-hint"],
        json!(true)
    );
    assert_eq!(tool.server_name(), "demo");
}

#[test]
fn discover_catalog_tools_skips_unreachable_servers() {
    let mut config = fake_mcp_config();
    // Add a server that cannot be started.
    config.servers.push(McpServerConfig {
        name: "bad-server".into(),
        command: "/no/such/binary".into(),
        args: Vec::new(),
        env: BTreeMap::new(),
        enabled: true,
        cwd: None,
        protocol_version: None,
    });

    // Should still return the tool from the reachable server.
    let tools = discover_catalog_tools(&config);
    assert_eq!(
        tools.len(),
        1,
        "unreachable server should be silently skipped"
    );
}

#[test]
fn discover_catalog_tools_skips_disabled_servers() {
    let mut config = fake_mcp_config();
    config.servers[0].enabled = false;

    let tools = discover_catalog_tools(&config);
    assert!(
        tools.is_empty(),
        "disabled servers must not contribute tools"
    );
}

#[test]
fn discover_catalog_tools_empty_config_returns_nothing() {
    let tools = discover_catalog_tools(&McpConfig::default());
    assert!(tools.is_empty(), "empty config should return no tools");
}
