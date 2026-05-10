//! Provides wonder of u cli support
//!

#![warn(missing_docs)]

use std::{
    ffi::OsString,
    io::{IsTerminal, Write},
    path::PathBuf,
};

use clap::{Parser, Subcommand};
use futures::executor::block_on;
use wonder_of_u_agent::{ProviderResolver, SettingsStore};
use wonder_of_u_core::{
    CommandContext, CommandInvocation, CommandOutput, CommandQuery, FeatureSet, PermissionMode,
    ProviderReadiness, Result, SessionId, WonderError, parse_slash_command,
};

mod commands;
mod tui_runtime;

/// Re-exports items from `commands`
pub use commands::registry as build_command_registry;

#[derive(Debug, Parser)]
#[command(
    name = "wonder-of-u",
    version,
    about = "Rust workspace foundation for the wonder-of-u agent CLI",
    disable_help_subcommand = true
)]
/// Represents cli
pub struct Cli {
    /// Override the wonder-of-u data directory used by storage-backed commands.
    #[arg(long, global = true)]
    pub storage_dir: Option<PathBuf>,
    /// Stores the command
    #[command(subcommand)]
    pub command: Option<Commands>,
}
/// Enumerates commands
#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    /// Show built-in command help.
    #[command(visible_alias = "h")]
    Help {
        #[arg()]
        /// Stores the command
        command: Option<String>,
    },
    /// Run lightweight startup diagnostics.
    Doctor,
    /// Summarize runtime and storage status.
    Status,
    /// Show session cost and token usage.
    Cost {
        #[arg(long)]
        /// Show cost for all sessions.
        all: bool,
    },
    /// Show token usage (alias for cost).
    Usage {
        #[arg(long)]
        /// Show cost for all sessions.
        all: bool,
    },
    /// Get or set the assistant output rendering style.
    OutputStyle {
        #[command(subcommand)]
        /// Stores the command
        command: Option<OutputStyleCliCommand>,
    },
    /// Inspect persisted provider settings and overrides.
    Config {
        #[command(subcommand)]
        /// Stores the command
        command: Option<ConfigCommand>,
    },
    /// Store provider credentials or run interactive provider login.
    Login {
        #[arg(long)]
        /// Stores the provider
        provider: String,
        #[arg(long)]
        /// Stores the api key
        api_key: Option<String>,
        #[arg(long, default_value_t = false)]
        /// Stores the no browser
        no_browser: bool,
    },
    /// Remove stored provider credentials.
    Logout {
        #[arg(long)]
        /// Stores the provider
        provider: Option<String>,
    },
    /// Inspect or change the active provider/model selection.
    Model {
        #[command(subcommand)]
        /// Stores the command
        command: Option<ModelCommand>,
    },
    /// Get or set the UI theme.
    Theme {
        #[command(subcommand)]
        /// Stores the command
        command: Option<ThemeCommand>,
    },
    /// Get or set vim keybinding mode.
    Vim {
        #[command(subcommand)]
        /// Stores the command
        command: Option<VimCommand>,
    },
    /// Execute a non-interactive model prompt.
    Prompt {
        #[arg(long)]
        /// Stores the provider
        provider: Option<String>,
        #[arg(long)]
        /// Stores the model
        model: Option<String>,
        #[arg(long)]
        /// Stores the system
        system: Option<String>,
        #[arg(long)]
        /// Stores the session id
        session_id: Option<String>,
        #[arg(long)]
        /// Stores the max output tokens
        max_output_tokens: Option<u32>,
        #[arg(long)]
        /// Stores the temperature
        temperature: Option<f32>,
        #[arg(long, default_value_t = false)]
        /// Stores the tools
        tools: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 1..)]
        /// Stores the prompt
        prompt: Vec<String>,
    },
    /// Launch the live interactive terminal shell (default when no subcommand is given in a TTY).
    Tui {
        #[arg(long)]
        /// Stores the session id
        session_id: Option<String>,
    },
    /// Print enabled first-release feature gates.
    Features,
    /// Inspect MCP server config and stdio discovery status.
    Mcp {
        #[command(subcommand)]
        /// Stores the command
        command: Option<McpCommand>,
    },
    /// Inspect discovered plugin manifests and trust state.
    Plugin {
        #[command(subcommand)]
        /// Stores the command
        command: Option<PluginCommand>,
    },
    /// Reload plugin and skill metadata catalogs.
    ReloadPlugins,
    /// Inspect and run bundled, local, and plugin-provided skills.
    Skills {
        #[command(subcommand)]
        /// Stores the command
        command: Option<SkillsCommand>,
    },
    /// Session persistence helpers.
    Session {
        #[command(subcommand)]
        /// Stores the command
        command: Option<SessionCommand>,
    },
    /// Resume a persisted session in the live shell when interactive, or print a summary.
    Resume {
        #[arg()]
        /// Stores the session id
        session_id: String,
    },
    /// Add or manage tags on persisted sessions.
    Tag {
        #[arg()]
        /// Tag name to add or remove.
        name: Option<String>,
        #[arg(long)]
        /// Stores the session id.
        session: Option<String>,
        #[arg(long)]
        /// Remove the tag instead of adding it.
        remove: bool,
        #[arg(long)]
        /// List all sessions grouped by tag.
        list: bool,
    },
    /// Print a summary of a conversation session.
    Summary {
        #[arg(long)]
        /// Stores the session id
        session: Option<String>,
        #[arg(long, default_value = "markdown")]
        /// Stores the format
        format: String,
    },
    /// Copy the last assistant response from a persisted session to the clipboard.
    Copy {
        #[arg(long)]
        /// Stores the session id
        session_id: Option<String>,
    },
    /// Rename a persisted session.
    Rename {
        #[arg()]
        /// Stores the session id
        session_id: String,
        #[arg()]
        /// Stores the title
        title: String,
    },
    /// Export a persisted session transcript.
    Export {
        #[arg()]
        /// Stores the session id
        session_id: String,
        #[arg(long, default_value = "text")]
        /// Stores the format
        format: String,
    },
    /// Persist a cleared resume view for the latest or specified session.
    Clear {
        #[arg()]
        /// Stores the session id
        session_id: Option<String>,
    },
    /// Persist a compacted resume view for the latest or specified session.
    Compact {
        #[arg()]
        /// Stores the session id
        session_id: Option<String>,
        #[arg(long, default_value_t = 8)]
        /// Stores the keep last
        keep_last: usize,
    },
    /// Rewind a session to remove recent message exchanges.
    Rewind {
        /// Session ID (defaults to most recent).
        #[arg(long)]
        session: Option<String>,
        /// Number of exchanges to remove.
        #[arg(long, short, default_value_t = 1)]
        n: usize,
        /// Skip the confirmation prompt.
        #[arg(long, short)]
        yes: bool,
    },
    /// Manage AI memory files (CLAUDE.md).
    Memory {
        #[command(subcommand)]
        /// Stores the command
        command: Option<MemoryCommand>,
    },
    /// List files beneath the current working directory.
    Files {
        #[arg()]
        /// Stores the path
        path: Option<PathBuf>,
        #[arg(long, default_value_t = 100)]
        /// Stores the limit
        limit: usize,
        #[arg(long)]
        /// Stores the hidden
        hidden: bool,
    },
    /// Show the current git branch.
    Branch,
    /// Show a git diff summary.
    Diff {
        #[arg(long)]
        /// Stores the staged
        staged: bool,
        #[arg(long)]
        /// Stores the name only
        name_only: bool,
    },
    /// Inspect permission defaults or evaluate a tool request.
    Permissions {
        #[command(subcommand)]
        /// Stores the command
        command: Option<PermissionsCommand>,
    },
    /// Show plan-mode readiness.
    Plan,
    /// Manage persisted local agent tasks.
    Agents {
        #[command(subcommand)]
        /// Stores the command
        command: Option<AgentsCliCommand>,
    },
    /// Manage persisted local background tasks.
    Tasks {
        #[command(subcommand)]
        /// Stores the command
        command: Option<TasksCliCommand>,
    },
    /// Request CLI exit.
    Exit,
    /// Execute a slash command through the shared command registry.
    Slash {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 1..)]
        /// Stores the command
        command: Vec<String>,
    },
}
/// Enumerates model command
#[derive(Debug, Clone, Subcommand)]
pub enum ModelCommand {
    /// Show current provider/model selection and built-in options.
    Show,
    /// Persist a provider/model selection.
    Set {
        #[arg(long)]
        /// Stores the provider
        provider: String,
        #[arg(long)]
        /// Stores the model
        model: String,
    },
}
/// Enumerates theme command
#[derive(Debug, Clone, Eq, PartialEq, Subcommand)]
pub enum ThemeCommand {
    /// Show current theme.
    Show,
    /// List available themes.
    List,
    /// Set the active theme.
    Set {
        #[arg()]
        /// Stores the theme name
        name: String,
    },
}
/// Enumerates vim command
#[derive(Debug, Clone, Eq, PartialEq, Subcommand)]
pub enum VimCommand {
    /// Show vim mode state.
    Show,
    /// Enable vim mode.
    On,
    /// Disable vim mode.
    Off,
    /// Toggle vim mode.
    Toggle,
}
/// Enumerates output-style command
#[derive(Debug, Clone, Eq, PartialEq, Subcommand)]
pub enum OutputStyleCliCommand {
    /// Show current output style.
    Show,
    /// List available output styles.
    List,
    /// Set the active output style.
    Set {
        #[arg()]
        /// Stores the output style name
        name: String,
    },
}
/// Enumerates config command
#[derive(Debug, Clone, Subcommand)]
pub enum ConfigCommand {
    /// Show stored settings and provider overrides.
    Show,
    /// Persist a provider-specific API base override.
    SetApiBase {
        #[arg(long)]
        /// Stores the provider
        provider: String,
        #[arg(long)]
        /// Stores the api base
        api_base: String,
    },
    /// Clear a provider-specific API base override.
    ClearApiBase {
        #[arg(long)]
        /// Stores the provider
        provider: String,
    },
}
/// Enumerates mcp command
#[derive(Debug, Clone, Subcommand)]
pub enum McpCommand {
    /// Show configured MCP servers.
    Show,
    /// Probe configured MCP servers over stdio.
    Status,
    /// Enable a configured MCP server.
    Enable {
        #[arg()]
        /// Stores the server
        server: String,
    },
    /// Disable a configured MCP server.
    Disable {
        #[arg()]
        /// Stores the server
        server: String,
    },
}
/// Enumerates plugin command
#[derive(Debug, Clone, Subcommand)]
pub enum PluginCommand {
    /// List discovered plugins.
    List,
    /// Show detailed plugin readiness and trust information.
    Status,
    /// Persist a trust decision for a plugin id.
    Trust {
        #[arg()]
        /// Stores the plugin
        plugin: String,
        #[arg(long)]
        /// Stores the state
        state: String,
    },
    /// Execute a trusted plugin command entry as a one-shot local subprocess.
    Run {
        #[arg()]
        /// Stores the plugin
        plugin: String,
        #[arg()]
        /// Stores the command
        command: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        /// Stores the args
        args: Vec<String>,
    },
}
/// Enumerates skills command
#[derive(Debug, Clone, Subcommand)]
pub enum SkillsCommand {
    /// List discovered skills.
    List,
    /// Show details for a specific skill.
    Show {
        #[arg()]
        /// Stores the skill
        skill: String,
    },
    /// Execute a skill as a prompt-based provider run.
    Run {
        #[arg()]
        /// Stores the skill
        skill: String,
        #[arg(long)]
        /// Stores the provider
        provider: Option<String>,
        #[arg(long)]
        /// Stores the model
        model: Option<String>,
        #[arg(long)]
        /// Stores the system
        system: Option<String>,
        #[arg(long)]
        /// Stores the session id
        session_id: Option<String>,
        #[arg(long)]
        /// Stores the max output tokens
        max_output_tokens: Option<u32>,
        #[arg(long)]
        /// Stores the temperature
        temperature: Option<f32>,
        #[arg(long, default_value_t = false)]
        /// Stores the tools
        tools: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 1..)]
        /// Stores the request
        request: Vec<String>,
    },
}
/// Enumerates session command
#[derive(Debug, Clone, Subcommand)]
pub enum SessionCommand {
    /// Create a new session metadata record and seed transcript.
    New {
        #[arg(long)]
        /// Stores the title
        title: Option<String>,
        #[arg(long)]
        /// Stores the cwd
        cwd: Option<PathBuf>,
    },
    /// List persisted sessions.
    List {
        #[arg(long, default_value_t = 20)]
        /// Stores the limit
        limit: usize,
    },
}
/// Enumerates memory command
#[derive(Debug, Clone, Eq, PartialEq, Subcommand)]
pub enum MemoryCommand {
    /// Show all memory file contents.
    Show,
    /// Open global memory file in $EDITOR.
    Edit,
    /// Print the path to the global memory file.
    Path,
}
/// Enumerates permissions command
#[derive(Debug, Clone, Subcommand)]
pub enum PermissionsCommand {
    /// Show current permission defaults.
    Show,
    /// Evaluate a concrete tool request.
    Check {
        #[arg(long)]
        /// Stores the tool
        tool: String,
        #[arg(long = "alias")]
        /// Stores the aliases
        aliases: Vec<String>,
        #[arg(long)]
        /// Stores the read only
        read_only: bool,
        #[arg(long)]
        /// Stores the destructive
        destructive: bool,
        #[arg(long = "path")]
        /// Stores the paths
        paths: Vec<PathBuf>,
        #[arg(long = "shell")]
        /// Stores the shell command
        shell_command: Option<String>,
        #[arg(long = "mode")]
        /// Stores the mode
        mode: Option<String>,
        #[arg(long = "add-dir")]
        /// Stores the add dirs
        add_dirs: Vec<PathBuf>,
    },
}
/// Enumerates agents cli command
#[derive(Debug, Clone, Subcommand)]
pub enum AgentsCliCommand {
    /// List persisted local agent entries.
    List {
        #[arg(long, default_value_t = 20)]
        /// Stores the limit
        limit: usize,
    },
    /// Show a persisted local agent entry.
    Show {
        #[arg()]
        /// Stores the task id
        task_id: String,
        #[arg(long = "tail", default_value_t = 20)]
        /// Stores the tail lines
        tail_lines: usize,
    },
    /// Start a persisted local agent task.
    Start {
        #[command(subcommand)]
        /// Stores the command
        command: AgentStartCliCommand,
    },
    /// Stop a persisted local agent task.
    Stop {
        #[arg()]
        /// Stores the task id
        task_id: String,
        #[arg(long)]
        /// Stores the force
        force: bool,
    },
    /// Show local agent runtime status.
    Status,
}
/// Enumerates agent start cli command
#[derive(Debug, Clone, Subcommand)]
pub enum AgentStartCliCommand {
    /// Run a local agent as a single provider-backed prompt subprocess.
    Local {
        #[arg(long)]
        /// Stores the name
        name: String,
        #[arg(long)]
        /// Stores the prompt
        prompt: String,
        #[arg(long)]
        /// Stores the description
        description: Option<String>,
        #[arg(long)]
        /// Stores the provider
        provider: Option<String>,
        #[arg(long)]
        /// Stores the model
        model: Option<String>,
    },
}
/// Enumerates tasks cli command
#[derive(Debug, Clone, Subcommand)]
pub enum TasksCliCommand {
    /// List persisted tasks.
    List {
        #[arg(long, default_value_t = 20)]
        /// Stores the limit
        limit: usize,
    },
    /// Show a persisted task and recent log lines.
    Show {
        #[arg()]
        /// Stores the task id
        task_id: String,
        #[arg(long = "tail", default_value_t = 20)]
        /// Stores the tail lines
        tail_lines: usize,
    },
    /// Start a new persisted task.
    Start {
        #[command(subcommand)]
        /// Stores the command
        command: TaskStartCliCommand,
    },
    /// Stop a persisted task.
    Stop {
        #[arg()]
        /// Stores the task id
        task_id: String,
        #[arg(long)]
        /// Stores the force
        force: bool,
    },
    /// Reconcile persisted task state against runtime artifacts.
    Reconcile,
    /// Show task runtime status.
    Status,
}
/// Enumerates task start cli command
#[derive(Debug, Clone, Subcommand)]
pub enum TaskStartCliCommand {
    /// Start a local background shell task.
    Shell {
        #[arg(long)]
        /// Stores the description
        description: String,
        #[arg(long)]
        /// Stores the command
        command: String,
        #[arg(long)]
        /// Stores the cwd
        cwd: Option<PathBuf>,
        #[arg(long)]
        /// Stores the read only
        read_only: bool,
        #[arg(long)]
        /// Stores the destructive
        destructive: bool,
        #[arg(long)]
        /// Stores the permission mode
        permission_mode: Option<String>,
    },
}

