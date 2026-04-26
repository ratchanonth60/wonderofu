use std::{
    ffi::OsString,
    io::{IsTerminal, Write},
    path::PathBuf,
};

use clap::{Parser, Subcommand};
use futures::executor::block_on;
use wonder_of_u_agent::ProviderResolver;
use wonder_of_u_core::{
    CommandContext, CommandInvocation, CommandOutput, CommandQuery, FeatureSet, PermissionMode,
    ProviderReadiness, Result, SessionId, WonderError, parse_slash_command,
};

mod commands;
mod tui_runtime;

#[derive(Debug, Parser)]
#[command(
    name = "wonder-of-u",
    version,
    about = "Rust workspace foundation for the wonder-of-u agent CLI",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Override the wonder-of-u data directory used by storage-backed commands.
    #[arg(long, global = true)]
    pub storage_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    /// Show built-in command help.
    #[command(visible_alias = "h")]
    Help {
        #[arg()]
        command: Option<String>,
    },
    /// Run lightweight startup diagnostics.
    Doctor,
    /// Summarize runtime and storage status.
    Status,
    /// Inspect persisted provider settings and overrides.
    Config {
        #[command(subcommand)]
        command: Option<ConfigCommand>,
    },
    /// Store provider credentials or run interactive provider login.
    Login {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        api_key: Option<String>,
        #[arg(long, default_value_t = false)]
        no_browser: bool,
    },
    /// Remove stored provider credentials.
    Logout {
        #[arg(long)]
        provider: Option<String>,
    },
    /// Inspect or change the active provider/model selection.
    Model {
        #[command(subcommand)]
        command: Option<ModelCommand>,
    },
    /// Execute a non-interactive model prompt.
    Prompt {
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        system: Option<String>,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        max_output_tokens: Option<u32>,
        #[arg(long)]
        temperature: Option<f32>,
        #[arg(long, default_value_t = false)]
        tools: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 1..)]
        prompt: Vec<String>,
    },
    /// Launch the live interactive terminal shell (default when no subcommand is given in a TTY).
    Tui {
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Print enabled first-release feature gates.
    Features,
    /// Inspect MCP server config and stdio discovery status.
    Mcp {
        #[command(subcommand)]
        command: Option<McpCommand>,
    },
    /// Inspect discovered plugin manifests and trust state.
    Plugin {
        #[command(subcommand)]
        command: Option<PluginCommand>,
    },
    /// Reload plugin and skill metadata catalogs.
    ReloadPlugins,
    /// Inspect and run bundled, local, and plugin-provided skills.
    Skills {
        #[command(subcommand)]
        command: Option<SkillsCommand>,
    },
    /// Session persistence helpers.
    Session {
        #[command(subcommand)]
        command: Option<SessionCommand>,
    },
    /// Resume a persisted session in the live shell when interactive, or print a summary.
    Resume {
        #[arg()]
        session_id: String,
    },
    /// Rename a persisted session.
    Rename {
        #[arg()]
        session_id: String,
        #[arg()]
        title: String,
    },
    /// Export a persisted session transcript.
    Export {
        #[arg()]
        session_id: String,
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Persist a cleared resume view for the latest or specified session.
    Clear {
        #[arg()]
        session_id: Option<String>,
    },
    /// Persist a compacted resume view for the latest or specified session.
    Compact {
        #[arg()]
        session_id: Option<String>,
        #[arg(long, default_value_t = 8)]
        keep_last: usize,
    },
    /// List files beneath the current working directory.
    Files {
        #[arg()]
        path: Option<PathBuf>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        hidden: bool,
    },
    /// Show the current git branch.
    Branch,
    /// Show a git diff summary.
    Diff {
        #[arg(long)]
        staged: bool,
        #[arg(long)]
        name_only: bool,
    },
    /// Inspect permission defaults or evaluate a tool request.
    Permissions {
        #[command(subcommand)]
        command: Option<PermissionsCommand>,
    },
    /// Show plan-mode readiness.
    Plan,
    /// Manage persisted local agent tasks.
    Agents {
        #[command(subcommand)]
        command: Option<AgentsCliCommand>,
    },
    /// Manage persisted local background tasks.
    Tasks {
        #[command(subcommand)]
        command: Option<TasksCliCommand>,
    },
    /// Request CLI exit.
    Exit,
    /// Execute a slash command through the shared command registry.
    Slash {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 1..)]
        command: Vec<String>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ModelCommand {
    /// Show current provider/model selection and built-in options.
    Show,
    /// Persist a provider/model selection.
    Set {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        model: String,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConfigCommand {
    /// Show stored settings and provider overrides.
    Show,
    /// Persist a provider-specific API base override.
    SetApiBase {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        api_base: String,
    },
    /// Clear a provider-specific API base override.
    ClearApiBase {
        #[arg(long)]
        provider: String,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum McpCommand {
    /// Show configured MCP servers.
    Show,
    /// Probe configured MCP servers over stdio.
    Status,
    /// Enable a configured MCP server.
    Enable {
        #[arg()]
        server: String,
    },
    /// Disable a configured MCP server.
    Disable {
        #[arg()]
        server: String,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum PluginCommand {
    /// List discovered plugins.
    List,
    /// Show detailed plugin readiness and trust information.
    Status,
    /// Persist a trust decision for a plugin id.
    Trust {
        #[arg()]
        plugin: String,
        #[arg(long)]
        state: String,
    },
    /// Execute a trusted plugin command entry as a one-shot local subprocess.
    Run {
        #[arg()]
        plugin: String,
        #[arg()]
        command: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum SkillsCommand {
    /// List discovered skills.
    List,
    /// Show details for a specific skill.
    Show {
        #[arg()]
        skill: String,
    },
    /// Execute a skill as a prompt-based provider run.
    Run {
        #[arg()]
        skill: String,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        system: Option<String>,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        max_output_tokens: Option<u32>,
        #[arg(long)]
        temperature: Option<f32>,
        #[arg(long, default_value_t = false)]
        tools: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 1..)]
        request: Vec<String>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum SessionCommand {
    /// Create a new session metadata record and seed transcript.
    New {
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
    /// List persisted sessions.
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum PermissionsCommand {
    /// Show current permission defaults.
    Show,
    /// Evaluate a concrete tool request.
    Check {
        #[arg(long)]
        tool: String,
        #[arg(long = "alias")]
        aliases: Vec<String>,
        #[arg(long)]
        read_only: bool,
        #[arg(long)]
        destructive: bool,
        #[arg(long = "path")]
        paths: Vec<PathBuf>,
        #[arg(long = "shell")]
        shell_command: Option<String>,
        #[arg(long = "mode")]
        mode: Option<String>,
        #[arg(long = "add-dir")]
        add_dirs: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum AgentsCliCommand {
    /// List persisted local agent entries.
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show a persisted local agent entry.
    Show {
        #[arg()]
        task_id: String,
        #[arg(long = "tail", default_value_t = 20)]
        tail_lines: usize,
    },
    /// Start a persisted local agent task.
    Start {
        #[command(subcommand)]
        command: AgentStartCliCommand,
    },
    /// Stop a persisted local agent task.
    Stop {
        #[arg()]
        task_id: String,
        #[arg(long)]
        force: bool,
    },
    /// Show local agent runtime status.
    Status,
}

#[derive(Debug, Clone, Subcommand)]
pub enum AgentStartCliCommand {
    /// Run a local agent as a single provider-backed prompt subprocess.
    Local {
        #[arg(long)]
        name: String,
        #[arg(long)]
        prompt: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum TasksCliCommand {
    /// List persisted tasks.
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show a persisted task and recent log lines.
    Show {
        #[arg()]
        task_id: String,
        #[arg(long = "tail", default_value_t = 20)]
        tail_lines: usize,
    },
    /// Start a new persisted task.
    Start {
        #[command(subcommand)]
        command: TaskStartCliCommand,
    },
    /// Stop a persisted task.
    Stop {
        #[arg()]
        task_id: String,
        #[arg(long)]
        force: bool,
    },
    /// Reconcile persisted task state against runtime artifacts.
    Reconcile,
    /// Show task runtime status.
    Status,
}

#[derive(Debug, Clone, Subcommand)]
pub enum TaskStartCliCommand {
    /// Start a local background shell task.
    Shell {
        #[arg(long)]
        description: String,
        #[arg(long)]
        command: String,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        read_only: bool,
        #[arg(long)]
        destructive: bool,
        #[arg(long)]
        permission_mode: Option<String>,
    },
}

pub fn run_from<I, T, W>(args: I, writer: &mut W) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    W: Write,
{
    let cli = Cli::parse_from(args);
    run_with_terminal_mode(cli, writer, default_terminal_mode_for_run_from())
}

pub fn run<W: Write>(cli: Cli, writer: &mut W) -> Result<()> {
    run_with_terminal_mode(cli, writer, terminal_is_interactive())
}

fn run_with_terminal_mode<W: Write>(
    cli: Cli,
    writer: &mut W,
    interactive_terminal: bool,
) -> Result<()> {
    let registry = commands::registry(cli.storage_dir.clone())?;
    match launch_plan(cli.command, interactive_terminal)? {
        LaunchPlan::Tui { session_id } => tui_runtime::run_tui(
            writer,
            &registry,
            cli.storage_dir.as_deref(),
            tui_runtime::TuiLaunchOptions { session_id },
        ),
        LaunchPlan::Invocation(invocation) => {
            run_registered_invocation(invocation, &registry, cli.storage_dir.as_deref(), writer)
        }
    }
}

#[cfg(test)]
fn default_terminal_mode_for_run_from() -> bool {
    false
}

#[cfg(not(test))]
fn default_terminal_mode_for_run_from() -> bool {
    terminal_is_interactive()
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LaunchPlan {
    Tui { session_id: Option<String> },
    Invocation(CommandInvocation),
}

fn terminal_is_interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn launch_plan(command: Option<Commands>, interactive_terminal: bool) -> Result<LaunchPlan> {
    match command {
        None if interactive_terminal => Ok(LaunchPlan::Tui { session_id: None }),
        None => Ok(LaunchPlan::Invocation(to_invocation(Commands::Doctor)?)),
        Some(Commands::Tui { session_id }) => Ok(LaunchPlan::Tui { session_id }),
        Some(Commands::Resume { session_id }) if interactive_terminal => Ok(LaunchPlan::Tui {
            session_id: Some(session_id),
        }),
        Some(command) => Ok(LaunchPlan::Invocation(to_invocation(command)?)),
    }
}

fn run_registered_invocation<W: Write>(
    invocation: CommandInvocation,
    registry: &wonder_of_u_core::CommandRegistry,
    storage_dir: Option<&std::path::Path>,
    writer: &mut W,
) -> Result<()> {
    let authenticated = ProviderResolver::builtin()
        .load_report(storage_dir)?
        .readiness
        == ProviderReadiness::Ready;
    let context = CommandContext {
        session_id: SessionId::new(),
        cwd: std::env::current_dir()?,
        features: FeatureSet::first_release(),
        authenticated,
        interactive: false,
        permission_mode: PermissionMode::Default,
        theme: None,
        session_color: None,
        effort_level: None,
        brief_mode: false,
        fast_mode: false,
        session_tags: Vec::new(),
        additional_working_directories: Vec::new(),
    };
    let query = CommandQuery::from(&context);
    let command = match registry.resolve_enabled(&invocation.name, &query) {
        Some(command) => command,
        None => {
            commands::resolve_dynamic_command(&context.cwd, storage_dir, &invocation.name, &query)?
                .ok_or_else(|| WonderError::not_found("command", invocation.name.clone()))?
        }
    };
    let output = block_on(command.execute(context, invocation))?;
    render_command_output(output, writer)
}

fn render_command_output<W: Write>(output: CommandOutput, writer: &mut W) -> Result<()> {
    match output {
        CommandOutput::Text(text)
        | CommandOutput::EnqueuePrompt(text)
        | CommandOutput::OpenUi(text) => {
            writeln!(writer, "{text}")?;
            Ok(())
        }
        CommandOutput::ExitRequested => {
            writeln!(writer, "exit requested")?;
            Ok(())
        }
        CommandOutput::Noop => Ok(()),
    }
}

fn to_invocation(command: Commands) -> Result<CommandInvocation> {
    Ok(match command {
        Commands::Help { command } => {
            commands::invocation_from_tokens("help", command.into_iter().collect::<Vec<_>>())
        }
        Commands::Doctor => {
            commands::invocation_from_tokens("doctor", std::iter::empty::<String>())
        }
        Commands::Status => {
            commands::invocation_from_tokens("status", std::iter::empty::<String>())
        }
        Commands::Config { command } => match command.unwrap_or(ConfigCommand::Show) {
            ConfigCommand::Show => commands::invocation_from_tokens("config", ["show"]),
            ConfigCommand::SetApiBase { provider, api_base } => commands::invocation_from_tokens(
                "config",
                [
                    "set-api-base".into(),
                    "--provider".into(),
                    provider,
                    "--api-base".into(),
                    api_base,
                ],
            ),
            ConfigCommand::ClearApiBase { provider } => commands::invocation_from_tokens(
                "config",
                ["clear-api-base".into(), "--provider".into(), provider],
            ),
        },
        Commands::Login {
            provider,
            api_key,
            no_browser,
        } => {
            let mut tokens = vec!["--provider".into(), provider];
            if let Some(api_key) = api_key {
                tokens.push("--api-key".into());
                tokens.push(api_key);
            }
            if no_browser {
                tokens.push("--no-browser".into());
            }
            commands::invocation_from_tokens("login", tokens)
        }
        Commands::Logout { provider } => {
            let mut tokens: Vec<String> = Vec::new();
            if let Some(provider) = provider {
                tokens.push("--provider".into());
                tokens.push(provider);
            }
            commands::invocation_from_tokens("logout", tokens)
        }
        Commands::Model { command } => match command.unwrap_or(ModelCommand::Show) {
            ModelCommand::Show => commands::invocation_from_tokens("model", ["show"]),
            ModelCommand::Set { provider, model } => commands::invocation_from_tokens(
                "model",
                [
                    "set".into(),
                    "--provider".into(),
                    provider,
                    "--model".into(),
                    model,
                ],
            ),
        },
        Commands::Prompt {
            provider,
            model,
            system,
            session_id,
            max_output_tokens,
            temperature,
            tools,
            prompt,
        } => {
            let mut tokens = Vec::new();
            if let Some(provider) = provider {
                tokens.push("--provider".into());
                tokens.push(provider);
            }
            if let Some(model) = model {
                tokens.push("--model".into());
                tokens.push(model);
            }
            if let Some(system) = system {
                tokens.push("--system".into());
                tokens.push(system);
            }
            if let Some(session_id) = session_id {
                tokens.push("--session-id".into());
                tokens.push(session_id);
            }
            if let Some(max_output_tokens) = max_output_tokens {
                tokens.push("--max-output-tokens".into());
                tokens.push(max_output_tokens.to_string());
            }
            if let Some(temperature) = temperature {
                tokens.push("--temperature".into());
                tokens.push(temperature.to_string());
            }
            if tools {
                tokens.push("--tools".into());
            }
            tokens.extend(prompt);
            commands::invocation_from_tokens("prompt", tokens)
        }
        Commands::Tui { session_id } => {
            let mut tokens = Vec::new();
            if let Some(session_id) = session_id {
                tokens.push("--session-id".into());
                tokens.push(session_id);
            }
            commands::invocation_from_tokens("tui", tokens)
        }
        Commands::Features => {
            commands::invocation_from_tokens("features", std::iter::empty::<String>())
        }
        Commands::Mcp { command } => match command.unwrap_or(McpCommand::Show) {
            McpCommand::Show => commands::invocation_from_tokens("mcp", ["show"]),
            McpCommand::Status => commands::invocation_from_tokens("mcp", ["status"]),
            McpCommand::Enable { server } => {
                commands::invocation_from_tokens("mcp", ["enable".into(), server])
            }
            McpCommand::Disable { server } => {
                commands::invocation_from_tokens("mcp", ["disable".into(), server])
            }
        },
        Commands::Plugin { command } => match command.unwrap_or(PluginCommand::List) {
            PluginCommand::List => commands::invocation_from_tokens("plugin", ["list"]),
            PluginCommand::Status => commands::invocation_from_tokens("plugin", ["status"]),
            PluginCommand::Trust { plugin, state } => commands::invocation_from_tokens(
                "plugin",
                ["trust".into(), plugin, "--state".into(), state],
            ),
            PluginCommand::Run {
                plugin,
                command,
                args,
            } => {
                let mut tokens = vec!["run".into(), plugin, command];
                tokens.extend(args);
                commands::invocation_from_tokens("plugin", tokens)
            }
        },
        Commands::ReloadPlugins => {
            commands::invocation_from_tokens("reload-plugins", std::iter::empty::<String>())
        }
        Commands::Skills { command } => match command.unwrap_or(SkillsCommand::List) {
            SkillsCommand::List => commands::invocation_from_tokens("skills", ["list"]),
            SkillsCommand::Show { skill } => {
                commands::invocation_from_tokens("skills", ["show".into(), skill])
            }
            SkillsCommand::Run {
                skill,
                provider,
                model,
                system,
                session_id,
                max_output_tokens,
                temperature,
                tools,
                request,
            } => {
                let mut tokens = vec!["run".into(), skill];
                if let Some(provider) = provider {
                    tokens.push("--provider".into());
                    tokens.push(provider);
                }
                if let Some(model) = model {
                    tokens.push("--model".into());
                    tokens.push(model);
                }
                if let Some(system) = system {
                    tokens.push("--system".into());
                    tokens.push(system);
                }
                if let Some(session_id) = session_id {
                    tokens.push("--session-id".into());
                    tokens.push(session_id);
                }
                if let Some(max_output_tokens) = max_output_tokens {
                    tokens.push("--max-output-tokens".into());
                    tokens.push(max_output_tokens.to_string());
                }
                if let Some(temperature) = temperature {
                    tokens.push("--temperature".into());
                    tokens.push(temperature.to_string());
                }
                if tools {
                    tokens.push("--tools".into());
                }
                tokens.extend(request);
                commands::invocation_from_tokens("skills", tokens)
            }
        },
        Commands::Session { command } => {
            match command.unwrap_or(SessionCommand::List { limit: 20 }) {
                SessionCommand::New { title, cwd } => {
                    let mut tokens = vec!["new".into()];
                    if let Some(title) = title {
                        tokens.push("--title".into());
                        tokens.push(title);
                    }
                    if let Some(cwd) = cwd {
                        tokens.push("--cwd".into());
                        tokens.push(cwd.display().to_string());
                    }
                    commands::invocation_from_tokens("session", tokens)
                }
                SessionCommand::List { limit } => commands::invocation_from_tokens(
                    "session",
                    ["list".into(), "--limit".into(), limit.to_string()],
                ),
            }
        }
        Commands::Resume { session_id } => commands::invocation_from_tokens("resume", [session_id]),
        Commands::Rename { session_id, title } => {
            commands::invocation_from_tokens("rename", [session_id, title])
        }
        Commands::Export { session_id, format } => {
            commands::invocation_from_tokens("export", [session_id, "--format".into(), format])
        }
        Commands::Clear { session_id } => {
            let tokens = session_id.into_iter().collect::<Vec<_>>();
            commands::invocation_from_tokens("clear", tokens)
        }
        Commands::Compact {
            session_id,
            keep_last,
        } => {
            let mut tokens = Vec::new();
            if let Some(session_id) = session_id {
                tokens.push(session_id);
            }
            tokens.push("--keep-last".into());
            tokens.push(keep_last.to_string());
            commands::invocation_from_tokens("compact", tokens)
        }
        Commands::Files {
            path,
            limit,
            hidden,
        } => {
            let mut tokens: Vec<String> = Vec::new();
            if let Some(path) = path {
                tokens.push(path.display().to_string());
            }
            tokens.push("--limit".into());
            tokens.push(limit.to_string());
            if hidden {
                tokens.push("--hidden".into());
            }
            commands::invocation_from_tokens("files", tokens)
        }
        Commands::Branch => {
            commands::invocation_from_tokens("branch", std::iter::empty::<String>())
        }
        Commands::Diff { staged, name_only } => {
            let mut tokens: Vec<String> = Vec::new();
            if staged {
                tokens.push("--staged".into());
            }
            if name_only {
                tokens.push("--name-only".into());
            }
            commands::invocation_from_tokens("diff", tokens)
        }
        Commands::Permissions { command } => match command.unwrap_or(PermissionsCommand::Show) {
            PermissionsCommand::Show => commands::invocation_from_tokens("permissions", ["show"]),
            PermissionsCommand::Check {
                tool,
                aliases,
                read_only,
                destructive,
                paths,
                shell_command,
                mode,
                add_dirs,
            } => {
                let mut tokens = vec!["check".into(), "--tool".into(), tool];
                for alias in aliases {
                    tokens.push("--alias".into());
                    tokens.push(alias);
                }
                if read_only {
                    tokens.push("--read-only".into());
                }
                if destructive {
                    tokens.push("--destructive".into());
                }
                for path in paths {
                    tokens.push("--path".into());
                    tokens.push(path.display().to_string());
                }
                if let Some(shell_command) = shell_command {
                    tokens.push("--shell".into());
                    tokens.push(shell_command);
                }
                if let Some(mode) = mode {
                    tokens.push("--mode".into());
                    tokens.push(mode);
                }
                for dir in add_dirs {
                    tokens.push("--add-dir".into());
                    tokens.push(dir.display().to_string());
                }
                commands::invocation_from_tokens("permissions", tokens)
            }
        },
        Commands::Plan => commands::invocation_from_tokens("plan", std::iter::empty::<String>()),
        Commands::Agents { command } => match command.unwrap_or(AgentsCliCommand::Status) {
            AgentsCliCommand::List { limit } => commands::invocation_from_tokens(
                "agents",
                ["list".into(), "--limit".into(), limit.to_string()],
            ),
            AgentsCliCommand::Show {
                task_id,
                tail_lines,
            } => commands::invocation_from_tokens(
                "agents",
                [
                    "show".into(),
                    task_id,
                    "--tail".into(),
                    tail_lines.to_string(),
                ],
            ),
            AgentsCliCommand::Start { command } => match command {
                AgentStartCliCommand::Local {
                    name,
                    prompt,
                    description,
                    provider,
                    model,
                } => {
                    let mut tokens = vec![
                        "start".into(),
                        "local".into(),
                        "--name".into(),
                        name,
                        "--prompt".into(),
                        prompt,
                    ];
                    if let Some(description) = description {
                        tokens.push("--description".into());
                        tokens.push(description);
                    }
                    if let Some(provider) = provider {
                        tokens.push("--provider".into());
                        tokens.push(provider);
                    }
                    if let Some(model) = model {
                        tokens.push("--model".into());
                        tokens.push(model);
                    }
                    commands::invocation_from_tokens("agents", tokens)
                }
            },
            AgentsCliCommand::Stop { task_id, force } => {
                let mut tokens = vec!["stop".into(), task_id];
                if force {
                    tokens.push("--force".into());
                }
                commands::invocation_from_tokens("agents", tokens)
            }
            AgentsCliCommand::Status => {
                commands::invocation_from_tokens("agents", std::iter::empty::<String>())
            }
        },
        Commands::Tasks { command } => match command.unwrap_or(TasksCliCommand::Status) {
            TasksCliCommand::List { limit } => commands::invocation_from_tokens(
                "tasks",
                ["list".into(), "--limit".into(), limit.to_string()],
            ),
            TasksCliCommand::Show {
                task_id,
                tail_lines,
            } => commands::invocation_from_tokens(
                "tasks",
                [
                    "show".into(),
                    task_id,
                    "--tail".into(),
                    tail_lines.to_string(),
                ],
            ),
            TasksCliCommand::Start { command } => match command {
                TaskStartCliCommand::Shell {
                    description,
                    command,
                    cwd,
                    read_only,
                    destructive,
                    permission_mode,
                } => {
                    let mut tokens = vec![
                        "start".into(),
                        "shell".into(),
                        "--description".into(),
                        description,
                        "--command".into(),
                        command,
                    ];
                    if let Some(cwd) = cwd {
                        tokens.push("--cwd".into());
                        tokens.push(cwd.display().to_string());
                    }
                    if read_only {
                        tokens.push("--read-only".into());
                    }
                    if destructive {
                        tokens.push("--destructive".into());
                    }
                    if let Some(permission_mode) = permission_mode {
                        tokens.push("--permission-mode".into());
                        tokens.push(permission_mode);
                    }
                    commands::invocation_from_tokens("tasks", tokens)
                }
            },
            TasksCliCommand::Stop { task_id, force } => {
                let mut tokens = vec!["stop".into(), task_id];
                if force {
                    tokens.push("--force".into());
                }
                commands::invocation_from_tokens("tasks", tokens)
            }
            TasksCliCommand::Reconcile => {
                commands::invocation_from_tokens("tasks", [String::from("reconcile")])
            }
            TasksCliCommand::Status => {
                commands::invocation_from_tokens("tasks", std::iter::empty::<String>())
            }
        },
        Commands::Exit => commands::invocation_from_tokens("exit", std::iter::empty::<String>()),
        Commands::Slash { command } => invocation_from_slash_tokens(command)?,
    })
}

fn invocation_from_slash_tokens(tokens: Vec<String>) -> Result<CommandInvocation> {
    if tokens.is_empty() {
        return Err(WonderError::validation("slash requires a command name"));
    }
    let mut tokens = tokens;
    if !tokens[0].starts_with('/') {
        tokens[0] = format!("/{}", tokens[0]);
    }
    let raw = shell_words::join(tokens.iter().map(String::as_str));
    parse_slash_command(&raw).ok_or_else(|| WonderError::validation("invalid slash command"))
}

#[cfg(test)]
mod tests {
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
    use wonder_of_u_core::{MessagePayload, SessionId, TaskState, TaskStatus};
    use wonder_of_u_plugins::{PluginConfig, PluginConfigStore, PluginTrustDecision};
    use wonder_of_u_storage::{CostStore, TaskStore, TranscriptStore};
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

    #[test]
    fn no_command_defaults_to_doctor_when_not_interactive() {
        let mut output = Vec::new();
        run_from(["wonder-of-u"], &mut output).expect("run doctor");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("foundation ok"));
    }

    #[test]
    fn no_command_prefers_tui_when_terminal_is_interactive() {
        let plan = launch_plan(None, true).expect("launch plan");

        assert_eq!(plan, LaunchPlan::Tui { session_id: None });
    }

    #[test]
    fn interactive_resume_routes_into_tui_with_session_id() {
        let plan = launch_plan(
            Some(Commands::Resume {
                session_id: "abc-123".into(),
            }),
            true,
        )
        .expect("launch plan");

        assert_eq!(
            plan,
            LaunchPlan::Tui {
                session_id: Some("abc-123".into())
            }
        );
    }

    #[test]
    fn noninteractive_resume_keeps_summary_invocation() {
        let plan = launch_plan(
            Some(Commands::Resume {
                session_id: "abc-123".into(),
            }),
            false,
        )
        .expect("launch plan");

        assert_eq!(
            plan,
            LaunchPlan::Invocation(commands::invocation_from_tokens("resume", ["abc-123"]))
        );
    }

    #[test]
    fn help_lists_expanded_command_catalog() {
        let mut output = Vec::new();
        run_from(["wonder-of-u", "help"], &mut output).expect("run help");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("config"));
        assert!(text.contains("resume"));
        assert!(text.contains("rename"));
        assert!(text.contains("files"));
        assert!(text.contains("permissions"));
        assert!(text.contains("mcp"));
        assert!(text.contains("plugin"));
        assert!(text.contains("reload-plugins"));
        assert!(text.contains("skills"));
        assert!(text.contains("agents"));
        assert!(text.contains("tui"));
        assert!(!text.contains("features"));
    }

    #[test]
    fn hidden_command_can_still_be_addressed_directly() {
        let mut output = Vec::new();
        run_from(["wonder-of-u", "help", "features"], &mut output).expect("run help features");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("Print enabled first-release feature gates"));
        assert!(text.contains("hidden from help listings"));
    }

    #[test]
    fn features_command_lists_session_persistence() {
        let mut output = Vec::new();
        run_from(["wonder-of-u", "features"], &mut output).expect("run features");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("SessionPersistence"));
    }

    #[test]
    fn status_reports_storage_counts() {
        let dir = unique_test_dir("cli-status");
        let storage_dir = dir.to_string_lossy().into_owned();

        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "session".to_string(),
                "new".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("create session");

        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "tasks".to_string(),
                "start".to_string(),
                "shell".to_string(),
                "--description".to_string(),
                "status smoke".to_string(),
                "--command".to_string(),
                "printf status-smoke".to_string(),
                "--read-only".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("start status task");
        std::thread::sleep(std::time::Duration::from_millis(200));

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
        assert!(text.contains("transcripts=1"));
        assert!(text.contains("metadata=1"));
        assert!(text.contains("snapshots=1"));
        assert!(text.contains("provider_readiness=unconfigured"));
        assert!(text.contains("skills=1"));
        assert!(text.contains("mcp_servers=0"));
        assert!(text.contains("active_tasks=0"));
        assert!(text.contains("terminal_tasks=1"));
        assert!(text.contains("completed_tasks=1"));
        assert!(text.contains("tasks_reconciled_at="));
        assert!(text.contains("fresh_task_heartbeats=0"));
        assert!(text.contains("plugin_runtime=command_subprocess"));
    }

    #[test]
    fn login_and_model_commands_drive_ready_status() {
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
        assert!(text.contains("provider_selection=openai:gpt-4.1"));
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
    fn prompt_command_runs_tool_loop_and_persists_tool_messages() {
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
        assert!(text.contains("provider_selection=openai:gpt-4.1"));
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
        assert!(reload_text.contains("skills=2"));

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

    #[test]
    fn help_lists_dynamic_plugin_commands() {
        let dir = unique_test_dir("cli-help-plugin-command");
        let storage_dir = dir.to_string_lossy().into_owned();
        let plugin_root = seed_runnable_plugin_fixture(
            &dir,
            "help-plugin",
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
            "printf 'help plugin\\n'\n",
        );

        let mut output = Vec::new();
        run_from(
            ["wonder-of-u", "--storage-dir", storage_dir.as_str(), "help"],
            &mut output,
        )
        .expect("run help");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("echo"));
        assert!(text.contains("Echo arguments"));
    }

    #[test]
    fn slash_transport_preserves_quoted_arguments_for_rename() {
        let dir = unique_test_dir("cli-rename");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut created = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "session".to_string(),
                "new".to_string(),
            ],
            &mut created,
        )
        .expect("create session");
        let created = String::from_utf8(created).expect("utf8");
        let session_id = created
            .lines()
            .find_map(|line| line.strip_prefix("session_id="))
            .expect("session id")
            .to_string();

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "slash".to_string(),
                "/rename".to_string(),
                session_id.clone(),
                "Quarterly Review".to_string(),
            ],
            &mut output,
        )
        .expect("rename session");
        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("title=Quarterly Review"));

        let mut resumed = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "resume".to_string(),
                session_id,
            ],
            &mut resumed,
        )
        .expect("resume session");
        let text = String::from_utf8(resumed).expect("utf8");
        assert!(text.contains("title=Quarterly Review"));
    }

    #[test]
    fn session_commands_round_trip_listing_resume_and_export() {
        let dir = unique_test_dir("cli-session-roundtrip");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut created = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "session".to_string(),
                "new".to_string(),
                "--title".to_string(),
                "Stored Session".to_string(),
            ],
            &mut created,
        )
        .expect("create session");
        let created = String::from_utf8(created).expect("utf8");
        let session_id = created
            .lines()
            .find_map(|line| line.strip_prefix("session_id="))
            .expect("session id")
            .to_string();

        let mut listed = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "session".to_string(),
                "list".to_string(),
            ],
            &mut listed,
        )
        .expect("list sessions");
        let listed = String::from_utf8(listed).expect("utf8");
        assert!(listed.contains(&format!("session[0].id={session_id}")));
        assert!(listed.contains("session[0].title=Stored Session"));

        let mut resumed = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "resume".to_string(),
                session_id.clone(),
            ],
            &mut resumed,
        )
        .expect("resume session");
        let resumed = String::from_utf8(resumed).expect("utf8");
        assert!(resumed.contains("resume_state=snapshot"));
        assert!(resumed.contains("transcript_messages=1"));
        assert!(resumed.contains("view_messages=1"));
        assert!(listed.contains("session[0].resume_source=snapshot"));

        let mut exported = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "export".to_string(),
                session_id,
                "--format".to_string(),
                "json".to_string(),
            ],
            &mut exported,
        )
        .expect("export session");
        let exported = String::from_utf8(exported).expect("utf8");
        assert!(exported.contains("\"title\": \"Stored Session\""));
        assert!(exported.contains("\"transcript\""));
        assert!(exported.contains("\"resume\""));
        assert!(exported.contains("\"source\": \"snapshot\""));
    }

    #[test]
    fn clear_and_compact_persist_resume_views() {
        let dir = unique_test_dir("cli-session-view-state");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut created = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "session".to_string(),
                "new".to_string(),
                "--title".to_string(),
                "View State".to_string(),
            ],
            &mut created,
        )
        .expect("create session");
        let created = String::from_utf8(created).expect("utf8");
        let session_id = extract_value(&created, "session_id=").to_string();

        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "rename".to_string(),
                session_id.clone(),
                "View State Renamed".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("rename session");

        let mut compacted = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "compact".to_string(),
                session_id.clone(),
                "--keep-last".to_string(),
                "1".to_string(),
            ],
            &mut compacted,
        )
        .expect("compact session");
        let compacted = String::from_utf8(compacted).expect("utf8");
        assert!(compacted.contains("view_action=compact"));
        assert!(compacted.contains("compacted_messages=1"));
        assert!(compacted.contains("transcript_unchanged=true"));

        let mut resumed = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "resume".to_string(),
                session_id.clone(),
            ],
            &mut resumed,
        )
        .expect("resume compacted session");
        let resumed = String::from_utf8(resumed).expect("utf8");
        assert!(resumed.contains("resume_state=snapshot"));
        assert!(resumed.contains("transcript_messages=2"));
        assert!(resumed.contains("view_messages=2"));
        assert!(resumed.contains("compacted_messages=1"));

        let mut cleared = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "clear".to_string(),
                session_id.clone(),
            ],
            &mut cleared,
        )
        .expect("clear session");
        let cleared = String::from_utf8(cleared).expect("utf8");
        assert!(cleared.contains("view_action=clear"));
        assert!(cleared.contains("compacted_messages=2"));
        assert!(cleared.contains("boundary_summary=Cleared the visible transcript"));

        let mut exported = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "export".to_string(),
                session_id,
                "--format".to_string(),
                "json".to_string(),
            ],
            &mut exported,
        )
        .expect("export cleared session");
        let exported = String::from_utf8(exported).expect("utf8");
        assert!(exported.contains("\"compacted_messages\": 2"));
        assert!(exported.contains("\"state\""));
    }

    #[test]
    fn config_and_permissions_commands_surface_foundations() {
        let dir = unique_test_dir("cli-config-permissions");
        let storage_dir = dir.to_string_lossy().into_owned();

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
                "https://example.invalid/v1".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("set api base");

        let mut config_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "config".to_string(),
                "show".to_string(),
            ],
            &mut config_output,
        )
        .expect("show config");
        let config_text = String::from_utf8(config_output).expect("utf8");
        assert!(
            config_text.contains("provider_override[openai].api_base=https://example.invalid/v1")
        );

        let mut permissions_output = Vec::new();
        run_from(
            [
                "wonder-of-u",
                "permissions",
                "check",
                "--tool",
                "bash",
                "--shell",
                "rm -rf target/test-workspaces/demo",
            ],
            &mut permissions_output,
        )
        .expect("check permissions");
        let permissions_text = String::from_utf8(permissions_output).expect("utf8");
        assert!(permissions_text.contains("decision=ask"));
        assert!(permissions_text.contains("removes files recursively"));
    }

    #[test]
    fn tasks_commands_manage_shell_task_lifecycle() {
        let dir = unique_test_dir("cli-tasks-shell");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut started = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "tasks".to_string(),
                "start".to_string(),
                "shell".to_string(),
                "--description".to_string(),
                "echo hello".to_string(),
                "--command".to_string(),
                "printf 'hello from task'".to_string(),
                "--read-only".to_string(),
            ],
            &mut started,
        )
        .expect("start task");
        let started = String::from_utf8(started).expect("utf8");
        let task_id = extract_value(&started, "task_id=").to_string();

        let mut listed = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "tasks".to_string(),
                "list".to_string(),
            ],
            &mut listed,
        )
        .expect("list tasks");
        let listed = String::from_utf8(listed).expect("utf8");
        assert!(listed.contains(&format!("task[0].id={task_id}")));

        let mut shown_text = String::new();
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let mut shown = Vec::new();
            run_from(
                vec![
                    "wonder-of-u".to_string(),
                    "--storage-dir".to_string(),
                    storage_dir.clone(),
                    "tasks".to_string(),
                    "show".to_string(),
                    task_id.clone(),
                    "--tail".to_string(),
                    "20".to_string(),
                ],
                &mut shown,
            )
            .expect("show task");
            shown_text = String::from_utf8(shown).expect("utf8");
            if shown_text.contains("status=completed") {
                break;
            }
        }

        assert!(shown_text.contains("status=completed"));
        assert!(shown_text.contains("hello from task"));
        assert!(shown_text.contains("last_heartbeat_at="));

        let mut status = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "tasks".to_string(),
            ],
            &mut status,
        )
        .expect("task status");
        let status = String::from_utf8(status).expect("utf8");
        assert!(status.contains("tasks=1"));
        assert!(status.contains("reconciled_at="));
    }

    #[test]
    fn tasks_commands_stop_background_processes() {
        let dir = unique_test_dir("cli-tasks-stop");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut started = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "tasks".to_string(),
                "start".to_string(),
                "shell".to_string(),
                "--description".to_string(),
                "sleep".to_string(),
                "--command".to_string(),
                "sleep 10".to_string(),
                "--read-only".to_string(),
            ],
            &mut started,
        )
        .expect("start task");
        let task_id =
            extract_value(&String::from_utf8(started).expect("utf8"), "task_id=").to_string();

        let mut stopped = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "tasks".to_string(),
                "stop".to_string(),
                task_id,
                "--force".to_string(),
            ],
            &mut stopped,
        )
        .expect("stop task");
        let stopped = String::from_utf8(stopped).expect("utf8");
        assert!(stopped.contains("status=killed"));
    }

    #[test]
    fn tasks_reconcile_command_surfaces_stale_heartbeat() {
        let dir = unique_test_dir("cli-tasks-reconcile");
        let storage_dir = dir.to_string_lossy().into_owned();
        let store = TaskStore::new(&dir);
        let mut task = TaskState::pending_shell("stale heartbeat", "sleep 1", &dir);
        task.status = TaskStatus::Running;
        task.pid = Some(std::process::id());
        task.output_log = Some(store.paths().task_log_path(task.id));
        store.write_task(&task).expect("write task");
        store
            .write_heartbeat_at(
                task.id,
                OffsetDateTime::now_utc() - time::Duration::seconds(10),
            )
            .expect("write stale heartbeat");

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "tasks".to_string(),
                "reconcile".to_string(),
            ],
            &mut output,
        )
        .expect("reconcile tasks");
        let output = String::from_utf8(output).expect("utf8");
        assert!(output.contains("tasks=1"));
        assert!(output.contains("reconcile_changed=1"));
        assert!(output.contains("stale_heartbeats=1"));
    }

    #[test]
    fn agents_commands_manage_prompt_subprocess_entries() {
        let dir = unique_test_dir("cli-agents");
        let storage_dir = dir.to_string_lossy().into_owned();
        let script = write_executable_script(
            &dir,
            "agent-cli.sh",
            "printf 'agent cli args: %s\\n' \"$*\"\nsleep 10\n",
        );
        let _env = EnvVarGuard::set("WONDER_OF_U_CLI_BIN", script.into_os_string());

        let mut started = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "agents".to_string(),
                "start".to_string(),
                "local".to_string(),
                "--name".to_string(),
                "planner".to_string(),
                "--prompt".to_string(),
                "Summarize release blockers".to_string(),
                "--provider".to_string(),
                "openai".to_string(),
                "--model".to_string(),
                "gpt-4.1".to_string(),
            ],
            &mut started,
        )
        .expect("start agent");
        let started = String::from_utf8(started).expect("utf8");
        let agent_id = extract_value(&started, "agent_id=").to_string();
        assert!(started.contains("runtime=prompt_subprocess"));
        assert!(started.contains("status=running"));
        assert!(started.contains("pid="));

        let mut shown = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "agents".to_string(),
                "show".to_string(),
                agent_id.clone(),
            ],
            &mut shown,
        )
        .expect("show agent");
        let shown = String::from_utf8(shown).expect("utf8");
        assert!(shown.contains("agent.runtime=prompt_subprocess"));
        assert!(shown.contains("agent.provider=openai"));
        assert!(shown.contains("status=running"));
        assert!(shown.contains("agent cli args:"));

        let mut status = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "agents".to_string(),
            ],
            &mut status,
        )
        .expect("agent status");
        let status = String::from_utf8(status).expect("utf8");
        assert!(status.contains("agents=1"));
        assert!(status.contains("prompt_subprocess=1"));
        assert!(status.contains("running=1"));

        let mut stopped = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "agents".to_string(),
                "stop".to_string(),
                agent_id,
            ],
            &mut stopped,
        )
        .expect("stop agent");
        let stopped = String::from_utf8(stopped).expect("utf8");
        assert!(stopped.contains("status=killed"));
    }

    #[test]
    fn task_start_respects_permission_gate() {
        let dir = unique_test_dir("cli-task-permissions");
        let storage_dir = dir.to_string_lossy().into_owned();
        let mut output = Vec::new();

        let error = run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "tasks".to_string(),
                "start".to_string(),
                "shell".to_string(),
                "--description".to_string(),
                "write file".to_string(),
                "--command".to_string(),
                "printf hi > task.txt".to_string(),
            ],
            &mut output,
        )
        .expect_err("permission gate should block default write task");

        assert!(
            error
                .to_string()
                .contains("task launch requires an allowed permission decision")
        );
    }

    #[test]
    fn mcp_commands_surface_config_and_status() {
        let dir = unique_test_dir("cli-mcp");
        let storage_dir = dir.to_string_lossy().into_owned();
        let config_dir = dir.join("config").join("mcp");
        fs::create_dir_all(&config_dir).expect("create mcp config dir");
        fs::write(
            config_dir.join("servers.json"),
            r#"{
  "schema_version": 1,
  "protocol_version": "2024-11-05",
  "client": {
    "name": "wonder-of-u",
    "version": "0.1.0"
  },
  "servers": [
    {
      "name": "demo",
      "command": "/bin/sh",
      "args": ["-c", "exit 0"],
      "enabled": true
    }
  ]
}
"#,
        )
        .expect("write mcp config");

        let mut show_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "mcp".to_string(),
                "show".to_string(),
            ],
            &mut show_output,
        )
        .expect("show mcp config");
        let show_text = String::from_utf8(show_output).expect("utf8");
        assert!(show_text.contains("servers=1"));
        assert!(show_text.contains("server[0].name=demo"));

        let mut disable_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "mcp".to_string(),
                "disable".to_string(),
                "demo".to_string(),
            ],
            &mut disable_output,
        )
        .expect("disable mcp server");
        let disable_text = String::from_utf8(disable_output).expect("utf8");
        assert!(disable_text.contains("enabled=false"));

        let mut status_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "mcp".to_string(),
                "status".to_string(),
            ],
            &mut status_output,
        )
        .expect("mcp status");
        let status_text = String::from_utf8(status_output).expect("utf8");
        assert!(status_text.contains("ready_servers=0"));
        assert!(status_text.contains("server[demo].state=disabled"));
    }

    #[test]
    fn files_command_lists_workspace_entries() {
        let _cwd_lock = CWD_TEST_LOCK.lock().expect("cwd lock");
        let dir = unique_test_dir("cli-files");
        fs::create_dir_all(dir.join("src")).expect("create src");
        fs::write(dir.join("Cargo.toml"), "[package]\nname='demo'\n").expect("write manifest");
        fs::write(dir.join("src").join("main.rs"), "fn main() {}\n").expect("write main");

        let mut output = Vec::new();
        let current = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(&dir).expect("set cwd");
        let result = run_from(["wonder-of-u", "files", "--limit", "10"], &mut output);
        std::env::set_current_dir(current).expect("restore cwd");
        result.expect("run files");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("Cargo.toml"));
        assert!(text.contains("src/main.rs"));
    }

    #[test]
    fn branch_and_diff_commands_report_git_state() {
        let _cwd_lock = CWD_TEST_LOCK.lock().expect("cwd lock");
        let dir = unique_test_dir("cli-git");
        init_git_repo(&dir);
        fs::write(dir.join("README.md"), "hello\n").expect("write readme");
        ProcessCommand::new("git")
            .args(["add", "README.md"])
            .current_dir(&dir)
            .status()
            .expect("git add");
        let current = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(&dir).expect("set cwd");

        let mut branch_output = Vec::new();
        let branch_result = run_from(["wonder-of-u", "branch"], &mut branch_output);
        let mut diff_output = Vec::new();
        let diff_result = run_from(
            ["wonder-of-u", "diff", "--name-only", "--staged"],
            &mut diff_output,
        );

        std::env::set_current_dir(current).expect("restore cwd");
        branch_result.expect("run branch");
        diff_result.expect("run diff");

        let branch_text = String::from_utf8(branch_output).expect("utf8");
        assert!(branch_text.contains("branch="));
        let diff_text = String::from_utf8(diff_output).expect("utf8");
        assert!(diff_text.contains("README.md"));
    }

    fn init_git_repo(path: &Path) {
        ProcessCommand::new("git")
            .args(["init", "--quiet", "-b", "main"])
            .current_dir(path)
            .status()
            .expect("git init");
    }
}
