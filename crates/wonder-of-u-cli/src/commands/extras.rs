use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde_json::Value;
use wonder_of_u_agent::SettingsStore;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec, Result,
    ToolSpec,
};
use wonder_of_u_storage::{StoragePaths, TranscriptStore};

pub struct HeapdumpCommand;

impl HeapdumpCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct AntTraceCommand;

impl AntTraceCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct CtxVizCommand;

impl CtxVizCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "ctx-viz",
            "Show context window usage diagnostics",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

pub struct DebugToolCallCommand;

impl DebugToolCallCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct GoodClaudeCommand;

impl GoodClaudeCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct BreakCacheCommand;

impl BreakCacheCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct BackfillSessionsCommand {
    storage_dir: Option<PathBuf>,
}

impl BackfillSessionsCommand {
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

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

pub struct PerfIssueCommand;

impl PerfIssueCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct BughunterCommand;

impl BughunterCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct BtwCommand;

impl BtwCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "btw",
            "Ask a quick side question without interrupting the main conversation",
            CommandKind::Local,
        )
    }
}

pub struct AdvisorCommand {
    storage_dir: Option<PathBuf>,
}

impl AdvisorCommand {
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "advisor",
            "Configure the advisor model for multi-model reasoning",
            CommandKind::Local,
        )
    }
}

pub struct StickersCommand;

impl StickersCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new("stickers", "Order Claude Code stickers", CommandKind::Local)
    }
}

pub struct RewindCommand {
    storage_dir: Option<PathBuf>,
}

impl RewindCommand {
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "rewind",
            "Restore the conversation to a previous checkpoint",
            CommandKind::Local,
        );
        spec.aliases.push("checkpoint".into());
        spec
    }
}

pub struct InitVerifiersCommand {
    tool_specs: Arc<[ToolSpec]>,
}

impl InitVerifiersCommand {
    pub fn new(tool_specs: Arc<[ToolSpec]>) -> Self {
        Self { tool_specs }
    }

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

pub struct ExtraUsageCommand;

impl ExtraUsageCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "extra-usage",
            "Configure extra usage allowance when limits are reached",
            CommandKind::Local,
        )
    }
}

pub struct PassesCommand;

impl PassesCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "passes",
            "Share Claude Code access passes with friends",
            CommandKind::Local,
        )
    }
}

pub struct RateLimitOptionsCommand;

impl RateLimitOptionsCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct MockLimitsCommand;

impl MockLimitsCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct ResetLimitsCommand;

impl ResetLimitsCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct OnboardingCommand;

impl OnboardingCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct TeleportCommand;

impl TeleportCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "teleport",
            "Teleport to a remote Claude Code session",
            CommandKind::Local,
        )
    }
}

pub struct RemoteEnvCommand;

impl RemoteEnvCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "remote-env",
            "Configure the default remote environment for teleport sessions",
            CommandKind::Local,
        )
    }
}

pub struct RemoteSetupCommand;

impl RemoteSetupCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct BridgeKickCommand;

impl BridgeKickCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct SandboxToggleCommand;

impl SandboxToggleCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct UltraplanCommand;

impl UltraplanCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "ultraplan",
            "Launch a multi-agent remote exploration (requires Claude.ai)",
            CommandKind::Local,
        )
    }
}

pub struct ThinkbackCommand;

impl ThinkbackCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct ThinkbackPlayCommand;

impl ThinkbackPlayCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct AutofixPrCommand;

impl AutofixPrCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct PrCommentsCommand;

impl PrCommentsCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct SummaryCommand;

impl SummaryCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "summary",
            "Show summary guidance for the current session",
            CommandKind::NonInteractive,
        );
        spec.hidden = true;
        spec
    }
}

pub struct EnvCommand;

impl EnvCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct OauthRefreshCommand {
    storage_dir: Option<PathBuf>,
}

impl OauthRefreshCommand {
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

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

pub struct IssueCommand;

impl IssueCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct ShareCommand;

impl ShareCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct InstallCommand;

impl InstallCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct InstallGithubAppCommand;

impl InstallGithubAppCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct InstallSlackAppCommand;

impl InstallSlackAppCommand {
    pub const fn new() -> Self {
        Self
    }

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

pub struct CreateMovedToPluginCommand;

impl CreateMovedToPluginCommand {
    pub const fn new() -> Self {
        Self
    }

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
        let path = write_heapdump(&context)?;
        Ok(CommandOutput::Text(format!(
            "Heap dump written to {}",
            path.display()
        )))
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
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_ant_trace(&context)))
    }
}