/// Handles run from
pub fn run_from<I, T, W>(args: I, writer: &mut W) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    W: Write,
{
    let cli = Cli::parse_from(args);
    run_with_terminal_mode(cli, writer, default_terminal_mode_for_run_from())
}

/// Handles run
pub fn run<W: Write>(cli: Cli, writer: &mut W) -> Result<()> {
    run_with_terminal_mode(cli, writer, terminal_is_interactive())
}

fn run_with_terminal_mode<W: Write>(
    cli: Cli,
    writer: &mut W,
    interactive_terminal: bool,
) -> Result<()> {
    let storage_dir = cli.storage_dir.or_else(resolve_default_storage_dir);
    match launch_plan(cli.command, interactive_terminal)? {
        LaunchPlan::Cost { all } => {
            let rendered =
                commands::cost::show(storage_dir.as_deref(), &std::env::current_dir()?, all)?;
            writeln!(writer, "{rendered}")?;
            Ok(())
        }
        LaunchPlan::OutputStyle { command } => {
            let rendered = match command.unwrap_or(OutputStyleCliCommand::Show) {
                OutputStyleCliCommand::Show => {
                    commands::output_style::show(storage_dir.as_deref())?
                }
                OutputStyleCliCommand::List => commands::output_style::list(),
                OutputStyleCliCommand::Set { name } => {
                    commands::output_style::set(storage_dir.as_deref(), &name)?
                }
            };
            writeln!(writer, "{rendered}")?;
            Ok(())
        }
        LaunchPlan::Theme { command } => {
            let rendered = match command.unwrap_or(ThemeCommand::Show) {
                ThemeCommand::Show => commands::theme::show(storage_dir.as_deref())?,
                ThemeCommand::List => commands::theme::list(),
                ThemeCommand::Set { name } => commands::theme::set(storage_dir.as_deref(), &name)?,
            };
            writeln!(writer, "{rendered}")?;
            Ok(())
        }
        LaunchPlan::Vim { command } => {
            let storage_dir = storage_dir.ok_or_else(|| {
                WonderError::validation(
                    "vim command requires --storage-dir or HOME/XDG_CONFIG_HOME",
                )
            })?;
            let rendered = match command.unwrap_or(VimCommand::Show) {
                VimCommand::Show => commands::vim::show(&storage_dir)?,
                VimCommand::On => commands::vim::set(&storage_dir, true)?,
                VimCommand::Off => commands::vim::set(&storage_dir, false)?,
                VimCommand::Toggle => commands::vim::toggle(&storage_dir)?,
            };
            writeln!(writer, "{rendered}")?;
            Ok(())
        }
        LaunchPlan::Tui { session_id } => tui_runtime::run_tui(
            writer,
            &commands::registry(storage_dir.clone())?,
            storage_dir.as_deref(),
            tui_runtime::TuiLaunchOptions { session_id },
        ),
        LaunchPlan::Copy { session_id } => {
            commands::copy::copy_last_response(storage_dir.as_deref(), session_id.as_deref())?;
            writeln!(writer, "Copied last assistant response to clipboard")?;
            Ok(())
        }
        LaunchPlan::Memory { command } => {
            let storage_dir = storage_dir.ok_or_else(|| {
                WonderError::validation(
                    "memory command requires --storage-dir or HOME/XDG_CONFIG_HOME",
                )
            })?;
            let cwd = std::env::current_dir()?;
            match command.unwrap_or(MemoryCommand::Show) {
                MemoryCommand::Show => {
                    commands::memory::show_with_writer(&storage_dir, &cwd, writer)
                }
                MemoryCommand::Edit => commands::memory::edit(&storage_dir),
                MemoryCommand::Path => commands::memory::path_cmd_with_writer(&storage_dir, writer),
            }
        }
        LaunchPlan::Invocation(invocation) => run_registered_invocation(
            invocation,
            &commands::registry(storage_dir.clone())?,
            storage_dir.as_deref(),
            writer,
        ),
    }
}

