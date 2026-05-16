//! Provides commands support
//!
use std::{
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    sync::Arc,
};

use clap::Parser;
use wonder_of_u_agent::builtin_tool_registry;
use wonder_of_u_core::{
    AppState, CommandInvocation, CommandRegistry, CommandSpec, Result, SessionId, ToolResult,
    ToolSpec, WonderError,
};
use wonder_of_u_tools::parse_worktree_runtime_action;

mod advanced;
pub(crate) mod auth;
pub(crate) mod copy;
pub(crate) mod cost;
mod doctor;
pub(crate) mod env;
mod extras;
mod features;
mod fleet;
mod help;
pub(crate) mod hooks;
pub(crate) mod import;
pub(crate) mod keybindings;
mod mcp;
pub mod memory;
pub(crate) mod output_style;
mod plugin;
pub(crate) mod project;
pub(crate) mod prompt;
pub mod rewind;
pub(crate) mod search;
mod session;
mod setup;
mod skills;
mod status;
mod summary;
pub mod tag;
mod task_runtime;
pub(crate) mod theme;
mod tui;
pub mod vim;
pub(crate) mod workflow;

use advanced::{BridgeCommand, DebugCommand, DiagnosticsCommand, VoiceCommand};
use auth::{ConfigCommand, LoginCommand, LogoutCommand, ModelCommand};
use doctor::DoctorCommand;
use extras::{
    AdvisorCommand, AntTraceCommand, AutofixPrCommand, BackfillSessionsCommand, BreakCacheCommand,
    BridgeKickCommand, BtwCommand, BughunterCommand, CreateMovedToPluginCommand, CtxVizCommand,
    DebugToolCallCommand, EnvCommand, ExtraUsageCommand, GoodClaudeCommand, HeapdumpCommand,
    InitVerifiersCommand, InstallCommand, InstallGithubAppCommand, InstallSlackAppCommand,
    IssueCommand, MockLimitsCommand, OauthRefreshCommand, OnboardingCommand, PassesCommand,
    PerfIssueCommand, PrCommentsCommand, RateLimitOptionsCommand, RemoteEnvCommand,
    RemoteSetupCommand, ResetLimitsCommand, SandboxToggleCommand, ShareCommand, StickersCommand,
    TeleportCommand, ThinkbackCommand, ThinkbackPlayCommand, UltraplanCommand,
};
pub(crate) use extras::{
    execute_help_command, execute_settings_command, execute_stats_command, execute_thinking_command,
};
use features::FeaturesCommand;
use fleet::FleetCommand;
use help::HelpCommand;
use mcp::McpCommand;
use plugin::{PluginCommand, ReloadPluginsCommand};
use project::{
    AddDirCommand, BranchCommand, ContextCommand, CopyCommand, DiffCommand, FilesCommand,
    InitCommand, MemoryCommand,
};
use prompt::PromptCommand;
use rewind::RewindCommand;
use session::{
    ClearCommand, CompactCommand, ExportCommand, RenameCommand, ResumeCommand, SessionCommand,
};
use setup::SetupCommand;
use skills::SkillsCommand;
use status::{
    ChromeCommand, CostCommand, DesktopCommand, FeedbackCommand, IdeCommand, InsightsCommand,
    MobileCommand, OutputStyleCommand, ReleaseNotesCommand, StatsCommand, StatusCommand,
    UpgradeCommand, UsageCommand, VersionCommand,
};
use summary::SummaryCommand;
use tag::TagCommand;
use tui::TuiCommand;
use workflow::{
    AgentsCommand, BriefCommand, ColorCommand, CommitCommand, CommitPushPrCommand, EffortCommand,
    ExitCommand, FastCommand, HooksCommand, KeybindingsCommand, OptimizeTonkenCommand,
    PermissionsCommand, PlanCommand, PrivacySettingsCommand, ReviewCommand, SecurityReviewCommand,
    StatuslineCommand, TasksCommand, TerminalSetupCommand, ThemeCommand, VimCommand,
};

