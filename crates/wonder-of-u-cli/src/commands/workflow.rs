use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    sync::Arc,
    thread,
    time::Duration,
};

use async_trait::async_trait;
use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_agent::{ProviderResolver, SettingsStore};
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec,
    FeatureFlag, PermissionDecision, PermissionMode, PermissionRequest, PermissionRuleSource,
    Result, TaskId, TaskKind, TaskState, TaskStatus, ToolPermissionContext, ToolSpec, WonderError,
};
use wonder_of_u_storage::StoragePaths;
use wonder_of_u_tui::{
    EditAction, KeyBinding, KeyBindingContext, KeyBindingResolver, KeyCode, KeyEvent, KeyModifiers,
    Motion, ResolvedKey, SystemAction, VimCommand as TuiVimCommand,
};

use super::{
    detect_git_branch, git_command_output, parse_command_args,
    task_runtime::{
        AgentTaskLaunch, ShellTaskLaunch, TaskManager, TaskReconcileReport, task_heartbeat_state,
        task_heartbeat_state_label,
    },
    try_open_browser,
};

/// Represents permissions command
pub struct PermissionsCommand {
    tool_specs: Arc<[ToolSpec]>,
}

impl PermissionsCommand {
    /// Creates a new value
    pub fn new(tool_specs: Arc<[ToolSpec]>) -> Self {
        Self { tool_specs }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "permissions",
            "Inspect permission defaults and evaluate tool requests",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::Permissions]);
        spec
    }
}

/// Represents vim command
pub struct VimCommand;

impl VimCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "vim",
            "Toggle between Vim normal and insert modes",
            CommandKind::Local,
        )
    }
}

/// Represents keybindings command
pub struct KeybindingsCommand {
    storage_dir: Option<PathBuf>,
}

impl KeybindingsCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "keybindings",
            "Show the active keyboard shortcuts for the TUI",
            CommandKind::Local,
        )
    }
}

/// Represents terminal setup command
pub struct TerminalSetupCommand {
    storage_dir: Option<PathBuf>,
}

impl TerminalSetupCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "terminal-setup",
            "Explain multiline prompt setup and keybinding options for the local TUI",
            CommandKind::Local,
        );
        spec.aliases.push("terminalSetup".into());
        spec
    }
}

/// Represents theme command
pub struct ThemeCommand;

impl ThemeCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "theme",
            "Show or change the active TUI theme",
            CommandKind::Local,
        )
    }
}

/// Represents color command
pub struct ColorCommand;

impl ColorCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "color",
            "Set the prompt bar color for this session",
            CommandKind::Local,
        )
    }
}

/// Represents brief command
pub struct BriefCommand;

impl BriefCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "brief",
            "Toggle brief response mode for the current session",
            CommandKind::Local,
        );
        spec.interactive_only = true;
        spec
    }
}

/// Toggles token-optimisation mode for the current session.
///
/// When enabled a system-prompt instruction is injected that tells the model
/// to minimise output tokens — omitting preambles, filler, and unnecessary
/// repetition.  The canonical name preserves the original spelling
/// (`optimize-tonken`); the alias `optimize-token` is also accepted.
pub struct OptimizeTonkenCommand;

impl OptimizeTonkenCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "optimize-tonken",
            "Toggle token-optimisation mode (minimise output tokens) for the current session",
            CommandKind::Local,
        );
        // Accept the correctly-spelled alias too so neither spelling is wrong.
        spec.aliases.push("optimize-token".into());
        spec.interactive_only = true;
        spec
    }
}

/// Represents fast command
pub struct FastCommand {
    storage_dir: Option<PathBuf>,
}

impl FastCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "fast",
            "Show or change fast-mode model remapping",
            CommandKind::Local,
        )
    }
}

/// Represents commit command
pub struct CommitCommand;

impl CommitCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "commit",
            "Queue a local git commit prompt for pending workspace changes",
            CommandKind::Local,
        )
    }
}

/// Represents commit push pr command
pub struct CommitPushPrCommand;

impl CommitPushPrCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "commit-push-pr",
            "Queue a local commit, push, and pull-request prompt for the current branch",
            CommandKind::Local,
        )
    }
}

/// Represents review command
pub struct ReviewCommand;

impl ReviewCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "review",
            "Queue a local pull-request review prompt",
            CommandKind::Local,
        );
        spec.interactive_only = true;
        spec
    }
}

/// Represents security review command
pub struct SecurityReviewCommand;

impl SecurityReviewCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "security-review",
            "Queue a local security review prompt for pending branch changes",
            CommandKind::Local,
        );
        spec.interactive_only = true;
        spec
    }
}

/// Represents statusline command
pub struct StatuslineCommand;

impl StatuslineCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "statusline",
            "Queue a status line setup prompt for the current session",
            CommandKind::Local,
        );
        spec.interactive_only = true;
        spec
    }
}

/// Represents effort command
pub struct EffortCommand {
    storage_dir: Option<PathBuf>,
}

impl EffortCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "effort",
            "Show or change the active effort level",
            CommandKind::Local,
        )
    }
}

/// Represents hooks command
pub struct HooksCommand {
    storage_dir: Option<PathBuf>,
    tool_specs: Arc<[ToolSpec]>,
}

impl HooksCommand {
    /// Creates a new value
    pub fn new(storage_dir: Option<PathBuf>, tool_specs: Arc<[ToolSpec]>) -> Self {
        Self {
            storage_dir,
            tool_specs,
        }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "hooks",
            "View hook configurations for tool events",
            CommandKind::Local,
        )
    }
}

/// Represents privacy settings command
pub struct PrivacySettingsCommand;

impl PrivacySettingsCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "privacy-settings",
            "View and update your privacy settings",
            CommandKind::Local,
        )
    }
}

#[derive(Debug, Parser)]
struct KeybindingsArgs {
    #[command(subcommand)]
    command: Option<KeybindingsSubcommand>,
}

#[derive(Debug, Subcommand)]
enum KeybindingsSubcommand {
    Show,
    Open,
}

#[derive(Debug, Parser)]
struct HooksArgs {
    #[command(subcommand)]
    command: Option<HooksSubcommand>,
}

#[derive(Debug, Subcommand)]
enum HooksSubcommand {
    Show,
    Open,
}

#[derive(Debug, Parser)]
struct PrivacySettingsArgs {
    #[command(subcommand)]
    command: Option<PrivacySettingsSubcommand>,
}

#[derive(Debug, Subcommand)]
enum PrivacySettingsSubcommand {
    Show,
}

#[derive(Debug, Parser)]
struct ThemeArgs {
    #[command(subcommand)]
    command: Option<ThemeSubcommand>,
}

#[derive(Debug, Subcommand)]
enum ThemeSubcommand {
    Show,
    Set(ThemeSetArgs),
}

#[derive(Debug, Args)]
struct ThemeSetArgs {
    #[arg()]
    theme: String,
}

#[derive(Debug, Parser)]
struct ColorArgs {
    #[command(subcommand)]
    command: Option<ColorSubcommand>,
}

#[derive(Debug, Subcommand)]
enum ColorSubcommand {
    Show,
    Set(ColorSetArgs),
}

#[derive(Debug, Args)]
struct ColorSetArgs {
    #[arg()]
    color: String,
}

#[derive(Debug, Parser)]
struct EffortArgs {
    #[command(subcommand)]
    command: Option<EffortSubcommand>,
}

#[derive(Debug, Subcommand)]
enum EffortSubcommand {
    Show,
    Set(EffortSetArgs),
}

#[derive(Debug, Args)]
struct EffortSetArgs {
    #[arg()]
    level: String,
}

#[derive(Debug, Parser)]
struct VimArgs {
    #[command(subcommand)]
    command: Option<VimSubcommand>,
}

#[derive(Debug, Subcommand)]
enum VimSubcommand {
    Show,
    Set(VimSetArgs),
}

#[derive(Debug, Args)]
struct VimSetArgs {
    #[arg()]
    mode: String,
}

#[derive(Debug, Parser)]
struct PermissionsArgs {
    #[command(subcommand)]
    command: Option<PermissionsSubcommand>,
}

#[derive(Debug, Subcommand)]
enum PermissionsSubcommand {
    Show,
    Set(PermissionSetArgs),
    Check(PermissionCheckArgs),
}

#[derive(Debug, Args)]
struct PermissionSetArgs {
    #[arg()]
    mode: String,
}

#[derive(Debug, Args)]
struct PermissionCheckArgs {
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
}

#[async_trait]
impl Command for PermissionsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let normalized_invocation = normalize_permissions_invocation(&invocation);
        let args = parse_command_args::<PermissionsArgs>("permissions", &normalized_invocation)?;
        match args.command.unwrap_or(PermissionsSubcommand::Show) {
            PermissionsSubcommand::Show => {
                if context.interactive && invocation.args.trim().is_empty() {
                    Ok(CommandOutput::Text(render_permission_picker(
                        context.permission_mode,
                    )))
                } else {
                    self.show(context)
                }
            }
            PermissionsSubcommand::Set(args) => Ok(CommandOutput::Text(
                render_permission_transition(parse_permission_mode(&args.mode)?),
            )),
            PermissionsSubcommand::Check(args) => self.check(context, args),
        }
    }
}

#[async_trait]
impl Command for VimCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let normalized = normalize_vim_invocation(&invocation);
        let args = parse_command_args::<VimArgs>("vim", &normalized)?;
        match args.command.unwrap_or(VimSubcommand::Show) {
            VimSubcommand::Show => {
                if context.interactive && invocation.args.trim().is_empty() {
                    Ok(CommandOutput::Text("vim_toggle=true".into()))
                } else {
                    Ok(CommandOutput::Text(render_vim_status(None)))
                }
            }
            VimSubcommand::Set(args) => Ok(CommandOutput::Text(render_vim_transition(
                parse_vim_mode(&args.mode)?,
            ))),
        }
    }
}

#[async_trait]
impl Command for KeybindingsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<KeybindingsArgs>("keybindings", &invocation)?;
        match args.command.unwrap_or(KeybindingsSubcommand::Show) {
            KeybindingsSubcommand::Show => Ok(CommandOutput::Text(render_keybindings_summary(
                &load_keybinding_resolver(self.storage_dir.as_deref())?,
                self.storage_dir.as_deref(),
            ))),
            KeybindingsSubcommand::Open => {
                keybindings_open_output(&context, self.storage_dir.as_deref())
                    .map(CommandOutput::Text)
            }
        }
    }
}

#[async_trait]
impl Command for TerminalSetupCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_terminal_setup_notice(
            self.storage_dir.as_deref(),
        )))
    }
}

#[async_trait]
impl Command for ThemeCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let normalized = normalize_theme_invocation(&invocation);
        let args = parse_command_args::<ThemeArgs>("theme", &normalized)?;
        let current_theme = theme_name(context.theme.as_deref());
        match args.command.unwrap_or(ThemeSubcommand::Show) {
            ThemeSubcommand::Show => {
                if context.interactive && invocation.args.trim().is_empty() {
                    Ok(CommandOutput::Text(render_theme_picker(current_theme)))
                } else {
                    Ok(CommandOutput::Text(render_theme_status(current_theme)))
                }
            }
            ThemeSubcommand::Set(args) => Ok(CommandOutput::Text(render_theme_transition(
                parse_theme_name(&args.theme)?,
            ))),
        }
    }
}

#[async_trait]
impl Command for ColorCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let normalized = normalize_color_invocation(&invocation);
        let args = parse_command_args::<ColorArgs>("color", &normalized)?;
        let current_color = session_color_name(context.session_color.as_deref());
        match args.command.unwrap_or(ColorSubcommand::Show) {
            ColorSubcommand::Show => Ok(CommandOutput::Text(render_color_status(current_color))),
            ColorSubcommand::Set(args) => Ok(CommandOutput::Text(render_color_transition(
                parse_session_color_name(&args.color)?,
            ))),
        }
    }
}

#[async_trait]
impl Command for BriefCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        match parse_brief_action(invocation.args.trim(), context.brief_mode)? {
            BriefAction::Show => Ok(CommandOutput::Text(render_brief_status(context.brief_mode))),
            BriefAction::Set(enabled) => Ok(CommandOutput::Text(render_brief_transition(enabled))),
        }
    }
}

#[async_trait]
impl Command for OptimizeTonkenCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        match parse_optimize_tonken_action(invocation.args.trim(), context.optimize_token_mode)? {
            OptimizeTonkenAction::Show => Ok(CommandOutput::Text(render_optimize_tonken_status(
                context.optimize_token_mode,
            ))),
            OptimizeTonkenAction::Set(enabled) => Ok(CommandOutput::Text(
                render_optimize_tonken_transition(enabled),
            )),
        }
    }
}

#[async_trait]
impl Command for FastCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let report = ProviderResolver::builtin().load_report(self.storage_dir.as_deref())?;
        match parse_fast_action(invocation.args.trim(), context.fast_mode)? {
            FastAction::Show => Ok(CommandOutput::Text(render_fast_status(
                context.fast_mode,
                persisted_fast(self.storage_dir.as_deref())?,
                self.storage_dir.as_deref(),
                &report,
            ))),
            FastAction::Set(enabled) => {
                let persisted = write_persisted_fast(self.storage_dir.as_deref(), enabled)?;
                Ok(CommandOutput::Text(render_fast_transition(
                    enabled,
                    persisted,
                    self.storage_dir.as_deref(),
                    &report,
                )))
            }
        }
    }
}

#[async_trait]
impl Command for ReviewCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_review_enqueue(
            &context.cwd,
            invocation.args.trim(),
        )))
    }
}

#[async_trait]
impl Command for CommitCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_commit_enqueue(
            &context.cwd,
            context.permission_mode,
            invocation.args.trim(),
        )))
    }
}

#[async_trait]
impl Command for CommitPushPrCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_commit_push_pr_enqueue(
            &context.cwd,
            context.permission_mode,
            invocation.args.trim(),
        )))
    }
}

#[async_trait]
impl Command for SecurityReviewCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_security_review_enqueue(
            invocation.args.trim(),
        )))
    }
}

#[async_trait]
impl Command for StatuslineCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_statusline_enqueue(
            invocation.args.trim(),
        )))
    }
}

#[async_trait]
impl Command for EffortCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let normalized = normalize_effort_invocation(&invocation);
        let args = parse_command_args::<EffortArgs>("effort", &normalized)?;
        let current_effort = effort_level_name(context.effort_level.as_deref());
        match args.command.unwrap_or(EffortSubcommand::Show) {
            EffortSubcommand::Show => Ok(CommandOutput::Text(render_effort_status(
                current_effort,
                persisted_effort(self.storage_dir.as_deref())?.as_deref(),
                self.storage_dir.as_deref(),
            ))),
            EffortSubcommand::Set(args) => {
                let level = parse_effort_level_name(&args.level)?;
                let persisted = write_persisted_effort(self.storage_dir.as_deref(), level)?;
                Ok(CommandOutput::Text(render_effort_transition(
                    level,
                    persisted,
                    self.storage_dir.as_deref(),
                )))
            }
        }
    }
}

#[async_trait]
impl Command for HooksCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<HooksArgs>("hooks", &invocation)?;
        match args.command.unwrap_or(HooksSubcommand::Show) {
            HooksSubcommand::Show => Ok(CommandOutput::Text(render_hooks_summary(
                self.storage_dir.as_deref(),
                self.tool_specs.as_ref(),
            )?)),
            HooksSubcommand::Open => {
                hooks_open_output(&context, self.storage_dir.as_deref()).map(CommandOutput::Text)
            }
        }
    }
}

#[async_trait]
impl Command for PrivacySettingsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<PrivacySettingsArgs>("privacy-settings", &invocation)?;
        match args.command.unwrap_or(PrivacySettingsSubcommand::Show) {
            PrivacySettingsSubcommand::Show => Ok(CommandOutput::Text(
                render_privacy_settings_summary(try_open_browser(PRIVACY_SETTINGS_URL)),
            )),
        }
    }
}

impl PermissionsCommand {
    fn show(&self, context: CommandContext) -> Result<CommandOutput> {
        let mut lines = vec![
            format!(
                "permission_mode={}",
                permission_mode_label(context.permission_mode)
            ),
            format!("working_directory={}", context.cwd.display()),
            format!(
                "additional_working_directories={}",
                context.additional_working_directories.len()
            ),
            "rules=0".into(),
            format!("tools={}", self.tool_specs.len()),
        ];
        for directory in &context.additional_working_directories {
            lines.push(format!(
                "working_directory_addition={};source={}",
                directory.path.display(),
                directory.source.label()
            ));
        }
        for spec in self.tool_specs.iter() {
            let aliases = if spec.aliases.is_empty() {
                "-".into()
            } else {
                spec.aliases.join(",")
            };
            lines.push(format!(
                "tool[{}]=kind={:?};aliases={};read_only={};destructive={}",
                spec.name, spec.kind, aliases, spec.read_only, spec.destructive
            ));
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }

    fn check(&self, context: CommandContext, args: PermissionCheckArgs) -> Result<CommandOutput> {
        let spec = self.resolve_tool_spec(&args.tool);
        let mut permission_context = ToolPermissionContext::new(
            &context.cwd,
            args.mode
                .as_deref()
                .map(parse_permission_mode)
                .transpose()?
                .unwrap_or(context.permission_mode),
        );
        for dir in args.add_dirs {
            permission_context =
                permission_context.with_additional_directory(dir, PermissionRuleSource::CliArg);
        }
        for directory in &context.additional_working_directories {
            permission_context = permission_context
                .with_additional_directory(directory.path.clone(), directory.source);
        }

        let mut request = PermissionRequest::new(args.tool)
            .with_aliases(args.aliases)
            .read_only(args.read_only || spec.is_some_and(|spec| spec.read_only))
            .destructive(args.destructive || spec.is_some_and(|spec| spec.destructive))
            .with_paths(args.paths);
        if let Some(spec) = spec {
            request = request.with_aliases(spec.aliases.iter().cloned());
        }
        if let Some(shell_command) = args.shell_command {
            request = request.with_shell_command(shell_command);
        }

        let decision = permission_context.evaluate(&request);
        Ok(CommandOutput::Text(render_permission_decision(
            &permission_context,
            &decision,
        )))
    }

    fn resolve_tool_spec(&self, tool_name: &str) -> Option<&ToolSpec> {
        self.tool_specs.iter().find(|spec| {
            spec.name.eq_ignore_ascii_case(tool_name)
                || spec
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(tool_name))
        })
    }
}

