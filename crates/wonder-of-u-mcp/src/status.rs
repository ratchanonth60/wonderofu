use std::path::PathBuf;

use crate::{McpCatalog, McpClient, McpConfig, McpServerConfig, ServerCapabilities, ServerInfo};
/// Enumerates mcp server state
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpServerState {
    /// Server is configured but disabled by the user.
    Disabled,
    /// Transient state while the server process is starting and MCP initialization is in flight.
    ///
    /// The synchronous `McpStatusReport::inspect` path never emits this variant (it blocks
    /// until the server responds or errors). It is provided for async / UI layers that need to
    /// represent in-flight connections with parity against the upstream Claude Code UI.
    Connecting,
    /// Server responded to `initialize` and is ready to serve tool and resource calls.
    Ready,
    /// Server failed to start or returned an error during initialization.
    Error,
}

impl McpServerState {
    /// Returns a short, stable ASCII label suitable for display and serialization.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Connecting => "connecting",
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

    /// Construct a status for a server that is currently starting or initializing.
    ///
    /// Intended for async / streaming UIs that render an in-progress spinner before the
    /// server's `initialize` response arrives. The synchronous `McpStatusReport::inspect`
    /// path does not emit this state.
    pub fn connecting(server: &McpServerConfig) -> Self {
        Self {
            name: server.name.clone(),
            enabled: true,
            state: Some(McpServerState::Connecting),
            command_line: server.command_line(),
            protocol_version: None,
            server_info: None,
            capabilities: ServerCapabilities::default(),
            catalog: McpCatalog::default(),
            error: None,
        }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn demo_server() -> McpServerConfig {
        McpServerConfig {
            name: "demo".into(),
            command: "demo-server".into(),
            args: vec![],
            env: BTreeMap::new(),
            enabled: true,
            cwd: None,
            protocol_version: None,
        }
    }

    #[test]
    fn connecting_state_has_correct_label() {
        assert_eq!(McpServerState::Connecting.label(), "connecting");
    }

    #[test]
    fn connecting_status_fields_match_server_config() {
        let server = demo_server();
        let status = McpServerStatus::connecting(&server);

        assert_eq!(status.name, "demo");
        assert!(status.enabled);
        assert_eq!(status.state, Some(McpServerState::Connecting));
        assert!(status.error.is_none());
        assert!(status.server_info.is_none());
        assert_eq!(status.tool_count(), 0);
        assert_eq!(status.resource_count(), 0);
    }

    #[test]
    fn all_state_labels_are_unique() {
        use std::collections::HashSet;
        let labels: HashSet<&str> = [
            McpServerState::Disabled,
            McpServerState::Connecting,
            McpServerState::Ready,
            McpServerState::Error,
        ]
        .iter()
        .map(|s| s.label())
        .collect();
        // Each variant must produce a distinct label string.
        assert_eq!(labels.len(), 4);
    }
}
