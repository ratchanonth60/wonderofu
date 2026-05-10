use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use wonder_of_u_agent::{AuthMaterial, CredentialStore, SettingsStore};
use wonder_of_u_core::{
    AppState, AuthMaterialKind, AuthState, Command, CommandContext, CommandInvocation, CommandKind,
    CommandOutput, CommandSpec, MessageEnvelope, MessagePayload, Result, SessionId, ToolSpec,
    WonderError, app::ThinkingEffort, permission_mode_label,
};
use wonder_of_u_storage::{STORAGE_SCHEMA_VERSION, SessionMetadata, StoragePaths, TranscriptStore};

/// Represents heapdump command
pub struct HeapdumpCommand;

impl HeapdumpCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "heapdump",
            "Write a lightweight process memory diagnostics dump",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents ant trace command
pub struct AntTraceCommand;

impl AntTraceCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "ant-trace",
            "Show internal trace and diagnostic context",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents ctx viz command
pub struct CtxVizCommand;

impl CtxVizCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "ctx-viz",
            "Show context window usage diagnostics",
            CommandKind::NonInteractive,
        );
        spec.aliases.push("ctx_viz".into());
        spec.hidden = true;
        spec
    }
}

/// Represents debug tool call command
pub struct DebugToolCallCommand;

impl DebugToolCallCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "debug-tool-call",
            "Show the last active tool call, if available",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents good claude command
pub struct GoodClaudeCommand;

impl GoodClaudeCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "good-claude",
            "Mark the current session as a positive example",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents break cache command
pub struct BreakCacheCommand;

impl BreakCacheCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "break-cache",
            "Clear transient command caches",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents backfill sessions command
pub struct BackfillSessionsCommand {
    storage_dir: Option<PathBuf>,
}

impl BackfillSessionsCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "backfill-sessions",
            "Re-index stored sessions for search and diagnostics",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents perf issue command
pub struct PerfIssueCommand;

impl PerfIssueCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "perf-issue",
            "Collect lightweight performance diagnostics",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents bughunter command
pub struct BughunterCommand;

impl BughunterCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "bughunter",
            "Show bug reporting guidance",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents btw command
pub struct BtwCommand;

impl BtwCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "btw",
            "Ask a quick side question without interrupting the main conversation",
            CommandKind::Local,
        )
    }
}

/// Represents advisor command
pub struct AdvisorCommand {
    storage_dir: Option<PathBuf>,
}

impl AdvisorCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "advisor",
            "Configure the advisor model for multi-model reasoning",
            CommandKind::Local,
        )
    }
}

/// Represents stickers command
pub struct StickersCommand;

impl StickersCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new("stickers", "Order Claude Code stickers", CommandKind::Local)
    }
}

/// Represents init verifiers command
pub struct InitVerifiersCommand {
    tool_specs: Arc<[ToolSpec]>,
}

impl InitVerifiersCommand {
    /// Creates a new value
    pub fn new(tool_specs: Arc<[ToolSpec]>) -> Self {
        Self { tool_specs }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "init-verifiers",
            "Initialize verification tools and validators",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents extra usage command
pub struct ExtraUsageCommand;

impl ExtraUsageCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "extra-usage",
            "Configure extra usage allowance when limits are reached",
            CommandKind::Local,
        )
    }
}

/// Represents passes command
pub struct PassesCommand;

impl PassesCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "passes",
            "Share Claude Code access passes with friends",
            CommandKind::Local,
        )
    }
}

/// Represents rate limit options command
pub struct RateLimitOptionsCommand;

impl RateLimitOptionsCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "rate-limit-options",
            "Show options when rate limit is reached",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents mock limits command
pub struct MockLimitsCommand;

impl MockLimitsCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "mock-limits",
            "Show mock rate-limit support for development builds",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents reset limits command
pub struct ResetLimitsCommand;

impl ResetLimitsCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "reset-limits",
            "Explain why client-side rate limits cannot be reset",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents onboarding command
pub struct OnboardingCommand;

impl OnboardingCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "onboarding",
            "Run the new user onboarding flow",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents teleport command
pub struct TeleportCommand;

impl TeleportCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "teleport",
            "Teleport to a remote Claude Code session",
            CommandKind::Local,
        )
    }
}

/// Represents remote env command
pub struct RemoteEnvCommand;

impl RemoteEnvCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "remote-env",
            "Configure the default remote environment for teleport sessions",
            CommandKind::Local,
        )
    }
}

/// Represents remote setup command
pub struct RemoteSetupCommand;

impl RemoteSetupCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "web-setup",
            "Set up Claude Code on the web (requires Claude.ai account)",
            CommandKind::Local,
        );
        spec.aliases.push("remote-setup".into());
        spec
    }
}

/// Represents bridge kick command
pub struct BridgeKickCommand;

impl BridgeKickCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "bridge-kick",
            "Force-reconnect the remote bridge transport",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents sandbox toggle command
pub struct SandboxToggleCommand;

impl SandboxToggleCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "sandbox",
            "Toggle sandbox isolation for bash command execution",
            CommandKind::Local,
        );
        spec.aliases.push("sandbox-toggle".into());
        spec
    }
}

/// Represents ultraplan command
pub struct UltraplanCommand;

impl UltraplanCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "ultraplan",
            "Launch a multi-agent remote exploration (requires Claude.ai)",
            CommandKind::Local,
        )
    }
}

/// Represents thinkback command
pub struct ThinkbackCommand;

impl ThinkbackCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "think-back",
            "Your Claude Code year in review",
            CommandKind::NonInteractive,
        );
        spec.aliases.push("thinkback".into());
        spec.hidden = true;
        spec
    }
}

/// Represents thinkback play command
pub struct ThinkbackPlayCommand;

impl ThinkbackPlayCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "thinkback-play",
            "Replay the think-back animation, if available",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents autofix pr command
pub struct AutofixPrCommand;

impl AutofixPrCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "autofix-pr",
            "Automatically fix issues in a GitHub pull request",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents pr comments command
pub struct PrCommentsCommand;

impl PrCommentsCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "pr-comments",
            "Fetch and display comments from a GitHub pull request",
            CommandKind::NonInteractive,
        );
        spec.aliases.push("pr_comments".into());
        spec
    }
}

/// Represents env command
pub struct EnvCommand;

impl EnvCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "env",
            "Show a redacted environment summary",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents oauth refresh command
pub struct OauthRefreshCommand {
    storage_dir: Option<PathBuf>,
}

impl OauthRefreshCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "oauth-refresh",
            "Show OAuth refresh guidance for stored credentials",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents issue command
pub struct IssueCommand;

impl IssueCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "issue",
            "Show bug reporting guidance",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents share command
pub struct ShareCommand;

impl ShareCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "share",
            "Show feature availability for session sharing",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents install command
pub struct InstallCommand;

impl InstallCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "install",
            "Show installation guidance for this platform",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents install github app command
pub struct InstallGithubAppCommand;

impl InstallGithubAppCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "install-github-app",
            "Show how to install the Claude GitHub App",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents install slack app command
pub struct InstallSlackAppCommand;

impl InstallSlackAppCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "install-slack-app",
            "Show how to install Claude for Slack",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

/// Represents create moved to plugin command
pub struct CreateMovedToPluginCommand;

impl CreateMovedToPluginCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "plugin-migrate",
            "Show plugin migration guidance for moved commands",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct ExtrasConfig {
    #[serde(default)]
    ant_trace_enabled: bool,
    #[serde(default)]
    mock_limits_enabled: bool,
    #[serde(default)]
    mock_limit_hits: u64,
    #[serde(default)]
    bughunter_enabled: bool,
    #[serde(default)]
    sandbox_enabled: bool,
    #[serde(default)]
    ultraplan_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extra_usage_quota_remaining: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote: Option<RemoteEnvConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct RemoteEnvConfig {
    host: String,
    port: u16,
    auth: String,
}

#[async_trait]
impl Command for HeapdumpCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_heapdump(&context)))
    }
}

#[async_trait]
impl Command for AntTraceCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(toggle_ant_trace(
            storage_root(None).as_deref(),
            &context,
            invocation.args.trim(),
        )?))
    }
}

