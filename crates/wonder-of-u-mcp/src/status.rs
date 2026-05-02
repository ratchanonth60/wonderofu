use std::path::PathBuf;

use crate::{McpCatalog, McpClient, McpConfig, McpServerConfig, ServerCapabilities, ServerInfo};
/// Enumerates mcp server state
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpServerState {
    /// Represents disabled
    Disabled,
    /// Represents ready
    Ready,
    /// Represents error
    Error,
}

impl McpServerState {
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Ready => "ready",
            Self::Error => "error",
        }
    }
}
/// Represents mcp server status
#[derive(Clone, Debug, Default, PartialEq)]
pub struct McpServerStatus {
    /// Stores the name
    pub name: String,
    /// Stores the enabled
    pub enabled: bool,
    /// Stores the state
    pub state: Option<McpServerState>,
    /// Stores the command line
    pub command_line: String,
    /// Stores the protocol version
    pub protocol_version: Option<String>,
    /// Stores the server info
    pub server_info: Option<ServerInfo>,
    /// Stores the capabilities
    pub capabilities: ServerCapabilities,
    /// Stores the catalog
    pub catalog: McpCatalog,
    /// Stores the error
    pub error: Option<String>,
}

impl McpServerStatus {
    /// Handles tool count
    #[must_use]
    pub fn tool_count(&self) -> usize {
        self.catalog.tools.len()
    }
    /// Handles resource count
    #[must_use]
    pub fn resource_count(&self) -> usize {
        self.catalog.resources.len()
    }
    /// Handles capability labels
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
/// Represents mcp status report
#[derive(Clone, Debug, Default, PartialEq)]
pub struct McpStatusReport {
    /// Stores the config path
    pub config_path: PathBuf,
    /// Stores the servers
    pub servers: Vec<McpServerStatus>,
}

impl McpStatusReport {
    /// Handles inspect
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
    /// Handles ready count
    #[must_use]
    pub fn ready_count(&self) -> usize {
        self.servers
            .iter()
            .filter(|server| server.state == Some(McpServerState::Ready))
            .count()
    }
    /// Handles error count
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