/// Represents plan command
pub struct PlanCommand;

impl PlanCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "plan",
            "Show plan-mode readiness and current permission mode",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::Permissions]);
        spec
    }
}

#[derive(Debug, Eq, PartialEq)]
enum PlanAction {
    Show,
    Enter { queued_prompt: Option<String> },
    Exit,
    Open,
}

#[async_trait]
impl Command for PlanCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        match parse_plan_action(&invocation.args, context.permission_mode)? {
            PlanAction::Show => Ok(CommandOutput::Text(render_plan_display(
                context.permission_mode,
                &resolve_plan_path(&context.cwd),
            ))),
            PlanAction::Enter { queued_prompt } => Ok(CommandOutput::Text(render_plan_transition(
                PermissionMode::Plan,
                "plan mode enabled",
                queued_prompt,
            ))),
            PlanAction::Exit => Ok(CommandOutput::Text(render_plan_transition(
                PermissionMode::Default,
                "plan mode disabled",
                None,
            ))),
            PlanAction::Open => open_plan_in_editor(&context),
        }
    }
}

/// Represents agents command
pub struct AgentsCommand {
    storage_dir: Option<PathBuf>,
}

impl AgentsCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "agents",
            "Manage persisted local agent tasks",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::Agents]);
        spec
    }
}

#[derive(Debug, Parser)]
struct AgentsArgs {
    #[command(subcommand)]
    command: Option<AgentsSubcommand>,
}

#[derive(Debug, Subcommand)]
enum AgentsSubcommand {
    List(ListArgs),
    Show(ShowArgs),
    Start(AgentStartArgs),
    Stop(StopArgs),
    Status,
    /// Manage agent definition files.
    Definitions(super::agent_definitions::DefinitionsArgs),
}

#[derive(Debug, Args)]
struct AgentStartArgs {
    #[command(subcommand)]
    command: AgentStartSubcommand,
}

#[derive(Debug, Subcommand)]
enum AgentStartSubcommand {
    Local(LocalAgentArgs),
}

#[derive(Debug, Args)]
struct LocalAgentArgs {
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
}

#[async_trait]
impl Command for AgentsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<AgentsArgs>("agents", &invocation)?;
        match args.command.unwrap_or(AgentsSubcommand::Status) {
            AgentsSubcommand::List(args) => self.list_agents(args),
            AgentsSubcommand::Show(args) => self.show_agent(args),
            AgentsSubcommand::Start(args) => self.start_agent(context.clone(), args),
            AgentsSubcommand::Stop(args) => self.stop_agent(args),
            AgentsSubcommand::Status => self.status(),
            AgentsSubcommand::Definitions(args) => {
                super::agent_definitions::execute_agent_definitions(context, args)
            }
        }
    }
}

impl AgentsCommand {
    fn list_agents(&self, args: ListArgs) -> Result<CommandOutput> {
        let Some(manager) = self.task_manager() else {
            return Ok(CommandOutput::Text(render_runtime_disabled(
                "agents",
                "agent runtime persistence requires --storage-dir",
            )));
        };
        let report = manager.reconcile_tasks(Some(TaskKind::LocalAgent))?;
        Ok(CommandOutput::Text(render_task_list(
            "agent", &manager, &report, args.limit,
        )))
    }

    fn show_agent(&self, args: ShowArgs) -> Result<CommandOutput> {
        let manager = self.require_task_manager("agents show requires --storage-dir")?;
        let report = manager.reconcile_tasks(Some(TaskKind::LocalAgent))?;
        let task_id = parse_task_id(&args.task_id)?;
        let task = require_task_kind(
            report
                .tasks
                .iter()
                .find(|task| task.id == task_id)
                .cloned()
                .ok_or_else(|| WonderError::not_found("task", task_id.to_string()))?,
            TaskKind::LocalAgent,
        )?;
        let log_tail = read_detail_log_tail(&manager, &task, args.tail_lines);
        Ok(CommandOutput::Text(render_task_detail(
            "agent",
            &task,
            &log_tail,
            report.reconciled_at,
        )))
    }

    fn start_agent(&self, context: CommandContext, args: AgentStartArgs) -> Result<CommandOutput> {
        let manager = self.require_task_manager("agents start requires --storage-dir")?;
        let _ = manager.reconcile_tasks(Some(TaskKind::LocalAgent))?;
        let task = match args.command {
            AgentStartSubcommand::Local(args) => {
                let report =
                    ProviderResolver::builtin().load_report(self.storage_dir.as_deref())?;
                manager.start_agent_task(AgentTaskLaunch {
                    name: args.name,
                    description: args.description,
                    prompt: args.prompt,
                    provider: args.provider.or(report.provider),
                    model: args.model.or(report.model),
                    cwd: context.cwd,
                    fleet_id: None,
                    fleet_request_id: None,
                    parent_task_id: None,
                    allowed_tools: None,
                    worktree_branch: None,
                    system_prompt: None,
                    fork_depth: None,
                })?
            }
        };
        Ok(CommandOutput::Text(render_task_started("agent", &task)))
    }

    fn stop_agent(&self, args: StopArgs) -> Result<CommandOutput> {
        let manager = self.require_task_manager("agents stop requires --storage-dir")?;
        let _ = manager.reconcile_tasks(Some(TaskKind::LocalAgent))?;
        let task = require_task_kind(
            manager.stop_task(parse_task_id(&args.task_id)?, args.force)?,
            TaskKind::LocalAgent,
        )?;
        Ok(CommandOutput::Text(render_task_stopped("agent", &task)))
    }

    fn status(&self) -> Result<CommandOutput> {
        let Some(manager) = self.task_manager() else {
            return Ok(CommandOutput::Text(render_runtime_disabled(
                "agents",
                "agent runtime persistence requires --storage-dir",
            )));
        };
        let report = manager.reconcile_tasks(Some(TaskKind::LocalAgent))?;
        let mut lines = render_task_summary_lines("agents", &manager, &report.tasks);
        append_reconcile_lines(&mut lines, &report);
        lines.push(format!(
            "metadata_only={}",
            report
                .tasks
                .iter()
                .filter(|task| {
                    task.agent.as_ref().is_some_and(|agent| {
                        matches!(agent.runtime, wonder_of_u_core::AgentRuntime::MetadataOnly)
                    })
                })
                .count()
        ));
        lines.push(format!(
            "prompt_subprocess={}",
            report
                .tasks
                .iter()
                .filter(|task| {
                    task.agent.as_ref().is_some_and(|agent| {
                        matches!(
                            agent.runtime,
                            wonder_of_u_core::AgentRuntime::PromptSubprocess
                        )
                    })
                })
                .count()
        ));
        lines.push("note=local agents run a single provider-backed prompt subprocess".into());
        Ok(CommandOutput::Text(lines.join("\n")))
    }

    fn task_manager(&self) -> Option<TaskManager> {
        self.storage_dir.clone().map(TaskManager::new)
    }

    fn require_task_manager(&self, message: &str) -> Result<TaskManager> {
        self.task_manager()
            .ok_or_else(|| WonderError::validation(message))
    }
}

/// Represents tasks command
pub struct TasksCommand {
    storage_dir: Option<PathBuf>,
}

impl TasksCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    ///
    /// Key sub-commands surfaced through the description:
    /// - Monitor: bare `/tasks` / `/tasks status` shows live background and fleet work.
    /// - Cleanup: `/tasks remove <id>` deletes a single task; `/tasks prune` bulk-removes
    ///   terminal tasks.
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "tasks",
            "Monitor/manage background tasks: status, remove <id>, prune (bulk-cleanup)",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::BackgroundTasks]);
        spec
    }
}

#[derive(Debug, Parser)]
struct TasksArgs {
    #[command(subcommand)]
    command: Option<TasksSubcommand>,
}

#[derive(Debug, Subcommand)]
enum TasksSubcommand {
    List(ListArgs),
    Show(ShowArgs),
    Start(TaskStartArgs),
    Stop(StopArgs),
    Remove(RemoveArgs),
    Prune(PruneArgs),
    Reconcile,
    Status,
}

#[derive(Debug, Args)]
struct TaskStartArgs {
    #[command(subcommand)]
    command: TaskStartSubcommand,
}

#[derive(Debug, Subcommand)]
enum TaskStartSubcommand {
    Shell(ShellTaskArgs),
}

#[derive(Debug, Args)]
struct ShellTaskArgs {
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
}

#[derive(Debug, Args)]
struct ListArgs {
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

#[derive(Debug, Args)]
struct ShowArgs {
    #[arg()]
    task_id: String,
    #[arg(long = "tail", default_value_t = 20)]
    tail_lines: usize,
}

#[derive(Debug, Args)]
struct StopArgs {
    #[arg()]
    task_id: String,
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct RemoveArgs {
    #[arg()]
    task_id: String,
    /// Remove even if the task is still active (pending or running).
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct PruneArgs {
    /// Remove only completed tasks (exit code 0).  Without this flag all
    /// terminal tasks (completed, failed, killed, cancelled) are removed.
    #[arg(long, conflicts_with = "terminal")]
    completed: bool,
    /// Explicitly prune all terminal tasks (the default behaviour; provided
    /// for clarity in scripts).
    #[arg(long, conflicts_with = "completed")]
    terminal: bool,
}

#[async_trait]
impl Command for TasksCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<TasksArgs>("tasks", &invocation)?;
        match args.command.unwrap_or(TasksSubcommand::Status) {
            TasksSubcommand::List(args) => self.list_tasks(args),
            TasksSubcommand::Show(args) => self.show_task(args),
            TasksSubcommand::Start(args) => self.start_task(context, args),
            TasksSubcommand::Stop(args) => self.stop_task(args),
            TasksSubcommand::Remove(args) => self.remove_task(args),
            TasksSubcommand::Prune(args) => self.prune_tasks(args),
            TasksSubcommand::Reconcile => self.reconcile_tasks(),
            TasksSubcommand::Status => self.status(),
        }
    }
}

impl TasksCommand {
    fn list_tasks(&self, args: ListArgs) -> Result<CommandOutput> {
        let Some(manager) = self.task_manager() else {
            return Ok(CommandOutput::Text(render_runtime_disabled(
                "tasks",
                "task runtime persistence requires --storage-dir",
            )));
        };
        let report = manager.reconcile_tasks(None)?;
        Ok(CommandOutput::Text(render_task_list(
            "task", &manager, &report, args.limit,
        )))
    }

    fn show_task(&self, args: ShowArgs) -> Result<CommandOutput> {
        let manager = self.require_task_manager("tasks show requires --storage-dir")?;
        let report = manager.reconcile_tasks(None)?;
        let task_id = parse_task_id(&args.task_id)?;
        let task = report
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .cloned()
            .ok_or_else(|| WonderError::not_found("task", task_id.to_string()))?;
        let log_tail = read_detail_log_tail(&manager, &task, args.tail_lines);
        Ok(CommandOutput::Text(render_task_detail(
            "task",
            &task,
            &log_tail,
            report.reconciled_at,
        )))
    }

    fn start_task(&self, context: CommandContext, args: TaskStartArgs) -> Result<CommandOutput> {
        let manager = self.require_task_manager("tasks start requires --storage-dir")?;
        let _ = manager.reconcile_tasks(None)?;
        let task = match args.command {
            TaskStartSubcommand::Shell(args) => manager.start_shell_task(
                &context,
                ShellTaskLaunch {
                    description: args.description,
                    command: args.command,
                    cwd: args.cwd,
                    permission_mode: args
                        .permission_mode
                        .as_deref()
                        .map(parse_permission_mode)
                        .transpose()?
                        .unwrap_or(context.permission_mode),
                    read_only: args.read_only,
                    destructive: args.destructive,
                },
            )?,
        };
        Ok(CommandOutput::Text(render_task_started("task", &task)))
    }

    fn stop_task(&self, args: StopArgs) -> Result<CommandOutput> {
        let manager = self.require_task_manager("tasks stop requires --storage-dir")?;
        let _ = manager.reconcile_tasks(None)?;
        let task = manager.stop_task(parse_task_id(&args.task_id)?, args.force)?;
        Ok(CommandOutput::Text(render_task_stopped("task", &task)))
    }

    fn remove_task(&self, args: RemoveArgs) -> Result<CommandOutput> {
        let manager = self.require_task_manager("tasks remove requires --storage-dir")?;
        let task = manager.remove_task(parse_task_id(&args.task_id)?, args.force)?;
        Ok(CommandOutput::Text(render_task_removed("task", &task)))
    }

    fn prune_tasks(&self, args: PruneArgs) -> Result<CommandOutput> {
        let manager = self.require_task_manager("tasks prune requires --storage-dir")?;
        // --completed restricts removal to exit-code-0 tasks only.
        // --terminal (or no flag) removes all terminal tasks.
        let completed_only = args.completed;
        let report = manager.prune_tasks(completed_only)?;
        Ok(CommandOutput::Text(render_task_prune_report(&report)))
    }

    fn reconcile_tasks(&self) -> Result<CommandOutput> {
        let manager = self.require_task_manager("tasks reconcile requires --storage-dir")?;
        let report = manager.reconcile_tasks(None)?;
        Ok(CommandOutput::Text(render_task_reconcile_report(
            "tasks", &manager, &report,
        )))
    }

    fn status(&self) -> Result<CommandOutput> {
        let Some(manager) = self.task_manager() else {
            return Ok(CommandOutput::Text(render_runtime_disabled(
                "tasks",
                "task runtime persistence requires --storage-dir",
            )));
        };
        let report = manager.reconcile_tasks(None)?;
        Ok(CommandOutput::Text(render_task_reconcile_report(
            "tasks", &manager, &report,
        )))
    }

    fn task_manager(&self) -> Option<TaskManager> {
        self.storage_dir.clone().map(TaskManager::new)
    }

    fn require_task_manager(&self, message: &str) -> Result<TaskManager> {
        self.task_manager()
            .ok_or_else(|| WonderError::validation(message))
    }
}

/// Represents exit command
pub struct ExitCommand;

impl ExitCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new("exit", "Request CLI exit", CommandKind::Local)
    }
}

#[async_trait]
impl Command for ExitCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::ExitRequested)
    }
}

fn render_permission_decision(
    context: &ToolPermissionContext,
    decision: &PermissionDecision,
) -> String {
    format!(
        concat!(
            "permission_mode={}\n",
            "working_directories={}\n",
            "decision={}\n",
            "reason={}"
        ),
        permission_mode_label(context.mode),
        context
            .working_directories()
            .into_iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(","),
        decision_label(decision),
        decision.reason(),
    )
}

fn render_permission_transition(mode: PermissionMode) -> String {
    format!(
        concat!(
            "permission_mode={}\n",
            "status=permission mode updated\n",
            "plan_mode_active={}"
        ),
        permission_mode_label(mode),
        matches!(mode, PermissionMode::Plan),
    )
}

fn render_permission_picker(current_mode: PermissionMode) -> String {
    let mut lines = vec![
        "permission_picker=true".into(),
        format!("permission_mode={}", permission_mode_label(current_mode)),
    ];
    for (mode, label, description) in [
        (
            PermissionMode::Default,
            "Default",
            "Ask before edits or destructive commands.",
        ),
        (
            PermissionMode::AcceptEdits,
            "Accept edits",
            "Allow edits automatically but still ask for risky actions.",
        ),
        (
            PermissionMode::BypassPermissions,
            "Bypass permissions",
            "Allow all tools and shell actions without prompts.",
        ),
        (
            PermissionMode::DontAsk,
            "Don't ask",
            "Deny actions that would normally require approval.",
        ),
        (
            PermissionMode::Plan,
            "Plan mode",
            "Switch into planning-oriented permission handling.",
        ),
    ] {
        lines.push(format!(
            "permission_option={}",
            json!({
                "mode": permission_mode_label(mode),
                "label": label,
                "description": description,
                "selected": mode == current_mode,
            })
        ));
    }
    lines.join("\n")
}

fn normalize_permissions_invocation(invocation: &CommandInvocation) -> CommandInvocation {
    let trimmed = invocation.args.trim();
    if trimmed.is_empty()
        || trimmed == "show"
        || trimmed.starts_with("show ")
        || trimmed == "check"
        || trimmed.starts_with("check ")
        || trimmed == "set"
        || trimmed.starts_with("set ")
    {
        return invocation.clone();
    }
    CommandInvocation {
        name: invocation.name.clone(),
        args: format!("set {}", invocation.args),
        raw: invocation.raw.clone(),
    }
}

fn normalize_vim_invocation(invocation: &CommandInvocation) -> CommandInvocation {
    let trimmed = invocation.args.trim();
    if trimmed.is_empty()
        || trimmed == "show"
        || trimmed.starts_with("show ")
        || trimmed == "set"
        || trimmed.starts_with("set ")
    {
        return invocation.clone();
    }
    CommandInvocation {
        name: invocation.name.clone(),
        args: format!("set {}", invocation.args),
        raw: invocation.raw.clone(),
    }
}

fn parse_vim_mode(value: &str) -> Result<&'static str> {
    match value {
        "insert" => Ok("insert"),
        "normal" => Ok("normal"),
        other => Err(WonderError::validation(format!(
            "unknown vim mode: {other}"
        ))),
    }
}

fn render_vim_transition(mode: &str) -> String {
    format!("vim_mode={mode}\nstatus=vim mode updated")
}

fn render_vim_status(mode: Option<&str>) -> String {
    match mode {
        Some(mode) => format!("vim_mode={mode}"),
        None => {
            "vim_mode=unavailable\nnote=vim mode is only available in the interactive TUI".into()
        }
    }
}

fn normalize_theme_invocation(invocation: &CommandInvocation) -> CommandInvocation {
    let trimmed = invocation.args.trim();
    if trimmed.is_empty()
        || trimmed == "show"
        || trimmed.starts_with("show ")
        || trimmed == "set"
        || trimmed.starts_with("set ")
    {
        return invocation.clone();
    }
    CommandInvocation {
        name: invocation.name.clone(),
        args: format!("set {}", invocation.args),
        raw: invocation.raw.clone(),
    }
}