#[async_trait]
impl Command for CtxVizCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_ctx_viz(
            storage_root(None).as_deref(),
            context.session_id,
        )?))
    }
}

#[async_trait]
impl Command for DebugToolCallCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let limit = invocation.args.trim().parse::<usize>().unwrap_or(5);
        Ok(CommandOutput::Text(render_debug_tool_call(
            storage_root(None).as_deref(),
            context.session_id,
            limit,
        )?))
    }
}

#[async_trait]
impl Command for GoodClaudeCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(record_good_claude(
            storage_root(None).as_deref(),
            &context,
        )?))
    }
}

#[async_trait]
impl Command for BreakCacheCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(break_cache(
            storage_root(None).as_deref(),
        )?))
    }
}

#[async_trait]
impl Command for BackfillSessionsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let (count, written) = backfill_sessions(self.storage_dir.as_deref())?;
        Ok(CommandOutput::Text(format!(
            "Backfill complete: {count} sessions scanned, {written} metadata records written."
        )))
    }
}

#[async_trait]
impl Command for PerfIssueCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_perf_issue(
            storage_root(None).as_deref(),
            &context,
        )?))
    }
}

#[async_trait]
impl Command for BughunterCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(toggle_bughunter(
            storage_root(None).as_deref(),
            invocation.args.trim(),
        )?))
    }
}

#[async_trait]
impl Command for BtwCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = invocation.args.trim();
        if args.is_empty() {
            return Ok(CommandOutput::Text(random_btw_tip().into()));
        }
        Ok(CommandOutput::Text(format!("By the way: {args}")))
    }
}

#[async_trait]
impl Command for AdvisorCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = invocation.args.trim();
        if args.is_empty() {
            return Ok(CommandOutput::Text(current_advisor_setting(
                self.storage_dir.as_deref(),
            )));
        }
        if matches!(args, "unset" | "off") {
            return Ok(CommandOutput::Text(
                "Advisor recommendation disabled for this session.".into(),
            ));
        }
        Ok(CommandOutput::Text(recommend_advisor(args)))
    }
}

#[async_trait]
impl Command for StickersCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_stickers().into()))
    }
}

#[async_trait]
impl Command for InitVerifiersCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_init_verifiers(
            self.tool_specs.as_ref(),
        )))
    }
}

#[async_trait]
impl Command for ExtraUsageCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_extra_usage(
            storage_root(None).as_deref(),
        )?))
    }
}

#[async_trait]
impl Command for PassesCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_passes(
            storage_root(None).as_deref(),
            &context,
        )?))
    }
}

#[async_trait]
impl Command for RateLimitOptionsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_rate_limit_options(
            storage_root(None).as_deref(),
        )?))
    }
}

#[async_trait]
impl Command for MockLimitsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(toggle_mock_limits(
            storage_root(None).as_deref(),
            invocation.args.trim(),
        )?))
    }
}

#[async_trait]
impl Command for ResetLimitsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(reset_limits(
            storage_root(None).as_deref(),
        )?))
    }
}

#[async_trait]
impl Command for OnboardingCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_onboarding().into()))
    }
}

#[async_trait]
impl Command for TeleportCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_teleport(
            storage_root(None).as_deref(),
            context.session_id,
        )?))
    }
}

#[async_trait]
impl Command for RemoteEnvCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_remote_env(
            storage_root(None).as_deref(),
        )?))
    }
}

#[async_trait]
impl Command for RemoteSetupCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(remote_setup(
            storage_root(None).as_deref(),
            invocation.args.trim(),
        )?))
    }
}

#[async_trait]
impl Command for BridgeKickCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_bridge_kick(
            storage_root(None).as_deref(),
        )?))
    }
}

#[async_trait]
impl Command for SandboxToggleCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(toggle_sandbox(
            storage_root(None).as_deref(),
            invocation.args.trim(),
        )?))
    }
}

#[async_trait]
impl Command for UltraplanCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(enable_ultraplan(
            storage_root(None).as_deref(),
        )?))
    }
}

#[async_trait]
impl Command for ThinkbackCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_thinkback(
            storage_root(None).as_deref(),
            context.session_id,
        )?))
    }
}

#[async_trait]
impl Command for ThinkbackPlayCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_thinkback_play(
            storage_root(None).as_deref(),
            context.session_id,
        )?))
    }
}

#[async_trait]
impl Command for AutofixPrCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(autofix_pr(invocation.args.trim())))
    }
}

#[async_trait]
impl Command for PrCommentsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = invocation.args.trim();
        if args.is_empty() || !executable_on_path("gh") {
            return Ok(CommandOutput::Text(pr_comments_usage(!executable_on_path(
                "gh",
            ))));
        }
        Ok(CommandOutput::Text(fetch_pr_comments(args)))
    }
}

#[async_trait]
impl Command for EnvCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_env_summary(&context)))
    }
}

#[async_trait]
impl Command for OauthRefreshCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let derived_storage = storage_root(None);
        let storage_dir = self.storage_dir.as_deref().or(derived_storage.as_deref());
        Ok(CommandOutput::Text(render_oauth_refresh(storage_dir)?))
    }
}

#[async_trait]
impl Command for IssueCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(open_issue_url()))
    }
}

#[async_trait]
impl Command for ShareCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(share_session(
            storage_root(None).as_deref(),
            context.session_id,
        )?))
    }
}

#[async_trait]
impl Command for InstallCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_install_help(&context)))
    }
}

#[async_trait]
impl Command for InstallGithubAppCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_install_github_app()))
    }
}

#[async_trait]
impl Command for InstallSlackAppCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_install_slack_app()))
    }
}

#[async_trait]
impl Command for CreateMovedToPluginCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(
            "This command has moved to a plugin. Run /plugin list to see available plugins.".into(),
        ))
    }
}

fn render_heapdump(context: &CommandContext) -> String {
    let status = parse_proc_status();
    [
        "## Heapdump".into(),
        "Rust does not expose a JS-style heap dump in this runtime.".into(),
        format!("pid={}", std::process::id()),
        format!("session_id={}", context.session_id),
        format!("cwd={}", context.cwd.display()),
        format!(
            "memory_rss={}",
            status_value(&status, "VmRSS").unwrap_or("unavailable")
        ),
        format!(
            "memory_size={}",
            status_value(&status, "VmSize").unwrap_or("unavailable")
        ),
        format!(
            "threads={}",
            status_value(&status, "Threads").unwrap_or("unavailable")
        ),
    ]
    .join("\n")
}

fn toggle_ant_trace(
    storage_dir: Option<&Path>,
    context: &CommandContext,
    args: &str,
) -> Result<String> {
    let mut config = read_extras_config(storage_dir)?;
    let requested = parse_toggle_request(args);
    if let Some(enabled) = requested {
        config.ant_trace_enabled = enabled;
        write_extras_config(storage_dir, &config)?;
    }
    let mut lines = vec![format!(
        "Ant trace {}.",
        if config.ant_trace_enabled {
            "enabled"
        } else {
            "disabled"
        }
    )];
    lines.push(render_ant_trace(context));
    Ok(lines.join("\n"))
}

fn render_ant_trace(context: &CommandContext) -> String {
    let features = context
        .features
        .iter()
        .map(|flag| format!("{flag:?}"))
        .collect::<Vec<_>>();
    [
        "## Ant Trace".into(),
        format!("session_id={}", context.session_id),
        format!("cwd={}", context.cwd.display()),
        format!("interactive={}", context.interactive),
        format!("authenticated={}", context.authenticated),
        format!("permission_mode={:?}", context.permission_mode),
        format!("features={}", features.join(",")),
        format!(
            "env.HOME={}",
            env::var("HOME").unwrap_or_else(|_| "unset".into())
        ),
        format!(
            "env.PATH={}",
            truncate_middle(&env::var("PATH").unwrap_or_else(|_| "unset".into()), 120)
        ),
        "tool_call_stack=unavailable in this build".into(),
    ]
    .join("\n")
}

fn render_ctx_viz(storage_dir: Option<&Path>, session_id: SessionId) -> Result<String> {
    let used = load_session_snapshot(storage_dir, session_id)
        .map(|snapshot| snapshot.state.costs.usage.total_tokens())
        .or_else(|| {
            env::var("WONDER_OF_U_CONTEXT_TOKENS_USED")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
        })
        .unwrap_or_default();
    let max = env::var("WONDER_OF_U_CONTEXT_TOKENS_MAX")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(200_000);
    let filled = ((used.saturating_mul(24)) / max.max(1)).min(24) as usize;
    let bar = format!(
        "[{}{}]",
        "#".repeat(filled),
        "-".repeat(24usize.saturating_sub(filled))
    );
    Ok(format!(
        "Context visualization\nused_tokens={used}\nmax_tokens={max}\nusage_bar={bar}"
    ))
}