/// Resolves the default storage directory from well-known environment variables.
///
/// Lookup order:
/// 1. `WONDER_OF_U_STORAGE_DIR` — explicit override.
/// 2. `XDG_CONFIG_HOME/wonder-of-u` — XDG Base Directory spec.
/// 3. `HOME/.config/wonder-of-u` — POSIX fallback.
///
/// Returns `None` when none of the variables are set; callers that require a
/// storage directory will surface a helpful error via [`wonder_of_u_agent::require_storage_dir`].
fn resolve_default_storage_dir() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("WONDER_OF_U_STORAGE_DIR") {
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    if let Ok(path) = std::env::var("XDG_CONFIG_HOME") {
        if !path.is_empty() {
            return Some(PathBuf::from(path).join("wonder-of-u"));
        }
    }
    if let Ok(path) = std::env::var("HOME") {
        if !path.is_empty() {
            return Some(PathBuf::from(path).join(".config").join("wonder-of-u"));
        }
    }
    None
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
    Cost {
        all: bool,
    },
    OutputStyle {
        command: Option<OutputStyleCliCommand>,
    },
    Theme {
        command: Option<ThemeCommand>,
    },
    Vim {
        command: Option<VimCommand>,
    },
    Tui {
        session_id: Option<String>,
    },
    Copy {
        session_id: Option<String>,
    },
    Invocation(CommandInvocation),
    Memory {
        command: Option<MemoryCommand>,
    },
}