fn normalize_color_invocation(invocation: &CommandInvocation) -> CommandInvocation {
    let trimmed = invocation.args.trim();
    if trimmed.is_empty()
        || trimmed == "show"
        || trimmed.starts_with("show ")
        || trimmed == "set"
        || trimmed.starts_with("set ")
    {
        return invocation.clone();
    }
    CommandInvocation {
        name: invocation.name.clone(),
        args: format!("set {}", invocation.args),
        raw: invocation.raw.clone(),
    }
}

fn normalize_effort_invocation(invocation: &CommandInvocation) -> CommandInvocation {
    let trimmed = invocation.args.trim();
    if trimmed.is_empty()
        || trimmed == "show"
        || trimmed.starts_with("show ")
        || trimmed == "current"
        || trimmed == "status"
        || trimmed == "set"
        || trimmed.starts_with("set ")
    {
        return if trimmed == "current" || trimmed == "status" {
            CommandInvocation {
                name: invocation.name.clone(),
                args: "show".into(),
                raw: invocation.raw.clone(),
            }
        } else {
            invocation.clone()
        };
    }
    CommandInvocation {
        name: invocation.name.clone(),
        args: format!("set {}", invocation.args),
        raw: invocation.raw.clone(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BriefAction {
    Show,
    Set(bool),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FastAction {
    Show,
    Set(bool),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OptimizeTonkenAction {
    Show,
    Set(bool),
}

fn parse_brief_action(args: &str, current: bool) -> Result<BriefAction> {
    let trimmed = args.trim();
    if trimmed.is_empty() || trimmed == "toggle" {
        return Ok(BriefAction::Set(!current));
    }
    match trimmed {
        "show" | "status" | "current" => Ok(BriefAction::Show),
        "on" | "enable" | "enabled" => Ok(BriefAction::Set(true)),
        "off" | "disable" | "disabled" => Ok(BriefAction::Set(false)),
        other => Err(WonderError::validation(format!(
            "unknown brief action: {other}"
        ))),
    }
}

fn parse_fast_action(args: &str, current: bool) -> Result<FastAction> {
    let trimmed = args.trim();
    if trimmed.is_empty() || trimmed == "toggle" {
        return Ok(FastAction::Set(!current));
    }
    match trimmed {
        "show" | "status" | "current" => Ok(FastAction::Show),
        "on" | "enable" | "enabled" => Ok(FastAction::Set(true)),
        "off" | "disable" | "disabled" => Ok(FastAction::Set(false)),
        other => Err(WonderError::validation(format!(
            "unknown fast action: {other}"
        ))),
    }
}

fn parse_optimize_tonken_action(args: &str, current: bool) -> Result<OptimizeTonkenAction> {
    let trimmed = args.trim();
    if trimmed.is_empty() || trimmed == "toggle" {
        return Ok(OptimizeTonkenAction::Set(!current));
    }
    match trimmed {
        "show" | "status" | "current" => Ok(OptimizeTonkenAction::Show),
        "on" | "enable" | "enabled" => Ok(OptimizeTonkenAction::Set(true)),
        "off" | "disable" | "disabled" => Ok(OptimizeTonkenAction::Set(false)),
        other => Err(WonderError::validation(format!(
            "unknown optimize-tonken action: {other}"
        ))),
    }
}

fn parse_theme_name(value: &str) -> Result<&'static str> {
    match value {
        "default" => Ok("default"),
        "midnight" => Ok("midnight"),
        "light" => Ok("light"),
        other => Err(WonderError::validation(format!("unknown theme: {other}"))),
    }
}

fn parse_session_color_name(value: &str) -> Result<&'static str> {
    match value {
        "default" | "reset" | "none" | "gray" | "grey" => Ok("default"),
        "red" => Ok("red"),
        "blue" => Ok("blue"),
        "green" => Ok("green"),
        "yellow" => Ok("yellow"),
        "purple" => Ok("purple"),
        "orange" => Ok("orange"),
        "pink" => Ok("pink"),
        "cyan" => Ok("cyan"),
        other => Err(WonderError::validation(format!("unknown color: {other}"))),
    }
}

fn parse_effort_level_name(value: &str) -> Result<Option<&'static str>> {
    match value {
        "auto" | "unset" => Ok(None),
        "low" => Ok(Some("low")),
        "medium" => Ok(Some("medium")),
        "high" => Ok(Some("high")),
        "max" => Ok(Some("max")),
        other => Err(WonderError::validation(format!(
            "unknown effort level: {other}"
        ))),
    }
}

fn theme_name(value: Option<&str>) -> &'static str {
    match value {
        Some("midnight") => "midnight",
        Some("light") => "light",
        _ => "default",
    }
}

fn session_color_name(value: Option<&str>) -> &'static str {
    match value {
        Some("red") => "red",
        Some("blue") => "blue",
        Some("green") => "green",
        Some("yellow") => "yellow",
        Some("purple") => "purple",
        Some("orange") => "orange",
        Some("pink") => "pink",
        Some("cyan") => "cyan",
        _ => "default",
    }
}

fn effort_level_name(value: Option<&str>) -> &'static str {
    match value {
        Some("low") => "low",
        Some("medium") => "medium",
        Some("high") => "high",
        Some("max") => "max",
        _ => "auto",
    }
}