fn render_debug_tool_call(
    storage_dir: Option<&Path>,
    session_id: SessionId,
    limit: usize,
) -> Result<String> {
    let messages = load_session_messages(storage_dir, session_id)?;
    let mut calls = recent_tool_calls(&messages, limit.max(1));
    if calls.is_empty() {
        return Ok("Tool call debug: no tool calls recorded for this session.".into());
    }
    calls.insert(0, format!("Recent tool calls (last {}):", calls.len()));
    Ok(calls.join("\n"))
}

fn backfill_sessions(storage_dir: Option<&Path>) -> Result<(usize, usize)> {
    let Some(storage_dir) = storage_dir else {
        return Ok((0, 0));
    };
    let store = TranscriptStore::new(storage_dir);
    store.ensure_layout()?;
    let mut scanned = 0;
    let mut written = 0;
    for entry in fs::read_dir(store.paths().sessions_dir())? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        scanned += 1;
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        let Ok(session_id) = SessionId::parse(stem) else {
            continue;
        };
        if store.paths().metadata_path(session_id).exists() {
            continue;
        }
        let transcript = store.load_session(session_id)?;
        let snapshot = store.read_snapshot_if_exists(session_id)?;
        let metadata = synthesize_metadata(session_id, &transcript.messages, snapshot.as_ref());
        store.write_metadata(&metadata)?;
        written += 1;
    }
    Ok((scanned, written))
}

fn render_perf_issue(storage_dir: Option<&Path>, context: &CommandContext) -> Result<String> {
    let status = parse_proc_status();
    let uptime = estimated_process_uptime_seconds()
        .map(|seconds| format!("{seconds:.2}"))
        .unwrap_or_else(|| "unavailable".into());
    let session_stats = session_storage_stats(storage_dir, context.session_id)?;
    Ok([
        "## Perf Issue".into(),
        format!("pid={}", std::process::id()),
        format!("process_uptime_seconds_estimate={uptime}"),
        format!(
            "memory_rss={}",
            status_value(&status, "VmRSS").unwrap_or("unavailable")
        ),
        format!(
            "memory_peak={}",
            status_value(&status, "VmHWM").unwrap_or("unavailable")
        ),
        format!(
            "open_file_descriptors={}",
            fd_count()
                .map(|count| count.to_string())
                .unwrap_or_else(|| "unavailable".into())
        ),
        format!("session_bytes={}", session_stats.0),
        format!("metadata_present={}", session_stats.1),
        format!("snapshot_present={}", session_stats.2),
    ]
    .join("\n"))
}

fn current_advisor_setting(storage_dir: Option<&Path>) -> String {
    if let Ok(model) = env::var("WONDER_OF_U_ADVISOR_MODEL").or_else(|_| env::var("ADVISOR_MODEL"))
    {
        return format!("Advisor: {model}");
    }
    if let Some(storage_dir) = storage_dir {
        if let Ok(settings) = SettingsStore::new(storage_dir).read() {
            if let Some(model) = settings.selected_model {
                return format!("Advisor: unset (current primary model: {model})");
            }
        }
    }
    "Advisor: unset".into()
}

fn recommend_advisor(task: &str) -> String {
    let lower = task.to_ascii_lowercase();
    let recommendation = if lower.contains("debug")
        || lower.contains("fix")
        || lower.contains("bug")
        || lower.contains("investigate")
    {
        "claude-3.7-sonnet"
    } else if lower.contains("plan")
        || lower.contains("architecture")
        || lower.contains("refactor")
        || lower.contains("design")
    {
        "claude-3.7-opus"
    } else if lower.contains("test")
        || lower.contains("lint")
        || lower.contains("review")
        || lower.contains("comment")
    {
        "claude-3.5-haiku"
    } else {
        "claude-3.7-sonnet"
    };
    format!("Advisor recommendation: {recommendation}\nreason={task}")
}

fn verifier_tool_names(tool_specs: &[ToolSpec]) -> Vec<String> {
    tool_specs
        .iter()
        .filter(|spec| {
            spec.read_only
                || matches!(
                    spec.name.as_str(),
                    "grep" | "rg" | "search" | "find" | "read-file" | "view"
                )
                || spec.description.to_ascii_lowercase().contains("verify")
                || spec.description.to_ascii_lowercase().contains("diagnostic")
        })
        .map(|spec| spec.name.clone())
        .collect()
}

fn render_init_verifiers(tool_specs: &[ToolSpec]) -> String {
    let mut lines = vec!["Verifier initialization".into()];
    for (tool, version_args) in [
        ("git", vec!["--version"]),
        ("node", vec!["--version"]),
        ("python3", vec!["--version"]),
    ] {
        lines.push(tool_version(tool, &version_args));
    }
    let verifiers = verifier_tool_names(tool_specs);
    lines.push(format!(
        "registered_read_only_tools={}",
        if verifiers.is_empty() {
            "none".into()
        } else {
            verifiers.join(", ")
        }
    ));
    lines.join("\n")
}

fn render_extra_usage(storage_dir: Option<&Path>) -> Result<String> {
    let config = read_extras_config(storage_dir)?;
    let disabled = env::var_os("DISABLE_EXTRA_USAGE_COMMAND").is_some();
    Ok(format!(
        "Extra usage\npolicy_disabled={disabled}\nquota_remaining={}\nstatus={}",
        config
            .extra_usage_quota_remaining
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unconfigured".into()),
        if disabled { "disabled" } else { "available" }
    ))
}

fn render_passes(storage_dir: Option<&Path>, context: &CommandContext) -> Result<String> {
    let settings = storage_dir
        .map(SettingsStore::new)
        .map(|store| store.read())
        .transpose()?
        .unwrap_or_default();
    let mut entitlements = context
        .features
        .iter()
        .map(|feature| format!("{feature:?}"))
        .collect::<Vec<_>>();
    if settings.fast_mode {
        entitlements.push("fast_mode".into());
    }
    if settings.effort_level.is_some() {
        entitlements.push("effort_controls".into());
    }
    entitlements.sort();
    entitlements.dedup();
    Ok(format!(
        "Available passes / entitlements\ncount={}\n{}",
        entitlements.len(),
        entitlements
            .into_iter()
            .map(|entry| format!("• {entry}"))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

fn render_rate_limit_options(storage_dir: Option<&Path>) -> Result<String> {
    let config = read_extras_config(storage_dir)?;
    Ok(format!(
        "Rate limit options\n• wait_for_reset=true\n• extra_usage_quota_remaining={}\n• mock_limits_enabled={}\n• mock_limit_hits={}\n• switch_model=/model\n• advisor_hint=/advisor <task>",
        config
            .extra_usage_quota_remaining
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unconfigured".into()),
        config.mock_limits_enabled,
        config.mock_limit_hits,
    ))
}

fn toggle_mock_limits(storage_dir: Option<&Path>, args: &str) -> Result<String> {
    if !cfg!(debug_assertions) {
        return Ok("Mock limits are only available in development builds.".into());
    }
    let mut config = read_extras_config(storage_dir)?;
    if let Some(enabled) = parse_toggle_request(args) {
        config.mock_limits_enabled = enabled;
        if enabled {
            config.mock_limit_hits = config.mock_limit_hits.saturating_add(1);
        }
        write_extras_config(storage_dir, &config)?;
    }
    Ok(format!(
        "Mock limits {}\nmock_limit_hits={}",
        if config.mock_limits_enabled {
            "enabled"
        } else {
            "disabled"
        },
        config.mock_limit_hits
    ))
}

fn reset_limits(storage_dir: Option<&Path>) -> Result<String> {
    let mut config = read_extras_config(storage_dir)?;
    config.mock_limit_hits = 0;
    write_extras_config(storage_dir, &config)?;
    Ok(format!(
        "Mock rate-limit state reset.\nmock_limits_enabled={}",
        config.mock_limits_enabled
    ))
}

fn render_onboarding() -> &'static str {
    "Onboarding checklist\n1. /help — inspect available commands\n2. /model — choose a model\n3. /config — review local defaults\n4. /init — bootstrap the repository\n5. /compact — checkpoint long sessions"
}

fn toggle_bughunter(storage_dir: Option<&Path>, args: &str) -> Result<String> {
    let mut config = read_extras_config(storage_dir)?;
    if let Some(enabled) = parse_toggle_request(args) {
        config.bughunter_enabled = enabled;
        write_extras_config(storage_dir, &config)?;
    } else if args.is_empty() {
        config.bughunter_enabled = true;
        write_extras_config(storage_dir, &config)?;
    }
    Ok(format!(
        "Bug hunter mode {}\nnext_steps=collect repro steps, run /perf-issue, then open /issue",
        if config.bughunter_enabled {
            "enabled"
        } else {
            "disabled"
        }
    ))
}

fn random_btw_tip() -> &'static str {
    const TIPS: &[&str] = &[
        "BTW: use /compact before long refactors to keep context tight.",
        "BTW: `/pr-comments <number>` is handy before addressing review feedback.",
        "BTW: `/summary` gives a quick health check of the current session.",
        "BTW: capture checkpoints early if you expect to use /rewind later.",
    ];
    let index = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() as usize % TIPS.len())
        .unwrap_or(0);
    TIPS[index]
}

