use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use wonder_of_u_core::{Result, WonderError};

use crate::McpServerConfig;

/// One server entry inside an upstream-compatible project `.mcp.json`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectMcpServerEntry {
    /// Command to spawn.
    pub command: String,
    /// Command arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment overrides.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Upstream uses `disabled`; absent means enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
}

/// Parsed upstream-compatible project `.mcp.json`.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectMcpConfig {
    /// Object-keyed server map.
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: BTreeMap<String, ProjectMcpServerEntry>,
}

impl ProjectMcpConfig {
    /// Loads and parses a project `.mcp.json` file.
    pub fn load(path: &Path) -> Result<Self> {
        serde_json::from_str(&fs::read_to_string(path)?).map_err(|error| {
            WonderError::validation(format!(
                "project .mcp.json at {} is invalid: {error}",
                path.display()
            ))
        })
    }

    /// Discovers the nearest `.mcp.json` while walking from `start` upward.
    pub fn discover(start: &Path, stop: Option<&Path>) -> Result<Option<(PathBuf, Self)>> {
        let mut current = if start.is_file() {
            start.parent().unwrap_or(start).to_path_buf()
        } else {
            start.to_path_buf()
        };
        let stop = stop.map(Path::to_path_buf);

        loop {
            let candidate = current.join(".mcp.json");
            if candidate.exists() {
                return Ok(Some((candidate.clone(), Self::load(&candidate)?)));
            }
            if stop.as_deref().is_some_and(|stop| current == stop) {
                return Ok(None);
            }
            if !current.pop() {
                return Ok(None);
            }
        }
    }

    /// Converts project entries into internal server configs.
    #[must_use]
    pub fn into_server_configs(self, config_dir: &Path) -> Vec<McpServerConfig> {
        self.mcp_servers
            .into_iter()
            .map(|(name, entry)| McpServerConfig {
                name,
                command: entry.command,
                args: entry.args,
                env: entry.env,
                enabled: !entry.disabled.unwrap_or(false),
                cwd: Some(config_dir.to_path_buf()),
                protocol_version: None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs};

    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    #[test]
    fn project_config_parses_mcp_servers_map() {
        let parsed: ProjectMcpConfig = serde_json::from_str(
            r#"{
              "mcpServers": {
                "github": {
                  "command": "npx",
                  "args": ["-y", "@modelcontextprotocol/server-github"],
                  "env": { "GITHUB_TOKEN": "${TOKEN}" }
                }
              }
            }"#,
        )
        .expect("parse");

        let server = parsed.mcp_servers.get("github").expect("server");
        assert_eq!(server.command, "npx");
        assert_eq!(server.args.len(), 2);
        assert_eq!(server.env["GITHUB_TOKEN"], "${TOKEN}");
    }

    #[test]
    fn project_config_disabled_field_inverts_to_enabled_false() {
        let config = ProjectMcpConfig {
            mcp_servers: BTreeMap::from([(
                "demo".into(),
                ProjectMcpServerEntry {
                    command: "demo-server".into(),
                    args: Vec::new(),
                    env: BTreeMap::new(),
                    disabled: Some(true),
                },
            )]),
        };
        let dir = unique_test_dir("project-mcp-disabled");

        let servers = config.into_server_configs(&dir);
        assert_eq!(servers.len(), 1);
        assert!(!servers[0].enabled);
        assert_eq!(servers[0].cwd.as_deref(), Some(dir.as_path()));
    }

    #[test]
    fn project_config_discovers_nearest_file() {
        let dir = unique_test_dir("project-mcp-discover");
        let nested = dir.join("a/b/c");
        fs::create_dir_all(&nested).expect("mkdir");
        fs::write(
            dir.join(".mcp.json"),
            r#"{"mcpServers":{"root":{"command":"root-server"}}}"#,
        )
        .expect("write root");
        fs::write(
            dir.join("a/b/.mcp.json"),
            r#"{"mcpServers":{"nearest":{"command":"nearest-server"}}}"#,
        )
        .expect("write nearest");

        let (path, config) = ProjectMcpConfig::discover(&nested, None)
            .expect("discover")
            .expect("project config");
        assert_eq!(path, dir.join("a/b/.mcp.json"));
        assert!(config.mcp_servers.contains_key("nearest"));
    }

    #[test]
    fn project_config_malformed_json_returns_error() {
        let dir = unique_test_dir("project-mcp-malformed");
        let path = dir.join(".mcp.json");
        fs::write(&path, "{").expect("write malformed");

        let error = ProjectMcpConfig::load(&path).expect_err("malformed");
        assert!(error.to_string().contains("invalid"));
    }
}