fn brief_mode_label(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

fn fast_mode_label(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

fn render_theme_status(current_theme: &str) -> String {
    let mut lines = vec![
        "## Theme".into(),
        format!("current_theme={current_theme}"),
        String::new(),
    ];
    for (name, description) in theme_catalog() {
        lines.push(format!(
            "- {name}: {description}{}",
            if name == current_theme {
                " (active)"
            } else {
                ""
            }
        ));
    }
    lines.join("\n")
}

fn render_theme_transition(theme: &str) -> String {
    format!("theme={theme}\nstatus=theme updated")
}

fn render_color_status(current_color: &str) -> String {
    let mut lines = vec![
        "## Color".into(),
        format!("current_color={current_color}"),
        String::new(),
        "Available colors: red, blue, green, yellow, purple, orange, pink, cyan, default".into(),
    ];
    for color in [
        "red", "blue", "green", "yellow", "purple", "orange", "pink", "cyan", "default",
    ] {
        lines.push(format!(
            "- {color}{}",
            if color == current_color {
                " (active)"
            } else {
                ""
            }
        ));
    }
    lines.join("\n")
}

fn render_color_transition(color: &str) -> String {
    format!("color={color}\nstatus=color updated")
}

fn render_effort_status(
    current_effort: &str,
    persisted_effort: Option<&str>,
    storage_dir: Option<&Path>,
) -> String {
    let mut lines = vec![
        "## Effort".into(),
        format!("current_effort={current_effort}"),
        format!("persisted_effort={}", effort_level_name(persisted_effort)),
        String::new(),
        "Available levels: low, medium, high, max, auto".into(),
        "- low: quicker and lighter-weight work".into(),
        "- medium: balanced default-style effort".into(),
        "- high: deeper reasoning before answering".into(),
        "- max: ask for the deepest available reasoning mode".into(),
        "- auto: clear the explicit override".into(),
        String::new(),
        "Provider runtime mapping is active: OpenAI/Copilot receive reasoning_effort for high/max, and Claude-family Anthropic requests receive thinking budget configuration.".into(),
    ];
    if storage_dir.is_none() {
        lines.push("Storage is disabled, so changes only apply to the current TUI session.".into());
    }
    lines.join("\n")
}

fn render_effort_transition(
    level: Option<&str>,
    persisted: bool,
    storage_dir: Option<&Path>,
) -> String {
    let effort = effort_level_name(level);
    let mut lines = vec![
        format!("effort_level={effort}"),
        format!("persisted={persisted}"),
        "status=effort updated".into(),
    ];
    if storage_dir.is_none() {
        lines.push("note=storage disabled; effort is session-only".into());
    } else {
        lines.push(
            "note=provider runtime maps high/max effort into provider-specific inference options"
                .into(),
        );
    }
    lines.join("\n")
}

fn render_brief_status(enabled: bool) -> String {
    [
        "## Brief".into(),
        format!("brief_mode={enabled}"),
        format!("current_brief={}", brief_mode_label(enabled)),
        String::new(),
        "The Rust port keeps replies concise by injecting a briefness system prompt into this session.".into(),
        "It does not yet replicate the leak's SendUserMessage-only tooling and hidden plain-text filtering semantics.".into(),
    ]
    .join("\n")
}

fn render_brief_transition(enabled: bool) -> String {
    format!(
        "brief_mode={enabled}\nstatus=brief mode {}\nnote=session prompts now request concise output",
        if enabled { "enabled" } else { "disabled" }
    )
}

fn optimize_token_mode_label(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

fn render_optimize_tonken_status(enabled: bool) -> String {
    [
        "## Optimize Token".into(),
        format!("optimize_token_mode={enabled}"),
        format!(
            "current_optimize_token={}",
            optimize_token_mode_label(enabled)
        ),
        String::new(),
        "When enabled, a system-prompt instruction is injected that tells the model to minimise"
            .into(),
        "output tokens — omitting preambles, filler words, and unnecessary repetition.".into(),
        "Use '/optimize-tonken on' to enable, '/optimize-tonken off' to disable, or".into(),
        "'/optimize-tonken' (no args) to toggle.  '/optimize-token' is also accepted.".into(),
    ]
    .join("\n")
}

fn render_optimize_tonken_transition(enabled: bool) -> String {
    format!(
        "optimize_token_mode={enabled}\nstatus=optimize token mode {}\nnote={}",
        if enabled { "enabled" } else { "disabled" },
        if enabled {
            "session prompts now instruct the model to minimise output tokens"
        } else {
            "token-optimisation mode disabled; model will respond normally"
        },
    )
}

fn render_fast_status(
    current_fast: bool,
    persisted_fast: Option<bool>,
    storage_dir: Option<&Path>,
    report: &wonder_of_u_agent::ProviderStatusReport,
) -> String {
    let resolver = ProviderResolver::builtin();
    let provider = report.provider.as_deref();
    let preferred_fast = provider
        .and_then(|provider_id| resolver.registry().get(provider_id))
        .and_then(|descriptor| descriptor.preferred_fast_model())
        .map(|model| model.id.as_str());

    let mut lines = vec![
        "## Fast".into(),
        format!("fast_mode={current_fast}"),
        format!("current_fast={}", fast_mode_label(current_fast)),
        format!(
            "persisted_fast={}",
            persisted_fast.map(fast_mode_label).unwrap_or("session-only")
        ),
        format!(
            "provider={}",
            provider.unwrap_or("unconfigured")
        ),
        format!(
            "current_model={}",
            report.model.as_deref().unwrap_or("unconfigured")
        ),
        format!(
            "fast_target={}",
            preferred_fast.unwrap_or("unavailable")
        ),
        String::new(),
        "Fast mode remaps the default provider model to the fastest built-in option this Rust port can identify.".into(),
        "Right now that means mini/haiku-style models when the selected provider exposes one.".into(),
        "It does not implement the leak's entitlement, quota, cooldown, or billing-aware fast-mode semantics.".into(),
    ];
    if storage_dir.is_none() {
        lines.push("Storage is disabled, so changes only apply to the current TUI session.".into());
    } else if preferred_fast.is_none() && provider.is_some() {
        lines.push(
            "The selected provider has no known fast-model mapping, so enabling fast mode will not change runtime model selection."
                .into(),
        );
    }
    lines.join("\n")
}

fn render_fast_transition(
    enabled: bool,
    persisted: bool,
    storage_dir: Option<&Path>,
    report: &wonder_of_u_agent::ProviderStatusReport,
) -> String {
    let resolver = ProviderResolver::builtin();
    let provider = report.provider.as_deref();
    let preferred_fast = provider
        .and_then(|provider_id| resolver.registry().get(provider_id))
        .and_then(|descriptor| descriptor.preferred_fast_model())
        .map(|model| model.id.as_str());

    let mut lines = vec![
        format!("fast_mode={enabled}"),
        format!("persisted={persisted}"),
        format!("fast_target={}", preferred_fast.unwrap_or("unavailable")),
        format!(
            "status=fast mode {}",
            if enabled { "enabled" } else { "disabled" }
        ),
    ];
    if storage_dir.is_none() {
        lines.push("note=storage disabled; fast mode is session-only".into());
    } else if enabled {
        lines.push(
            "note=runtime will remap default model selection to the provider's built-in fast model when available"
                .into(),
        );
    } else {
        lines.push("note=runtime will use the configured default model selection again".into());
    }
    if enabled && preferred_fast.is_none() {
        lines.push(
            "warning=the current provider has no known fast-model mapping; runtime behavior will not change until you switch to a supported provider"
                .into(),
        );
    }
    lines.join("\n")
}

fn render_review_enqueue(cwd: &Path, args: &str) -> String {
    let review_target = if args.trim().is_empty() {
        current_pr_number(cwd).unwrap_or_else(|| "list-open-prs".to_string())
    } else {
        sanitize_single_line(args)
    };
    let gh_available = command_available("gh");
    let current_branch = git_command_output(cwd, &["branch", "--show-current"]);
    let pr_url = if review_target == "list-open-prs" {
        None
    } else {
        gh_pr_url(cwd, &review_target)
    };
    let prompt = format!(
        concat!(
            "You are an expert code reviewer. ",
            "Use the detected GitHub PR metadata below. ",
            "If a PR number is available, run `gh pr view <number>` and `gh pr diff <number>`. ",
            "If no PR number is available, run `gh pr list` to show open PRs. ",
            "Then provide a concise but thorough review covering correctness, project conventions, performance, test coverage, and security considerations. ",
            "PR target: {}."
        ),
        review_target
    );
    let mut lines = vec![
        "review_prompt_ready=true".into(),
        format!("review_target={review_target}"),
        format!("gh_available={gh_available}"),
        format!(
            "current_branch={}",
            current_branch.unwrap_or_else(|| "unknown".into())
        ),
        "status=review prompt queued".into(),
        format!("enqueue_prompt={}", sanitize_single_line(&prompt)),
    ];
    if let Some(pr_url) = pr_url {
        lines.push(format!("pr_url={pr_url}"));
    }
    lines.join("\n")
}

#[derive(Debug, Clone)]
struct CommandCapture {
    success: bool,
    status_code: Option<i32>,
    stdout: String,
}

#[derive(Debug, Clone)]
struct CommitRepoState {
    git_root: String,
    current_branch: String,
    default_branch: Option<String>,
    status_short: String,
    recent_commits: String,
    repo_dirty: bool,
    branch_has_changes: bool,
    origin_remote: Option<String>,
    gh_available: bool,
    existing_pr: Option<String>,
}

fn render_commit_enqueue(cwd: &Path, mode: PermissionMode, args: &str) -> String {
    let Some(state) = inspect_commit_repo_state(cwd) else {
        return [
            "commit_prompt_ready=false".into(),
            format!("permission_mode={}", permission_mode_label(mode)),
            "git_repository=false".into(),
            "status=commit prompt unavailable".into(),
            "note=current working directory is not a readable git worktree".into(),
        ]
        .join("\n");
    };

    let mut lines = commit_state_lines("commit", &state, mode);
    if let Some(note) = git_mutation_block_reason(mode) {
        lines.push("commit_prompt_ready=false".into());
        lines.push("git_mutations_allowed=false".into());
        lines.push("status=commit blocked by permission mode".into());
        lines.push(format!("note={note}"));
        return lines.join("\n");
    }
    if !state.repo_dirty {
        lines.push("commit_prompt_ready=false".into());
        lines.push("git_mutations_allowed=true".into());
        lines.push("status=no changes to commit".into());
        lines.push(
            "note=working tree is clean; the Rust port will not queue an empty commit".into(),
        );
        return lines.join("\n");
    }

    let prompt = build_commit_prompt(&state, args);
    lines.push("commit_prompt_ready=true".into());
    lines.push(format!(
        "git_mutations_require_confirmation={}",
        !matches!(mode, PermissionMode::BypassPermissions)
    ));
    lines.push("status=commit prompt queued".into());
    lines.push(
        "note=the queued prompt must use normal tool permissions for git add and git commit".into(),
    );
    lines.push(format!("enqueue_prompt={}", sanitize_single_line(&prompt)));
    lines.join("\n")
}

fn render_commit_push_pr_enqueue(cwd: &Path, mode: PermissionMode, args: &str) -> String {
    let Some(state) = inspect_commit_repo_state(cwd) else {
        return [
            "commit_push_pr_prompt_ready=false".into(),
            format!("permission_mode={}", permission_mode_label(mode)),
            "git_repository=false".into(),
            "status=commit/push/pr prompt unavailable".into(),
            "note=current working directory is not a readable git worktree".into(),
        ]
        .join("\n");
    };

    let backend_supported = state.origin_remote.is_some() && state.gh_available;
    let mut lines = commit_state_lines("commit-push-pr", &state, mode);
    lines.push(format!("push_supported={}", state.origin_remote.is_some()));
    lines.push(format!("pr_backend={}", pr_backend_label(&state)));
    lines.push(format!("pr_creation_supported={backend_supported}"));

    if let Some(note) = git_mutation_block_reason(mode) {
        lines.push("commit_push_pr_prompt_ready=false".into());
        lines.push("git_mutations_allowed=false".into());
        lines.push("status=commit/push/pr blocked by permission mode".into());
        lines.push(format!("note={note}"));
        return lines.join("\n");
    }

    if !state.repo_dirty && !state.branch_has_changes {
        lines.push("commit_push_pr_prompt_ready=false".into());
        lines.push("git_mutations_allowed=true".into());
        lines.push("status=no local changes or branch diff to publish".into());
        lines.push("note=the working tree is clean and there is no diff against the detected default branch".into());
        return lines.join("\n");
    }

    let prompt = if backend_supported {
        build_commit_push_pr_prompt(&state, args)
    } else {
        build_commit_push_pr_deferred_prompt(&state, args)
    };
    lines.push("commit_push_pr_prompt_ready=true".into());
    lines.push(format!(
        "git_mutations_require_confirmation={}",
        !matches!(mode, PermissionMode::BypassPermissions)
    ));
    if backend_supported {
        lines.push("status=commit/push/pr prompt queued".into());
        lines.push(
            "note=the queued prompt must respect normal tool permissions for git and gh mutations"
                .into(),
        );
    } else {
        lines.push("status=validation-only prompt queued".into());
        lines.push("note=push and PR creation are deferred because the local environment lacks a supported origin remote or gh backend".into());
    }
    lines.push(format!("enqueue_prompt={}", sanitize_single_line(&prompt)));
    lines.join("\n")
}

fn current_pr_number(cwd: &Path) -> Option<String> {
    if !command_available("gh") {
        return None;
    }
    let output = ProcessCommand::new("gh")
        .args(["pr", "view", "--json", "number", "--jq", ".number"])
        .current_dir(cwd)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn gh_pr_url(cwd: &Path, number: &str) -> Option<String> {
    if !command_available("gh") {
        return None;
    }
    let output = ProcessCommand::new("gh")
        .args(["pr", "view", number, "--json", "url", "--jq", ".url"])
        .current_dir(cwd)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn command_available(name: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|path| path.join(name).is_file())
}

fn capture_command(cwd: &Path, program: &str, args: &[&str]) -> Option<CommandCapture> {
    let output = ProcessCommand::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    Some(CommandCapture {
        success: output.status.success(),
        status_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
    })
}

fn capture_git(cwd: &Path, args: &[&str]) -> Option<CommandCapture> {
    capture_command(cwd, "git", args)
}

fn inspect_commit_repo_state(cwd: &Path) -> Option<CommitRepoState> {
    let git_root = capture_git(cwd, &["rev-parse", "--show-toplevel"])?;
    if !git_root.success {
        return None;
    }
    let git_root = (!git_root.stdout.is_empty()).then_some(git_root.stdout)?;
    let current_branch = detect_git_branch(cwd).unwrap_or_else(|| "detached-head".into());
    let default_branch = detect_default_branch(cwd);
    let status_short = capture_git(cwd, &["status", "--short", "--branch"])
        .filter(|capture| capture.success)
        .map(|capture| capture.stdout)
        .unwrap_or_else(|| "unavailable".into());
    let repo_dirty = capture_git(cwd, &["status", "--porcelain"])
        .filter(|capture| capture.success)
        .is_some_and(|capture| !capture.stdout.is_empty());
    let recent_commits = capture_git(cwd, &["log", "--oneline", "-10"])
        .filter(|capture| capture.success)
        .map(|capture| {
            if capture.stdout.is_empty() {
                "unavailable".into()
            } else {
                capture.stdout
            }
        })
        .unwrap_or_else(|| "unavailable".into());
    let branch_has_changes = default_branch
        .as_deref()
        .and_then(|branch| git_range_has_changes(cwd, &format!("{branch}...HEAD")))
        .unwrap_or(repo_dirty);
    let gh_available = command_available("gh");
    Some(CommitRepoState {
        git_root,
        current_branch,
        default_branch,
        status_short,
        recent_commits,
        repo_dirty,
        branch_has_changes,
        origin_remote: git_command_output(cwd, &["remote", "get-url", "origin"]),
        existing_pr: gh_available.then(|| current_pr_number(cwd)).flatten(),
        gh_available,
    })
}

fn git_range_has_changes(cwd: &Path, range: &str) -> Option<bool> {
    let capture = capture_git(cwd, &["diff", "--quiet", range])?;
    Some(match (capture.success, capture.status_code) {
        (true, _) => false,
        (false, Some(1)) => true,
        _ => return None,
    })
}

fn detect_default_branch(cwd: &Path) -> Option<String> {
    let origin_head = git_command_output(cwd, &["symbolic-ref", "refs/remotes/origin/HEAD"])
        .and_then(|value| {
            value
                .strip_prefix("refs/remotes/origin/")
                .map(str::to_string)
        });
    origin_head.or_else(|| {
        ["main", "master"]
            .into_iter()
            .find(|branch| {
                capture_git(
                    cwd,
                    &["show-ref", "--verify", &format!("refs/heads/{branch}")],
                )
                .is_some_and(|capture| capture.success)
            })
            .map(str::to_string)
    })
}

fn commit_state_lines(command: &str, state: &CommitRepoState, mode: PermissionMode) -> Vec<String> {
    vec![
        format!("command={command}"),
        format!("permission_mode={}", permission_mode_label(mode)),
        "git_repository=true".into(),
        format!("git_root={}", state.git_root),
        format!("current_branch={}", state.current_branch),
        format!(
            "default_branch={}",
            state.default_branch.as_deref().unwrap_or("unavailable")
        ),
        format!("repo_dirty={}", state.repo_dirty),
        format!("branch_has_changes={}", state.branch_has_changes),
        format!("gh_available={}", state.gh_available),
        format!(
            "origin_remote={}",
            state.origin_remote.as_deref().unwrap_or("unavailable")
        ),
        format!(
            "existing_pr={}",
            state.existing_pr.as_deref().unwrap_or("none")
        ),
        format!("status_short={}", sanitize_single_line(&state.status_short)),
        format!(
            "recent_commits={}",
            sanitize_single_line(&state.recent_commits)
        ),
    ]
}

fn build_commit_prompt(state: &CommitRepoState, args: &str) -> String {
    let mut prompt = format!(
        concat!(
            "Create a single git commit for the current repository. ",
            "First inspect `git status --short`, `git diff --cached`, `git diff`, and `git log --oneline -10` so the commit matches the pending changes and the repository's recent commit style. ",
            "Current branch: {}. Default branch hint: {}. Current status summary: {}. Recent commits: {}. ",
            "Git safety protocol: never change git config, never skip hooks, never use `git commit --amend`, never create an empty commit, never commit likely secrets, and never use interactive git flags. ",
            "Stage only the relevant files and create exactly one new commit with heredoc syntax: `git commit -m \"$(cat <<'EOF'\nCommit message here.\nEOF\n)\"`. ",
            "Use normal tool permissions for `git add` and `git commit`; do not bypass approval."
        ),
        state.current_branch,
        state.default_branch.as_deref().unwrap_or("unknown"),
        sanitize_single_line(&state.status_short),
        sanitize_single_line(&state.recent_commits),
    );
    append_additional_instructions(&mut prompt, args);
    prompt
}

fn build_commit_push_pr_prompt(state: &CommitRepoState, args: &str) -> String {
    let default_branch = state.default_branch.as_deref().unwrap_or("main");
    let branch_prefix = preferred_branch_prefix();
    let mut prompt = format!(
        concat!(
            "Prepare the current branch for review. ",
            "Inspect `git status --short`, `git diff --cached`, `git diff`, `git log --oneline -10`, and `git diff {}...HEAD` before making changes. ",
            "Current branch: {}. Origin remote: {}. Existing PR: {}. ",
            "If the current branch is `{}`, create a new branch with the prefix `{}/<short-topic>`. ",
            "If the working tree is dirty, stage only the relevant files and create exactly one new commit with heredoc syntax. ",
            "Then push the branch to origin. ",
            "If `gh pr view` reports an existing PR, update it with `gh pr edit`; otherwise create one with `gh pr create`. ",
            "Keep the PR title under 70 characters and put details in the body. ",
            "Git safety protocol: never change git config, never force push, never skip hooks, never use interactive git flags, and do not commit likely secrets. ",
            "Use normal tool permissions for git and gh mutations; do not bypass approval. ",
            "Return the PR URL or explain any remaining blockers."
        ),
        default_branch,
        state.current_branch,
        state.origin_remote.as_deref().unwrap_or("unknown"),
        state.existing_pr.as_deref().unwrap_or("none"),
        default_branch,
        branch_prefix,
    );
    append_additional_instructions(&mut prompt, args);
    prompt
}

fn build_commit_push_pr_deferred_prompt(state: &CommitRepoState, args: &str) -> String {
    let mut prompt = format!(
        concat!(
            "Validate the local branch for a future push/PR handoff. ",
            "Inspect `git status --short`, `git diff --cached`, `git diff`, `git log --oneline -10`, and `git diff {}...HEAD` when available. ",
            "Current branch: {}. Origin remote: {}. gh available: {}. ",
            "If the working tree is dirty, stage only the relevant files and create exactly one new commit with heredoc syntax. ",
            "Do not push and do not attempt PR creation in this run because the local environment does not provide the required remote or gh PR backend. ",
            "Instead, summarize what was validated locally and explicitly call out that push/PR steps remain deferred. ",
            "Git safety protocol: never change git config, never skip hooks, never use `git commit --amend`, never create an empty commit, and do not commit likely secrets. ",
            "Use normal tool permissions for any git mutations."
        ),
        state.default_branch.as_deref().unwrap_or("HEAD"),
        state.current_branch,
        state.origin_remote.as_deref().unwrap_or("unavailable"),
        state.gh_available,
    );
    append_additional_instructions(&mut prompt, args);
    prompt
}

fn append_additional_instructions(prompt: &mut String, args: &str) {
    let trimmed = args.trim();
    if !trimmed.is_empty() {
        prompt.push_str(" Additional user instructions: ");
        prompt.push_str(trimmed);
    }
}

fn preferred_branch_prefix() -> String {
    std::env::var("SAFEUSER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("USER")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "user".into())
}

fn git_mutation_block_reason(mode: PermissionMode) -> Option<&'static str> {
    match mode {
        PermissionMode::Plan => Some(
            "plan mode is read-only for destructive shell mutations, so commit/push actions stay deferred",
        ),
        PermissionMode::DontAsk => Some(
            "dont-ask mode would deny git mutations, so the Rust port does not queue commit or push prompts",
        ),
        PermissionMode::Default
        | PermissionMode::AcceptEdits
        | PermissionMode::BypassPermissions => None,
    }
}

fn pr_backend_label(state: &CommitRepoState) -> &'static str {
    match (state.origin_remote.is_some(), state.gh_available) {
        (true, true) => "github_cli",
        _ => "unsupported",
    }
}

fn render_security_review_enqueue(args: &str) -> String {
    let focus = sanitize_single_line(args.trim());
    let prompt = format!(
        concat!(
            "Perform a security-focused review of the current local branch changes. ",
            "Inspect local git state with `git status --short`, `git diff --stat`, `git diff --cached`, and `git diff`. ",
            "Compare commits against the upstream branch with `git log --oneline @{{upstream}}..HEAD` when available, and fall back to recent local commits if no upstream is configured. ",
            "Report concrete findings first, then note residual risks and missing tests. ",
            "Focus on secrets, auth/authz, input validation, command execution, filesystem access, network exposure, dependency or configuration risk, and unsafe data handling. ",
            "If nothing looks wrong, explicitly say that no security issues were found in the inspected diff. ",
            "Additional focus: {}."
        ),
        if focus.is_empty() {
            "none provided"
        } else {
            focus.as_str()
        }
    );
    [
        "security_review_prompt_ready=true".into(),
        "security_review_scope=local_branch_changes".into(),
        format!(
            "security_review_focus={}",
            if focus.is_empty() { "default" } else { &focus }
        ),
        "status=security review prompt queued".into(),
        "note=the Rust port queues a local security review prompt; it does not integrate with marketplace or plugin security services".into(),
        format!("enqueue_prompt={}", sanitize_single_line(&prompt)),
    ]
    .join("\n")
}

fn render_statusline_enqueue(args: &str) -> String {
    let prompt = if args.trim().is_empty() {
        "Configure my status line from my shell PS1 configuration".to_string()
    } else {
        args.trim().to_string()
    };
    [
        "statusline_prompt_ready=true".into(),
        "status=statusline setup prompt queued".into(),
        "note=the Rust port queues a local setup prompt instead of the leak's remote statusline subagent".into(),
        format!("enqueue_prompt={}", sanitize_single_line(&prompt)),
    ]
    .join("\n")
}

/// Terminals that natively decode Shift+Enter via the Kitty keyboard protocol,
/// so no extra keybinding setup is needed for multi-line input.
const NATIVE_CSIU_TERMINALS: &[&str] =
    &["ghostty", "kitty", "iTerm.app", "WezTerm", "WarpTerminal"];

/// Returns the value of `TERM_PROGRAM`, or `"unknown"` when the variable is
/// not set or is empty.  Used by `render_terminal_setup_notice` to tailor its advice.
pub(crate) fn detect_terminal_type() -> String {
    std::env::var("TERM_PROGRAM")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Returns `true` when `terminal` natively handles the Kitty keyboard protocol
/// (so Shift+Enter already works without any extra setup).
fn is_native_csiu_terminal(terminal: &str) -> bool {
    NATIVE_CSIU_TERMINALS
        .iter()
        .any(|&t| t.eq_ignore_ascii_case(terminal))
}

/// Returns a short human-readable recommendation for the given terminal.
fn terminal_setup_recommendation(terminal: &str) -> &'static str {
    match terminal {
        "Apple_Terminal" => {
            "Use Option+Enter (⌥↵) for newlines. Shift+Enter is not supported in Apple Terminal."
        }
        "vscode" | "cursor" | "windsurf" => {
            "Use Shift+Enter for newlines. Your editor's built-in terminal handles the keybinding."
        }
        t if is_native_csiu_terminal(t) => {
            "Your terminal natively supports Shift+Enter via the Kitty keyboard protocol — no extra setup needed."
        }
        _ => {
            "Use `/keybindings open` to add a Shift+Enter → insert_newline keybinding to the config file."
        }
    }
}

fn render_terminal_setup_notice(storage_dir: Option<&Path>) -> String {
    let path = resolve_keybindings_path(storage_dir);
    let terminal = detect_terminal_type();
    let setup_needed = !is_native_csiu_terminal(&terminal)
        && terminal != "vscode"
        && terminal != "cursor"
        && terminal != "windsurf";
    let recommendation = terminal_setup_recommendation(&terminal);

    [
        "## Terminal Setup".into(),
        format!("terminal_type={terminal}"),
        format!("setup_needed={setup_needed}"),
        recommendation.into(),
        String::new(),
        "Use `/keybindings` to inspect the active shortcuts.".into(),
        format!(
            "Use `/keybindings open` to create or edit {} with the starter template, which includes a Shift+Enter -> insert_newline example.",
            path.display()
        ),
        "status=terminal keybinding template ready".into(),
    ]
    .join("\n")
}

fn persisted_effort(storage_dir: Option<&Path>) -> Result<Option<String>> {
    let Some(storage_dir) = storage_dir else {
        return Ok(None);
    };
    Ok(SettingsStore::new(storage_dir).read()?.effort_level)
}

fn persisted_fast(storage_dir: Option<&Path>) -> Result<Option<bool>> {
    let Some(storage_dir) = storage_dir else {
        return Ok(None);
    };
    Ok(Some(SettingsStore::new(storage_dir).read()?.fast_mode))
}

fn write_persisted_effort(storage_dir: Option<&Path>, level: Option<&str>) -> Result<bool> {
    let Some(storage_dir) = storage_dir else {
        return Ok(false);
    };
    let store = SettingsStore::new(storage_dir);
    let mut settings = store.read()?;
    settings.effort_level = level.map(ToString::to_string);
    store.write(&settings)?;
    Ok(true)
}

fn write_persisted_fast(storage_dir: Option<&Path>, enabled: bool) -> Result<bool> {
    let Some(storage_dir) = storage_dir else {
        return Ok(false);
    };
    let store = SettingsStore::new(storage_dir);
    let mut settings = store.read()?;
    settings.fast_mode = enabled;
    store.write(&settings)?;
    Ok(true)
}

fn render_theme_picker(current_theme: &str) -> String {
    let mut lines = vec![
        "theme_picker=true".into(),
        format!("current_theme={current_theme}"),
    ];
    for (name, description) in theme_catalog() {
        lines.push(format!(
            "theme_option={}",
            json!({
                "theme": name,
                "label": name,
                "description": description,
                "selected": name == current_theme,
            })
        ));
    }
    lines.join("\n")
}

fn theme_catalog() -> [(&'static str, &'static str); 3] {
    [
        (
            "default",
            "Dark shell with blue borders and cyan prompt accents.",
        ),
        (
            "midnight",
            "Deeper dark background with brighter cyan and magenta emphasis.",
        ),
        (
            "light",
            "Light background with dark text and blue status accents.",
        ),
    ]
}

pub(crate) fn load_keybinding_resolver(storage_dir: Option<&Path>) -> Result<KeyBindingResolver> {
    let path = resolve_keybindings_path(storage_dir);
    if !path.exists() {
        return Ok(KeyBindingResolver::new());
    }
    let content = fs::read_to_string(&path)?;
    let bindings = parse_keybinding_overrides(&content).map_err(|error| {
        WonderError::validation(format!(
            "invalid keybindings config `{}`: {error}",
            path.display()
        ))
    })?;
    KeyBindingResolver::with_overrides(bindings).map_err(|error| {
        WonderError::validation(format!(
            "invalid keybindings overrides `{}`: {error}",
            path.display()
        ))
    })
}

fn render_keybindings_summary(resolver: &KeyBindingResolver, storage_dir: Option<&Path>) -> String {
    let mut lines = vec![
        "## Keybindings".into(),
        format!(
            "config_path={}",
            resolve_keybindings_path(storage_dir).display()
        ),
        String::new(),
    ];
    for (context, title) in [
        (KeyBindingContext::Any, "Global"),
        (KeyBindingContext::Prompt, "Prompt"),
        (KeyBindingContext::VimInsert, "Vim insert"),
        (KeyBindingContext::VimNormal, "Vim normal"),
    ] {
        lines.push(format!("### {title}"));
        for line in render_binding_lines(resolver, context) {
            lines.push(format!("- {line}"));
        }
        lines.push(String::new());
    }
    lines.extend([
        "### Dialogs and pickers".into(),
        "- Up / Down: move the current selection".into(),
        "- Enter: confirm or close the active dialog".into(),
        "- Esc: cancel or dismiss the active dialog".into(),
        "- Permission prompt: Enter or y allows, n or Esc denies".into(),
        String::new(),
        "### Vim extras".into(),
        "- 0 / $: jump to line start or line end".into(),
        "- I / A: insert at line start or append at line end".into(),
        "- d{motion} / c{motion}: delete or change by motion".into(),
        "- counts like 3w and 2dw are supported".into(),
    ]);
    lines.join("\n")
}

fn render_binding_lines(resolver: &KeyBindingResolver, context: KeyBindingContext) -> Vec<String> {
    let mut by_key = BTreeMap::new();
    for binding in resolver.bindings() {
        if binding.context == context {
            by_key.insert(
                format_key_event(binding.event),
                format_resolved_key(binding.result),
            );
        }
    }
    if by_key.is_empty() {
        return vec!["(no bindings)".into()];
    }
    by_key
        .into_iter()
        .map(|(key, action)| format!("{key}: {action}"))
        .collect()
}

const PRIVACY_SETTINGS_URL: &str = "https://claude.ai/settings/data-privacy-controls";

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct HooksConfig {
    #[serde(default)]
    disable_all_hooks: bool,
    #[serde(default)]
    allow_managed_hooks_only: bool,
    #[serde(default)]
    hooks: BTreeMap<String, Vec<HookMatcherConfig>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct HookMatcherConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    matcher: Option<String>,
    #[serde(default)]
    hooks: Vec<HookActionConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HookActionConfig {
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell: Option<String>,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
    },
    Prompt {
        prompt: String,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
    },
    Agent {
        prompt: String,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
    },
    Http {
        url: String,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
    },
}

fn render_hooks_summary(storage_dir: Option<&Path>, tool_specs: &[ToolSpec]) -> Result<String> {
    let path = resolve_hooks_path(storage_dir);
    let config = read_hooks_config(&path)?;
    let matcher_count = config.hooks.values().map(Vec::len).sum::<usize>();
    let hook_count = config
        .hooks
        .values()
        .flat_map(|matchers| matchers.iter())
        .map(|matcher| matcher.hooks.len())
        .sum::<usize>();
    let mut lines = vec![
        "## Hooks".into(),
        format!("config_path={}", path.display()),
        format!("available_tools={}", tool_specs.len()),
        format!("events={}", config.hooks.len()),
        format!("matchers={matcher_count}"),
        format!("hooks={hook_count}"),
        format!("disable_all_hooks={}", config.disable_all_hooks),
        format!(
            "allow_managed_hooks_only={}",
            config.allow_managed_hooks_only
        ),
    ];
    if config.hooks.is_empty() {
        lines.push("No hooks configured yet.".into());
    } else {
        for (event, matchers) in &config.hooks {
            let configured_hooks = matchers
                .iter()
                .map(|matcher| matcher.hooks.len())
                .sum::<usize>();
            lines.push(format!(
                "- {event}: {} matcher(s), {configured_hooks} hook(s)",
                matchers.len()
            ));
            if let Some(summary) = hook_event_summary(event) {
                lines.push(format!("  {summary}"));
            }
        }
    }
    lines.push(String::new());
    lines.push(format!(
        "Edit {} with `/hooks open` to review or update the config template.",
        path.display()
    ));
    lines.push(
        "Note: hook execution is not yet wired into the Rust runtime; this command currently provides config parity only."
            .into(),
    );
    Ok(lines.join("\n"))
}

fn hooks_open_output(context: &CommandContext, storage_dir: Option<&Path>) -> Result<String> {
    let path = resolve_hooks_path(storage_dir);
    let existed = path.exists();
    ensure_hooks_file(&path)?;
    if context.interactive {
        let mut lines = vec![
            "hooks=ready".into(),
            format!("hooks_path={}", path.display()),
            format!("created={}", !existed),
        ];
        if plan_editor_command().is_some() {
            lines.push("open_external=true".into());
            lines.push(format!("external_path={}", path.display()));
            lines.push("status=opening hooks config".into());
        } else {
            lines.push("note=set VISUAL or EDITOR to enable `/hooks open`".into());
        }
        return Ok(lines.join("\n"));
    }
    let Some((editor, args)) = plan_editor_command() else {
        return Ok(format!(
            concat!(
                "hooks=ready\n",
                "hooks_path={}\n",
                "created={}\n",
                "note=set VISUAL or EDITOR to enable `/hooks open`"
            ),
            path.display(),
            !existed,
        ));
    };
    let status = ProcessCommand::new(&editor)
        .args(args)
        .arg(&path)
        .current_dir(&context.cwd)
        .status()?;
    Ok(format!(
        concat!(
            "hooks=ready\n",
            "hooks_path={}\n",
            "created={}\n",
            "editor={}\n",
            "editor_success={}"
        ),
        path.display(),
        !existed,
        editor,
        status.success(),
    ))
}

fn render_privacy_settings_summary(browser_launch_attempted: bool) -> String {
    [
        "## Privacy Settings".into(),
        format!("privacy_settings_url={PRIVACY_SETTINGS_URL}"),
        format!("browser_launch_attempted={browser_launch_attempted}"),
        "status=privacy settings opened".into(),
        format!("Review and manage your privacy settings at {PRIVACY_SETTINGS_URL}"),
    ]
    .join("\n")
}

fn keybindings_open_output(context: &CommandContext, storage_dir: Option<&Path>) -> Result<String> {
    let path = resolve_keybindings_path(storage_dir);
    let existed = path.exists();
    ensure_keybindings_file(&path)?;
    if context.interactive {
        let mut lines = vec![
            "keybindings=ready".into(),
            format!("keybindings_path={}", path.display()),
            format!("created={}", !existed),
        ];
        if plan_editor_command().is_some() {
            lines.push("open_external=true".into());
            lines.push(format!("external_path={}", path.display()));
            lines.push("status=opening keybindings config".into());
        } else {
            lines.push("note=set VISUAL or EDITOR to enable `/keybindings open`".into());
        }
        return Ok(lines.join("\n"));
    }
    let Some((editor, args)) = plan_editor_command() else {
        return Ok(format!(
            concat!(
                "keybindings=ready\n",
                "keybindings_path={}\n",
                "created={}\n",
                "note=set VISUAL or EDITOR to enable `/keybindings open`"
            ),
            path.display(),
            !existed,
        ));
    };
    let status = ProcessCommand::new(&editor)
        .args(args)
        .arg(&path)
        .current_dir(&context.cwd)
        .status()?;
    Ok(format!(
        concat!(
            "keybindings=ready\n",
            "keybindings_path={}\n",
            "created={}\n",
            "editor={}\n",
            "editor_success={}"
        ),
        path.display(),
        !existed,
        editor,
        status.success(),
    ))
}

fn resolve_keybindings_path(storage_dir: Option<&Path>) -> PathBuf {
    storage_dir
        .map(StoragePaths::new)
        .map(|paths| paths.config_dir())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".wonder-of-u")
                .join("config")
        })
        .join("keybindings.json")
}