fn render_stickers() -> &'static str {
    "Claude stickers\n /\\_/\\\\\n( ^.^ )\n > ^ <\n\n(=^･ω･^=)\n  /|_|\\\\"
}

fn render_teleport(storage_dir: Option<&Path>, session_id: SessionId) -> Result<String> {
    let Some(storage_dir) = storage_dir else {
        return Ok("Teleport: storage directory unavailable.".into());
    };
    let store = TranscriptStore::new(storage_dir);
    let metadata = store.list_metadata()?;
    if metadata.is_empty() {
        return Ok("Teleport: no stored sessions found.".into());
    }
    let mut lines = vec!["Teleport sessions".into()];
    for entry in metadata.into_iter().take(10) {
        let snapshot = store.read_snapshot_if_exists(entry.session_id)?.is_some();
        let status = if entry.session_id == session_id {
            "current"
        } else if snapshot {
            "checkpoint"
        } else {
            "transcript"
        };
        lines.push(format!(
            "• {} [{}] {}",
            entry.session_id, status, entry.title
        ));
    }
    Ok(lines.join("\n"))
}

fn render_remote_env(storage_dir: Option<&Path>) -> Result<String> {
    let config = read_extras_config(storage_dir)?;
    Ok(match config.remote {
        Some(remote) => format!(
            "Remote environment\nhost={}\nport={}\nauth={}",
            remote.host, remote.port, remote.auth
        ),
        None => "Remote environment not configured.\nUse /remote-setup <host[:port]> [auth]".into(),
    })
}

fn remote_setup(storage_dir: Option<&Path>, args: &str) -> Result<String> {
    if args.is_empty() {
        return render_remote_env(storage_dir)
            .map(|current| format!("{current}\nusage=/remote-setup example.com:22 ssh"));
    }
    let mut config = read_extras_config(storage_dir)?;
    if args.eq_ignore_ascii_case("clear") {
        config.remote = None;
        write_extras_config(storage_dir, &config)?;
        return Ok("Remote environment cleared.".into());
    }
    let mut parts = args.split_whitespace();
    let host_port = parts.next().unwrap_or_default();
    let auth = parts.next().unwrap_or("ssh").to_string();
    let (host, port) = match host_port.split_once(':') {
        Some((host, port)) => (host.to_string(), port.parse::<u16>().unwrap_or(22)),
        None => (host_port.to_string(), 22),
    };
    config.remote = Some(RemoteEnvConfig { host, port, auth });
    write_extras_config(storage_dir, &config)?;
    render_remote_env(storage_dir)
}

fn render_bridge_kick(storage_dir: Option<&Path>) -> Result<String> {
    let config = read_extras_config(storage_dir)?;
    Ok(match config.remote {
        Some(remote) => format!(
            "Bridge status\nremote_target={}:{}\nstatus=restart required\ninstructions=restart the bridge process, then retry /teleport",
            remote.host, remote.port
        ),
        None => "Bridge status\nstatus=not configured\ninstructions=run /remote-setup first".into(),
    })
}

fn toggle_sandbox(storage_dir: Option<&Path>, args: &str) -> Result<String> {
    let mut config = read_extras_config(storage_dir)?;
    if let Some(enabled) = parse_toggle_request(args) {
        config.sandbox_enabled = enabled;
        write_extras_config(storage_dir, &config)?;
    } else if args.is_empty() {
        config.sandbox_enabled = !config.sandbox_enabled;
        write_extras_config(storage_dir, &config)?;
    }
    Ok(format!(
        "Sandbox {}\nplatform={}",
        if config.sandbox_enabled {
            "enabled"
        } else {
            "disabled"
        },
        env::consts::OS
    ))
}

fn enable_ultraplan(storage_dir: Option<&Path>) -> Result<String> {
    let mut config = read_extras_config(storage_dir)?;
    config.ultraplan_enabled = true;
    write_extras_config(storage_dir, &config)?;
    Ok("Ultraplan enabled.\nGuidance=spend extra time decomposing the task, enumerate risks, and checkpoint before implementation.".into())
}

fn render_thinkback(storage_dir: Option<&Path>, session_id: SessionId) -> Result<String> {
    let Some(block) = thinking_blocks(&load_session_messages(storage_dir, session_id)?)
        .into_iter()
        .last()
    else {
        return Ok("Thinkback: no extended thinking blocks recorded.".into());
    };
    Ok(format!("Most recent extended thinking\n{block}"))
}

fn render_thinkback_play(storage_dir: Option<&Path>, session_id: SessionId) -> Result<String> {
    let blocks = thinking_blocks(&load_session_messages(storage_dir, session_id)?);
    if blocks.is_empty() {
        return Ok("Thinkback playback: no extended thinking blocks recorded.".into());
    }
    Ok(blocks
        .into_iter()
        .enumerate()
        .map(|(index, block)| format!("## Block {}\n{}", index + 1, block))
        .collect::<Vec<_>>()
        .join("\n\n"))
}

fn storage_root(explicit: Option<&Path>) -> Option<PathBuf> {
    explicit
        .map(Path::to_path_buf)
        .or_else(|| env::var_os("WONDER_OF_U_STORAGE_DIR").map(PathBuf::from))
        .or_else(|| {
            env::var_os("XDG_CONFIG_HOME").map(|path| PathBuf::from(path).join("wonder-of-u"))
        })
        .or_else(|| env::var_os("HOME").map(|path| PathBuf::from(path).join(".config/wonder-of-u")))
}

fn extras_config_path(storage_dir: &Path) -> PathBuf {
    StoragePaths::new(storage_dir)
        .config_dir()
        .join("extras-config.json")
}

