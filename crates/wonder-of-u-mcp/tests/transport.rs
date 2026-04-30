use std::collections::BTreeMap;

use wonder_of_u_core::ToolSource;
use wonder_of_u_mcp::{
    McpClient, McpClientIdentity, McpConfig, McpServerConfig, McpServerState, McpStatusReport,
};

fn fake_server_path() -> String {
    env!("CARGO_BIN_EXE_fake_mcp_server").to_string()
}

fn fake_server_config() -> McpServerConfig {
    McpServerConfig {
        name: "demo".into(),
        command: fake_server_path(),
        args: Vec::new(),
        env: BTreeMap::new(),
        enabled: true,
        cwd: None,
        protocol_version: None,
    }
}

fn failing_server_config() -> McpServerConfig {
    McpServerConfig {
        name: "broken".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "exit 1".into()],
        env: BTreeMap::new(),
        enabled: true,
        cwd: None,
        protocol_version: None,
    }
}

#[test]
fn client_initializes_and_discovers_catalog() {
    let mut client = McpClient::connect(
        &fake_server_config(),
        &McpClientIdentity::default(),
        "2024-11-05",
    )
    .expect("connect fake server");

    assert_eq!(client.initialize_result().server_info.name, "fake-mcp");
    let catalog = client.discover_catalog().expect("discover catalog");
    assert_eq!(catalog.tools.len(), 1);
    assert_eq!(catalog.resources.len(), 1);
    let spec = catalog.tool_specs().pop().expect("tool spec");
    assert_eq!(spec.name, "mcp__demo__echo_text");
    assert_eq!(spec.source, ToolSource::Mcp);
    client.shutdown().expect("shutdown fake server");
}

#[test]
fn status_report_marks_connected_server_ready() {
    let config = McpConfig {
        servers: vec![fake_server_config()],
        ..McpConfig::default()
    };

    let report = McpStatusReport::inspect("mcp-config.json".into(), &config);
    assert_eq!(report.ready_count(), 1);
    assert_eq!(report.error_count(), 0);
    let server = &report.servers[0];
    assert_eq!(server.state, Some(McpServerState::Ready));
    assert_eq!(server.tool_count(), 1);
    assert_eq!(server.resource_count(), 1);
    assert_eq!(server.capability_labels(), vec!["tools", "resources"]);
}

#[test]
fn mcp_server_init_failure_does_not_block_other_tools() {
    let config = McpConfig {
        servers: vec![failing_server_config(), fake_server_config()],
        ..McpConfig::default()
    };

    let report = McpStatusReport::inspect("mcp-config.json".into(), &config);

    assert_eq!(report.ready_count(), 1);
    assert_eq!(report.error_count(), 1);

    let broken = report
        .servers
        .iter()
        .find(|server| server.name == "broken")
        .expect("broken server");
    assert_eq!(broken.state, Some(McpServerState::Error));
    assert!(broken.catalog.tools.is_empty());
    assert!(broken.error.is_some());

    let ready = report
        .servers
        .iter()
        .find(|server| server.name == "demo")
        .expect("ready server");
    assert_eq!(ready.state, Some(McpServerState::Ready));
    assert_eq!(ready.tool_count(), 1);
    assert_eq!(ready.catalog.tool_specs()[0].name, "mcp__demo__echo_text");
}