fn resolve_hooks_path(storage_dir: Option<&Path>) -> PathBuf {
    storage_dir
        .map(StoragePaths::new)
        .map(|paths| paths.config_dir())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".wonder-of-u")
                .join("config")
        })
        .join("hooks.json")
}

fn ensure_keybindings_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        fs::write(path, render_keybindings_template())?;
    } else {
        OpenOptions::new().create(true).append(true).open(path)?;
    }
    Ok(())
}

fn ensure_hooks_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        fs::write(path, render_hooks_template())?;
    } else {
        OpenOptions::new().create(true).append(true).open(path)?;
    }
    Ok(())
}

fn read_hooks_config(path: &Path) -> Result<HooksConfig> {
    if !path.exists() {
        return Ok(HooksConfig::default());
    }
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(|error| {
        WonderError::validation(format!(
            "invalid hooks config `{}`: {error}",
            path.display()
        ))
    })
}

fn render_keybindings_template() -> String {
    serde_json::to_string_pretty(&json!({
        "bindings": [
            {
                "context": "prompt",
                "key": "enter",
                "modifiers": ["shift"],
                "action": {
                    "kind": "edit",
                    "action": "insert_newline"
                }
            },
            {
                "context": "vim_normal",
                "key": "q",
                "action": {
                    "kind": "vim",
                    "action": "cancel_pending"
                }
            }
        ]
    }))
    .unwrap_or_else(|_| "{\"bindings\":[]}".into())
        + "\n"
}

fn render_hooks_template() -> String {
    serde_json::to_string_pretty(&json!({
        "disable_all_hooks": false,
        "allow_managed_hooks_only": false,
        "hooks": {
            "PreToolUse": [
                {
                    "matcher": "bash",
                    "hooks": [
                        {
                            "type": "command",
                            "command": "echo auditing bash tool invocation"
                        }
                    ]
                }
            ],
            "Notification": [
                {
                    "matcher": "auth_success",
                    "hooks": [
                        {
                            "type": "http",
                            "url": "https://example.invalid/hooks/notify"
                        }
                    ]
                }
            ]
        }
    }))
    .unwrap_or_else(|_| "{\"hooks\":{}}".into())
        + "\n"
}

fn hook_event_summary(event: &str) -> Option<&'static str> {
    match event {
        "PreToolUse" => Some("Before tool execution; match on tool_name."),
        "PostToolUse" => Some("After tool execution; match on tool_name."),
        "PostToolUseFailure" => Some("After a tool fails; match on tool_name."),
        "PermissionDenied" => Some("After auto mode denies a tool call; match on tool_name."),
        "Notification" => Some("When notifications are sent; match on notification_type."),
        "UserPromptSubmit" => Some("When the user submits a prompt."),
        "SessionStart" => Some("When a session starts; match on source."),
        "Stop" => Some("Right before the assistant concludes a response."),
        "StopFailure" => Some("When a turn ends because of an API error."),
        "SubagentStart" => Some("When a subagent starts; match on agent_type."),
        "SubagentStop" => Some("Right before a subagent concludes; match on agent_type."),
        "PreCompact" => Some("Before compaction; match on trigger."),
        "PostCompact" => Some("After compaction; match on trigger."),
        "SessionEnd" => Some("When a session ends; match on reason."),
        "PermissionRequest" => Some("When a permission dialog is displayed; match on tool_name."),
        "Setup" => Some("Repo setup hooks for init and maintenance."),
        "TeammateIdle" => Some("When a teammate is about to go idle."),
        "TaskCreated" => Some("When a task is created."),
        "TaskCompleted" => Some("When a task is completed."),
        "Elicitation" => Some("When an MCP server requests user input."),
        "ElicitationResult" => Some("After a user responds to an MCP elicitation."),
        "ConfigChange" => Some("When configuration files change during a session."),
        "InstructionsLoaded" => Some("When an instruction file is loaded."),
        "WorktreeCreate" => Some("When a worktree should be created."),
        "WorktreeRemove" => Some("When a worktree should be removed."),
        "CwdChanged" => Some("After the working directory changes."),
        "FileChanged" => Some("When a watched file changes."),
        _ => None,
    }
}

fn parse_keybinding_overrides(content: &str) -> std::result::Result<Vec<KeyBinding>, String> {
    let value: Value = serde_json::from_str(content).map_err(|error| error.to_string())?;
    let bindings = value
        .get("bindings")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing `bindings` array".to_string())?;
    bindings
        .iter()
        .enumerate()
        .map(|(index, entry)| parse_keybinding_entry(index, entry))
        .collect()
}

fn parse_keybinding_entry(index: usize, value: &Value) -> std::result::Result<KeyBinding, String> {
    let context = parse_keybinding_context(
        value
            .get("context")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("binding[{index}] missing `context`"))?,
    )?;
    let event = KeyEvent {
        code: parse_key_code(
            value
                .get("key")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("binding[{index}] missing `key`"))?,
        )?,
        modifiers: parse_key_modifiers(value.get("modifiers"))?,
    };
    let result = parse_keybinding_result(
        value
            .get("action")
            .ok_or_else(|| format!("binding[{index}] missing `action`"))?,
    )?;
    Ok(KeyBinding {
        context,
        event,
        result,
    })
}

fn parse_keybinding_context(value: &str) -> std::result::Result<KeyBindingContext, String> {
    match value {
        "any" | "global" => Ok(KeyBindingContext::Any),
        "prompt" => Ok(KeyBindingContext::Prompt),
        "vim_insert" | "insert" => Ok(KeyBindingContext::VimInsert),
        "vim_normal" | "normal" => Ok(KeyBindingContext::VimNormal),
        other => Err(format!("unknown binding context `{other}`")),
    }
}

fn parse_key_code(value: &str) -> std::result::Result<KeyCode, String> {
    let lower = value.to_ascii_lowercase();
    match lower.as_str() {
        "backspace" => Ok(KeyCode::Backspace),
        "enter" => Ok(KeyCode::Enter),
        "left" => Ok(KeyCode::Left),
        "right" => Ok(KeyCode::Right),
        "up" => Ok(KeyCode::Up),
        "down" => Ok(KeyCode::Down),
        "home" => Ok(KeyCode::Home),
        "end" => Ok(KeyCode::End),
        "pageup" => Ok(KeyCode::PageUp),
        "pagedown" => Ok(KeyCode::PageDown),
        "tab" => Ok(KeyCode::Tab),
        "backtab" => Ok(KeyCode::BackTab),
        "delete" => Ok(KeyCode::Delete),
        "insert" => Ok(KeyCode::Insert),
        "esc" | "escape" => Ok(KeyCode::Esc),
        _ if value.len() == 1 => Ok(KeyCode::Char(value.chars().next().unwrap_or_default())),
        _ if lower.starts_with('f') => lower[1..]
            .parse::<u8>()
            .map(KeyCode::F)
            .map_err(|_| format!("unknown key `{value}`")),
        _ => Err(format!("unknown key `{value}`")),
    }
}

fn parse_key_modifiers(value: Option<&Value>) -> std::result::Result<KeyModifiers, String> {
    let mut modifiers = KeyModifiers::default();
    let Some(value) = value else {
        return Ok(modifiers);
    };
    let entries = value
        .as_array()
        .ok_or_else(|| "`modifiers` must be an array".to_string())?;
    for modifier in entries {
        match modifier
            .as_str()
            .ok_or_else(|| "modifier entries must be strings".to_string())?
        {
            "control" | "ctrl" => modifiers.control = true,
            "shift" => modifiers.shift = true,
            "alt" => modifiers.alt = true,
            other => return Err(format!("unknown modifier `{other}`")),
        }
    }
    Ok(modifiers)
}

fn parse_keybinding_result(value: &Value) -> std::result::Result<ResolvedKey, String> {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "action missing `kind`".to_string())?;
    match kind {
        "system" => Ok(ResolvedKey::System(parse_system_action(
            value
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "system action missing `action`".to_string())?,
        )?)),
        "edit" => Ok(ResolvedKey::Edit(parse_edit_action(
            value
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "edit action missing `action`".to_string())?,
        )?)),
        "vim" => Ok(ResolvedKey::Vim(parse_vim_action(
            value
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "vim action missing `action`".to_string())?,
        )?)),
        "insert_char" => {
            let ch = value
                .get("char")
                .and_then(Value::as_str)
                .and_then(|raw| raw.chars().next())
                .ok_or_else(|| "insert_char action missing `char`".to_string())?;
            Ok(ResolvedKey::InsertChar(ch))
        }
        other => Err(format!("unknown action kind `{other}`")),
    }
}

fn parse_system_action(value: &str) -> std::result::Result<SystemAction, String> {
    match value {
        "interrupt" => Ok(SystemAction::Interrupt),
        "redraw" => Ok(SystemAction::Redraw),
        "history_search" => Ok(SystemAction::HistorySearch),
        "open_global_search" => Ok(SystemAction::OpenGlobalSearch),
        "expand_tool_output" => Ok(SystemAction::ExpandToolOutput),
        other => Err(format!("unknown system action `{other}`")),
    }
}

fn parse_edit_action(value: &str) -> std::result::Result<EditAction, String> {
    match value {
        "move_left" => Ok(EditAction::Move(Motion::Left)),
        "move_right" => Ok(EditAction::Move(Motion::Right)),
        "move_up" => Ok(EditAction::Move(Motion::Up)),
        "move_down" => Ok(EditAction::Move(Motion::Down)),
        "move_line_start" => Ok(EditAction::Move(Motion::LineStart)),
        "move_line_end" => Ok(EditAction::Move(Motion::LineEnd)),
        "move_word_forward" => Ok(EditAction::Move(Motion::WordForward)),
        "move_word_backward" => Ok(EditAction::Move(Motion::WordBackward)),
        "move_word_end" => Ok(EditAction::Move(Motion::WordEnd)),
        "backspace" => Ok(EditAction::Backspace),
        "delete" => Ok(EditAction::Delete),
        "insert_newline" => Ok(EditAction::InsertNewline),
        other => Err(format!("unknown edit action `{other}`")),
    }
}

fn parse_vim_action(value: &str) -> std::result::Result<TuiVimCommand, String> {
    match value {
        "enter_insert_mode" => Ok(TuiVimCommand::EnterInsertMode),
        "enter_normal_mode" => Ok(TuiVimCommand::EnterNormalMode),
        "append_after_cursor" => Ok(TuiVimCommand::AppendAfterCursor),
        "append_line_end" => Ok(TuiVimCommand::AppendLineEnd),
        "insert_line_start" => Ok(TuiVimCommand::InsertLineStart),
        "delete_char" => Ok(TuiVimCommand::DeleteChar),
        "start_delete" => Ok(TuiVimCommand::StartDelete),
        "start_change" => Ok(TuiVimCommand::StartChange),
        "cancel_pending" => Ok(TuiVimCommand::CancelPending),
        other => Err(format!("unknown vim action `{other}`")),
    }
}

fn format_key_event(event: KeyEvent) -> String {
    let mut parts = Vec::new();
    if event.modifiers.control {
        parts.push("Ctrl".to_string());
    }
    if event.modifiers.alt {
        parts.push("Alt".to_string());
    }
    if event.modifiers.shift {
        parts.push("Shift".to_string());
    }
    parts.push(match event.code {
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PageUp".into(),
        KeyCode::PageDown => "PageDown".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "BackTab".into(),
        KeyCode::Delete => "Delete".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Char(ch) => {
            if (event.modifiers.control || event.modifiers.alt || event.modifiers.shift)
                && ch.is_ascii_alphabetic()
            {
                ch.to_ascii_uppercase().to_string()
            } else {
                ch.to_string()
            }
        }
        KeyCode::F(index) => format!("F{index}"),
        KeyCode::Null => "Null".into(),
    });
    parts.join("-")
}