fn read_extras_config(storage_dir: Option<&Path>) -> Result<ExtrasConfig> {
    let Some(storage_dir) = storage_dir else {
        return Ok(ExtrasConfig::default());
    };
    let path = extras_config_path(storage_dir);
    if !path.exists() {
        return Ok(ExtrasConfig::default());
    }
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

fn write_extras_config(storage_dir: Option<&Path>, config: &ExtrasConfig) -> Result<()> {
    let Some(storage_dir) = storage_dir else {
        return Ok(());
    };
    let path = extras_config_path(storage_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(config)?)?;
    Ok(())
}

fn parse_toggle_request(args: &str) -> Option<bool> {
    match args.trim().to_ascii_lowercase().as_str() {
        "1" | "on" | "enable" | "enabled" | "true" => Some(true),
        "0" | "off" | "disable" | "disabled" | "false" => Some(false),
        _ => None,
    }
}

fn load_session_snapshot(
    storage_dir: Option<&Path>,
    session_id: SessionId,
) -> Option<wonder_of_u_storage::SessionSnapshot> {
    let storage_dir = storage_dir?;
    TranscriptStore::new(storage_dir)
        .read_snapshot_if_exists(session_id)
        .ok()
        .flatten()
}

fn load_session_messages(
    storage_dir: Option<&Path>,
    session_id: SessionId,
) -> Result<Vec<MessageEnvelope>> {
    let Some(storage_dir) = storage_dir else {
        return Ok(Vec::new());
    };
    let store = TranscriptStore::new(storage_dir);
    if let Some(snapshot) = store.read_snapshot_if_exists(session_id)? {
        return Ok(snapshot.state.messages);
    }
    Ok(store
        .load_session(session_id)
        .map(|transcript| transcript.messages)
        .unwrap_or_default())
}

fn recent_tool_calls(messages: &[MessageEnvelope], limit: usize) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| match &message.payload {
            MessagePayload::AssistantToolUse { tool, input, .. } => Some(format!(
                "• {} @ {} input={}",
                tool,
                message.timestamp,
                preview_json(input)
            )),
            _ => None,
        })
        .rev()
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn thinking_blocks(messages: &[MessageEnvelope]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| match &message.payload {
            MessagePayload::AssistantThinking { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect()
}

const HELP_SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/help", "Show this help"),
    ("/clear", "Clear conversation history"),
    ("/compact", "Compact conversation context"),
    ("/thinking", "Toggle extended thinking on/off"),
    ("/brief", "Toggle concise-response mode"),
    (
        "/optimize-tonken",
        "Toggle token-optimisation mode (alias: /optimize-token)",
    ),
    ("/stats", "Show session statistics"),
    ("/login", "Authenticate with a provider"),
    ("/logout", "Sign out"),
    ("/doctor", "Run diagnostic checks"),
    ("/model", "Switch AI model"),
    ("/search", "(ctrl+f) Search workspace files"),
    ("/status", "Show provider status"),
    ("/review", "Start code review mode"),
    ("/exit", "Exit the TUI"),
];

const HELP_KEYBOARD_SHORTCUTS: &[(&str, &str)] = &[
    ("ctrl+c", "Interrupt current operation"),
    ("ctrl+l", "Clear screen"),
    ("ctrl+r", "History search"),
    ("ctrl+f", "Global file search"),
    ("ctrl+o", "Expand/collapse tool output"),
    ("esc", "Cancel / close overlay"),
    ("enter", "Submit prompt"),
    ("shift+enter", "Insert newline"),
    ("↑/↓", "Scroll transcript"),
];

/// Renders the `/help` slash-command response for the TUI transcript.
pub fn execute_help_command() -> Result<String> {
    let mut lines = vec!["Slash Commands".into()];
    append_help_rows(&mut lines, HELP_SLASH_COMMANDS);
    lines.push(String::new());
    lines.push("Keyboard Shortcuts".into());
    append_help_rows(&mut lines, HELP_KEYBOARD_SHORTCUTS);
    Ok(lines.join("\n"))
}

fn append_help_rows(lines: &mut Vec<String>, rows: &[(&str, &str)]) {
    let label_width = rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or_default()
        + 2;
    lines.extend(
        rows.iter()
            .map(|(label, description)| format!("{label:<label_width$}{description}")),
    );
}

/// Renders the `/thinking` slash-command response for the current session state.
pub fn execute_thinking_command(app: &AppState, arg: Option<&str>) -> Result<String> {
    let current = app.thinking_enabled;
    let effort = match app.thinking_effort {
        ThinkingEffort::Low => "low",
        ThinkingEffort::Medium => "medium",
        ThinkingEffort::High => "high",
    };
    match arg {
        Some("on") => Ok("Thinking enabled — Claude will reason before responding".into()),
        Some("off") => Ok("Thinking disabled".into()),
        Some("low") => Ok("Thinking effort set to low".into()),
        Some("medium") => Ok("Thinking effort set to medium".into()),
        Some("high") => Ok("Thinking effort set to high".into()),
        None | Some("") => Ok(format!(
            "Thinking is currently {} (effort: {effort}). Options: on, off, low, medium, high",
            if current { "enabled" } else { "disabled" }
        )),
        Some(other) => Err(WonderError::Validation(format!(
            "Unknown argument: {other}. Use 'on', 'off', 'low', 'medium', or 'high'"
        ))),
    }
}

/// Renders session-local token usage and estimated cost statistics.
pub fn execute_stats_command(app: &AppState) -> Result<String> {
    let usage = app.costs.usage;
    let cost = app.costs.estimated_cost_usd.unwrap_or_default();
    let model = app.model.as_deref().unwrap_or("unknown");
    let provider = app.provider.as_deref().unwrap_or("unknown");

    let mut lines = vec![
        "Session Statistics".into(),
        "─────────────────────────────".into(),
        format!("Provider:  {provider}"),
        format!("Model:     {model}"),
        "─────────────────────────────".into(),
        format!("Input tokens:        {:>10}", usage.input_tokens),
        format!("Output tokens:       {:>10}", usage.output_tokens),
        format!("Cache create tokens: {:>10}", usage.cache_creation_tokens),
        format!("Cache read tokens:   {:>10}", usage.cache_read_tokens),
        format!("Total tokens:        {:>10}", usage.total_tokens()),
        "─────────────────────────────".into(),
    ];
    if cost > 0.0 {
        lines.push(format!("Estimated cost:      ${cost:.4}"));
    }
    Ok(lines.join("\n"))
}

/// Renders the `/settings` slash-command response for the current session state.
pub fn execute_settings_command(app: &AppState) -> Result<String> {
    let usage = app.costs.usage;
    let provider = app.provider.as_deref().unwrap_or("unknown");
    let model = app.model.as_deref().unwrap_or("unknown");
    let storage = storage_root(None)
        .map(|root| StoragePaths::new(root).sessions_dir())
        .map(|path| home_relative_path(&path))
        .unwrap_or_else(|| "unavailable".into());
    let total_cost = app.costs.estimated_cost_usd.unwrap_or_default();

    Ok([
        "── Configuration ──────────────────────────────".into(),
        format_settings_row("Provider:", provider),
        format_settings_row("Model:", model),
        format_settings_row("Storage:", &storage),
        format_settings_row("Permission:", permission_mode_label(app.permission_mode)),
        format_settings_row("Thinking:", if app.thinking_enabled { "on" } else { "off" }),
        String::new(),
        "── Session Usage ───────────────────────────────".into(),
        format_settings_row("Input tokens:", &format_token_count(usage.input_tokens)),
        format_settings_row("Output tokens:", &format_token_count(usage.output_tokens)),
        format_settings_row("Cache read:", &format_token_count(usage.cache_read_tokens)),
        format_settings_row(
            "Cache write:",
            &format_token_count(usage.cache_creation_tokens),
        ),
        format_settings_row("Total cost:", &format!("${total_cost:.4}")),
        String::new(),
        "── Provider Status ─────────────────────────────".into(),
        format_settings_row("Auth:", &render_auth_status(&app.auth)),
        format_settings_row(
            "Context:",
            &render_context_status(usage.total_tokens(), app.context_window_size),
        ),
    ]
    .join("\n"))
}

fn format_settings_row(label: &str, value: &str) -> String {
    format!("  {label:<14}{value}")
}

fn render_auth_status(auth: &AuthState) -> String {
    let symbol = if auth.is_ready() { "✓" } else { "!" };
    let status = match auth.status_label() {
        "not_required" => "ready",
        other => other,
    };
    format!("{symbol} {status} ({})", auth_kind_display(auth.kind))
}

fn auth_kind_display(kind: AuthMaterialKind) -> &'static str {
    match kind {
        AuthMaterialKind::None => "none",
        AuthMaterialKind::ApiKey => "api-key",
        AuthMaterialKind::OAuth => "oauth",
        AuthMaterialKind::AwsSigV4 => "aws-sigv4",
    }
}

fn render_context_status(used_tokens: u64, max_tokens: Option<u64>) -> String {
    let Some(max_tokens) = max_tokens.filter(|max_tokens| *max_tokens > 0) else {
        return "unknown".into();
    };
    let percentage = used_tokens.saturating_mul(100) / max_tokens;
    format!(
        "{} / {} tokens ({}%)",
        format_token_count(used_tokens),
        format_token_count(max_tokens),
        percentage.min(100)
    )
}

fn format_token_count(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(digit);
    }
    formatted
}