#[async_trait]
impl Command for CtxVizCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_ctx_viz()))
    }
}

#[async_trait]
impl Command for DebugToolCallCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_debug_tool_call()))
    }
}

#[async_trait]
impl Command for GoodClaudeCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(
            "✓ Good Claude! Session marked as positive example.".into(),
        ))
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
        Ok(CommandOutput::Text("Cache cleared.".into()))
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
        let count = session_count(self.storage_dir.as_deref())?;
        Ok(CommandOutput::Text(format!(
            "Backfill complete: {count} sessions indexed."
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
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_perf_issue()))
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
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(
            "Bug hunter mode: active. Report issues at https://github.com/anthropics/claude-code/issues"
                .into(),
        ))
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
            return Ok(CommandOutput::Text("Usage: /btw <question>".into()));
        }
        Ok(CommandOutput::Text(format!(
            "Side question noted: {args}\n(Use /btw <question> to inject a side note into the conversation)"
        )))
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
            return Ok(CommandOutput::Text("Advisor disabled".into()));
        }
        Ok(CommandOutput::Text(format!(
            "Advisor set to: {args}. (Restart to apply)"
        )))
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
        Ok(CommandOutput::Text(
            "Visit https://anthropic.com/stickers to order Claude Code stickers! 🎉".into(),
        ))
    }
}

#[async_trait]
impl Command for RewindCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_rewind(
            self.storage_dir.as_deref(),
            &context,
        )?))
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
        let verifiers = verifier_tool_names(self.tool_specs.as_ref());
        let mut lines = vec!["Verifiers initialized.".into()];
        if verifiers.is_empty() {
            lines.push("Available verification tools: none detected.".into());
        } else {
            lines.push(format!(
                "Available verification tools: {}",
                verifiers.join(", ")
            ));
        }
        Ok(CommandOutput::Text(lines.join("\n")))
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
        let message = if env::var_os("DISABLE_EXTRA_USAGE_COMMAND").is_some() {
            "Extra usage: disabled by policy."
        } else {
            "Extra usage: contact your plan administrator to configure overage provisioning. See https://console.anthropic.com/settings for account limits."
        };
        Ok(CommandOutput::Text(message.into()))
    }
}

#[async_trait]
impl Command for PassesCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(
            "Claude Code Passes: Visit https://console.anthropic.com/referral to share access with friends and earn extra usage."
                .into(),
        ))
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
        Ok(CommandOutput::Text(
            "Rate Limit Options:\n• Wait for reset (usually 1 hour)\n• Upgrade your plan at https://console.anthropic.com\n• Use /extra-usage to configure overage\n• Try a different model with /model"
                .into(),
        ))
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
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let message = if cfg!(test) || env::var_os("ANTHROPIC_API_KEY").is_some() {
            "Mock limits: enabled for testing."
        } else {
            "Mock limits: not available in this build."
        };
        Ok(CommandOutput::Text(message.into()))
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
        Ok(CommandOutput::Text(
            "Rate limit counters are managed server-side and cannot be reset from the client. Use /rate-limit-options for available options."
                .into(),
        ))
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
        Ok(CommandOutput::Text(
            "Welcome to Wonder of U (Claude Code)!\n\nQuick start:\n• /help - show all commands\n• /model - choose AI model\n• /mcp - configure tools\n• /init - initialize project\n• /config - manage settings\n\nStart chatting to begin!"
                .into(),
        ))
    }
}

#[async_trait]
impl Command for TeleportCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(
            "Teleport requires Claude.ai subscription with remote sessions enabled. See https://claude.ai/code for details.\n\nCurrently: remote sessions not configured."
                .into(),
        ))
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
        Ok(CommandOutput::Text(
            "Remote environment configuration requires a Claude.ai subscription. Use /config to manage local settings."
                .into(),
        ))
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
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(
            "Web setup:\n1. Sign in at https://claude.ai\n2. Navigate to Claude Code settings\n3. Connect your GitHub account\n4. Return here and run /teleport"
                .into(),
        ))
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
        Ok(CommandOutput::Text(
            "Bridge kick: no active bridge connection. Start a bridge with /bridge first.".into(),
        ))
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
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_sandbox_status()))
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
        Ok(CommandOutput::Text(
            "Ultraplan requires a Claude.ai subscription with remote agents enabled.\n\nFor local multi-agent: use /agents to configure agent coordination."
                .into(),
        ))
    }
}