fn format_resolved_key(result: ResolvedKey) -> String {
    match result {
        ResolvedKey::Edit(action) => match action {
            EditAction::Move(Motion::Left) => "move left".into(),
            EditAction::Move(Motion::Right) => "move right".into(),
            EditAction::Move(Motion::Up) => "move up".into(),
            EditAction::Move(Motion::Down) => "move down".into(),
            EditAction::Move(Motion::LineStart) => "move to line start".into(),
            EditAction::Move(Motion::LineEnd) => "move to line end".into(),
            EditAction::Move(Motion::FirstNonBlank) => "move to first non-blank".into(),
            EditAction::Move(Motion::WordForward) => "move forward by word".into(),
            EditAction::Move(Motion::WordBackward) => "move backward by word".into(),
            EditAction::Move(Motion::WordEnd) => "move to word end".into(),
            EditAction::Backspace => "delete backward".into(),
            EditAction::Delete => "delete forward".into(),
            EditAction::InsertNewline => "submit / insert newline".into(),
            EditAction::InsertLiteralNewline => "insert literal newline".into(),
        },
        ResolvedKey::InsertChar(ch) => format!("insert `{ch}`"),
        ResolvedKey::System(SystemAction::Interrupt) => "interrupt or exit".into(),
        ResolvedKey::System(SystemAction::Redraw) => "redraw terminal".into(),
        ResolvedKey::System(SystemAction::HistorySearch) => "recall previous prompt".into(),
        ResolvedKey::System(SystemAction::OpenGlobalSearch) => "search workspace".into(),
        ResolvedKey::System(SystemAction::ExpandToolOutput) => {
            "expand or collapse tool output".into()
        }
        ResolvedKey::Vim(command) => match command {
            TuiVimCommand::EnterInsertMode => "enter insert mode".into(),
            TuiVimCommand::EnterNormalMode => "enter normal mode".into(),
            TuiVimCommand::AppendAfterCursor => "append after cursor".into(),
            TuiVimCommand::AppendLineEnd => "append at line end".into(),
            TuiVimCommand::InsertLineStart => "insert at line start".into(),
            TuiVimCommand::DeleteChar => "delete char under cursor".into(),
            TuiVimCommand::StartDelete => "start delete operator".into(),
            TuiVimCommand::StartChange => "start change operator".into(),
            TuiVimCommand::CancelPending => "cancel pending operator".into(),
        },
    }
}

fn render_plan_transition(
    mode: PermissionMode,
    status: &str,
    queued_prompt: Option<String>,
) -> String {
    let mut lines = vec![
        format!("permission_mode={}", permission_mode_label(mode)),
        "plan_mode_ready=true".into(),
        format!("plan_mode_active={}", matches!(mode, PermissionMode::Plan)),
        format!("status={status}"),
    ];
    if let Some(queued_prompt) = queued_prompt.filter(|prompt| !prompt.trim().is_empty()) {
        lines.push(format!(
            "enqueue_prompt={}",
            sanitize_single_line(&queued_prompt)
        ));
    }
    lines.join("\n")
}

fn parse_plan_action(args: &str, current_mode: PermissionMode) -> Result<PlanAction> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok(if matches!(current_mode, PermissionMode::Plan) {
            PlanAction::Show
        } else {
            PlanAction::Enter {
                queued_prompt: None,
            }
        });
    }
    let tokens = shell_words::split(trimmed)
        .map_err(|error| WonderError::validation(format!("invalid /plan arguments: {error}")))?;
    match tokens.first().map(String::as_str) {
        Some("show") if tokens.len() == 1 => Ok(PlanAction::Show),
        Some("exit") if tokens.len() == 1 => Ok(PlanAction::Exit),
        Some("open") if tokens.len() == 1 => Ok(PlanAction::Open),
        Some("enter") => {
            let remainder = trimmed.strip_prefix("enter").unwrap_or_default().trim();
            Ok(PlanAction::Enter {
                queued_prompt: (!remainder.is_empty()).then(|| remainder.to_string()),
            })
        }
        Some("show" | "exit" | "open") => Err(WonderError::validation(
            "`/plan show`, `/plan exit`, and `/plan open` do not take additional arguments",
        )),
        _ => Ok(if matches!(current_mode, PermissionMode::Plan) {
            PlanAction::Show
        } else {
            PlanAction::Enter {
                queued_prompt: Some(trimmed.to_string()),
            }
        }),
    }
}

fn resolve_plan_path(cwd: &Path) -> PathBuf {
    for ancestor in cwd.ancestors() {
        let candidate = ancestor.join("plan.md");
        if candidate.is_file() {
            return candidate;
        }
    }
    cwd.join("plan.md")
}