fn home_relative_path(path: &Path) -> String {
    let Some(home) = env::var_os("HOME").filter(|home| !home.is_empty()) else {
        return path.display().to_string();
    };
    let home = PathBuf::from(home);
    match path.strip_prefix(&home) {
        Ok(suffix) if suffix.as_os_str().is_empty() => "~".into(),
        Ok(suffix) => format!("~/{}", suffix.display()),
        Err(_) => path.display().to_string(),
    }
}

fn synthesize_metadata(
    session_id: SessionId,
    messages: &[MessageEnvelope],
    snapshot: Option<&wonder_of_u_storage::SessionSnapshot>,
) -> SessionMetadata {
    let created_at = messages
        .first()
        .map(|message| message.timestamp)
        .unwrap_or_else(OffsetDateTime::now_utc);
    let updated_at = messages
        .last()
        .map(|message| message.timestamp)
        .unwrap_or(created_at);
    let cwd = snapshot
        .map(|snapshot| snapshot.state.session.cwd.clone())
        .or_else(|| messages.iter().find_map(|message| message.cwd.clone()))
        .unwrap_or_else(|| PathBuf::from("."));
    let title = snapshot
        .map(|snapshot| snapshot.state.session.title.clone())
        .unwrap_or_else(|| format!("Session {}", &session_id.to_string()[..8]));
    SessionMetadata {
        schema_version: STORAGE_SCHEMA_VERSION,
        session_id,
        title,
        cwd,
        git_branch: snapshot
            .and_then(|snapshot| snapshot.state.session.git_branch.clone())
            .or_else(|| {
                messages
                    .iter()
                    .find_map(|message| message.git_branch.clone())
            }),
        entrypoint: snapshot
            .and_then(|snapshot| snapshot.state.session.entrypoint.clone())
            .or_else(|| {
                messages
                    .iter()
                    .find_map(|message| message.entrypoint.clone())
            }),
        app_version: snapshot
            .and_then(|snapshot| snapshot.state.session.app_version.clone())
            .or_else(|| {
                messages
                    .iter()
                    .find_map(|message| message.app_version.clone())
            }),
        created_at,
        updated_at,
        message_count: messages.len(),
        tags: snapshot
            .map(|snapshot| snapshot.state.session.tags.clone())
            .unwrap_or_default(),
        provider: snapshot.and_then(|snapshot| snapshot.state.provider.clone()),
        model: snapshot.and_then(|snapshot| snapshot.state.model.clone()),
        auth: snapshot
            .map(|snapshot| snapshot.state.auth.clone())
            .unwrap_or_default(),
        costs: snapshot
            .map(|snapshot| snapshot.state.costs.clone())
            .unwrap_or_default(),
    }
}

fn session_storage_stats(
    storage_dir: Option<&Path>,
    session_id: SessionId,
) -> Result<(u64, bool, bool)> {
    let Some(storage_dir) = storage_dir else {
        return Ok((0, false, false));
    };
    let paths = StoragePaths::new(storage_dir);
    let transcript_bytes = fs::metadata(paths.transcript_path(session_id))
        .map(|metadata| metadata.len())
        .unwrap_or_default();
    Ok((
        transcript_bytes,
        paths.metadata_path(session_id).exists(),
        paths.snapshot_path(session_id).exists(),
    ))
}

fn record_good_claude(storage_dir: Option<&Path>, context: &CommandContext) -> Result<String> {
    let Some(storage_dir) = storage_dir else {
        return Ok("✓ Good Claude! Positive reinforcement noted.".into());
    };
    let path = StoragePaths::new(storage_dir)
        .config_dir()
        .join("good-claude.log");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(
        file,
        "{}\tsession={}\tcwd={}",
        OffsetDateTime::now_utc(),
        context.session_id,
        context.cwd.display()
    )?;
    Ok(format!(
        "✓ Good Claude! Logged positive reinforcement to {}",
        path.display()
    ))
}

fn break_cache(storage_dir: Option<&Path>) -> Result<String> {
    let Some(storage_dir) = storage_dir else {
        return Ok("Cache clear skipped: storage directory unavailable.".into());
    };
    let path = storage_dir.join("cache-break.sentinel");
    fs::create_dir_all(storage_dir)?;
    fs::write(&path, format!("{}", OffsetDateTime::now_utc()))?;
    Ok(format!("Cache sentinel updated: {}", path.display()))
}

fn tool_version(tool: &str, version_args: &[&str]) -> String {
    match ProcessCommand::new(tool).args(version_args).output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let line = if !stdout.is_empty() { stdout } else { stderr };
            format!("{tool}={line}")
        }
        Ok(output) => format!("{tool}=unavailable(status={})", output.status),
        Err(_) => format!("{tool}=missing"),
    }
}

fn preview_json(value: &Value) -> String {
    let rendered = serde_json::to_string(value).unwrap_or_else(|_| "<invalid-json>".into());
    truncate_middle(&rendered, 80)
}

fn pr_comments_usage(include_gh_note: bool) -> String {
    let mut usage =
        "Usage: /pr-comments [pr-number]\nFetches PR-level and code review comments from GitHub."
            .to_string();
    if include_gh_note {
        usage.push_str("\nRequires gh CLI (https://cli.github.com).");
    }
    usage
}

fn fetch_pr_comments(pr_number: &str) -> String {
    match fetch_pr_comments_inner(pr_number) {
        Ok(output) => output,
        Err(error) => format!("pr-comments: {error}"),
    }
}

fn fetch_pr_comments_inner(pr_number: &str) -> std::result::Result<String, String> {
    let pr = gh_json([
        "pr",
        "view",
        pr_number,
        "--json",
        "number,url,headRefName,headRepository",
    ])?;
    let number = pr
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(|| "missing PR number in gh output".to_string())?;
    let url = pr.get("url").and_then(Value::as_str).unwrap_or("unknown");
    let branch = pr
        .get("headRefName")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let repository = pr
        .get("headRepository")
        .ok_or_else(|| "missing headRepository in gh output".to_string())?;
    let owner = repository
        .get("owner")
        .and_then(|value| value.get("login"))
        .and_then(Value::as_str)
        .ok_or_else(|| "missing repository owner in gh output".to_string())?;
    let repo = repository
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing repository name in gh output".to_string())?;

    let issue_comments = gh_json([
        "api",
        &format!("repos/{owner}/{repo}/issues/{number}/comments"),
    ])?;
    let review_comments = gh_json([
        "api",
        &format!("repos/{owner}/{repo}/pulls/{number}/comments"),
    ])?;
    format_pr_comments(number, url, branch, &issue_comments, &review_comments)
}

fn gh_json<const N: usize>(args: [&str; N]) -> std::result::Result<Value, String> {
    let output = ProcessCommand::new("gh")
        .args(args)
        .output()
        .map_err(|error| format!("failed to invoke gh: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(if detail.is_empty() {
            "gh command failed".into()
        } else {
            detail
        });
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid gh JSON output: {error}"))
}

fn format_pr_comments(
    number: u64,
    url: &str,
    branch: &str,
    issue_comments: &Value,
    review_comments: &Value,
) -> std::result::Result<String, String> {
    let issue_comments = issue_comments
        .as_array()
        .ok_or_else(|| "issue comments payload was not an array".to_string())?;
    let review_comments = review_comments
        .as_array()
        .ok_or_else(|| "review comments payload was not an array".to_string())?;

    let mut lines = vec![
        format!("PR Comments for #{number}"),
        format!("url={url}"),
        format!("branch={branch}"),
        format!("issue_comments={}", issue_comments.len()),
        format!("review_comments={}", review_comments.len()),
    ];
    if !issue_comments.is_empty() {
        lines.push("Issue comments:".into());
        for comment in issue_comments {
            lines.push(format!(
                "• {} @ {}: {}",
                json_path_str(comment, &["user", "login"]).unwrap_or("unknown"),
                comment
                    .get("created_at")
                    .or_else(|| comment.get("createdAt"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
                preview_comment_body(comment.get("body").and_then(Value::as_str).unwrap_or(""))
            ));
        }
    }
    if !review_comments.is_empty() {
        lines.push("Review comments:".into());
        for comment in review_comments {
            let path = comment
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let line = comment
                .get("line")
                .and_then(Value::as_i64)
                .map(|line| line.to_string())
                .unwrap_or_else(|| "unknown".into());
            lines.push(format!(
                "• {} {}:{}: {}",
                json_path_str(comment, &["user", "login"]).unwrap_or("unknown"),
                path,
                line,
                preview_comment_body(comment.get("body").and_then(Value::as_str).unwrap_or(""))
            ));
        }
    }
    Ok(lines.join("\n"))
}

fn json_path_str<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_str()
}

fn preview_comment_body(body: &str) -> String {
    let preview = body.lines().next().unwrap_or_default().trim();
    if preview.chars().count() <= 100 {
        preview.to_string()
    } else {
        format!("{}…", preview.chars().take(100).collect::<String>())
    }
}

fn render_env_summary(context: &CommandContext) -> String {
    let shell = env::var("SHELL").unwrap_or_else(|_| "unset".into());
    let path_entries = env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).count())
        .unwrap_or_default();
    [
        "## Environment".into(),
        format!("cwd={}", context.cwd.display()),
        format!("shell={shell}"),
        format!("os={}", env::consts::OS),
        format!(
            "rust_env={}",
            env::var("RUST_ENV").unwrap_or_else(|_| "unset".into())
        ),
        format!("path_entries={path_entries}"),
        format!(
            "path_summary={}",
            truncate_middle(&env::var("PATH").unwrap_or_else(|_| "unset".into()), 120)
        ),
    ]
    .join("\n")
}

