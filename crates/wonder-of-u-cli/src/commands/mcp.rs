use std::{collections::BTreeSet, path::PathBuf};

use async_trait::async_trait;
use clap::{Args, Parser, Subcommand};
use wonder_of_u_agent::require_storage_dir;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec,
    FeatureFlag, Result,
};
use wonder_of_u_mcp::{McpConfigStore, McpServerState, McpStatusReport};

use super::parse_command_args;

/// Represents mcp command
pub struct McpCommand {
    storage_dir: Option<PathBuf>,
}

impl McpCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "mcp",
            "Inspect MCP server configuration and stdio discovery status",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::Mcp]);
        spec
    }
}

#[derive(Debug, Parser)]
struct McpArgs {
    #[command(subcommand)]
    command: Option<McpSubcommand>,
}

#[derive(Debug, Subcommand)]
enum McpSubcommand {
    Show,
    Status,
    Enable(McpServerArgs),
    Disable(McpServerArgs),
}

#[derive(Debug, Args)]
struct McpServerArgs {
    #[arg()]
    server: String,
}

#[async_trait]
impl Command for McpCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<McpArgs>("mcp", &invocation)?;
        match args.command.unwrap_or(McpSubcommand::Show) {
            McpSubcommand::Show => self.show(&context),
            McpSubcommand::Status => self.status(&context),
            McpSubcommand::Enable(args) => self.set_enabled(&args.server, true),
            McpSubcommand::Disable(args) => self.set_enabled(&args.server, false),
        }
    }
}

impl McpCommand {
    fn show(&self, context: &CommandContext) -> Result<CommandOutput> {
        let storage_dir = require_storage_dir(self.storage_dir.clone())?;
        let store = McpConfigStore::new(&storage_dir);
        let (config, project_config_path) = store.read_with_project(&context.cwd, None)?;
        let mut lines = vec![
            format!("config_path={}", store.paths().mcp_servers_path().display()),
            format!(
                "project_config_path={}",
                project_config_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "none".into())
            ),
            format!("schema_version={}", config.schema_version),
            format!("protocol_version={}", config.protocol_version),
            format!("client_name={}", config.client.name),
            format!("client_version={}", config.client.version),
            format!("servers={}", config.servers.len()),
        ];
        if config.servers.is_empty() {
            lines.push("note=no MCP servers configured".into());
        }
        for (index, server) in config.servers.iter().enumerate() {
            lines.push(format!("server[{index}].name={}", server.name));
            lines.push(format!("server[{index}].enabled={}", server.enabled));
            lines.push(format!("server[{index}].command={}", server.command));
            lines.push(format!("server[{index}].args={}", server.args.join(" ")));
            if let Some(cwd) = &server.cwd {
                lines.push(format!("server[{index}].cwd={}", cwd.display()));
            }
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }

    fn status(&self, context: &CommandContext) -> Result<CommandOutput> {
        let storage_dir = require_storage_dir(self.storage_dir.clone())?;
        let store = McpConfigStore::new(&storage_dir);
        let (config, project_config_path) = store.read_with_project(&context.cwd, None)?;
        let report = McpStatusReport::inspect(store.paths().mcp_servers_path(), &config);
        let mut lines = vec![
            format!("config_path={}", report.config_path.display()),
            format!(
                "project_config_path={}",
                project_config_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "none".into())
            ),
            format!("servers={}", report.servers.len()),
            format!("ready_servers={}", report.ready_count()),
            format!("error_servers={}", report.error_count()),
        ];
        if report.servers.is_empty() {
            lines.push("note=no MCP servers configured".into());
        }
        for server in report.servers {
            let state = server.state.unwrap_or(McpServerState::Error).label();
            lines.push(format!(
                "server[{}].enabled={}",
                server.name, server.enabled
            ));
            lines.push(format!("server[{}].state={state}", server.name));
            lines.push(format!(
                "server[{}].command_line={}",
                server.name, server.command_line
            ));
            lines.push(format!(
                "server[{}].tools={}",
                server.name,
                server.tool_count()
            ));
            lines.push(format!(
                "server[{}].resources={}",
                server.name,
                server.resource_count()
            ));
            let capabilities = server.capability_labels();
            lines.push(format!(
                "server[{}].capabilities={}",
                server.name,
                if capabilities.is_empty() {
                    "-".to_string()
                } else {
                    capabilities.join(",")
                }
            ));
            if let Some(protocol_version) = server.protocol_version {
                lines.push(format!(
                    "server[{}].protocol_version={protocol_version}",
                    server.name
                ));
            }
            if let Some(server_info) = server.server_info {
                lines.push(format!(
                    "server[{}].server_info={}:{}",
                    server.name, server_info.name, server_info.version
                ));
            }
            if let Some(error) = server.error {
                lines.push(format!("server[{}].error={error}", server.name));
            }
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }

    fn set_enabled(&self, server_name: &str, enabled: bool) -> Result<CommandOutput> {
        let storage_dir = require_storage_dir(self.storage_dir.clone())?;
        let store = McpConfigStore::new(&storage_dir);
        let server = store.set_enabled(server_name, enabled)?;
        Ok(CommandOutput::Text(format!(
            concat!("config_path={}\n", "server={}\n", "enabled={}"),
            store.paths().mcp_servers_path().display(),
            server.name,
            server.enabled,
        )))
    }
}