fn render_plan_display(mode: PermissionMode, plan_path: &Path) -> String {
    let active = matches!(mode, PermissionMode::Plan);
    let mut lines = vec![
        format!("permission_mode={}", permission_mode_label(mode)),
        "plan_mode_ready=true".into(),
        format!("plan_mode_active={active}"),
        format!("plan_path={}", plan_path.display()),
    ];
    match fs::read_to_string(plan_path) {
        Ok(content) => {
            lines.push("plan_exists=true".into());
            lines.push(String::new());
            lines.push("Current Plan".into());
            lines.push(plan_path.display().to_string());
            lines.push(String::new());
            lines.push(content.trim_end().to_string());
            if let Some(editor) = plan_editor_command() {
                lines.push(String::new());
                lines.push(format!(
                    "hint=`/plan open` will launch {} when available outside the live TUI",
                    editor.0
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            lines.push("plan_exists=false".into());
            lines.push(if active {
                "note=already in plan mode. no plan written yet".into()
            } else {
                "note=no plan written yet".into()
            });
        }
        Err(error) => {
            lines.push("plan_exists=false".into());
            lines.push(format!("note=failed to read plan: {error}"));
        }
    }
    lines.join("\n")
}

fn open_plan_in_editor(context: &CommandContext) -> Result<CommandOutput> {
    let plan_path = resolve_plan_path(&context.cwd);
    if !plan_path.is_file() {
        return Ok(CommandOutput::Text(render_plan_display(
            context.permission_mode,
            &plan_path,
        )));
    }
    if context.interactive {
        let mut lines = vec![
            format!(
                "permission_mode={}",
                permission_mode_label(context.permission_mode)
            ),
            "plan_mode_ready=true".into(),
            format!(
                "plan_mode_active={}",
                matches!(context.permission_mode, PermissionMode::Plan)
            ),
            format!("plan_path={}", plan_path.display()),
        ];
        if plan_editor_command().is_some() {
            lines.push("plan_open_external=true".into());
            lines.push("note=launching plan in external editor".into());
        } else {
            lines.push("note=set VISUAL or EDITOR to enable `/plan open`".into());
        }
        return Ok(CommandOutput::Text(lines.join("\n")));
    }
    let Some((editor, args)) = plan_editor_command() else {
        return Ok(CommandOutput::Text(format!(
            concat!(
                "permission_mode={}\n",
                "plan_mode_ready=true\n",
                "plan_mode_active={}\n",
                "plan_path={}\n",
                "note=set VISUAL or EDITOR to enable `/plan open`"
            ),
            permission_mode_label(context.permission_mode),
            matches!(context.permission_mode, PermissionMode::Plan),
            plan_path.display(),
        )));
    };
    let status = ProcessCommand::new(&editor)
        .args(args)
        .arg(&plan_path)
        .current_dir(&context.cwd)
        .status()?;
    Ok(CommandOutput::Text(format!(
        concat!(
            "permission_mode={}\n",
            "plan_mode_ready=true\n",
            "plan_mode_active={}\n",
            "plan_path={}\n",
            "editor={}\n",
            "editor_success={}"
        ),
        permission_mode_label(context.permission_mode),
        matches!(context.permission_mode, PermissionMode::Plan),
        plan_path.display(),
        editor,
        status.success(),
    )))
}

fn plan_editor_command() -> Option<(String, Vec<String>)> {
    let raw = std::env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })?;
    let mut tokens = shell_words::split(&raw).ok()?;
    let command = tokens.first()?.clone();
    Some((command, tokens.drain(1..).collect()))
}

fn render_runtime_disabled(kind: &str, note: &str) -> String {
    format!("{kind}=0\nstorage_dir=disabled\nnote={note}")
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::{fs, path::Path, process::Command as ProcessCommand};

    use futures::executor::block_on;
    use wonder_of_u_agent::{AgentSettings, ProviderResolver, SettingsStore};
    use wonder_of_u_core::{
        Command, CommandContext, CommandInvocation, CommandOutput, FeatureSet, PermissionMode,
        SessionId,
    };
    use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

    use super::{
        CommitCommand, CommitPushPrCommand, OptimizeTonkenAction, PlanAction,
        PrivacySettingsCommand, SecurityReviewCommand, TerminalSetupCommand, detect_terminal_type,
        ensure_hooks_file, ensure_keybindings_file, is_native_csiu_terminal,
        load_keybinding_resolver, normalize_color_invocation, normalize_effort_invocation,
        normalize_permissions_invocation, normalize_theme_invocation, normalize_vim_invocation,
        parse_brief_action, parse_effort_level_name, parse_fast_action,
        parse_optimize_tonken_action, parse_plan_action, parse_session_color_name,
        render_brief_status, render_brief_transition, render_color_status, render_color_transition,
        render_commit_enqueue, render_commit_push_pr_enqueue, render_effort_status,
        render_effort_transition, render_fast_status, render_fast_transition, render_hooks_summary,
        render_keybindings_summary, render_optimize_tonken_status,
        render_optimize_tonken_transition, render_plan_display, render_privacy_settings_summary,
        render_review_enqueue, render_security_review_enqueue, render_statusline_enqueue,
        render_terminal_setup_notice, render_theme_status, resolve_hooks_path,
        resolve_keybindings_path, resolve_plan_path, terminal_setup_recommendation,
        write_persisted_effort, write_persisted_fast,
    };

    fn test_context(cwd: &Path) -> CommandContext {
        CommandContext {
            session_id: SessionId::new(),
            cwd: cwd.to_path_buf(),
            features: FeatureSet::first_release(),
            authenticated: false,
            interactive: true,
            permission_mode: PermissionMode::Default,
            theme: None,
            session_color: None,
            effort_level: None,
            brief_mode: false,
            fast_mode: false,
            optimize_token_mode: false,
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        }
    }

    #[test]
    fn plan_defaults_to_enter_outside_plan_mode_and_show_inside() {
        assert_eq!(
            parse_plan_action("", PermissionMode::Default).expect("default plan action"),
            PlanAction::Enter {
                queued_prompt: None
            }
        );
        assert_eq!(
            parse_plan_action("", PermissionMode::Plan).expect("plan-mode plan action"),
            PlanAction::Show
        );
        assert_eq!(
            parse_plan_action(
                "draft a detailed migration checklist",
                PermissionMode::Default
            )
            .expect("plan prompt action"),
            PlanAction::Enter {
                queued_prompt: Some("draft a detailed migration checklist".into())
            }
        );
    }

    #[test]
    fn plan_display_reads_nearest_plan_file() {
        let dir = unique_test_dir("workflow-plan-display");
        let nested = dir.join("nested/project");
        std::fs::create_dir_all(&nested).expect("create nested");
        let plan_path = dir.join("plan.md");
        std::fs::write(&plan_path, "# test plan\n- keep `/plan` parity\n").expect("write plan");

        let resolved = resolve_plan_path(&nested);
        assert_eq!(resolved, plan_path);

        let rendered = render_plan_display(PermissionMode::Plan, &resolved);
        assert!(rendered.contains("plan_exists=true"));
        assert!(rendered.contains("Current Plan"));
        assert!(rendered.contains("- keep `/plan` parity"));
    }

    #[test]
    fn permissions_shorthand_normalizes_to_set_subcommand() {
        let invocation = CommandInvocation {
            name: "permissions".into(),
            args: "accept-edits".into(),
            raw: "/permissions accept-edits".into(),
        };

        let normalized = normalize_permissions_invocation(&invocation);

        assert_eq!(normalized.args, "set accept-edits");
    }

    #[test]
    fn vim_shorthand_normalizes_to_set_subcommand() {
        let invocation = CommandInvocation {
            name: "vim".into(),
            args: "normal".into(),
            raw: "/vim normal".into(),
        };

        let normalized = normalize_vim_invocation(&invocation);

        assert_eq!(normalized.args, "set normal");
    }

    #[test]
    fn keybindings_summary_lists_supported_sections() {
        let rendered =
            render_keybindings_summary(&load_keybinding_resolver(None).expect("resolver"), None);

        assert!(rendered.contains("## Keybindings"));
        assert!(rendered.contains("### Global"));
        assert!(rendered.contains("### Prompt"));
        assert!(rendered.contains("### Vim insert"));
        assert!(rendered.contains("### Dialogs and pickers"));
        assert!(rendered.contains("### Vim normal"));
        assert!(rendered.contains("Ctrl-R"));
        assert!(rendered.contains("d{motion} / c{motion}"));
    }

    #[test]
    fn keybindings_template_can_be_loaded_as_overrides() {
        let dir = unique_test_dir("workflow-keybindings-template");
        let path = dir.join("config/keybindings.json");
        ensure_keybindings_file(&path).expect("write template");

        let resolver = load_keybinding_resolver(Some(dir.as_path())).expect("load resolver");
        let rendered = render_keybindings_summary(&resolver, Some(dir.as_path()));

        assert!(rendered.contains("Shift-Enter"));
        assert!(rendered.contains("q: cancel pending operator"));
    }

    #[test]
    fn keybindings_open_hints_external_editor_when_interactive() {
        let dir = unique_test_dir("workflow-keybindings-open");
        let _editor = EnvVarGuard::set("EDITOR", "true");
        let context = CommandContext {
            session_id: SessionId::new(),
            cwd: dir.clone(),
            features: FeatureSet::default(),
            authenticated: false,
            interactive: true,
            permission_mode: PermissionMode::Default,
            theme: None,
            session_color: None,
            effort_level: None,
            brief_mode: false,
            fast_mode: false,
            optimize_token_mode: false,
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        };
        let output =
            super::keybindings_open_output(&context, Some(dir.as_path())).expect("open output");

        assert!(output.contains("open_external=true"));
        assert!(output.contains(&format!(
            "external_path={}",
            resolve_keybindings_path(Some(dir.as_path())).display()
        )));
    }

    #[test]
    fn hooks_summary_reports_template_events() {
        let dir = unique_test_dir("workflow-hooks-template");
        let path = dir.join("config/hooks.json");
        ensure_hooks_file(&path).expect("write hooks template");

        let rendered = render_hooks_summary(Some(dir.as_path()), &[]).expect("render hooks");

        assert!(rendered.contains("## Hooks"));
        assert!(rendered.contains("events=2"));
        assert!(rendered.contains("PreToolUse"));
        assert!(rendered.contains("Notification"));
        assert!(rendered.contains("config parity only"));
    }

    #[test]
    fn hooks_open_hints_external_editor_when_interactive() {
        let dir = unique_test_dir("workflow-hooks-open");
        let _editor = EnvVarGuard::set("EDITOR", "true");
        let context = CommandContext {
            session_id: SessionId::new(),
            cwd: dir.clone(),
            features: FeatureSet::default(),
            authenticated: false,
            interactive: true,
            permission_mode: PermissionMode::Default,
            theme: None,
            session_color: None,
            effort_level: None,
            brief_mode: false,
            fast_mode: false,
            optimize_token_mode: false,
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        };
        let output = super::hooks_open_output(&context, Some(dir.as_path())).expect("open output");

        assert!(output.contains("open_external=true"));
        assert!(output.contains(&format!(
            "external_path={}",
            resolve_hooks_path(Some(dir.as_path())).display()
        )));
    }

    #[test]
    fn privacy_settings_summary_points_to_web_controls() {
        let rendered = render_privacy_settings_summary(false);

        assert!(rendered.contains("## Privacy Settings"));
        assert!(rendered.contains("status=privacy settings opened"));
        assert!(rendered.contains("browser_launch_attempted=false"));
        assert!(rendered.contains("https://claude.ai/settings/data-privacy-controls"));
    }

    #[test]
    fn privacy_settings_command_does_not_launch_browser_under_tests() {
        let output = block_on(PrivacySettingsCommand::new().execute(
            test_context(Path::new("/workspace")),
            CommandInvocation {
                name: "privacy-settings".into(),
                args: String::new(),
                raw: "/privacy-settings".into(),
            },
        ))
        .expect("run privacy settings command");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };

        assert!(text.contains("browser_launch_attempted=false"));
    }

    #[test]
    fn theme_shorthand_normalizes_to_set_subcommand() {
        let invocation = CommandInvocation {
            name: "theme".into(),
            args: "midnight".into(),
            raw: "/theme midnight".into(),
        };

        let normalized = normalize_theme_invocation(&invocation);

        assert_eq!(normalized.args, "set midnight");
    }

    #[test]
    fn theme_status_marks_active_theme() {
        let rendered = render_theme_status("midnight");

        assert!(rendered.contains("## Theme"));
        assert!(rendered.contains("current_theme=midnight"));
        assert!(rendered.contains("midnight: Deeper dark background"));
        assert!(rendered.contains("(active)"));
    }

    #[test]
    fn color_shorthand_normalizes_to_set_subcommand() {
        let invocation = CommandInvocation {
            name: "color".into(),
            args: "purple".into(),
            raw: "/color purple".into(),
        };

        let normalized = normalize_color_invocation(&invocation);

        assert_eq!(normalized.args, "set purple");
    }

    #[test]
    fn color_status_marks_active_color() {
        let rendered = render_color_status("cyan");

        assert!(rendered.contains("## Color"));
        assert!(rendered.contains("current_color=cyan"));
        assert!(rendered.contains("- cyan (active)"));
    }

    #[test]
    fn color_transition_accepts_reset_aliases() {
        let rendered = render_color_transition(parse_session_color_name("grey").expect("color"));

        assert_eq!(rendered, "color=default\nstatus=color updated");
    }

    #[test]
    fn brief_defaults_to_toggle_and_accepts_show() {
        assert_eq!(
            parse_brief_action("", false).expect("toggle on"),
            super::BriefAction::Set(true)
        );
        assert_eq!(
            parse_brief_action("toggle", true).expect("toggle off"),
            super::BriefAction::Set(false)
        );
        assert_eq!(
            parse_brief_action("show", true).expect("show"),
            super::BriefAction::Show
        );
    }

    #[test]
    fn brief_status_reports_runtime_limitations() {
        let rendered = render_brief_status(true);

        assert!(rendered.contains("## Brief"));
        assert!(rendered.contains("brief_mode=true"));
        assert!(rendered.contains("current_brief=on"));
        assert!(rendered.contains("hidden plain-text filtering semantics"));
    }

    #[test]
    fn brief_transition_reports_new_state() {
        let rendered = render_brief_transition(false);

        assert!(rendered.contains("brief_mode=false"));
        assert!(rendered.contains("status=brief mode disabled"));
    }

    #[test]
    fn optimize_tonken_defaults_to_toggle_and_accepts_show() {
        assert_eq!(
            parse_optimize_tonken_action("", false).expect("toggle on"),
            OptimizeTonkenAction::Set(true)
        );
        assert_eq!(
            parse_optimize_tonken_action("toggle", true).expect("toggle off"),
            OptimizeTonkenAction::Set(false)
        );
        assert_eq!(
            parse_optimize_tonken_action("show", false).expect("show"),
            OptimizeTonkenAction::Show
        );
        assert_eq!(
            parse_optimize_tonken_action("on", false).expect("on"),
            OptimizeTonkenAction::Set(true)
        );
        assert_eq!(
            parse_optimize_tonken_action("off", true).expect("off"),
            OptimizeTonkenAction::Set(false)
        );
    }

    #[test]
    fn optimize_tonken_action_rejects_unknown_arg() {
        assert!(parse_optimize_tonken_action("maybe", false).is_err());
    }

    #[test]
    fn optimize_tonken_status_renders_notice_heading_and_hint_lines() {
        let rendered = render_optimize_tonken_status(true);

        assert!(rendered.starts_with("## Optimize Token"));
        assert!(rendered.contains("optimize_token_mode=true"));
        assert!(rendered.contains("current_optimize_token=on"));
        assert!(rendered.contains("minimise"));
    }

    #[test]
    fn optimize_tonken_status_shows_off_when_disabled() {
        let rendered = render_optimize_tonken_status(false);

        assert!(rendered.contains("optimize_token_mode=false"));
        assert!(rendered.contains("current_optimize_token=off"));
    }

    #[test]
    fn optimize_tonken_transition_reports_enabled_state() {
        let rendered = render_optimize_tonken_transition(true);

        assert!(rendered.contains("optimize_token_mode=true"));
        assert!(rendered.contains("status=optimize token mode enabled"));
        assert!(rendered.contains("minimise output tokens"));
    }

    #[test]
    fn optimize_tonken_transition_reports_disabled_state() {
        let rendered = render_optimize_tonken_transition(false);

        assert!(rendered.contains("optimize_token_mode=false"));
        assert!(rendered.contains("status=optimize token mode disabled"));
        assert!(rendered.contains("respond normally"));
    }

    #[test]
    fn fast_action_defaults_to_toggle_and_accepts_show() {
        assert_eq!(
            parse_fast_action("", false).expect("toggle on"),
            super::FastAction::Set(true)
        );
        assert_eq!(
            parse_fast_action("toggle", true).expect("toggle off"),
            super::FastAction::Set(false)
        );
        assert_eq!(
            parse_fast_action("show", true).expect("show"),
            super::FastAction::Show
        );
    }

    #[test]
    fn fast_status_reports_provider_mapping_limitations() {
        let dir = unique_test_dir("workflow-fast-status");
        let store = SettingsStore::new(&dir);
        store
            .write(&AgentSettings {
                selected_provider: Some("openai".into()),
                fast_mode: true,
                ..AgentSettings::default()
            })
            .expect("write settings");
        let report = ProviderResolver::builtin()
            .load_report(Some(dir.as_path()))
            .expect("load report");

        let rendered = render_fast_status(true, Some(true), Some(dir.as_path()), &report);

        assert!(rendered.contains("## Fast"));
        assert!(rendered.contains("fast_mode=true"));
        assert!(rendered.contains("current_fast=on"));
        assert!(rendered.contains("fast_target=gpt-4o-mini"));
        assert!(rendered.contains("does not implement the leak's entitlement"));
    }

    #[test]
    fn fast_transition_reports_new_state_and_persists_flag() {
        let dir = unique_test_dir("workflow-fast-persist");
        let store = SettingsStore::new(&dir);
        store
            .write(&AgentSettings {
                selected_provider: Some("openai".into()),
                ..AgentSettings::default()
            })
            .expect("write settings");
        let report = ProviderResolver::builtin()
            .load_report(Some(dir.as_path()))
            .expect("load report");

        let persisted = write_persisted_fast(Some(dir.as_path()), true).expect("persist fast");
        let rendered = render_fast_transition(true, persisted, Some(dir.as_path()), &report);

        assert!(persisted);
        assert!(rendered.contains("fast_mode=true"));
        assert!(rendered.contains("status=fast mode enabled"));
        assert!(store.read().expect("read settings").fast_mode);
    }

    #[test]
    fn commit_prompt_reports_clean_repo_without_enqueueing() {
        let dir = unique_test_dir("workflow-commit-clean");
        init_git_repo(&dir);

        let rendered = render_commit_enqueue(&dir, PermissionMode::Default, "");

        assert!(rendered.contains("commit_prompt_ready=false"));
        assert!(rendered.contains("repo_dirty=false"));
        assert!(rendered.contains("status=no changes to commit"));
        assert!(!rendered.contains("enqueue_prompt="));
    }

    #[test]
    fn commit_command_queues_prompt_for_dirty_repo() {
        let dir = unique_test_dir("workflow-commit-dirty");
        init_git_repo(&dir);
        fs::write(dir.join("README.md"), "dirty\n").expect("write readme");

        let output = block_on(CommitCommand::new().execute(
            test_context(&dir),
            CommandInvocation {
                name: "commit".into(),
                args: "mention the README update".into(),
                raw: "/commit mention the README update".into(),
            },
        ))
        .expect("commit output");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("commit_prompt_ready=true"));
        assert!(text.contains("repo_dirty=true"));
        assert!(text.contains("git_mutations_require_confirmation=true"));
        assert!(text.contains("enqueue_prompt=Create a single git commit"));
        assert!(text.contains("Additional user instructions: mention the README update"));
    }

    #[test]
    fn commit_prompt_blocks_git_mutations_in_plan_mode() {
        let dir = unique_test_dir("workflow-commit-plan");
        init_git_repo(&dir);
        fs::write(dir.join("README.md"), "dirty\n").expect("write readme");

        let rendered = render_commit_enqueue(&dir, PermissionMode::Plan, "");

        assert!(rendered.contains("commit_prompt_ready=false"));
        assert!(rendered.contains("git_mutations_allowed=false"));
        assert!(rendered.contains("status=commit blocked by permission mode"));
        assert!(!rendered.contains("enqueue_prompt="));
    }

    #[test]
    fn commit_push_pr_reports_unsupported_backend_when_gh_is_missing() {
        let dir = unique_test_dir("workflow-commit-push-pr-unsupported");
        init_git_repo(&dir);
        fs::write(dir.join("README.md"), "dirty\n").expect("write readme");
        ProcessCommand::new("git")
            .args(["remote", "add", "origin", "https://example.com/demo.git"])
            .current_dir(&dir)
            .status()
            .expect("git remote add");
        let git_path = find_git_binary();
        let bin_dir = dir.join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        write_git_proxy(&bin_dir, &git_path);
        let _path = EnvVarGuard::set("PATH", bin_dir.to_string_lossy().into_owned());

        let rendered = render_commit_push_pr_enqueue(&dir, PermissionMode::Default, "");

        assert!(rendered.contains("commit_push_pr_prompt_ready=true"));
        assert!(rendered.contains("pr_backend=unsupported"));
        assert!(rendered.contains("pr_creation_supported=false"));
        assert!(rendered.contains("status=validation-only prompt queued"));
        assert!(rendered.contains("Do not push and do not attempt PR creation"));
    }

    #[test]
    fn commit_push_pr_command_enqueues_prompt_with_user_args() {
        let dir = unique_test_dir("workflow-commit-push-pr-command");
        init_git_repo(&dir);
        fs::write(dir.join("README.md"), "dirty\n").expect("write readme");

        let output = block_on(CommitPushPrCommand::new().execute(
            test_context(&dir),
            CommandInvocation {
                name: "commit-push-pr".into(),
                args: "call out the README change".into(),
                raw: "/commit-push-pr call out the README change".into(),
            },
        ))
        .expect("commit push pr output");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("commit_push_pr_prompt_ready=true"));
        assert!(text.contains("enqueue_prompt="));
        assert!(text.contains("Additional user instructions: call out the README change"));
    }

    #[test]
    fn review_enqueue_defaults_to_listing_open_prs() {
        let rendered = render_review_enqueue(Path::new("/tmp"), "");

        assert!(rendered.contains("review_prompt_ready=true"));
        assert!(rendered.contains("review_target="));
        assert!(rendered.contains("gh_available="));
        assert!(rendered.contains("status=review prompt queued"));
        assert!(rendered.contains("enqueue_prompt=You are an expert code reviewer."));
    }

    #[test]
    fn review_enqueue_carries_requested_pr_number() {
        let rendered = render_review_enqueue(Path::new("/tmp"), "123");

        assert!(rendered.contains("review_target=123"));
        assert!(rendered.contains("PR target: 123."));
    }

    #[test]
    fn security_review_enqueue_defaults_to_local_branch_scope() {
        let rendered = render_security_review_enqueue("");

        assert!(rendered.contains("security_review_prompt_ready=true"));
        assert!(rendered.contains("security_review_scope=local_branch_changes"));
        assert!(rendered.contains("security_review_focus=default"));
        assert!(rendered.contains("status=security review prompt queued"));
        assert!(rendered.contains("git status --short"));
    }

    #[test]
    fn security_review_command_enqueues_custom_focus() {
        let dir = unique_test_dir("workflow-security-review-command");
        let output = block_on(SecurityReviewCommand::new().execute(
            test_context(&dir),
            CommandInvocation {
                name: "security-review".into(),
                args: "focus on secret handling".into(),
                raw: "/security-review focus on secret handling".into(),
            },
        ))
        .expect("security review output");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("security_review_focus=focus on secret handling"));
        assert!(text.contains("Additional focus: focus on secret handling."));
    }

    #[test]
    fn statusline_enqueue_uses_default_setup_prompt() {
        let rendered = render_statusline_enqueue("");

        assert!(rendered.contains("statusline_prompt_ready=true"));
        assert!(rendered.contains("status=statusline setup prompt queued"));
        assert!(
            rendered.contains(
                "enqueue_prompt=Configure my status line from my shell PS1 configuration"
            )
        );
    }

    #[test]
    fn statusline_enqueue_preserves_custom_prompt() {
        let rendered = render_statusline_enqueue("Match my tmux and starship layout");

        assert!(rendered.contains("enqueue_prompt=Match my tmux and starship layout"));
    }

    #[test]
    fn terminal_setup_notice_points_to_keybindings_flow() {
        let dir = unique_test_dir("workflow-terminal-setup-notice");
        // Run with TERM_PROGRAM unset so the output is deterministic regardless
        // of the CI environment.
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "");
        let rendered = render_terminal_setup_notice(Some(dir.as_path()));

        assert!(rendered.starts_with("## Terminal Setup"));
        assert!(
            rendered.contains("terminal_type="),
            "must include terminal_type field; got:\n{rendered}"
        );
        assert!(
            rendered.contains("setup_needed="),
            "must include setup_needed field; got:\n{rendered}"
        );
        assert!(rendered.contains("Use `/keybindings` to inspect the active shortcuts."));
        assert!(rendered.contains(&format!(
            "Use `/keybindings open` to create or edit {}",
            resolve_keybindings_path(Some(dir.as_path())).display()
        )));
    }

    #[test]
    fn terminal_setup_command_renders_notice() {
        let dir = unique_test_dir("workflow-terminal-setup-command");
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "");
        let output = block_on(TerminalSetupCommand::new(Some(dir.clone())).execute(
            test_context(&dir),
            CommandInvocation {
                name: "terminal-setup".into(),
                args: String::new(),
                raw: "/terminal-setup".into(),
            },
        ))
        .expect("terminal setup output");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("## Terminal Setup"));
        assert!(
            text.contains("terminal_type="),
            "must include terminal_type field; got:\n{text}"
        );
        assert!(
            text.contains("setup_needed="),
            "must include setup_needed field; got:\n{text}"
        );
        assert!(text.contains("Shift+Enter -> insert_newline"));
    }

    // ── terminal detection / per-terminal advice ─────────────────────────────

    /// With TERM_PROGRAM unset, detect_terminal_type returns "unknown".
    #[test]
    fn detect_terminal_type_returns_unknown_when_unset() {
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "");
        assert_eq!(detect_terminal_type(), "unknown");
    }

    /// With TERM_PROGRAM set, detect_terminal_type echoes its value.
    #[test]
    fn detect_terminal_type_echoes_term_program() {
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "ghostty");
        assert_eq!(detect_terminal_type(), "ghostty");
    }

    /// Kitty-protocol-native terminals must report setup_needed=false.
    #[test]
    fn native_csiu_terminals_report_setup_not_needed() {
        for terminal in &["ghostty", "kitty", "WezTerm", "WarpTerminal"] {
            assert!(
                is_native_csiu_terminal(terminal),
                "{terminal} should be recognised as a native Kitty-protocol terminal"
            );
        }
    }

    /// iTerm.app (mixed case) must also be recognised.
    #[test]
    fn iterm_app_is_native_csiu_terminal() {
        assert!(is_native_csiu_terminal("iTerm.app"));
    }

    /// Apple Terminal gets the Option+Enter advice.
    #[test]
    fn terminal_setup_apple_terminal_shows_option_enter_advice() {
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "Apple_Terminal");
        let dir = unique_test_dir("workflow-terminal-setup-apple");
        let rendered = render_terminal_setup_notice(Some(dir.as_path()));
        assert!(
            rendered.contains("Option+Enter"),
            "Apple Terminal advice must mention Option+Enter; got:\n{rendered}"
        );
        assert!(
            rendered.contains("setup_needed=true"),
            "Apple Terminal must require setup; got:\n{rendered}"
        );
    }

    /// Ghostty (a native Kitty-protocol terminal) must report setup_needed=false.
    #[test]
    fn terminal_setup_ghostty_reports_no_setup_needed() {
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "ghostty");
        let dir = unique_test_dir("workflow-terminal-setup-ghostty");
        let rendered = render_terminal_setup_notice(Some(dir.as_path()));
        assert!(
            rendered.contains("setup_needed=false"),
            "ghostty must not need setup; got:\n{rendered}"
        );
    }

    /// VSCode reports setup_needed=false and Shift+Enter advice.
    #[test]
    fn terminal_setup_vscode_shows_shift_enter_advice() {
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "vscode");
        let dir = unique_test_dir("workflow-terminal-setup-vscode");
        let rendered = render_terminal_setup_notice(Some(dir.as_path()));
        assert!(
            terminal_setup_recommendation("vscode").contains("Shift+Enter"),
            "VSCode advice must mention Shift+Enter"
        );
        assert!(
            rendered.contains("terminal_type=vscode"),
            "must include terminal_type=vscode; got:\n{rendered}"
        );
    }

    #[test]
    fn effort_shorthand_normalizes_to_set_subcommand() {
        let invocation = CommandInvocation {
            name: "effort".into(),
            args: "high".into(),
            raw: "/effort high".into(),
        };

        let normalized = normalize_effort_invocation(&invocation);

        assert_eq!(normalized.args, "set high");
    }

    #[test]
    fn effort_status_reports_current_and_persisted_levels() {
        let rendered = render_effort_status(
            "medium",
            Some("high"),
            Some(std::path::Path::new("/workspace")),
        );

        assert!(rendered.contains("## Effort"));
        assert!(rendered.contains("current_effort=medium"));
        assert!(rendered.contains("persisted_effort=high"));
        assert!(rendered.contains("Provider runtime mapping is active"));
    }

    #[test]
    fn effort_transition_persists_to_settings_store() {
        let dir = unique_test_dir("workflow-effort-settings");

        let persisted =
            write_persisted_effort(Some(dir.as_path()), parse_effort_level_name("max").unwrap())
                .expect("write effort");
        let settings = SettingsStore::new(&dir).read().expect("read settings");
        let rendered = render_effort_transition(Some("max"), persisted, Some(dir.as_path()));

        assert!(persisted);
        assert_eq!(settings.effort_level.as_deref(), Some("max"));
        assert!(rendered.contains("effort_level=max"));
        assert!(rendered.contains("persisted=true"));
    }

    fn init_git_repo(path: &Path) {
        fs::create_dir_all(path).expect("create repo dir");
        ProcessCommand::new("git")
            .args(["init", "--quiet", "-b", "main"])
            .current_dir(path)
            .status()
            .expect("git init");
        ProcessCommand::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .status()
            .expect("git config user.name");
        ProcessCommand::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .status()
            .expect("git config user.email");
        fs::write(path.join("README.md"), "seed\n").expect("write seed");
        ProcessCommand::new("git")
            .args(["add", "README.md"])
            .current_dir(path)
            .status()
            .expect("git add seed");
        ProcessCommand::new("git")
            .args(["commit", "-m", "seed"])
            .current_dir(path)
            .status()
            .expect("git commit seed");
    }

    fn find_git_binary() -> String {
        let output = ProcessCommand::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .expect("locate git");
        String::from_utf8(output.stdout)
            .expect("git path utf8")
            .trim()
            .to_string()
    }

    #[cfg(unix)]
    fn write_git_proxy(dir: &Path, git_path: &str) {
        let script_path = dir.join("git");
        fs::write(
            &script_path,
            format!(
                "#!/bin/sh\nexec '{}' \"$@\"\n",
                git_path.replace('\'', "'\"'\"'")
            ),
        )
        .expect("write git proxy");
        let mut permissions = fs::metadata(&script_path)
            .expect("git proxy metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("chmod git proxy");
    }

    // ── Fleet metadata in task list / detail output ───────────────────────────

    #[test]
    fn task_list_includes_fleet_metadata_when_present() {
        use wonder_of_u_core::{AgentTaskState, FleetId, TaskId, TaskState};

        use super::{TaskManager, render_task_list};
        use crate::commands::task_runtime::TaskReconcileReport;

        let dir = unique_test_dir("workflow-task-list-fleet");
        let manager = TaskManager::new(&dir);

        let fleet_id = FleetId::new();
        let parent_id = TaskId::new();

        let mut task = TaskState::pending_agent(
            "fleet agent",
            AgentTaskState::metadata_only("planner", "do work", None, None),
        );
        task.fleet_id = Some(fleet_id);
        task.fleet_request_id = Some("req-001".into());
        task.parent_id = Some(parent_id);
        task.worktree_branch = Some("feat/fleet-branch".into());

        let now = time::OffsetDateTime::now_utc();
        let report = TaskReconcileReport {
            reconciled_at: now,
            tasks: vec![task],
            changed: 0,
            finished: 0,
            fresh_heartbeats: 0,
            stale_heartbeats: 0,
            missing_heartbeats: 0,
        };

        let output = render_task_list("task", &manager, &report, 20);

        assert!(
            output.contains(&format!("task[0].fleet_id={fleet_id}")),
            "fleet_id missing;\n{output}"
        );
        assert!(
            output.contains("task[0].fleet_request_id=req-001"),
            "fleet_request_id missing;\n{output}"
        );
        assert!(
            output.contains(&format!("task[0].parent_id={parent_id}")),
            "parent_id missing;\n{output}"
        );
        assert!(
            output.contains("task[0].worktree_branch=feat/fleet-branch"),
            "worktree_branch missing;\n{output}"
        );
    }

    #[test]
    fn task_list_omits_fleet_metadata_for_non_fleet_tasks() {
        use wonder_of_u_core::{AgentTaskState, TaskState};

        use super::{TaskManager, render_task_list};
        use crate::commands::task_runtime::TaskReconcileReport;

        let dir = unique_test_dir("workflow-task-list-no-fleet");
        let manager = TaskManager::new(&dir);

        let task = TaskState::pending_agent(
            "plain agent",
            AgentTaskState::metadata_only("planner", "do work", None, None),
        );
        assert!(task.fleet_id.is_none());

        let now = time::OffsetDateTime::now_utc();
        let report = TaskReconcileReport {
            reconciled_at: now,
            tasks: vec![task],
            changed: 0,
            finished: 0,
            fresh_heartbeats: 0,
            stale_heartbeats: 0,
            missing_heartbeats: 0,
        };

        let output = render_task_list("task", &manager, &report, 20);

        assert!(
            !output.contains("fleet_id"),
            "fleet_id should be absent for non-fleet task;\n{output}"
        );
        assert!(
            !output.contains("fleet_request_id"),
            "fleet_request_id should be absent;\n{output}"
        );
        assert!(
            !output.contains("parent_id"),
            "parent_id should be absent;\n{output}"
        );
        assert!(
            !output.contains("worktree_branch"),
            "worktree_branch should be absent;\n{output}"
        );
    }

    #[test]
    fn task_detail_includes_fleet_metadata_when_present() {
        use wonder_of_u_core::{AgentTaskState, FleetId, TaskId, TaskState};

        use super::render_task_detail;

        let fleet_id = FleetId::new();
        let parent_id = TaskId::new();

        let mut task = TaskState::pending_agent(
            "fleet detail agent",
            AgentTaskState::metadata_only("planner", "prompt", None, None),
        );
        task.fleet_id = Some(fleet_id);
        task.fleet_request_id = Some("req-detail".into());
        task.parent_id = Some(parent_id);
        task.worktree_branch = Some("feat/detail-branch".into());

        let now = time::OffsetDateTime::now_utc();
        let output = render_task_detail("task", &task, &[], now);

        assert!(
            output.contains(&format!("fleet_id={fleet_id}")),
            "fleet_id missing in detail;\n{output}"
        );
        assert!(
            output.contains("fleet_request_id=req-detail"),
            "fleet_request_id missing;\n{output}"
        );
        assert!(
            output.contains(&format!("parent_id={parent_id}")),
            "parent_id missing;\n{output}"
        );
        assert!(
            output.contains("worktree_branch=feat/detail-branch"),
            "worktree_branch missing;\n{output}"
        );
    }

    #[test]
    fn agents_status_runtime_behavior_unaffected() {
        // Verify the existing /agents status command still works correctly
        // after the definitions sub-surface was extracted.
        let cmd = super::AgentsCommand::new(None);
        let out = cmd.status().expect("agents status");
        let text = match out {
            CommandOutput::Text(t) => t,
            other => panic!("expected Text, got {other:?}"),
        };
        assert!(
            text.contains("storage_dir=disabled"),
            "status without storage should say disabled; got:\n{text}"
        );
    }
}