/// Builds the registry
pub fn registry(storage_dir: Option<PathBuf>) -> Result<CommandRegistry> {
    let tool_specs: Arc<[ToolSpec]> = builtin_tool_registry()?.all_specs().into();
    let specs: Arc<[CommandSpec]> = vec![
        HelpCommand::command_spec(),
        DoctorCommand::command_spec(),
        StatusCommand::command_spec(),
        VersionCommand::command_spec(),
        ReleaseNotesCommand::command_spec(),
        FeedbackCommand::command_spec(),
        DiagnosticsCommand::command_spec(),
        BridgeCommand::command_spec(),
        VoiceCommand::command_spec(),
        DebugCommand::command_spec(),
        HeapdumpCommand::command_spec(),
        AntTraceCommand::command_spec(),
        CtxVizCommand::command_spec(),
        DebugToolCallCommand::command_spec(),
        GoodClaudeCommand::command_spec(),
        BreakCacheCommand::command_spec(),
        BackfillSessionsCommand::command_spec(),
        PerfIssueCommand::command_spec(),
        BughunterCommand::command_spec(),
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
        OptimizeTonkenCommand::command_spec(),
        FastCommand::command_spec(),
        CommitCommand::command_spec(),
        CommitPushPrCommand::command_spec(),
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
        BtwCommand::command_spec(),
        AdvisorCommand::command_spec(),
        StickersCommand::command_spec(),
        RewindCommand::command_spec(),
        InitVerifiersCommand::command_spec(),
        ExtraUsageCommand::command_spec(),
        PassesCommand::command_spec(),
        RateLimitOptionsCommand::command_spec(),
        MockLimitsCommand::command_spec(),
        ResetLimitsCommand::command_spec(),
        OnboardingCommand::command_spec(),
        TeleportCommand::command_spec(),
        RemoteEnvCommand::command_spec(),
        RemoteSetupCommand::command_spec(),
        BridgeKickCommand::command_spec(),
        SandboxToggleCommand::command_spec(),
        UltraplanCommand::command_spec(),
        ThinkbackCommand::command_spec(),
        ThinkbackPlayCommand::command_spec(),
        AutofixPrCommand::command_spec(),
        PrCommentsCommand::command_spec(),
        SummaryCommand::command_spec(),
        EnvCommand::command_spec(),
        OauthRefreshCommand::command_spec(),
        IssueCommand::command_spec(),
        ShareCommand::command_spec(),
        InstallCommand::command_spec(),
        InstallGithubAppCommand::command_spec(),
        InstallSlackAppCommand::command_spec(),
        CreateMovedToPluginCommand::command_spec(),
        TuiCommand::command_spec(),
        SetupCommand::command_spec(),
        FleetCommand::command_spec(),
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
    registry.register(Arc::new(DiagnosticsCommand::new()))?;
    registry.register(Arc::new(BridgeCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(VoiceCommand::new()))?;
    registry.register(Arc::new(DebugCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(HeapdumpCommand::new()))?;
    registry.register(Arc::new(AntTraceCommand::new()))?;
    registry.register(Arc::new(CtxVizCommand::new()))?;
    registry.register(Arc::new(DebugToolCallCommand::new()))?;
    registry.register(Arc::new(GoodClaudeCommand::new()))?;
    registry.register(Arc::new(BreakCacheCommand::new()))?;
    registry.register(Arc::new(BackfillSessionsCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(PerfIssueCommand::new()))?;
    registry.register(Arc::new(BughunterCommand::new()))?;
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
    registry.register(Arc::new(OptimizeTonkenCommand::new()))?;
    registry.register(Arc::new(FastCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(CommitCommand::new()))?;
    registry.register(Arc::new(CommitPushPrCommand::new()))?;
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
    registry.register(Arc::new(BtwCommand::new()))?;
    registry.register(Arc::new(AdvisorCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(StickersCommand::new()))?;
    registry.register(Arc::new(RewindCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(InitVerifiersCommand::new(Arc::clone(&tool_specs))))?;
    registry.register(Arc::new(ExtraUsageCommand::new()))?;
    registry.register(Arc::new(PassesCommand::new()))?;
    registry.register(Arc::new(RateLimitOptionsCommand::new()))?;
    registry.register(Arc::new(MockLimitsCommand::new()))?;
    registry.register(Arc::new(ResetLimitsCommand::new()))?;
    registry.register(Arc::new(OnboardingCommand::new()))?;
    registry.register(Arc::new(TeleportCommand::new()))?;
    registry.register(Arc::new(RemoteEnvCommand::new()))?;
    registry.register(Arc::new(RemoteSetupCommand::new()))?;
    registry.register(Arc::new(BridgeKickCommand::new()))?;
    registry.register(Arc::new(SandboxToggleCommand::new()))?;
    registry.register(Arc::new(UltraplanCommand::new()))?;
    registry.register(Arc::new(ThinkbackCommand::new()))?;
    registry.register(Arc::new(ThinkbackPlayCommand::new()))?;
    registry.register(Arc::new(AutofixPrCommand::new()))?;
    registry.register(Arc::new(PrCommentsCommand::new()))?;
    registry.register(Arc::new(SummaryCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(EnvCommand::new()))?;
    registry.register(Arc::new(OauthRefreshCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(IssueCommand::new()))?;
    registry.register(Arc::new(ShareCommand::new()))?;
    registry.register(Arc::new(InstallCommand::new()))?;
    registry.register(Arc::new(InstallGithubAppCommand::new()))?;
    registry.register(Arc::new(InstallSlackAppCommand::new()))?;
    registry.register(Arc::new(CreateMovedToPluginCommand::new()))?;
    registry.register(Arc::new(TuiCommand::new()))?;
    registry.register(Arc::new(SetupCommand::new(storage_dir.clone())))?;
    registry.register(Arc::new(FleetCommand::new(storage_dir.clone())))?;
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

pub(crate) fn apply_worktree_tool_result(
    state: &mut AppState,
    result: &ToolResult,
) -> Result<bool> {
    let Some(action) = parse_worktree_runtime_action(&result.metadata)? else {
        return Ok(false);
    };

    std::env::set_current_dir(&action.cwd)?;
    state.session.cwd = action.cwd.clone();
    state.session.git_branch = detect_git_branch(&action.cwd).or_else(|| {
        action
            .session_state
            .as_ref()
            .and_then(|session| session.worktree_branch.clone())
    });
    state.session.worktree = action.session_state;
    Ok(true)
}

pub(crate) fn open_browser(url: &str) {
    let _ = try_open_browser(url);
}

pub(crate) fn try_open_browser(url: &str) -> bool {
    if browser_launch_disabled() {
        return false;
    }

    #[cfg(target_os = "macos")]
    let command = ("open", vec![url]);
    #[cfg(target_os = "linux")]
    let command = ("xdg-open", vec![url]);
    #[cfg(target_os = "windows")]
    let command = ("cmd", vec!["/c", "start", "", url]);

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    {
        ProcessCommand::new(command.0)
            .args(command.1)
            .spawn()
            .is_ok()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = url;
        false
    }
}

pub(crate) fn browser_launch_disabled() -> bool {
    cfg!(test) || env_flag_enabled("WONDER_OF_U_NO_BROWSER")
}

fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub(crate) fn is_hidden_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use wonder_of_u_core::{AppState, RuntimeWorktreeState, ToolResult, ToolUseId};
    use wonder_of_u_test_support::unique_test_dir;

    use super::{apply_worktree_tool_result, browser_launch_disabled, try_open_browser};

    #[test]
    fn shared_browser_launch_is_disabled_under_tests() {
        assert!(
            browser_launch_disabled(),
            "tests must never spawn a real system browser"
        );
        assert!(!try_open_browser("https://example.invalid/browser-guard"));
    }

    #[test]
    fn apply_worktree_tool_result_switches_session_state() {
        let original_cwd = std::env::current_dir().expect("current dir");
        let repo = unique_test_dir("commands-worktree-runtime");
        let worktree = repo.join("worktree");
        fs::create_dir_all(&worktree).expect("create worktree dir");

        let mut state = AppState::new(repo.clone());
        let result =
            ToolResult::success(ToolUseId::new(), "entered").with_metadata(serde_json::json!({
                "worktree_runtime_action": {
                    "action": "enter",
                    "cwd": worktree.clone(),
                    "session_state": {
                        "original_cwd": repo.clone(),
                        "repository_root": repo.clone(),
                        "worktree_path": worktree.clone(),
                        "worktree_branch": "worktree-topic",
                        "original_branch": "main",
                        "original_head_commit": "abc123",
                    }
                }
            }));

        let changed =
            apply_worktree_tool_result(&mut state, &result).expect("apply worktree action");

        assert!(changed);
        assert_eq!(state.session.cwd, worktree);
        assert_eq!(
            state.session.worktree,
            Some(RuntimeWorktreeState {
                original_cwd: repo.clone(),
                repository_root: repo,
                worktree_path: worktree.clone(),
                worktree_branch: Some("worktree-topic".into()),
                original_branch: Some("main".into()),
                original_head_commit: Some("abc123".into()),
                tmux_session_name: None,
            })
        );

        std::env::set_current_dir(original_cwd).expect("restore cwd");
    }
}
