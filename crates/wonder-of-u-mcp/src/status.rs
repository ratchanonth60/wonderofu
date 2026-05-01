use std::path::PathBuf;

use crate::{McpCatalog, McpClient, McpConfig, McpServerConfig, ServerCapabilities, ServerInfo};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpServerState {
    Disabled,
    Ready,
    Error,
}

impl McpServerState {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Ready => "ready",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct McpServerStatus {
    pub name: String,
    pub enabled: bool,
    pub state: Option<McpServerState>,
    pub command_line: String,
    pub protocol_version: Option<String>,
    pub server_info: Option<ServerInfo>,
    pub capabilities: ServerCapabilities,
    pub catalog: McpCatalog,
    pub error: Option<String>,
}

impl McpServerStatus {
    #[must_use]
    pub fn tool_count(&self) -> usize {
        self.catalog.tools.len()
    }

    #[must_use]
    pub fn resource_count(&self) -> usize {
        self.catalog.resources.len()
    }

    #[must_use]
    pub fn capability_labels(&self) -> Vec<&'static str> {
        self.capabilities.labels()
    }

    fn disabled(server: &McpServerConfig) -> Self {
        Self {
            name: server.name.clone(),
            enabled: false,
            state: Some(McpServerState::Disabled),
            command_line: server.command_line(),
            protocol_version: None,
            server_info: None,
            capabilities: ServerCapabilities::default(),
            catalog: McpCatalog::default(),
            error: None,
        }
    }

    fn ready(
        server: &McpServerConfig,
        initialize: crate::InitializeResult,
        catalog: McpCatalog,
    ) -> Self {
        Self {
            name: server.name.clone(),
            enabled: true,
            state: Some(McpServerState::Ready),
            command_line: server.command_line(),
            protocol_version: Some(initialize.protocol_version),
            server_info: Some(initialize.server_info),
            capabilities: initialize.capabilities,
            catalog,
            error: None,
        }
    }

    fn error(server: &McpServerConfig, error: impl Into<String>) -> Self {
        Self {
            name: server.name.clone(),
            enabled: server.enabled,
            state: Some(McpServerState::Error),
            command_line: server.command_line(),
            protocol_version: None,
            server_info: None,
            capabilities: ServerCapabilities::default(),
            catalog: McpCatalog::default(),
            error: Some(error.into()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct McpStatusReport {
    pub config_path: PathBuf,
    pub servers: Vec<McpServerStatus>,
}

impl McpStatusReport {
    #[must_use]
    pub fn inspect(config_path: PathBuf, config: &McpConfig) -> Self {
        let servers = config
            .servers
            .iter()
            .map(|server| inspect_server(config, server))
            .collect();
        Self {
            config_path,
            servers,
        }
    }

    #[must_use]
    pub fn ready_count(&self) -> usize {
        self.servers
            .iter()
            .filter(|server| server.state == Some(McpServerState::Ready))
            .count()
    }

    #[must_use]
    pub fn error_count(&self) -> usize {
        self.servers
            .iter()
            .filter(|server| server.state == Some(McpServerState::Error))
            .count()
    }
}

fn inspect_server(config: &McpConfig, server: &McpServerConfig) -> McpServerStatus {
    if !server.enabled {
        return McpServerStatus::disabled(server);
    }

    match McpClient::discover_server(server, &config.client, &config.protocol_version) {
        Ok((initialize, catalog)) => McpServerStatus::ready(server, initialize, catalog),
        Err(error) => McpServerStatus::error(server, error.to_string()),
    }
}
