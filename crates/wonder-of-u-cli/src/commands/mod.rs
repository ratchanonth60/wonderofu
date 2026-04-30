use std::{
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    sync::Arc,
};

use clap::Parser;
use wonder_of_u_agent::builtin_tool_registry;
use wonder_of_u_core::{
    CommandInvocation, CommandRegistry, CommandSpec, Result, SessionId, ToolSpec, WonderError,
};

pub(crate) mod auth;
mod doctor;
mod features;
mod help;
mod mcp;
mod plugin;
pub(crate) mod project;
pub(crate) mod prompt;
mod session;
mod skills;
mod status;
mod task_runtime;
mod tui;
pub(crate) mod workflow;

use auth::{ConfigCommand, LoginCommand, LogoutCommand, ModelCommand};
use doctor::DoctorCommand;
use features::FeaturesCommand;
use help::HelpCommand;
use mcp::McpCommand;
use plugin::{PluginCommand, ReloadPluginsCommand};
use project::{
    AddDirCommand, BranchCommand, ContextCommand, CopyCommand, DiffCommand, FilesCommand,
    InitCommand, MemoryCommand,
};
use prompt::PromptCommand;
use session::{
    ClearCommand, CompactCommand, ExportCommand, RenameCommand, ResumeCommand, SessionCommand,
    TagCommand,
};
use skills::SkillsCommand;
use status::{
    ChromeCommand, CostCommand, DesktopCommand, FeedbackCommand, IdeCommand, InsightsCommand,
    MobileCommand, OutputStyleCommand, ReleaseNotesCommand, StatsCommand, StatusCommand,
    UpgradeCommand, UsageCommand, VersionCommand,
};
use tui::TuiCommand;
use workflow::{
    AgentsCommand, BriefCommand, ColorCommand, EffortCommand, ExitCommand, FastCommand,
    HooksCommand, KeybindingsCommand, PermissionsCommand, PlanCommand, PrivacySettingsCommand,
    ReviewCommand, SecurityReviewCommand, StatuslineCommand, TasksCommand, TerminalSetupCommand,
    ThemeCommand, VimCommand,
};