#[async_trait]
impl Command for ThinkbackCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(
            "Think Back: this feature requires the thinkback feature gate. Use /usage to see your statistics."
                .into(),
        ))
    }
}

#[async_trait]
impl Command for ThinkbackPlayCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(
            "Thinkback playback: no animation data available.".into(),
        ))
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
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let message = if executable_on_path("gh") {
            "autofix-pr: use /review with --pr flag for PR review and auto-fix capabilities"
        } else {
            "autofix-pr: requires gh CLI (https://cli.github.com)."
        };
        Ok(CommandOutput::Text(message.into()))
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
impl Command for SummaryCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(
            "Session summary: use /compact to generate a conversation summary and compress context."
                .into(),
        ))
    }
}

#[async_trait]
impl Command for EnvCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_env_summary()))
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
        let mut lines = Vec::new();
        if let Some(storage_dir) = self.storage_dir.as_deref() {
            let credentials = StoragePaths::new(storage_dir).credentials_path();
            lines.push(format!("credentials_path={}", credentials.display()));
            lines.push(format!("credentials_present={}", credentials.exists()));
        }
        lines.push("OAuth token refresh: use /login to refresh authentication credentials.".into());
        Ok(CommandOutput::Text(lines.join("\n")))
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
        Ok(CommandOutput::Text(
            "Issues: this feature has been moved. Report bugs at https://github.com/anthropics/claude-code/issues"
                .into(),
        ))
    }
}

#[async_trait]
impl Command for ShareCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(
            "Share: this feature is not available in this build.".into(),
        ))
    }
}

#[async_trait]
impl Command for InstallCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_install_help()))
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
        let auth_status = github_auth_status();
        Ok(CommandOutput::Text(format!(
            "GitHub App installation: visit https://github.com/apps/claude to install the Claude GitHub App.\n\nRun `gh auth status` to check GitHub authentication.\nstatus={auth_status}"
        )))
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
        Ok(CommandOutput::Text(
            "Slack App installation: visit https://slack.com/apps/claude to install Claude for Slack."
                .into(),
        ))
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