fn render_task_summary_lines(
    label: &str,
    manager: &TaskManager,
    tasks: &[TaskState],
) -> Vec<String> {
    let summary = super::task_runtime::TaskSummary::from_tasks(tasks);
    let mut lines = vec![
        format!("{label}={}", summary.total),
        format!("active={}", summary.active),
        format!("terminal={}", summary.terminal),
        format!("pending={}", summary.pending),
        format!("running={}", summary.running),
        format!("completed={}", summary.completed),
        format!("failed={}", summary.failed),
        format!("killed={}", summary.killed),
        format!("cancelled={}", summary.cancelled),
        format!("storage_dir={}", manager.storage_dir().display()),
        format!("log_dir={}", manager.logs_dir().display()),
    ];
    if label == "tasks" {
        lines.push(format!("shell_tasks={}", summary.shell));
        lines.push(format!("agent_tasks={}", summary.agents));
        lines.push(format!("remote_tasks={}", summary.remote));
    }
    lines
}

fn append_reconcile_lines(lines: &mut Vec<String>, report: &TaskReconcileReport) {
    lines.push(format!("reconciled_at={}", report.reconciled_at));
    lines.push(format!("reconcile_changed={}", report.changed));
    lines.push(format!("reconcile_finished={}", report.finished));
    lines.push(format!("fresh_heartbeats={}", report.fresh_heartbeats));
    lines.push(format!("stale_heartbeats={}", report.stale_heartbeats));
    lines.push(format!("missing_heartbeats={}", report.missing_heartbeats));
}

fn render_task_list(
    prefix: &str,
    manager: &TaskManager,
    report: &TaskReconcileReport,
    limit: usize,
) -> String {
    let limit = limit.max(1);
    let mut lines = render_task_summary_lines(
        if prefix == "agent" { "agents" } else { "tasks" },
        manager,
        &report.tasks,
    );
    append_reconcile_lines(&mut lines, report);
    for (index, task) in report.tasks.iter().take(limit).enumerate() {
        lines.push(format!("{prefix}[{index}].id={}", task.id));
        lines.push(format!(
            "{prefix}[{index}].kind={}",
            task_kind_label(task.kind)
        ));
        lines.push(format!(
            "{prefix}[{index}].status={}",
            task_status_label(task.status)
        ));
        lines.push(format!(
            "{prefix}[{index}].description={}",
            sanitize_single_line(&task.description)
        ));
        if let Some(status_message) = &task.status_message {
            lines.push(format!(
                "{prefix}[{index}].status_detail={}",
                sanitize_single_line(status_message)
            ));
        }
        if let Some(pid) = task.pid {
            lines.push(format!("{prefix}[{index}].pid={pid}"));
        }
        if let Some(state) = task_heartbeat_state(task, report.reconciled_at) {
            lines.push(format!(
                "{prefix}[{index}].heartbeat={}",
                task_heartbeat_state_label(state)
            ));
        }
        if let Some(agent) = &task.agent {
            lines.push(format!(
                "{prefix}[{index}].agent_name={}",
                sanitize_single_line(&agent.name)
            ));
            lines.push(format!(
                "{prefix}[{index}].runtime={}",
                agent_runtime_label(agent.runtime)
            ));
        }
        if let Some(remote) = &task.remote {
            lines.push(format!(
                "{prefix}[{index}].remote_type={}",
                remote.task_type.label()
            ));
            lines.push(format!(
                "{prefix}[{index}].remote_backend={}",
                sanitize_single_line(&remote.status_summary())
            ));
        }
        // Fleet metadata – only emitted when present so non-fleet output is unchanged.
        if let Some(fleet_id) = task.fleet_id {
            lines.push(format!("{prefix}[{index}].fleet_id={fleet_id}"));
        }
        if let Some(ref req_id) = task.fleet_request_id {
            lines.push(format!(
                "{prefix}[{index}].fleet_request_id={}",
                sanitize_single_line(req_id)
            ));
        }
        if let Some(parent_id) = task.parent_id {
            lines.push(format!("{prefix}[{index}].parent_id={parent_id}"));
        }
        if let Some(ref branch) = task.worktree_branch {
            lines.push(format!(
                "{prefix}[{index}].worktree_branch={}",
                sanitize_single_line(branch)
            ));
        }
    }
    if report.tasks.len() > limit {
        lines.push(format!("truncated=true ({} shown)", limit));
    }
    lines.join("\n")
}

fn render_task_detail(
    prefix: &str,
    task: &TaskState,
    log_tail: &[String],
    reconciled_at: time::OffsetDateTime,
) -> String {
    let mut lines = vec![
        format!("{prefix}_id={}", task.id),
        format!("kind={}", task_kind_label(task.kind)),
        format!("status={}", task_status_label(task.status)),
        format!("description={}", sanitize_single_line(&task.description)),
        format!("started_at={}", task.started_at),
        format!("reconciled_at={reconciled_at}"),
    ];
    if let Some(finished_at) = task.finished_at {
        lines.push(format!("finished_at={finished_at}"));
    }
    if let Some(cwd) = &task.cwd {
        lines.push(format!("cwd={}", cwd.display()));
    }
    if let Some(command) = &task.command {
        lines.push(format!("command={}", sanitize_single_line(command)));
    }
    if let Some(status_message) = &task.status_message {
        lines.push(format!(
            "status_detail={}",
            sanitize_single_line(status_message)
        ));
    }
    if let Some(pid) = task.pid {
        lines.push(format!("pid={pid}"));
    }
    if let Some(last_heartbeat_at) = task.last_heartbeat_at {
        lines.push(format!("last_heartbeat_at={last_heartbeat_at}"));
    }
    if let Some(state) = task_heartbeat_state(task, reconciled_at) {
        lines.push(format!(
            "heartbeat_state={}",
            task_heartbeat_state_label(state)
        ));
    }
    if let Some(exit_code) = task.exit_code {
        lines.push(format!("exit_code={exit_code}"));
    }
    if let Some(output_log) = &task.output_log {
        lines.push(format!("output_log={}", output_log.display()));
    }
    if let Some(agent) = &task.agent {
        lines.push(format!("agent.name={}", sanitize_single_line(&agent.name)));
        lines.push(format!(
            "agent.runtime={}",
            agent_runtime_label(agent.runtime)
        ));
        if let Some(provider) = &agent.provider {
            lines.push(format!("agent.provider={provider}"));
        }
        if let Some(model) = &agent.model {
            lines.push(format!("agent.model={model}"));
        }
        if let Some(prompt) = &agent.prompt {
            lines.push(format!("agent.prompt={}", sanitize_single_line(prompt)));
        }
    }
    if let Some(remote) = &task.remote {
        lines.push(format!("remote.type={}", remote.task_type.label()));
        lines.push(format!(
            "remote.transport={}",
            sanitize_single_line(&remote.transport.summary())
        ));
        if let Some(monitor) = &remote.monitor {
            lines.push(format!(
                "remote.monitor={}",
                sanitize_single_line(&monitor.summary())
            ));
        }
        if let Some(checker) = &remote.checker {
            lines.push(format!(
                "remote.checker={}",
                sanitize_single_line(&checker.summary())
            ));
        }
    }
    // Fleet metadata – only emitted when present so non-fleet output is unchanged.
    if let Some(fleet_id) = task.fleet_id {
        lines.push(format!("fleet_id={fleet_id}"));
    }
    if let Some(ref req_id) = task.fleet_request_id {
        lines.push(format!("fleet_request_id={}", sanitize_single_line(req_id)));
    }
    if let Some(parent_id) = task.parent_id {
        lines.push(format!("parent_id={parent_id}"));
    }
    if let Some(ref branch) = task.worktree_branch {
        lines.push(format!("worktree_branch={}", sanitize_single_line(branch)));
    }
    lines.push(format!("log_tail_lines={}", log_tail.len()));
    for (index, line) in log_tail.iter().enumerate() {
        lines.push(format!("log_tail[{index}]={}", sanitize_single_line(line)));
    }
    lines.join("\n")
}

fn read_detail_log_tail(manager: &TaskManager, task: &TaskState, tail_lines: usize) -> Vec<String> {
    let mut log_tail = manager
        .read_log_tail(task.id, tail_lines)
        .unwrap_or_default();
    if task.status.is_terminal() || tail_lines == 0 {
        return log_tail;
    }

    let startup_header_lines = match task.kind {
        TaskKind::LocalShell => 7,
        TaskKind::LocalAgent => 10,
        TaskKind::RemoteAgent => 0,
    };
    for _ in 0..10 {
        if log_tail.len() > startup_header_lines {
            break;
        }
        thread::sleep(Duration::from_millis(50));
        log_tail = manager
            .read_log_tail(task.id, tail_lines)
            .unwrap_or_default();
    }
    log_tail
}

fn render_task_reconcile_report(
    label: &str,
    manager: &TaskManager,
    report: &TaskReconcileReport,
) -> String {
    let mut lines = render_task_summary_lines(label, manager, &report.tasks);
    append_reconcile_lines(&mut lines, report);
    lines.join("\n")
}

fn render_task_started(prefix: &str, task: &TaskState) -> String {
    let mut lines = vec![
        format!("{prefix}_id={}", task.id),
        format!("kind={}", task_kind_label(task.kind)),
        format!("status={}", task_status_label(task.status)),
        format!("description={}", sanitize_single_line(&task.description)),
    ];
    if let Some(pid) = task.pid {
        lines.push(format!("pid={pid}"));
    }
    if let Some(last_heartbeat_at) = task.last_heartbeat_at {
        lines.push(format!("last_heartbeat_at={last_heartbeat_at}"));
    }
    if let Some(output_log) = &task.output_log {
        lines.push(format!("output_log={}", output_log.display()));
    }
    if let Some(status_message) = &task.status_message {
        lines.push(format!("note={}", sanitize_single_line(status_message)));
    }
    if let Some(agent) = &task.agent {
        lines.push(format!("runtime={}", agent_runtime_label(agent.runtime)));
    }
    lines.join("\n")
}

fn render_task_stopped(prefix: &str, task: &TaskState) -> String {
    format!(
        concat!(
            "{prefix}_id={}\n",
            "status={}\n",
            "kind={}\n",
            "description={}"
        ),
        task.id,
        task_status_label(task.status),
        task_kind_label(task.kind),
        sanitize_single_line(&task.description),
        prefix = prefix,
    )
}

fn render_task_removed(prefix: &str, task: &TaskState) -> String {
    format!(
        concat!(
            "{prefix}_id={}\n",
            "action=removed\n",
            "kind={}\n",
            "status={}\n",
            "description={}"
        ),
        task.id,
        task_kind_label(task.kind),
        task_status_label(task.status),
        sanitize_single_line(&task.description),
        prefix = prefix,
    )
}

fn render_task_prune_report(report: &super::task_runtime::TaskPruneReport) -> String {
    let removed_ids = report
        .removed
        .iter()
        .map(|t| t.id.to_string())
        .collect::<Vec<_>>();
    let skipped_ids = report
        .skipped_active
        .iter()
        .map(|t| t.id.to_string())
        .collect::<Vec<_>>();
    let skipped_terminal_ids = report
        .skipped_terminal
        .iter()
        .map(|t| t.id.to_string())
        .collect::<Vec<_>>();
    let mut lines = vec![
        format!("removed={}", report.removed.len()),
        format!("skipped_active={}", report.skipped_active.len()),
        format!("skipped_terminal={}", report.skipped_terminal.len()),
    ];
    for (index, id) in removed_ids.iter().enumerate() {
        lines.push(format!("removed_ids[{index}]={id}"));
    }
    for (index, id) in skipped_ids.iter().enumerate() {
        lines.push(format!("skipped_active_ids[{index}]={id}"));
    }
    for (index, id) in skipped_terminal_ids.iter().enumerate() {
        lines.push(format!("skipped_terminal_ids[{index}]={id}"));
    }
    lines.join("\n")
}

fn require_task_kind(task: TaskState, kind: TaskKind) -> Result<TaskState> {
    if task.kind == kind {
        Ok(task)
    } else {
        Err(WonderError::validation(format!(
            "task {} is not a {} entry",
            task.id,
            task_kind_label(kind)
        )))
    }
}

fn parse_task_id(value: &str) -> Result<TaskId> {
    TaskId::parse(value)
}

fn sanitize_single_line(value: &str) -> String {
    value.lines().collect::<Vec<_>>().join("\\n")
}

fn parse_permission_mode(value: &str) -> Result<PermissionMode> {
    match value {
        "default" => Ok(PermissionMode::Default),
        "accept-edits" | "acceptEdits" => Ok(PermissionMode::AcceptEdits),
        "bypass-permissions" | "bypassPermissions" => Ok(PermissionMode::BypassPermissions),
        "dont-ask" | "dontAsk" => Ok(PermissionMode::DontAsk),
        "plan" => Ok(PermissionMode::Plan),
        other => Err(WonderError::validation(format!(
            "unknown permission mode: {other}"
        ))),
    }
}

fn permission_mode_label(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "accept-edits",
        PermissionMode::BypassPermissions => "bypass-permissions",
        PermissionMode::DontAsk => "dont-ask",
        PermissionMode::Plan => "plan",
    }
}

fn decision_label(decision: &PermissionDecision) -> &'static str {
    match decision {
        PermissionDecision::Allow { .. } => "allow",
        PermissionDecision::Ask { .. } => "ask",
        PermissionDecision::Deny { .. } => "deny",
    }
}

fn task_kind_label(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::LocalShell => "local_shell",
        TaskKind::LocalAgent => "local_agent",
        TaskKind::RemoteAgent => "remote_agent",
    }
}

fn task_status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::Running => "running",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Killed => "killed",
        TaskStatus::Cancelled => "cancelled",
    }
}

fn agent_runtime_label(runtime: wonder_of_u_core::AgentRuntime) -> &'static str {
    match runtime {
        wonder_of_u_core::AgentRuntime::MetadataOnly => "metadata_only",
        wonder_of_u_core::AgentRuntime::PromptSubprocess => "prompt_subprocess",
        wonder_of_u_core::AgentRuntime::Deferred => "legacy_relaunch_required",
    }
}