pub fn registry(storage_dir: Option<PathBuf>) -> Result<CommandRegistry> {
    let tool_specs: Arc<[ToolSpec]> = builtin_tool_registry()?.all_specs().into();
    let specs: Arc<[CommandSpec]> = vec![
        HelpCommand::command_spec(),
        DoctorCommand::command_spec(),
        StatusCommand::command_spec(),
        VersionCommand::command_spec(),
        ReleaseNotesCommand::command_spec(),
        FeedbackCommand::command_spec(),
        UpgradeCommand::command_spec(),
        DesktopCommand::command_spec(),
        MobileCommand::command_spec(),
        ChromeCommand::command_spec(),
        IdeCommand::command_spec(),
        InsightsCommand::command_spec(),
        UsageCommand::command_spec(),
        CostCommand::command_spec(),
        StatsCommand::command_spec(),
        ConfigCommand::command_spec(),
        LoginCommand::command_spec(),
        LogoutCommand::command_spec(),
        ModelCommand::command_spec(),
        McpCommand::command_spec(),
        PluginCommand::command_spec(),
        ReloadPluginsCommand::command_spec(),
        PromptCommand::command_spec(),
        SkillsCommand::command_spec(),
        SessionCommand::command_spec(),
        ResumeCommand::command_spec(),
        TagCommand::command_spec(),
        InitCommand::command_spec(),
        AddDirCommand::command_spec(),
        ContextCommand::command_spec(),
        CopyCommand::command_spec(),
        MemoryCommand::command_spec(),
        RenameCommand::command_spec(),
        ExportCommand::command_spec(),
        ClearCommand::command_spec(),
        CompactCommand::command_spec(),
        FilesCommand::command_spec(),
        BranchCommand::command_spec(),
        DiffCommand::command_spec(),
        PermissionsCommand::command_spec(),
        HooksCommand::command_spec(),
        PrivacySettingsCommand::command_spec(),
        ColorCommand::command_spec(),
        BriefCommand::command_spec(),
        FastCommand::command_spec(),
        ReviewCommand::command_spec(),
        SecurityReviewCommand::command_spec(),
        StatuslineCommand::command_spec(),
        EffortCommand::command_spec(),
        KeybindingsCommand::command_spec(),
        TerminalSetupCommand::command_spec(),
        ThemeCommand::command_spec(),
        VimCommand::command_spec(),
        PlanCommand::command_spec(),
        AgentsCommand::command_spec(),
        TasksCommand::command_spec(),
        ExitCommand::command_spec(),
        FeaturesCommand::command_spec(),
        OutputStyleCommand::command_spec(),
        TuiCommand::command_spec(),
    ]
    .into();

    let mut registry = CommandRegistry::new();
    registry.register(Arc::new(HelpCommand::new(
        Arc::clone(&specs),
        storage_dir.clone(),
    )))?;
    registry.register(Arc::new(DoctorCommand::new(
        specs.len(),
        specs.iter().filter(|spec| !spec.hidden).count(),
        storage_dir.clone(),
    )))?;
    registry.register(Arc::new(StatusCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(VersionCommand::new()))?;
    registry.register(Arc::new(ReleaseNotesCommand::new()))?;
    registry.register(Arc::new(FeedbackCommand::new()))?;
    registry.register(Arc::new(UpgradeCommand::new()))?;
    registry.register(Arc::new(DesktopCommand::new()))?;
    registry.register(Arc::new(MobileCommand::new()))?;
    registry.register(Arc::new(ChromeCommand::new()))?;
    registry.register(Arc::new(IdeCommand::new()))?;
    registry.register(Arc::new(InsightsCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(UsageCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(CostCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(StatsCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(ConfigCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(LoginCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(LogoutCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(ModelCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(McpCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(PluginCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(ReloadPluginsCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(PromptCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(SkillsCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(SessionCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(ResumeCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(TagCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(InitCommand::new()))?;
    registry.register(Arc::new(AddDirCommand::new()))?;
    registry.register(Arc::new(ContextCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(CopyCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(MemoryCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(RenameCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(ExportCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(ClearCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(CompactCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(FilesCommand::new()))?;
    registry.register(Arc::new(BranchCommand::new()))?;
    registry.register(Arc::new(DiffCommand::new()))?;
    registry.register(Arc::new(PermissionsCommand::new(Arc::clone(&tool_specs))))?;
    registry.register(Arc::new(HooksCommand::new(
        storage_dir.clone(),
        Arc::clone(&tool_specs),
    )))?;
    registry.register(Arc::new(PrivacySettingsCommand::new()))?;
    registry.register(Arc::new(ColorCommand::new()))?;
    registry.register(Arc::new(BriefCommand::new()))?;
    registry.register(Arc::new(FastCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(ReviewCommand::new()))?;
    registry.register(Arc::new(SecurityReviewCommand::new()))?;
    registry.register(Arc::new(StatuslineCommand::new()))?;
    registry.register(Arc::new(EffortCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(KeybindingsCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(TerminalSetupCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(ThemeCommand::new()))?;
    registry.register(Arc::new(VimCommand::new()))?;
    registry.register(Arc::new(PlanCommand::new()))?;
    registry.register(Arc::new(AgentsCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(TasksCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(ExitCommand::new()))?;
    registry.register(Arc::new(FeaturesCommand::new()))?;
    registry.register(Arc::new(OutputStyleCommand::new()))?;
    registry.register(Arc::new(TuiCommand::new()))?;
    Ok(registry)
}

pub(crate) fn resolve_dynamic_command(
    cwd: &Path,
    storage_dir: Option<&Path>,
    name: &str,
    query: &wonder_of_u_core::CommandQuery,
) -> Result<Option<std::sync::Arc<dyn wonder_of_u_core::Command>>> {
    plugin::resolve_dynamic_plugin_command(cwd, storage_dir, name, query)
}

pub(crate) fn parse_command_args<T: Parser>(
    command_name: &str,
    invocation: &CommandInvocation,
) -> Result<T> {
    let mut argv = vec![command_name.to_string()];
    argv.extend(shell_words::split(&invocation.args).map_err(|error| {
        WonderError::validation(format!("invalid /{command_name} arguments: {error}"))
    })?);
    T::try_parse_from(argv).map_err(|error| WonderError::validation(error.to_string()))
}

pub(crate) fn invocation_from_tokens(
    name: &str,
    tokens: impl IntoIterator<Item = impl Into<String>>,
) -> CommandInvocation {
    let tokens = tokens.into_iter().map(Into::into).collect::<Vec<_>>();
    let args = shell_words::join(tokens.iter().map(String::as_str));
    let raw = if args.is_empty() {
        format!("/{name}")
    } else {
        format!("/{name} {args}")
    };
    CommandInvocation {
        name: name.into(),
        args,
        raw,
    }
}

pub(crate) fn parse_session_id(value: &str) -> Result<SessionId> {
    SessionId::parse(value)
}

pub(crate) fn git_command_output(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = ProcessCommand::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(crate) fn detect_git_branch(cwd: &Path) -> Option<String> {
    git_command_output(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])
        .filter(|branch| branch != "HEAD")
}

pub(crate) fn is_hidden_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}