fn render_install_help(context: &CommandContext) -> String {
    let shell = env::var("SHELL").unwrap_or_else(|_| "unknown".into());
    let rc_file = shell_rc_path();
    let exe_dir = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let on_path = exe_dir.as_deref().map(path_contains).unwrap_or(false);
    format!(
        "Install diagnostics\ncwd={}\nos={}\nshell={shell}\npath_configured={on_path}\nrc_file={}\ncompletion_hint={}",
        context.cwd.display(),
        env::consts::OS,
        rc_file
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "unavailable".into()),
        completion_hint(&shell),
    )
}

fn github_auth_status() -> &'static str {
    let Ok(status) = ProcessCommand::new("gh").args(["auth", "status"]).status() else {
        return "unavailable";
    };
    if status.success() {
        "authenticated"
    } else {
        "not-authenticated"
    }
}

fn render_oauth_refresh(storage_dir: Option<&Path>) -> Result<String> {
    let mut lines = vec!["OAuth refresh diagnostics".into()];
    if let Some(home) = env::var_os("HOME") {
        let auth_path = PathBuf::from(home).join(".claude").join("auth.json");
        lines.push(format!("claude_auth_path={}", auth_path.display()));
        lines.push(format!("claude_auth_present={}", auth_path.exists()));
        if auth_path.exists() {
            let value: Value = serde_json::from_str(&fs::read_to_string(&auth_path)?)?;
            lines.push(format!(
                "claude_refresh_token_present={}",
                value.get("refresh_token").is_some() || value.get("refreshToken").is_some()
            ));
        }
    }
    if let Some(storage_dir) = storage_dir {
        let credentials = CredentialStore::new(storage_dir).read()?;
        let oauth_providers = credentials
            .providers
            .iter()
            .filter(|(_, auth)| matches!(auth, AuthMaterial::OAuth { .. }))
            .count();
        lines.push(format!("stored_oauth_providers={oauth_providers}"));
    }
    lines.push("refresh_status=validation_only".into());
    Ok(lines.join("\n"))
}

fn open_issue_url() -> String {
    let url = "https://github.com/anthropics/claude-code/issues";
    if super::try_open_browser(url) {
        format!("Opened issue tracker: {url}")
    } else {
        format!("Issue tracker: {url}")
    }
}

fn share_session(storage_dir: Option<&Path>, session_id: SessionId) -> Result<String> {
    let Some(storage_dir) = storage_dir else {
        return Ok("Share export unavailable: storage directory not configured.".into());
    };
    let store = TranscriptStore::new(storage_dir);
    let export_dir = storage_dir.join("exports");
    fs::create_dir_all(&export_dir)?;
    let export_path = export_dir.join(format!("session-{session_id}.json"));
    let snapshot = store.read_snapshot_if_exists(session_id)?;
    let transcript = store.load_session(session_id).ok();
    let payload = json!({
        "session_id": session_id,
        "snapshot": snapshot,
        "transcript": transcript,
    });
    fs::write(&export_path, serde_json::to_vec_pretty(&payload)?)?;
    Ok(format!(
        "Session export written to {}",
        export_path.display()
    ))
}

fn render_install_github_app() -> String {
    format!(
        "Install GitHub App\nurl=https://github.com/apps/claude\n1. Open the app page\n2. Pick the target repository or org\n3. Confirm permissions\n4. Re-run `gh auth status`\ngh_auth_status={}",
        github_auth_status()
    )
}

fn render_install_slack_app() -> String {
    "Install Slack App\nurl=https://slack.com/apps/claude\n1. Open the app listing\n2. Choose a workspace\n3. Approve the requested scopes\n4. Reconnect Claude Code if prompted".into()
}

fn autofix_pr(args: &str) -> String {
    if !executable_on_path("gh") {
        return "autofix-pr: requires gh CLI (https://cli.github.com).".into();
    }
    let mut command = ProcessCommand::new("gh");
    command.arg("pr").arg("diff");
    if !args.is_empty() {
        command.arg(args);
    }
    match command.output() {
        Ok(output) if output.status.success() => {
            let diff = String::from_utf8_lossy(&output.stdout);
            let files = diff
                .lines()
                .filter(|line| line.starts_with("diff --git"))
                .count();
            let excerpt_source = diff.lines().take(20).collect::<Vec<_>>().join("\n");
            let excerpt = truncate_middle(&excerpt_source, 400);
            format!(
                "Autofix PR suggestion\nchanged_files={files}\nSuggested prompt: \"Review this PR diff, identify the highest-impact fix, and produce a minimal patch.\"\nexcerpt=\n{excerpt}"
            )
        }
        Ok(output) => format!(
            "autofix-pr: gh pr diff failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => format!("autofix-pr: failed to invoke gh: {error}"),
    }
}

fn shell_rc_path() -> Option<PathBuf> {
    let home = PathBuf::from(env::var_os("HOME")?);
    let shell = env::var("SHELL").ok()?;
    let file = if shell.contains("zsh") {
        ".zshrc"
    } else if shell.contains("fish") {
        ".config/fish/config.fish"
    } else {
        ".bashrc"
    };
    Some(home.join(file))
}

fn path_contains(dir: &Path) -> bool {
    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|entry| entry == dir))
        .unwrap_or(false)
}

fn completion_hint(shell: &str) -> &'static str {
    if shell.contains("zsh") {
        "install completion script into a zsh fpath directory"
    } else if shell.contains("fish") {
        "install completion script into ~/.config/fish/completions"
    } else {
        "install completion script into bash-completion"
    }
}

fn parse_proc_status() -> Vec<(String, String)> {
    fs::read_to_string("/proc/self/status")
        .ok()
        .into_iter()
        .flat_map(|contents| {
            contents
                .lines()
                .filter_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    Some((key.trim().to_string(), value.trim().to_string()))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn status_value<'a>(status: &'a [(String, String)], key: &str) -> Option<&'a str> {
    status
        .iter()
        .find(|(entry_key, _)| entry_key == key)
        .map(|(_, value)| value.as_str())
}

fn estimated_process_uptime_seconds() -> Option<f64> {
    let uptime = fs::read_to_string("/proc/uptime").ok()?;
    let total_uptime = uptime.split_whitespace().next()?.parse::<f64>().ok()?;
    let stat = fs::read_to_string("/proc/self/stat").ok()?;
    let (_, rest) = stat.rsplit_once(") ")?;
    let fields = rest.split_whitespace().collect::<Vec<_>>();
    let start_ticks = fields.get(19)?.parse::<f64>().ok()?;
    Some((total_uptime - (start_ticks / 100.0)).max(0.0))
}

fn fd_count() -> Option<usize> {
    fs::read_dir("/proc/self/fd")
        .ok()
        .map(|entries| entries.count())
}

fn executable_on_path(name: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|path| path.join(name).is_file())
}