fn write_heapdump(context: &CommandContext) -> Result<PathBuf> {
    let pid = std::process::id();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let output_path = env::temp_dir().join(format!("wonder-of-u-heapdump-{pid}-{timestamp}.txt"));
    let status = read_proc_file("/proc/self/status");
    let maps = read_proc_file("/proc/self/maps");
    let dump = format!(
        "pid={pid}\nsession_id={}\ncwd={}\n\n[status]\n{}\n\n[maps]\n{}\n",
        context.session_id,
        context.cwd.display(),
        status,
        maps,
    );
    fs::write(&output_path, dump)?;
    Ok(output_path)
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

fn render_ctx_viz() -> String {
    let used = env::var("WONDER_OF_U_CONTEXT_TOKENS_USED").ok();
    let max = env::var("WONDER_OF_U_CONTEXT_TOKENS_MAX").ok();
    match (used, max) {
        (Some(used), Some(max)) => {
            format!("Context visualization:\nused_tokens={used}\nmax_tokens={max}")
        }
        _ => "Context visualization: unavailable in this build".into(),
    }
}

fn render_debug_tool_call() -> String {
    env::var("WONDER_OF_U_LAST_TOOL_CALL")
        .map(|value| format!("Tool call debug:\n{value}"))
        .unwrap_or_else(|_| "Tool call debug: no active tool call".into())
}

fn session_count(storage_dir: Option<&Path>) -> Result<usize> {
    let Some(storage_dir) = storage_dir else {
        return Ok(0);
    };
    Ok(TranscriptStore::new(storage_dir).list_metadata()?.len())
}

fn render_perf_issue() -> String {
    let status = parse_proc_status();
    let uptime = estimated_process_uptime_seconds()
        .map(|seconds| format!("{seconds:.2}"))
        .unwrap_or_else(|| "unavailable".into());
    [
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
    ]
    .join("\n")
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

fn render_rewind(storage_dir: Option<&Path>, context: &CommandContext) -> Result<String> {
    let Some(storage_dir) = storage_dir else {
        return Ok(
            "Rewind: no checkpoints found in current session. Use /compact to create a checkpoint."
                .into(),
        );
    };
    let store = TranscriptStore::new(storage_dir);
    let mut checkpoints = Vec::new();
    if let Some(snapshot) = store.read_snapshot_if_exists(context.session_id)? {
        checkpoints.push(format!(
            "• current session ({}) - {} messages",
            context.session_id,
            snapshot.state.messages.len()
        ));
    }
    for metadata in store.list_metadata()?.into_iter().take(10) {
        if metadata.session_id == context.session_id {
            continue;
        }
        if let Some(snapshot) = store.read_snapshot_if_exists(metadata.session_id)? {
            checkpoints.push(format!(
                "• {} - {} ({})",
                metadata.session_id,
                metadata.title,
                snapshot.state.messages.len()
            ));
        }
    }
    if checkpoints.is_empty() {
        return Ok(
            "Rewind: no checkpoints found in current session. Use /compact to create a checkpoint."
                .into(),
        );
    }
    let mut lines = vec!["Rewind checkpoints:".into()];
    lines.extend(checkpoints);
    Ok(lines.join("\n"))
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

fn render_sandbox_status() -> String {
    let os = env::consts::OS;
    if os == "linux" {
        "Sandbox: Linux namespace/seccomp isolation may be available depending on launch configuration.\nCurrent build does not expose a runtime toggle.\nUse permission settings or launch-time sandbox configuration to change isolation."
            .into()
    } else {
        format!("Sandbox: not supported on this platform ({os}). Use permission settings instead.")
    }
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

fn render_env_summary() -> String {
    let anthropic_api_key = env::var("ANTHROPIC_API_KEY").ok();
    [
        "## Environment".into(),
        format!(
            "ANTHROPIC_API_KEY={}",
            anthropic_api_key
                .as_deref()
                .map(mask_secret)
                .unwrap_or_else(|| "unset".into())
        ),
        format!(
            "PATH={}",
            truncate_middle(&env::var("PATH").unwrap_or_else(|_| "unset".into()), 120)
        ),
        format!(
            "HOME={}",
            env::var("HOME").unwrap_or_else(|_| "unset".into())
        ),
        format!(
            "WONDER_OF_U_ADVISOR_MODEL={}",
            env::var("WONDER_OF_U_ADVISOR_MODEL").unwrap_or_else(|_| "unset".into())
        ),
        format!(
            "DISABLE_EXTRA_USAGE_COMMAND={}",
            env::var("DISABLE_EXTRA_USAGE_COMMAND").unwrap_or_else(|_| "unset".into())
        ),
    ]
    .join("\n")
}

fn render_install_help() -> String {
    let os = env::consts::OS;
    let command = match os {
        "macos" => "brew install wonder-of-u",
        "windows" => "npm install -g @anthropic-ai/claude-code",
        _ => "npm install -g @anthropic-ai/claude-code",
    };
    format!(
        "Install guidance for {os}:\n{command}\nMore info: https://docs.anthropic.com/en/docs/claude-code/setup"
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

fn read_proc_file(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|_| "unavailable".into())
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

fn mask_secret(secret: &str) -> String {
    if secret.len() <= 8 {
        return "********".into();
    }
    format!("{}***{}", &secret[..4], &secret[secret.len() - 4..])
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
    use std::{fs, path::PathBuf};

    use futures::executor::block_on;
    use serde_json::json;
    use tempfile::tempdir;
    use time::OffsetDateTime;
    use wonder_of_u_core::{CostState, FeatureSet, PermissionMode, SessionId};
    use wonder_of_u_storage::{SessionMetadata, TranscriptStore};

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
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        }
    }

    #[test]
    fn mask_secret_redacts_middle_bytes() {
        assert_eq!(mask_secret("sk-ant-api-key-12345678"), "sk-a***5678");
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
    fn backfill_sessions_counts_stored_metadata() {
        let dir = tempdir().expect("tempdir");
        let store = TranscriptStore::new(dir.path());
        let now = OffsetDateTime::now_utc();
        let metadata = SessionMetadata {
            schema_version: wonder_of_u_storage::STORAGE_SCHEMA_VERSION,
            session_id: SessionId::new(),
            title: "Session".into(),
            cwd: PathBuf::from("/workspace"),
            git_branch: None,
            entrypoint: None,
            app_version: None,
            created_at: now,
            updated_at: now,
            message_count: 3,
            tags: Vec::new(),
            provider: None,
            model: None,
            auth: Default::default(),
            costs: CostState::default(),
        };
        store.write_metadata(&metadata).expect("write metadata");

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
                assert_eq!(text, "Backfill complete: 1 sessions indexed.");
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[test]
    fn heapdump_writes_diagnostic_file() {
        let context = test_context();
        let path = write_heapdump(&context).expect("heapdump");

        let contents = fs::read_to_string(&path).expect("heapdump contents");
        assert!(contents.contains("pid="));
        assert!(contents.contains("[status]"));

        fs::remove_file(path).expect("cleanup heapdump");
    }
}