fn terminal_is_interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn launch_plan(command: Option<Commands>, interactive_terminal: bool) -> Result<LaunchPlan> {
    match command {
        None if interactive_terminal => Ok(LaunchPlan::Tui { session_id: None }),
        None => Ok(LaunchPlan::Invocation(to_invocation(Commands::Doctor)?)),
        Some(Commands::Cost { all } | Commands::Usage { all }) => Ok(LaunchPlan::Cost { all }),
        Some(Commands::OutputStyle { command }) => Ok(LaunchPlan::OutputStyle { command }),
        Some(Commands::Theme { command }) => Ok(LaunchPlan::Theme { command }),
        Some(Commands::Vim { command }) => Ok(LaunchPlan::Vim { command }),
        Some(Commands::Tui { session_id }) => Ok(LaunchPlan::Tui { session_id }),
        Some(Commands::Copy { session_id }) => Ok(LaunchPlan::Copy { session_id }),
        Some(Commands::Memory { command }) => Ok(LaunchPlan::Memory { command }),
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
    let persisted_theme = match storage_dir {
        Some(storage_dir) => SettingsStore::new(storage_dir).read()?.theme,
        None => None,
    };
    let context = CommandContext {
        session_id: SessionId::new(),
        cwd: std::env::current_dir()?,
        features: FeatureSet::first_release(),
        authenticated,
        interactive: false,
        permission_mode: PermissionMode::Default,
        theme: persisted_theme,
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
        Commands::Cost { .. } | Commands::Usage { .. } => {
            unreachable!("cost and usage are handled directly by the top-level CLI")
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
        Commands::Theme { .. } => {
            unreachable!("theme is handled directly by the top-level CLI")
        }
        Commands::Vim { .. } => {
            unreachable!("vim is handled directly by the top-level CLI")
        }
        Commands::Copy { .. } => {
            unreachable!("copy is handled directly by the top-level CLI")
        }
        Commands::OutputStyle { .. } => {
            unreachable!("output-style is handled directly by the top-level CLI")
        }
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
        Commands::Tag {
            name,
            session,
            remove,
            list,
        } => {
            let mut tokens = Vec::new();
            if let Some(session) = session {
                tokens.push("--session".into());
                tokens.push(session);
            }
            if remove {
                tokens.push("--remove".into());
            }
            if list {
                tokens.push("--list".into());
            }
            if let Some(name) = name {
                tokens.push(name);
            }
            commands::invocation_from_tokens("tag", tokens)
        }
        Commands::Summary { session, format } => {
            let mut tokens = Vec::new();
            if let Some(session) = session {
                tokens.push("--session".into());
                tokens.push(session);
            }
            tokens.push("--format".into());
            tokens.push(format);
            commands::invocation_from_tokens("summary", tokens)
        }
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
        Commands::Rewind { session, n, yes } => {
            let mut tokens = Vec::new();
            if let Some(session) = session {
                tokens.push("--session".into());
                tokens.push(session);
            }
            tokens.push("--n".into());
            tokens.push(n.to_string());
            if yes {
                tokens.push("--yes".into());
            }
            commands::invocation_from_tokens("rewind", tokens)
        }
        Commands::Memory { .. } => {
            return Err(WonderError::validation(
                "memory is handled directly and cannot be converted into a slash invocation",
            ));
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
    fn cost_subcommand_uses_direct_launch_plan() {
        let plan = launch_plan(Some(Commands::Cost { all: true }), false).expect("launch plan");

        assert_eq!(plan, LaunchPlan::Cost { all: true });
    }

    #[test]
    fn vim_subcommand_uses_direct_launch_plan() {
        let plan = launch_plan(Some(Commands::Vim { command: None }), false).expect("launch plan");

        assert_eq!(plan, LaunchPlan::Vim { command: None });
    }

    #[test]
    fn output_style_subcommand_uses_direct_launch_plan() {
        let plan =
            launch_plan(Some(Commands::OutputStyle { command: None }), false).expect("launch plan");

        assert_eq!(plan, LaunchPlan::OutputStyle { command: None });
    }

    #[test]
    fn copy_subcommand_uses_direct_launch_plan() {
        let plan = launch_plan(
            Some(Commands::Copy {
                session_id: Some("session-123".into()),
            }),
            false,
        )
        .expect("launch plan");

        assert_eq!(
            plan,
            LaunchPlan::Copy {
                session_id: Some("session-123".into()),
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
    fn rewind_command_keeps_cli_flags_in_the_registry_invocation() {
        let invocation = to_invocation(Commands::Rewind {
            session: Some("abc-123".into()),
            n: 3,
            yes: true,
        })
        .expect("rewind invocation");

        assert_eq!(
            invocation,
            commands::invocation_from_tokens(
                "rewind",
                ["--session", "abc-123", "--n", "3", "--yes"]
            )
        );
    }

    #[test]
    fn memory_command_uses_direct_launch_plan() {
        let plan =
            launch_plan(Some(Commands::Memory { command: None }), false).expect("launch plan");

        assert_eq!(plan, LaunchPlan::Memory { command: None });
    }

    #[test]
    fn vim_command_reports_current_state() {
        let dir = unique_test_dir("cli-vim-show");
        let mut output = Vec::new();

        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                dir.display().to_string(),
                "vim".to_string(),
            ],
            &mut output,
        )
        .expect("run vim show");

        let text = String::from_utf8(output).expect("utf8");
        assert_eq!(text.trim(), "vim mode: on");
    }

    #[test]
    fn vim_command_toggle_persists_state() {
        let dir = unique_test_dir("cli-vim-toggle-command");
        let mut output = Vec::new();

        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                dir.display().to_string(),
                "vim".to_string(),
                "toggle".to_string(),
            ],
            &mut output,
        )
        .expect("run vim toggle");

        let text = String::from_utf8(output).expect("utf8");
        assert_eq!(text.trim(), "vim mode: off");
        assert_eq!(
            wonder_of_u_agent::SettingsStore::new(&dir)
                .read()
                .expect("read settings")
                .vim_mode,
            Some(false)
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
    fn memory_path_command_prints_global_memory_path() {
        let storage_dir = unique_test_dir("cli-memory-path");
        let mut output = Vec::new();

        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.display().to_string(),
                "memory".to_string(),
                "path".to_string(),
            ],
            &mut output,
        )
        .expect("run memory path");

        let text = String::from_utf8(output).expect("utf8");
        assert_eq!(
            text.trim(),
            storage_dir.join("CLAUDE.md").display().to_string()
        );
    }

    #[test]
    fn theme_list_command_prints_available_themes() {
        let mut output = Vec::new();

        run_from(["wonder-of-u", "theme", "list"], &mut output).expect("run theme list");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.lines().any(|theme| theme == "default"));
        assert!(text.lines().any(|theme| theme == "midnight"));
    }

    #[test]
    fn theme_set_command_persists_theme() {
        let dir = unique_test_dir("cli-theme-command-set");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "theme".to_string(),
                "set".to_string(),
                "midnight".to_string(),
            ],
            &mut output,
        )
        .expect("run theme set");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("theme=midnight"));
        assert_eq!(
            SettingsStore::new(&dir)
                .read()
                .expect("read settings")
                .theme
                .as_deref(),
            Some("midnight")
        );

        let mut show_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "theme".to_string(),
            ],
            &mut show_output,
        )
        .expect("run theme show");
        let show_text = String::from_utf8(show_output).expect("utf8");
        assert_eq!(show_text.trim(), "current_theme=midnight");
    }

    #[test]
    fn theme_set_command_rejects_unknown_theme() {
        let dir = unique_test_dir("cli-theme-command-invalid");
        let storage_dir = dir.to_string_lossy().into_owned();

        let error = run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "theme".to_string(),
                "set".to_string(),
                "aurora".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect_err("unknown theme should fail");

        assert!(error.to_string().contains("unknown theme: aurora"));
    }

    #[test]
    fn output_style_list_command_prints_available_styles() {
        let mut output = Vec::new();

        run_from(["wonder-of-u", "output-style", "list"], &mut output)
            .expect("run output-style list");

        assert_eq!(
            String::from_utf8(output).expect("utf8").trim(),
            "markdown\nplain\nraw"
        );
    }

    #[test]
    fn output_style_set_command_persists_style() {
        let dir = unique_test_dir("cli-output-style-command-set");
        let storage_dir = dir.to_string_lossy().into_owned();

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "output-style".to_string(),
                "set".to_string(),
                "raw".to_string(),
            ],
            &mut output,
        )
        .expect("run output-style set");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("output_style=raw"));
        assert_eq!(
            wonder_of_u_agent::SettingsStore::new(&dir)
                .read()
                .expect("read settings")
                .output_style
                .as_deref(),
            Some("raw")
        );

        let mut show_output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "output-style".to_string(),
            ],
            &mut show_output,
        )
        .expect("run output-style show");
        let show_text = String::from_utf8(show_output).expect("utf8");
        assert_eq!(show_text.trim(), "current_output_style=raw");
    }

    #[test]
    fn output_style_set_command_rejects_unknown_style() {
        let dir = unique_test_dir("cli-output-style-command-invalid");
        let storage_dir = dir.to_string_lossy().into_owned();

        let error = run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "output-style".to_string(),
                "set".to_string(),
                "html".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect_err("unknown output style should fail");

        assert!(error.to_string().contains("unknown output style: html"));
    }

    #[test]
    fn status_reports_storage_counts() {
        let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "");
        let _oai_key = EnvVarGuard::set("OPENAI_API_KEY", "");
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
        assert!(text.contains("session_memory_indexes=1"));
        assert!(text.contains("provider_readiness=unconfigured"));
        assert!(text.contains("settings_sync=local_only"));
        assert!(text.contains("settings_sync_cloud=unsupported"));
        assert!(text.contains("remote_managed_settings=deferred"));
        assert!(text.contains("team_memory_sync=unsupported"));
        assert!(text.contains("skills=14"));
        assert!(text.contains("mcp_servers=0"));
        assert!(text.contains("active_tasks=0"));
        assert!(text.contains("terminal_tasks=1"));
        assert!(text.contains("completed_tasks=1"));
        assert!(text.contains("tasks_reconciled_at="));
        assert!(text.contains("fresh_task_heartbeats=0"));
        assert!(text.contains("plugin_runtime=command_subprocess"));
    }

    #[test]
    fn cost_and_usage_commands_render_human_readable_cost_summary() {
        let dir = unique_test_dir("cli-cost-command");
        let storage_dir = dir.to_string_lossy().into_owned();
        let store = TranscriptStore::new(&dir);
        let cost_store = CostStore::new(&dir);

        let mut state = AppState::new(dir.join("workspace"));
        fs::create_dir_all(&state.session.cwd).expect("create workspace");
        state.record_cost_usage(
            TokenUsage {
                input_tokens: 12_345,
                output_tokens: 3_456,
                cache_creation_tokens: 567,
                cache_read_tokens: 1_234,
            },
            Some(0.0842),
        );
        store
            .write_metadata(&SessionMetadata::from_app_state(&state))
            .expect("write metadata");
        cost_store
            .write_costs(&SessionCostLedger::from_app_state(&state))
            .expect("write costs");

        let _cwd_lock = CWD_TEST_LOCK.lock().expect("cwd lock");
        let original_cwd = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(&state.session.cwd).expect("set current dir");

        let mut cost_output = Vec::new();
        let mut usage_output = Vec::new();
        let result = (|| -> Result<()> {
            run_from(
                vec![
                    "wonder-of-u".to_string(),
                    "--storage-dir".to_string(),
                    storage_dir.clone(),
                    "cost".to_string(),
                ],
                &mut cost_output,
            )?;
            run_from(
                vec![
                    "wonder-of-u".to_string(),
                    "--storage-dir".to_string(),
                    storage_dir,
                    "usage".to_string(),
                ],
                &mut usage_output,
            )?;
            Ok(())
        })();
        std::env::set_current_dir(&original_cwd).expect("restore current dir");
        result.expect("run cost commands");

        let cost_text = String::from_utf8(cost_output).expect("cost utf8");
        let usage_text = String::from_utf8(usage_output).expect("usage utf8");

        assert_eq!(cost_text, usage_text);
        assert!(cost_text.contains(&format!("Session: {}", state.session.id)));
        assert!(cost_text.contains("  Input tokens:   12,345"));
        assert!(cost_text.contains("  Output tokens:  3,456"));
        assert!(cost_text.contains("  Cache read:     1,234"));
        assert!(cost_text.contains("  Cache write:    567"));
        assert!(cost_text.contains("  Total cost:     $0.0842"));
        assert!(cost_text.contains("All-time total:   $0.0842"));
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
    fn prompt_command_generates_local_suggestions_without_extra_network_requests() {
        let dir = unique_test_dir("cli-prompt-suggestion");
        let storage_dir = dir.to_string_lossy().into_owned();
        let (api_base, server) = spawn_json_server(
            |headers, body| {
                let headers = headers.to_ascii_lowercase();
                assert!(headers.contains("post /v1/chat/completions http/1.1"));
                assert!(headers.contains("authorization: bearer suggest-key"));
                assert_eq!(
                    body.pointer("/messages/0/content").and_then(Value::as_str),
                    Some("Fix the bug in the failing tests")
                );
            },
            json!({
                "choices": [{
                    "finish_reason": "stop",
                    "message": {
                        "content": "I fixed the bug and updated the tests, but I haven't run them yet."
                    }
                }],
                "usage": {
                    "prompt_tokens": 11,
                    "completion_tokens": 8
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
        .expect("set prompt suggestion api base");
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "login".to_string(),
                "--provider".to_string(),
                "openai".to_string(),
                "--api-key".to_string(),
                "suggest-key".to_string(),
            ],
            &mut Vec::new(),
        )
        .expect("login for prompt suggestion");

        let mut output = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                dir.to_string_lossy().into_owned(),
                "prompt".to_string(),
                "Fix the bug in the failing tests".to_string(),
            ],
            &mut output,
        )
        .expect("run prompt with local suggestion");
        server.join().expect("prompt suggestion server finished");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("prompt_suggestion=run the tests"));
        assert!(text.contains("prompt_suggestion_kind=verify"));
        assert!(text.contains("query_phase=completed"));
        assert!(text.contains("coordinator_mode=direct"));
        assert!(text.contains("coordinator_cloud_queries=unsupported"));
        assert!(text.contains("coordinator_external_backend=deferred"));
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
        assert!(reload_text.contains("skills=15"));

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
    fn registry_includes_commit_and_commit_push_pr_commands() {
        let mut registry = wonder_of_u_core::CommandRegistry::new();
        registry
            .register(std::sync::Arc::new(commands::workflow::CommitCommand::new()))
            .expect("register commit");
        registry
            .register(std::sync::Arc::new(
                commands::workflow::CommitPushPrCommand::new(),
            ))
            .expect("register commit-push-pr");

        let commit = registry.resolve_spec("commit").expect("commit spec");
        assert_eq!(commit.name, "commit");
        let commit_push_pr = registry
            .resolve_spec("commit-push-pr")
            .expect("commit-push-pr spec");
        assert_eq!(commit_push_pr.name, "commit-push-pr");
    }

    #[test]
    fn slash_transport_preserves_commit_prompt_arguments() {
        let invocation = invocation_from_slash_tokens(vec![
            "/commit".into(),
            "use".into(),
            "the README change".into(),
        ])
        .expect("slash invocation");

        assert_eq!(invocation.name, "commit");
        assert_eq!(invocation.args, "use 'the README change'");
        assert_eq!(invocation.raw, "/commit use 'the README change'");
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
    fn tag_command_round_trips_through_cli() {
        let dir = unique_test_dir("cli-tag");
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
                "Tagged Session".to_string(),
            ],
            &mut created,
        )
        .expect("create session");
        let created = String::from_utf8(created).expect("utf8");
        let session_id = extract_value(&created, "session_id=").to_string();

        let mut tagged = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir.clone(),
                "tag".to_string(),
                "bugfix".to_string(),
            ],
            &mut tagged,
        )
        .expect("tag session");
        let tagged = String::from_utf8(tagged).expect("utf8");
        assert!(tagged.contains(&format!("session_id={session_id}")));
        assert!(tagged.contains("session_tags=bugfix"));

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
        assert!(listed.contains("session[0].tags=#bugfix"));

        let mut tag_list = Vec::new();
        run_from(
            vec![
                "wonder-of-u".to_string(),
                "--storage-dir".to_string(),
                storage_dir,
                "tag".to_string(),
                "--list".to_string(),
            ],
            &mut tag_list,
        )
        .expect("list tags");
        let tag_list = String::from_utf8(tag_list).expect("utf8");
        assert!(tag_list.contains("tag[0].name=bugfix"));
        assert!(tag_list.contains(&format!("tag[0].session[0].id={session_id}")));
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
        fs::create_dir_all(dir.join("config")).expect("create config dir");
        fs::write(
            dir.join("config").join("CLAUDE.md"),
            "# local user memory\n",
        )
        .expect("write user memory");

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
        assert!(config_text.contains("settings_sync=local_only"));
        assert!(config_text.contains("settings_sync_cloud=unsupported"));
        assert!(config_text.contains("settings_sync_settings_exists=true"));
        assert!(config_text.contains("settings_sync_user_memory_exists=true"));
        assert!(config_text.contains("remote_managed_settings=deferred"));
        assert!(config_text.contains("team_memory_sync=unsupported"));

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
        assert!(stopped.contains("status=cancelled"));
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
