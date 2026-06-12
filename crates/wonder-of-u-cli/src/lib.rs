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

/// Resolves [`wonder_of_u_agent::RuntimeOptions`] from the settings persisted
/// in `storage_dir` (`request_max_retries`, `stream_max_retries`,
/// `stream_idle_timeout_ms`), falling back to defaults when settings are
/// absent or unreadable.
pub(crate) fn runtime_options(
    storage_dir: Option<&std::path::Path>,
) -> wonder_of_u_agent::RuntimeOptions {
    storage_dir
        .and_then(|dir| SettingsStore::new(dir).read().ok())
        .map(|settings| wonder_of_u_agent::RuntimeOptions::from_settings(&settings))
        .unwrap_or_default()
}

/// Builds a [`wonder_of_u_agent::ProviderRuntime`] honoring transport
/// settings persisted in `storage_dir`.
pub(crate) fn provider_runtime(
    storage_dir: Option<&std::path::Path>,
) -> wonder_of_u_agent::ProviderRuntime {
    wonder_of_u_agent::ProviderRuntime::with_options(runtime_options(storage_dir))
}

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
    /// Show or manage environment variables injected into agent context.
    Env {
        #[command(subcommand)]
        /// Stores the command
        command: Option<EnvCliCommand>,
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
    /// Print the TUI keyboard shortcut reference.
    Keybindings,
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
    /// Import a TypeScript-upstream Claude Code transcript (JSONL) into Rust
    /// storage schema.
    ///
    /// This is an explicit one-shot migration command.  The normal session load
    /// path is never involved; strict Rust schema validation stays intact.
    Import {
        /// Path to the TS JSONL transcript file.
        #[arg(long)]
        source: PathBuf,
        /// Parse and report without writing any data.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Override the session ID for all imported messages (UUID format).
        #[arg(long)]
        session_id: Option<String>,
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
/// Enumerates env command
#[derive(Debug, Clone, Eq, PartialEq, Subcommand)]
pub enum EnvCliCommand {
    /// Show current environment configuration.
    Show,
    /// Persist an environment variable assignment.
    Set {
        #[arg()]
        /// Stores the KEY=VALUE assignment
        assignment: String,
    },
    /// Remove a persisted environment variable.
    Unset {
        #[arg()]
        /// Stores the environment variable name
        key: String,
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
    /// Add a plugin directory to the configured discovery paths.
    Install {
        #[arg()]
        /// Stores the plugin path
        path: PathBuf,
    },
    /// Validate a plugin directory or manifest file.
    Validate {
        #[arg(default_value = ".")]
        /// Stores the path
        path: PathBuf,
    },
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
        LaunchPlan::Env { command } => {
            let rendered = match command.unwrap_or(EnvCliCommand::Show) {
                EnvCliCommand::Show => commands::env::show(storage_dir.as_deref())?,
                EnvCliCommand::Set { assignment } => {
                    let (key, value) = parse_env_assignment(&assignment)?;
                    commands::env::set_var(storage_dir.as_deref(), key, value)?
                }
                EnvCliCommand::Unset { key } => {
                    commands::env::unset_var(storage_dir.as_deref(), &key)?
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
        LaunchPlan::Keybindings => {
            writeln!(writer, "{}", commands::keybindings::show())?;
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
        LaunchPlan::Import {
            source,
            dry_run,
            session_id,
        } => {
            use wonder_of_u_core::SessionId;
            let override_session = session_id.as_deref().map(SessionId::parse).transpose()?;
            commands::import::run_import(
                &source,
                storage_dir.as_deref(),
                dry_run,
                override_session,
                writer,
            )
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
    Env {
        command: Option<EnvCliCommand>,
    },
    Theme {
        command: Option<ThemeCommand>,
    },
    Vim {
        command: Option<VimCommand>,
    },
    Keybindings,
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
    Import {
        source: PathBuf,
        dry_run: bool,
        session_id: Option<String>,
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
        Some(Commands::Env { command }) => Ok(LaunchPlan::Env { command }),
        Some(Commands::Theme { command }) => Ok(LaunchPlan::Theme { command }),
        Some(Commands::Vim { command }) => Ok(LaunchPlan::Vim { command }),
        Some(Commands::Keybindings) => Ok(LaunchPlan::Keybindings),
        Some(Commands::Tui { session_id }) => Ok(LaunchPlan::Tui { session_id }),
        Some(Commands::Copy { session_id }) => Ok(LaunchPlan::Copy { session_id }),
        Some(Commands::Memory { command }) => Ok(LaunchPlan::Memory { command }),
        Some(Commands::Import {
            source,
            dry_run,
            session_id,
        }) => Ok(LaunchPlan::Import {
            source,
            dry_run,
            session_id,
        }),
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
        optimize_token_mode: false,
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

fn parse_env_assignment(assignment: &str) -> Result<(&str, &str)> {
    assignment.split_once('=').ok_or_else(|| {
        WonderError::validation("environment variable assignment must use KEY=VALUE syntax")
    })
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
        Commands::Keybindings => {
            unreachable!("keybindings is handled directly by the top-level CLI")
        }
        Commands::Copy { .. } => {
            unreachable!("copy is handled directly by the top-level CLI")
        }
        Commands::OutputStyle { .. } => {
            unreachable!("output-style is handled directly by the top-level CLI")
        }
        Commands::Env { .. } => {
            unreachable!("env is handled directly by the top-level CLI")
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
            PluginCommand::Install { path } => commands::invocation_from_tokens(
                "plugin",
                vec!["install".to_string(), path.to_string_lossy().into_owned()],
            ),
            PluginCommand::Validate { path } => commands::invocation_from_tokens(
                "plugin",
                vec!["validate".to_string(), path.to_string_lossy().into_owned()],
            ),
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
        // Import is handled directly by LaunchPlan::Import in run_with_terminal_mode and
        // never reaches to_invocation.
        Commands::Import { .. } => unreachable!("import is handled via LaunchPlan::Import"),
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
    include!("cli_tests.rs");
}
