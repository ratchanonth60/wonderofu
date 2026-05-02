use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use wonder_of_u_core::{Result, WonderError};
use wonder_of_u_storage::StoragePaths;

use crate::normalize_mcp_name;

/// Schema version for mcp config
pub const MCP_CONFIG_SCHEMA_VERSION: u16 = 1;
/// Default mcp protocol version value
pub const DEFAULT_MCP_PROTOCOL_VERSION: &str = "2024-11-05";

fn default_schema_version() -> u16 {
    MCP_CONFIG_SCHEMA_VERSION
}

fn default_protocol_version() -> String {
    DEFAULT_MCP_PROTOCOL_VERSION.into()
}

fn default_client_name() -> String {
    "wonder-of-u".into()
}

fn default_client_version() -> String {
    env!("CARGO_PKG_VERSION").into()
}

const fn default_enabled() -> bool {
    true
}

fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let next_extension = match path.extension().and_then(OsStr::to_str) {
        Some(extension) => format!("{extension}.next"),
        None => "next".into(),
    };
    let pending_path = path.with_extension(next_extension);

    let file = File::create(&pending_path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    fs::rename(pending_path, path)?;
    Ok(())
}
/// Represents mcp client identity
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct McpClientIdentity {
    /// Stores the name
    #[serde(default = "default_client_name")]
    pub name: String,
    /// Stores the version
    #[serde(default = "default_client_version")]
    pub version: String,
}

impl Default for McpClientIdentity {
    fn default() -> Self {
        Self {
            name: default_client_name(),
            version: default_client_version(),
        }
    }
}
/// Represents mcp server config
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Stores the name
    pub name: String,
    /// Stores the command
    pub command: String,
    /// Stores the args
    #[serde(default)]
    pub args: Vec<String>,
    /// Stores the env
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Stores the enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Stores the cwd
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// Stores the protocol version
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
}

impl McpServerConfig {
    /// Validates the value
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(WonderError::validation("mcp server name cannot be empty"));
        }
        if self.command.trim().is_empty() {
            return Err(WonderError::validation(format!(
                "mcp server `{}` command cannot be empty",
                self.name
            )));
        }
        Ok(())
    }
    /// Handles command line
    #[must_use]
    pub fn command_line(&self) -> String {
        if self.args.is_empty() {
            self.command.clone()
        } else {
            format!("{} {}", self.command, self.args.join(" "))
        }
    }
}
/// Represents mcp config
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct McpConfig {
    /// Stores the schema version
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    /// Stores the client
    #[serde(default)]
    pub client: McpClientIdentity,
    /// Stores the protocol version
    #[serde(default = "default_protocol_version")]
    pub protocol_version: String,
    /// Stores the servers
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            schema_version: MCP_CONFIG_SCHEMA_VERSION,
            client: McpClientIdentity::default(),
            protocol_version: default_protocol_version(),
            servers: Vec::new(),
        }
    }
}

impl McpConfig {
    /// Validates the value
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != MCP_CONFIG_SCHEMA_VERSION {
            return Err(WonderError::validation(format!(
                "unsupported mcp schema version: {}",
                self.schema_version
            )));
        }
        if self.protocol_version.trim().is_empty() {
            return Err(WonderError::validation(
                "mcp protocol version cannot be empty",
            ));
        }

        let mut seen = BTreeMap::new();
        for server in &self.servers {
            server.validate()?;
            let normalized = normalize_mcp_name(&server.name);
            if let Some(previous) = seen.insert(normalized, server.name.clone()) {
                return Err(WonderError::validation(format!(
                    "duplicate mcp server name: {} conflicts with {}",
                    previous, server.name
                )));
            }
        }
        Ok(())
    }
    /// Handles server
    #[must_use]
    pub fn server(&self, name: &str) -> Option<&McpServerConfig> {
        self.servers.iter().find(|server| server.name == name)
    }
}
/// Stores mcp config store
#[derive(Clone, Debug)]
pub struct McpConfigStore {
    paths: StoragePaths,
}

impl McpConfigStore {
    /// Creates a new value
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
        }
    }
    /// Handles paths
    #[must_use]
    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    /// Handles read
    pub fn read(&self) -> Result<McpConfig> {
        let path = self.paths.mcp_servers_path();
        if !path.exists() {
            return Ok(McpConfig::default());
        }

        let config: McpConfig = serde_json::from_str(&fs::read_to_string(path)?)?;
        config.validate()?;
        Ok(config)
    }

    /// Handles write
    pub fn write(&self, config: &McpConfig) -> Result<()> {
        config.validate()?;
        fs::create_dir_all(self.paths.mcp_config_dir())?;
        write_json_atomically(&self.paths.mcp_servers_path(), config)
    }

    /// Handles set enabled
    pub fn set_enabled(&self, name: &str, enabled: bool) -> Result<McpServerConfig> {
        let mut config = self.read()?;
        let server = config
            .servers
            .iter_mut()
            .find(|server| server.name == name)
            .ok_or_else(|| WonderError::not_found("mcp server", name))?;
        server.enabled = enabled;
        let updated = server.clone();
        self.write(&config)?;
        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    #[test]
    fn config_store_round_trips_and_toggles_servers() {
        let dir = unique_test_dir("mcp-config");
        let store = McpConfigStore::new(&dir);
        let config = McpConfig {
            servers: vec![McpServerConfig {
                name: "demo".into(),
                command: "demo-server".into(),
                args: vec!["--stdio".into()],
                env: BTreeMap::from([("DEMO".into(), "1".into())]),
                enabled: true,
                cwd: Some(dir.join("workspace")),
                protocol_version: None,
            }],
            ..McpConfig::default()
        };

        store.write(&config).expect("write config");
        let loaded = store.read().expect("read config");
        assert_eq!(loaded, config);

        let updated = store.set_enabled("demo", false).expect("disable server");
        assert!(!updated.enabled);
        assert!(
            !store
                .read()
                .expect("read updated")
                .server("demo")
                .expect("server")
                .enabled
        );
    }

    #[test]
    fn config_validation_rejects_duplicate_normalized_server_names() {
        let config = McpConfig {
            servers: vec![
                McpServerConfig {
                    name: "GitHub Tools".into(),
                    command: "server-a".into(),
                    args: Vec::new(),
                    env: BTreeMap::new(),
                    enabled: true,
                    cwd: None,
                    protocol_version: None,
                },
                McpServerConfig {
                    name: "github-tools".into(),
                    command: "server-b".into(),
                    args: Vec::new(),
                    env: BTreeMap::new(),
                    enabled: true,
                    cwd: None,
                    protocol_version: None,
                },
            ],
            ..McpConfig::default()
        };

        let error = config.validate().expect_err("duplicate names");
        assert!(error.to_string().contains("duplicate mcp server name"));
    }
}