fn truncate_middle(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        return value.into();
    }
    let keep = max_len.saturating_sub(3) / 2;
    format!("{}...{}", &value[..keep], &value[value.len() - keep..])
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use futures::executor::block_on;
    use serde_json::json;
    use tempfile::tempdir;
    use wonder_of_u_core::{FeatureSet, PermissionMode, SessionId, TokenUsage};
    use wonder_of_u_storage::TranscriptStore;

    use super::*;

    fn test_context() -> CommandContext {
        CommandContext {
            session_id: SessionId::new(),
            cwd: PathBuf::from("/workspace"),
            features: FeatureSet::first_release(),
            authenticated: true,
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
    fn issue_command_does_not_launch_browser_under_tests() {
        let output = block_on(IssueCommand::new().execute(
            test_context(),
            CommandInvocation {
                name: "issue".into(),
                args: String::new(),
                raw: "/issue".into(),
            },
        ))
        .expect("run issue command");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };

        assert!(text.contains("Issue tracker: https://github.com/anthropics/claude-code/issues"));
    }

    #[test]
    fn pr_comment_formatter_includes_issue_and_review_counts() {
        let issue_comments = json!([
            {
                "user": { "login": "reviewer-a" },
                "body": "Looks good overall",
                "createdAt": "2025-01-01T00:00:00Z"
            }
        ]);
        let review_comments = json!([
            {
                "user": { "login": "reviewer-b" },
                "body": "Please rename this variable",
                "path": "src/lib.rs",
                "line": 42
            }
        ]);

        let rendered = format_pr_comments(
            17,
            "https://github.com/example/repo/pull/17",
            "feat/extras",
            &issue_comments,
            &review_comments,
        )
        .expect("formatted comments");

        assert!(rendered.contains("PR Comments for #17"));
        assert!(rendered.contains("issue_comments=1"));
        assert!(rendered.contains("review_comments=1"));
        assert!(rendered.contains("reviewer-a"));
        assert!(rendered.contains("src/lib.rs:42"));
    }

    #[test]
    fn backfill_sessions_writes_missing_metadata() {
        let dir = tempdir().expect("tempdir");
        let store = TranscriptStore::new(dir.path());
        let session_id = SessionId::new();
        store
            .append_message(
                &MessageEnvelope::user_text(session_id, "hello")
                    .with_context(Some(PathBuf::from("/workspace")), Some("main".into())),
            )
            .expect("write transcript");

        let output = block_on(
            BackfillSessionsCommand::new(Some(dir.path().to_path_buf())).execute(
                test_context(),
                CommandInvocation {
                    name: "backfill-sessions".into(),
                    args: String::new(),
                    raw: "/backfill-sessions".into(),
                },
            ),
        )
        .expect("command output");

        match output {
            CommandOutput::Text(text) => {
                assert_eq!(
                    text,
                    "Backfill complete: 1 sessions scanned, 1 metadata records written."
                );
            }
            other => panic!("unexpected output: {other:?}"),
        }

        let metadata = store
            .read_metadata(session_id)
            .expect("metadata backfilled");
        assert_eq!(metadata.message_count, 1);
    }

    #[test]
    fn heapdump_renders_memory_summary() {
        let context = test_context();
        let rendered = render_heapdump(&context);
        assert!(rendered.contains("pid="));
        assert!(rendered.contains("memory_rss="));
    }

    #[test]
    fn debug_tool_call_renders_recent_calls() {
        let dir = tempdir().expect("tempdir");
        let store = TranscriptStore::new(dir.path());
        let session_id = SessionId::new();
        store
            .append_message(&MessageEnvelope::new(
                session_id,
                MessagePayload::AssistantToolUse {
                    tool: "grep".into(),
                    use_id: wonder_of_u_core::ToolUseId::new(),
                    input: json!({ "pattern": "todo" }),
                },
            ))
            .expect("tool message");

        let rendered =
            render_debug_tool_call(Some(dir.path()), session_id, 5).expect("debug tool calls");
        assert!(rendered.contains("grep"));
        assert!(rendered.contains("\"pattern\":\"todo\""));
    }

    #[test]
    fn thinkback_returns_latest_thinking_block() {
        let messages = vec![
            MessageEnvelope::new(
                SessionId::new(),
                MessagePayload::AssistantThinking {
                    content: "first".into(),
                    collapsed: false,
                },
            ),
            MessageEnvelope::new(
                SessionId::new(),
                MessagePayload::AssistantThinking {
                    content: "second".into(),
                    collapsed: true,
                },
            ),
        ];

        let blocks = thinking_blocks(&messages);
        assert_eq!(blocks.last().map(String::as_str), Some("second"));
    }

    #[test]
    fn thinking_command_reports_and_validates_state() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        assert_eq!(
            execute_thinking_command(&app, None).expect("report thinking state"),
            "Thinking is currently disabled (effort: medium). Options: on, off, low, medium, high"
        );

        app.set_thinking_enabled(true);
        assert_eq!(
            execute_thinking_command(&app, Some("")).expect("report thinking state"),
            "Thinking is currently enabled (effort: medium). Options: on, off, low, medium, high"
        );
        assert_eq!(
            execute_thinking_command(&app, Some("on")).expect("enable thinking"),
            "Thinking enabled — Claude will reason before responding"
        );
        assert_eq!(
            execute_thinking_command(&app, Some("off")).expect("disable thinking"),
            "Thinking disabled"
        );
        assert_eq!(
            execute_thinking_command(&app, Some("low")).expect("set low effort"),
            "Thinking effort set to low"
        );
        app.set_thinking_effort(ThinkingEffort::High);
        assert_eq!(
            execute_thinking_command(&app, Some("")).expect("report thinking effort"),
            "Thinking is currently enabled (effort: high). Options: on, off, low, medium, high"
        );
        assert_eq!(
            execute_thinking_command(&app, Some("medium")).expect("set medium effort"),
            "Thinking effort set to medium"
        );
        assert_eq!(
            execute_thinking_command(&app, Some("high")).expect("set high effort"),
            "Thinking effort set to high"
        );
        assert!(matches!(
            execute_thinking_command(&app, Some("maybe")),
            Err(WonderError::Validation(message))
                if message == "Unknown argument: maybe. Use 'on', 'off', 'low', 'medium', or 'high'"
        ));
    }

    #[test]
    fn help_command_lists_expected_slash_commands_and_shortcuts() {
        let rendered = execute_help_command().expect("render help");

        assert!(rendered.contains("Slash Commands"));
        assert!(rendered.contains("/help"));
        assert!(rendered.contains("/search"));
        assert!(rendered.contains("/exit"));
        assert!(rendered.contains("Keyboard Shortcuts"));
        assert!(rendered.contains("ctrl+f"));
        assert!(rendered.contains("shift+enter"));
    }

    #[test]
    fn stats_command_renders_session_usage_table() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        app.provider = Some("openai".into());
        app.model = Some("gpt-4.1".into());
        app.record_cost_usage(
            TokenUsage {
                input_tokens: 128,
                output_tokens: 32,
                cache_creation_tokens: 16,
                cache_read_tokens: 8,
            },
            Some(0.42),
        );

        let rendered = execute_stats_command(&app).expect("render stats");

        assert!(rendered.contains("Session Statistics"));
        assert!(rendered.contains("Provider:  openai"));
        assert!(rendered.contains("Model:     gpt-4.1"));
        assert!(rendered.contains("Input tokens:               128"));
        assert!(rendered.contains("Total tokens:               184"));
        assert!(rendered.contains("Estimated cost:      $0.4200"));
    }

    #[test]
    fn settings_command_renders_configuration_and_usage_sections() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        app.provider = Some("anthropic".into());
        app.model = Some("claude-3-5-sonnet-20241022".into());
        app.permission_mode = PermissionMode::Default;
        app.auth = AuthState::ready(
            AuthMaterialKind::ApiKey,
            wonder_of_u_core::AuthSource::Environment,
        );
        app.set_context_window_size(Some(200_000));
        app.record_cost_usage(
            TokenUsage {
                input_tokens: 12_450,
                output_tokens: 3_821,
                cache_creation_tokens: 1_200,
                cache_read_tokens: 8_100,
            },
            Some(0.0412),
        );

        let rendered = execute_settings_command(&app).expect("render settings");

        assert!(rendered.contains("Configuration"));
        assert!(rendered.contains("Session Usage"));
        assert!(rendered.contains("Provider Status"));
        assert!(rendered.contains("Auth:"));
        assert!(rendered.contains("12,450"));
        assert!(rendered.contains("$0.0412"));
    }
}
