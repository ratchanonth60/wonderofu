use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use async_trait::async_trait;
use serde_json::json;
use wonder_of_u_agent::ProviderResolver;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec, CostState,
    MESSAGE_SCHEMA_VERSION, Result,
};
use wonder_of_u_mcp::{McpConfigStore, McpStatusReport};
use wonder_of_u_storage::{STORAGE_SCHEMA_VERSION, SessionMetadata, TranscriptStore};

use super::git_command_output;
use super::plugin::load_catalogs;
use super::task_runtime::TaskManager;

pub struct StatusCommand {
    storage_dir: Option<PathBuf>,
}

pub struct CostCommand {
    storage_dir: Option<PathBuf>,
}

pub struct UsageCommand {
    storage_dir: Option<PathBuf>,
}

pub struct StatsCommand {
    storage_dir: Option<PathBuf>,
}

pub struct VersionCommand;

pub struct ReleaseNotesCommand;

pub struct FeedbackCommand;

pub struct UpgradeCommand;

pub struct DesktopCommand;

pub struct MobileCommand;

pub struct ChromeCommand;

pub struct IdeCommand;

pub struct InsightsCommand {
    storage_dir: Option<PathBuf>,
}

pub struct OutputStyleCommand;

impl StatusCommand {
    pub fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "status",
            "Show runtime, storage, MCP, plugin, skill, and task status",
            CommandKind::NonInteractive,
        )
    }
}

impl CostCommand {
    pub fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "cost",
            "Show current-session and aggregate token/cost totals",
            CommandKind::Local,
        )
    }
}

impl UsageCommand {
    pub fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new("usage", "Show plan usage limits", CommandKind::Local)
    }
}

impl StatsCommand {
    pub fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "stats",
            "Show aggregate session activity statistics",
            CommandKind::Local,
        )
    }
}

impl VersionCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "version",
            "Show application and schema version details",
            CommandKind::Local,
        )
    }
}

impl ReleaseNotesCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "release-notes",
            "Show recent release notes or the repository releases page",
            CommandKind::Local,
        )
    }
}

impl FeedbackCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "feedback",
            "Submit feedback about wonder-of-u",
            CommandKind::Local,
        );
        spec.aliases.push("bug".into());
        spec
    }
}

impl UpgradeCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "upgrade",
            "Open the Claude upgrade page and show account upgrade guidance",
            CommandKind::Local,
        )
    }
}

impl DesktopCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "desktop",
            "Open Claude Desktop handoff/download guidance for the current platform",
            CommandKind::Local,
        )
    }
}

impl MobileCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "mobile",
            "Show Claude mobile download links for iOS and Android",
            CommandKind::Local,
        );
        spec.aliases.extend(["ios".into(), "android".into()]);
        spec
    }
}

impl ChromeCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "chrome",
            "Open Claude in Chrome setup guidance and extension links",
            CommandKind::Local,
        )
    }
}

impl IdeCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "ide",
            "Open Claude Code IDE integration docs and extension links",
            CommandKind::Local,
        )
    }
}

impl InsightsCommand {
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "insights",
            "Queue a usage insights report based on local session history",
            CommandKind::Local,
        );
        spec.interactive_only = true;
        spec
    }
}

impl OutputStyleCommand {
    pub const fn new() -> Self {
        Self
    }

    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "output-style",
            "Deprecated: use /config to change output style",
            CommandKind::Local,
        );
        spec.hidden = true;
        spec
    }
}

#[async_trait]
impl Command for StatusCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let mut lines = vec![
            "wonder-of-u status".to_string(),
            format!("cwd={}", context.cwd.display()),
            format!("interactive={}", context.interactive),
            format!("authenticated={}", context.authenticated),
            format!("features={}", context.features.iter().count()),
        ];
        let report = ProviderResolver::builtin().load_report(self.storage_dir.as_deref())?;
        lines.push(format!(
            "provider_selection={}",
            report
                .selection_label()
                .unwrap_or_else(|| "unconfigured".into())
        ));
        lines.push(format!("provider_readiness={}", report.readiness.label()));
        lines.push(format!("auth_kind={}", report.auth.kind_label()));
        lines.push(format!("auth_status={}", report.auth.status_label()));
        if let Some(source) = report.auth.source_label() {
            lines.push(format!("auth_source={source}"));
        }
        lines.push(format!(
            "available_providers={}",
            report
                .available_providers
                .iter()
                .map(|provider| provider.id.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ));

        let (_, plugins, skills) = load_catalogs(&context.cwd, self.storage_dir.as_deref())?;
        lines.push(format!("plugins={}", plugins.entries().len()));
        lines.push(format!("ready_plugins={}", plugins.ready_count()));
        lines.push(format!("plugin_errors={}", plugins.errors().len()));
        lines.push(format!("skills={}", skills.len()));
        lines.push(format!("bundled_skills={}", skills.bundled_count()));
        lines.push(format!("plugin_skills={}", skills.plugin_count()));
        lines.push(format!("skill_commands={}", skills.command_count()));
        lines.push(format!("skill_errors={}", skills.errors().len()));
        lines.push("plugin_runtime=command_subprocess".into());
        lines.push("skill_runtime=provider_prompt".into());
        lines.push("default_entry=tui_if_tty_else_doctor".into());
        lines.push("resume_entry=tui_if_tty_else_summary".into());

        match &self.storage_dir {
            Some(storage_dir) => {
                let store = TranscriptStore::new(storage_dir.clone());
                lines.push(format!(
                    "storage_dir={}",
                    store.paths().base_dir().display()
                ));
                lines.push(format!(
                    "transcripts={}",
                    count_files(store.paths().sessions_dir(), Some("jsonl"))?
                ));
                lines.push(format!(
                    "metadata={}",
                    count_files(store.paths().metadata_dir(), Some("json"))?
                ));
                lines.push(format!(
                    "snapshots={}",
                    count_files(store.paths().snapshot_dir(), Some("json"))?
                ));
                lines.push(format!(
                    "costs={}",
                    count_files(store.paths().sessions_dir(), Some("costs"))?
                ));
                lines.push(format!(
                    "pastes={}",
                    count_files(store.paths().pastes_dir(), None)?
                ));
                let mcp_store = McpConfigStore::new(storage_dir.clone());
                let mcp_config = mcp_store.read()?;
                let mcp_report =
                    McpStatusReport::inspect(mcp_store.paths().mcp_servers_path(), &mcp_config);
                lines.push(format!("mcp_servers={}", mcp_report.servers.len()));
                lines.push(format!("mcp_ready_servers={}", mcp_report.ready_count()));
                lines.push(format!("mcp_error_servers={}", mcp_report.error_count()));
                lines.push(format!(
                    "mcp_tools={}",
                    mcp_report
                        .servers
                        .iter()
                        .map(wonder_of_u_mcp::McpServerStatus::tool_count)
                        .sum::<usize>()
                ));
                lines.push(format!(
                    "mcp_resources={}",
                    mcp_report
                        .servers
                        .iter()
                        .map(wonder_of_u_mcp::McpServerStatus::resource_count)
                        .sum::<usize>()
                ));

                let task_manager = TaskManager::new(storage_dir.clone());
                let report = task_manager.reconcile_tasks(None)?;
                let summary = super::task_runtime::TaskSummary::from_tasks(&report.tasks);
                lines.push(format!("tasks={}", summary.total));
                lines.push(format!("active_tasks={}", summary.active));
                lines.push(format!("terminal_tasks={}", summary.terminal));
                lines.push(format!("pending_tasks={}", summary.pending));
                lines.push(format!("running_tasks={}", summary.running));
                lines.push(format!("completed_tasks={}", summary.completed));
                lines.push(format!("failed_tasks={}", summary.failed));
                lines.push(format!("killed_tasks={}", summary.killed));
                lines.push(format!("cancelled_tasks={}", summary.cancelled));
                lines.push(format!("agent_tasks={}", summary.agents));
                lines.push(format!("shell_tasks={}", summary.shell));
                lines.push(format!("tasks_reconciled_at={}", report.reconciled_at));
                lines.push(format!("task_reconcile_changed={}", report.changed));
                lines.push(format!("task_reconcile_finished={}", report.finished));
                lines.push(format!("fresh_task_heartbeats={}", report.fresh_heartbeats));
                lines.push(format!("stale_task_heartbeats={}", report.stale_heartbeats));
                lines.push(format!(
                    "missing_task_heartbeats={}",
                    report.missing_heartbeats
                ));
            }
            None => lines.push("storage_dir=disabled".into()),
        }

        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

#[async_trait]
impl Command for CostCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return Ok(CommandOutput::Text(
                "cost=unavailable\nstorage_dir=disabled".into(),
            ));
        };
        let store = TranscriptStore::new(storage_dir);
        let metadata = store.list_metadata()?;
        let current = current_session_metadata(&store, &context, &metadata)?;
        let aggregate = aggregate_costs(&metadata);

        let mut lines = vec!["cost=ready".into()];
        lines.push(format!(
            "storage_dir={}",
            store.paths().base_dir().display()
        ));
        lines.push(format!("session_id={}", context.session_id));
        match current {
            Some(current) => {
                lines.push(format!("session_title={}", current.title));
                lines.push(format!("session_messages={}", current.message_count));
                lines.push(format!(
                    "session_total_tokens={}",
                    current.costs.usage.total_tokens()
                ));
                lines.push(format!(
                    "session_input_tokens={}",
                    current.costs.usage.input_tokens
                ));
                lines.push(format!(
                    "session_output_tokens={}",
                    current.costs.usage.output_tokens
                ));
                lines.push(format!(
                    "session_cache_creation_tokens={}",
                    current.costs.usage.cache_creation_tokens
                ));
                lines.push(format!(
                    "session_cache_read_tokens={}",
                    current.costs.usage.cache_read_tokens
                ));
                if let Some(cost) = current.costs.estimated_cost_usd {
                    lines.push(format!("session_estimated_cost_usd={cost:.4}"));
                }
                lines.push(format!(
                    "session_duration_seconds={}",
                    (current.updated_at - current.created_at)
                        .whole_seconds()
                        .max(0)
                ));
            }
            None => lines.push("session_costs=unavailable".into()),
        }

        lines.push(format!("tracked_sessions={}", metadata.len()));
        lines.push(format!("aggregate_messages={}", aggregate.messages));
        lines.push(format!(
            "aggregate_total_tokens={}",
            aggregate.costs.usage.total_tokens()
        ));
        lines.push(format!(
            "aggregate_input_tokens={}",
            aggregate.costs.usage.input_tokens
        ));
        lines.push(format!(
            "aggregate_output_tokens={}",
            aggregate.costs.usage.output_tokens
        ));
        lines.push(format!(
            "aggregate_cache_creation_tokens={}",
            aggregate.costs.usage.cache_creation_tokens
        ));
        lines.push(format!(
            "aggregate_cache_read_tokens={}",
            aggregate.costs.usage.cache_read_tokens
        ));
        if let Some(cost) = aggregate.costs.estimated_cost_usd {
            lines.push(format!("aggregate_estimated_cost_usd={cost:.4}"));
        }

        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

#[async_trait]
impl Command for UsageCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_usage_summary(
            self.storage_dir.as_deref(),
            &context,
        )?))
    }
}

#[async_trait]
impl Command for StatsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return Ok(CommandOutput::Text(
                "stats=unavailable\nstorage_dir=disabled".into(),
            ));
        };
        let store = TranscriptStore::new(storage_dir);
        let metadata = store.list_metadata()?;
        let aggregate = aggregate_stats(&metadata);

        let mut lines = vec!["## Activity Stats".into()];
        lines.push(format!("tracked_sessions={}", metadata.len()));
        lines.push(format!("active_days={}", aggregate.active_days));
        lines.push(format!("total_messages={}", aggregate.total_messages));
        lines.push(format!(
            "average_messages_per_session={:.2}",
            aggregate.average_messages_per_session
        ));
        lines.push(format!("unique_workdirs={}", aggregate.unique_workdirs));
        lines.push(format!("unique_branches={}", aggregate.unique_branches));
        lines.push(format!("providers={}", aggregate.provider_counts.len()));
        for (provider, count) in aggregate.provider_counts {
            lines.push(format!("provider[{provider}]={count}"));
        }
        lines.push(format!("models={}", aggregate.model_counts.len()));
        for (model, count) in aggregate.model_counts.into_iter().take(10) {
            lines.push(format!("model[{model}]={count}"));
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

#[async_trait]
impl Command for VersionCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_version_summary()))
    }
}

#[async_trait]
impl Command for ReleaseNotesCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_release_notes_summary(
            &context.cwd,
        )))
    }
}

#[async_trait]
impl Command for FeedbackCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_feedback_summary(
            &context.cwd,
            invocation.args.trim(),
        )))
    }
}

#[async_trait]
impl Command for UpgradeCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_upgrade_summary(
            try_open_browser(UPGRADE_URL),
        )))
    }
}

#[async_trait]
impl Command for DesktopCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_desktop_summary(
            try_open_browser(DESKTOP_DOCS_URL),
        )))
    }
}

#[async_trait]
impl Command for MobileCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_mobile_summary()))
    }
}

#[async_trait]
impl Command for ChromeCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_chrome_summary(
            try_open_browser(CHROME_EXTENSION_URL),
        )))
    }
}

#[async_trait]
impl Command for IdeCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_ide_summary(try_open_browser(
            IDE_DOCS_URL,
        ))))
    }
}

#[async_trait]
impl Command for InsightsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        render_insights_enqueue(self.storage_dir.as_deref(), invocation.args.trim())
            .map(CommandOutput::Text)
    }
}

#[async_trait]
impl Command for OutputStyleCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(
            "/output-style has been deprecated. Use /config to change your output style, or set it in your settings file. Changes take effect on the next session.".into(),
        ))
    }
}

#[derive(Default)]
struct AggregateCostTotals {
    messages: usize,
    costs: CostState,
}

const UPGRADE_URL: &str = "https://claude.ai/upgrade/max";
const DESKTOP_DOCS_URL: &str = "https://clau.de/desktop";
const MOBILE_IOS_URL: &str = "https://apps.apple.com/app/claude-by-anthropic/id6473753684";
const MOBILE_ANDROID_URL: &str =
    "https://play.google.com/store/apps/details?id=com.anthropic.claude";
const CHROME_EXTENSION_URL: &str = "https://claude.ai/chrome";
const CHROME_PERMISSIONS_URL: &str = "https://clau.de/chrome/permissions";
const CHROME_DOCS_URL: &str = "https://code.claude.com/docs/en/chrome";
const IDE_DOCS_URL: &str = "https://code.claude.com/docs/en/ide";

#[derive(Default)]
struct AggregateStats {
    active_days: usize,
    total_messages: usize,
    average_messages_per_session: f64,
    unique_workdirs: usize,
    unique_branches: usize,
    provider_counts: BTreeMap<String, usize>,
    model_counts: BTreeMap<String, usize>,
}

fn current_session_metadata(
    store: &TranscriptStore,
    context: &CommandContext,
    metadata: &[SessionMetadata],
) -> Result<Option<SessionMetadata>> {
    if let Some(snapshot) = store.read_snapshot_if_exists(context.session_id)? {
        return Ok(Some(SessionMetadata::from_app_state(&snapshot.state)));
    }
    Ok(metadata
        .iter()
        .find(|entry| entry.session_id == context.session_id)
        .cloned())
}

fn aggregate_costs(metadata: &[SessionMetadata]) -> AggregateCostTotals {
    let mut aggregate = AggregateCostTotals::default();
    for entry in metadata {
        aggregate.messages += entry.message_count;
        aggregate
            .costs
            .record_usage(entry.costs.usage, entry.costs.estimated_cost_usd);
    }
    aggregate
}

fn aggregate_stats(metadata: &[SessionMetadata]) -> AggregateStats {
    let total_messages = metadata
        .iter()
        .map(|entry| entry.message_count)
        .sum::<usize>();
    let active_days = metadata
        .iter()
        .map(|entry| entry.updated_at.date())
        .collect::<BTreeSet<_>>()
        .len();
    let unique_workdirs = metadata
        .iter()
        .map(|entry| entry.cwd.clone())
        .collect::<BTreeSet<_>>()
        .len();
    let unique_branches = metadata
        .iter()
        .filter_map(|entry| entry.git_branch.clone())
        .collect::<BTreeSet<_>>()
        .len();
    let mut provider_counts = BTreeMap::new();
    let mut model_counts = BTreeMap::new();
    for entry in metadata {
        if let Some(provider) = &entry.provider {
            *provider_counts.entry(provider.clone()).or_insert(0) += 1;
        }
        if let (Some(provider), Some(model)) = (&entry.provider, &entry.model) {
            *model_counts
                .entry(format!("{provider}:{model}"))
                .or_insert(0) += 1;
        }
    }
    AggregateStats {
        active_days,
        total_messages,
        average_messages_per_session: if metadata.is_empty() {
            0.0
        } else {
            total_messages as f64 / metadata.len() as f64
        },
        unique_workdirs,
        unique_branches,
        provider_counts,
        model_counts,
    }
}

fn render_version_summary() -> String {
    [
        "## Version".into(),
        "application=wonder-of-u".into(),
        format!("version={}", env!("CARGO_PKG_VERSION")),
        format!("message_schema_version={MESSAGE_SCHEMA_VERSION}"),
        format!("storage_schema_version={STORAGE_SCHEMA_VERSION}"),
        format!("target_os={}", std::env::consts::OS),
        format!("target_arch={}", std::env::consts::ARCH),
    ]
    .join("\n")
}

fn render_usage_summary(storage_dir: Option<&Path>, context: &CommandContext) -> Result<String> {
    let mut lines = vec!["## Usage".into()];
    let Some(storage_dir) = storage_dir else {
        lines.push("usage=unavailable".into());
        lines.push("storage_dir=disabled".into());
        lines.push("Plan usage limits are unavailable without persisted session storage.".into());
        lines.push("Run /cost after launching with --storage-dir for raw token totals.".into());
        return Ok(lines.join("\n"));
    };
    let store = TranscriptStore::new(storage_dir);
    let metadata = store.list_metadata()?;
    let current = current_session_metadata(&store, context, &metadata)?;
    let aggregate = aggregate_costs(&metadata);

    lines.push(format!("tracked_sessions={}", metadata.len()));
    match current {
        Some(current) => {
            lines.push(format!("current_session_title={}", current.title));
            lines.push(format!(
                "current_session_total_tokens={}",
                current.costs.usage.total_tokens()
            ));
            if let Some(cost) = current.costs.estimated_cost_usd {
                lines.push(format!("current_session_estimated_cost_usd={cost:.4}"));
            }
        }
        None => lines.push("current_session_usage=unavailable".into()),
    }
    lines.push(format!(
        "aggregate_total_tokens={}",
        aggregate.costs.usage.total_tokens()
    ));
    if let Some(cost) = aggregate.costs.estimated_cost_usd {
        lines.push(format!("aggregate_estimated_cost_usd={cost:.4}"));
    }
    lines.push("Plan/subscription quota APIs are not yet wired in the Rust port.".into());
    lines.push("Use /cost for raw token and estimated-cost details.".into());
    Ok(lines.join("\n"))
}

fn render_release_notes_summary(cwd: &Path) -> String {
    if let Some(path) = find_ancestor_file(cwd, "CHANGELOG.md") {
        if let Ok(content) = fs::read_to_string(&path) {
            let notes = parse_release_notes(&content);
            if !notes.is_empty() {
                return format_release_notes(&path, &notes);
            }
        }
    }

    let mut lines = vec!["## Release Notes".into()];
    if let Some(url) = detect_releases_url(cwd) {
        lines.push("source=releases_page".into());
        lines.push(format!("url={url}"));
        lines.push("No local CHANGELOG.md was found, so wonder-of-u is pointing to the repository releases page.".into());
    } else {
        lines.push("source=unavailable".into());
        lines.push("No local CHANGELOG.md or repository release URL is available yet.".into());
    }
    lines.join("\n")
}

fn find_ancestor_file(start: &Path, name: &str) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        current = dir.parent();
    }
    None
}

fn parse_release_notes(content: &str) -> Vec<(String, Vec<String>)> {
    let mut sections = Vec::new();
    let mut current_version: Option<String> = None;
    let mut current_notes = Vec::new();

    for line in content.lines() {
        if let Some(version_line) = line.trim().strip_prefix("## ") {
            if let Some(version) = current_version.take() {
                if !current_notes.is_empty() {
                    sections.push((version, std::mem::take(&mut current_notes)));
                } else {
                    current_notes.clear();
                }
            }
            let version = version_line
                .split(" - ")
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            current_version = (!version.is_empty()).then_some(version);
            continue;
        }

        if current_version.is_some() && line.trim().starts_with("- ") {
            current_notes.push(line.trim()[2..].trim().to_string());
        }
    }

    if let Some(version) = current_version {
        if !current_notes.is_empty() {
            sections.push((version, current_notes));
        }
    }

    const MAX_VERSIONS: usize = 5;
    if sections.len() > MAX_VERSIONS {
        sections[sections.len() - MAX_VERSIONS..].to_vec()
    } else {
        sections
    }
}

fn format_release_notes(path: &Path, notes: &[(String, Vec<String>)]) -> String {
    let mut lines = vec![
        "## Release Notes".into(),
        "source=changelog".into(),
        format!("path={}", path.display()),
    ];
    for (version, entries) in notes {
        lines.push(String::new());
        lines.push(format!("Version {version}:"));
        lines.extend(entries.iter().map(|entry| format!("· {entry}")));
    }
    lines.join("\n")
}

fn detect_releases_url(cwd: &Path) -> Option<String> {
    git_command_output(cwd, &["config", "--get", "remote.origin.url"])
        .and_then(|remote| releases_url_from_remote(&remote))
}

fn releases_url_from_remote(remote: &str) -> Option<String> {
    let remote = remote.trim().trim_end_matches('/');
    if remote.is_empty() {
        return None;
    }
    if let Some(path) = remote.strip_prefix("git@github.com:") {
        return Some(format!(
            "https://github.com/{}/releases",
            path.trim_end_matches(".git")
        ));
    }
    if let Some(path) = remote.strip_prefix("ssh://git@github.com/") {
        return Some(format!(
            "https://github.com/{}/releases",
            path.trim_end_matches(".git")
        ));
    }
    if remote.starts_with("https://") || remote.starts_with("http://") {
        return Some(format!("{}/releases", remote.trim_end_matches(".git")));
    }
    None
}

fn render_feedback_summary(cwd: &Path, draft: &str) -> String {
    let mut lines = vec!["## Feedback".into()];
    if let Some(url) = detect_issues_url(cwd) {
        lines.push("source=issues_page".into());
        lines.push(format!("url={url}"));
        lines.push("The Rust port does not yet implement in-app feedback submission.".into());
        lines.push(
            "Use the repository issues page to report bugs, UX problems, or parity gaps.".into(),
        );
    } else {
        lines.push("source=unavailable".into());
        lines.push("The Rust port does not yet implement in-app feedback submission or a repository issue URL fallback.".into());
    }
    if !draft.is_empty() {
        lines.push(String::new());
        lines.push("Draft report:".into());
        lines.push(draft.into());
    }
    lines.join("\n")
}

fn render_upgrade_summary(browser_launch_attempted: bool) -> String {
    [
        "## Upgrade".into(),
        format!("upgrade_url={UPGRADE_URL}"),
        format!("browser_launch_attempted={browser_launch_attempted}"),
        String::new(),
        "Open the Claude upgrade page to change your subscription tier.".into(),
        "After upgrading, rerun /login if you want wonder-of-u to pick up the new account state.".into(),
        "The Rust port does not yet replicate the leak's in-command subscription introspection flow.".into(),
    ]
    .join("\n")
}

fn render_desktop_summary(browser_launch_attempted: bool) -> String {
    // Keep the fallback explicit until the Rust port can perform a real deep-link session handoff.
    [
        "## Desktop".into(),
        format!("desktop_docs_url={DESKTOP_DOCS_URL}"),
        format!(
            "platform_download_url={}",
            desktop_platform_download_url()
        ),
        format!("browser_launch_attempted={browser_launch_attempted}"),
        String::new(),
        "The Rust port does not yet transfer the live session into Claude Desktop.".into(),
        "Use the docs/download flow to install or update the app, then continue from Claude Desktop separately.".into(),
        "This keeps the command honest until deep-link handoff parity exists.".into(),
    ]
    .join("\n")
}

fn render_mobile_summary() -> String {
    // The reference renders QR codes; for now we expose the same destinations without pretending
    // the ratatui path already has QR rendering parity.
    [
        "## Mobile".into(),
        format!("ios_url={MOBILE_IOS_URL}"),
        format!("android_url={MOBILE_ANDROID_URL}"),
        "qr_rendered=false".into(),
        String::new(),
        "The reference command shows a QR code for the Claude mobile app.".into(),
        "The Rust port does not yet render QR handoff in the TUI, so it exposes direct App Store and Play Store links instead.".into(),
        "Use the links above from your phone or copy them into a browser.".into(),
    ]
    .join("\n")
}

fn render_chrome_summary(browser_launch_attempted: bool) -> String {
    [
        "## Chrome".into(),
        format!("extension_url={CHROME_EXTENSION_URL}"),
        format!("permissions_url={CHROME_PERMISSIONS_URL}"),
        format!("docs_url={CHROME_DOCS_URL}"),
        format!("browser_launch_attempted={browser_launch_attempted}"),
        String::new(),
        "The Rust port does not yet implement the Claude in Chrome extension status picker or default-on config flow.".into(),
        "Open the extension page to install or reconnect Claude in Chrome, then manage site permissions in the extension settings.".into(),
        "Use the docs link for the full browser-control setup guide.".into(),
    ]
    .join("\n")
}

fn render_ide_summary(browser_launch_attempted: bool) -> String {
    [
        "## IDE Integration".into(),
        format!("ide_docs_url={IDE_DOCS_URL}"),
        format!("browser_launch_attempted={browser_launch_attempted}"),
        String::new(),
        "The Rust port does not yet implement the Claude Code IDE extension installer or editor-detection flow.".into(),
        "Open the docs link to install the VS Code or JetBrains extension, then follow the setup guide to connect Claude Code to your editor.".into(),
        "This keeps the command honest until IDE deep-link handoff parity exists.".into(),
    ]
    .join("\n")
}

fn render_insights_enqueue(storage_dir: Option<&Path>, focus: &str) -> Result<String> {
    let Some(storage_dir) = storage_dir else {
        return Ok(vec![
            "## Insights".to_string(),
            "storage_dir=disabled".to_string(),
            "The Rust port needs --storage-dir session history before it can generate an insights report.".to_string(),
        ]
        .join("\n"));
    };
    let store = TranscriptStore::new(storage_dir);
    let metadata = store.list_metadata()?;
    if metadata.is_empty() {
        return Ok(vec![
            "## Insights".to_string(),
            format!("storage_dir={}", store.paths().base_dir().display()),
            "No persisted sessions were found yet, so there is nothing to analyze.".to_string(),
        ]
        .join("\n"));
    }

    let aggregate = aggregate_stats(&metadata);
    let recent_sessions = metadata
        .iter()
        .rev()
        .take(16)
        .map(|entry| {
            json!({
                "session_id": entry.session_id.to_string(),
                "title": entry.title,
                "cwd": entry.cwd.display().to_string(),
                "git_branch": entry.git_branch,
                "updated_at": entry.updated_at.to_string(),
                "message_count": entry.message_count,
                "tags": entry.tags,
                "provider": entry.provider,
                "model": entry.model,
                "total_tokens": entry.costs.usage.total_tokens(),
                "estimated_cost_usd": entry.costs.estimated_cost_usd,
            })
        })
        .collect::<Vec<_>>();
    let top_workdirs = metadata
        .iter()
        .fold(BTreeMap::<String, usize>::new(), |mut counts, entry| {
            *counts.entry(entry.cwd.display().to_string()).or_insert(0) += 1;
            counts
        })
        .into_iter()
        .collect::<Vec<_>>();
    let data = json!({
        "tracked_sessions": metadata.len(),
        "active_days": aggregate.active_days,
        "total_messages": aggregate.total_messages,
        "average_messages_per_session": format!("{:.2}", aggregate.average_messages_per_session),
        "unique_workdirs": aggregate.unique_workdirs,
        "unique_branches": aggregate.unique_branches,
        "provider_counts": aggregate.provider_counts,
        "model_counts": aggregate.model_counts,
        "top_workdirs": top_workdirs,
        "recent_sessions": recent_sessions,
    });
    let mut prompt = concat!(
        "Generate a report analyzing my wonder-of-u session history. ",
        "Be concrete and evidence-based. ",
        "Cover: at a glance, what I'm using the assistant for, what is working well, friction or repeated pain points, and the next experiments or workflow changes I should try. ",
        "Call out recurring repositories, branches, models, token-heavy sessions, and any obvious usage patterns. ",
        "Keep the tone practical."
    )
    .to_string();
    if !focus.is_empty() {
        prompt.push_str(" Additional focus request: ");
        prompt.push_str(focus);
        prompt.push('.');
    }
    prompt.push_str(" Session data JSON: ");
    prompt.push_str(&serde_json::to_string(&data)?);

    Ok([
        "insights_prompt_ready=true".into(),
        format!("tracked_sessions={}", metadata.len()),
        "status=insights prompt queued".into(),
        "note=local-only insights; remote homespace and facet extraction parity are not wired yet"
            .into(),
        format!("enqueue_prompt={prompt}"),
    ]
    .join("\n"))
}

fn desktop_platform_download_url() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "https://claude.ai/api/desktop/win32/x64/exe/latest/redirect"
    }

    #[cfg(not(target_os = "windows"))]
    {
        "https://claude.ai/api/desktop/darwin/universal/dmg/latest/redirect"
    }
}

fn try_open_browser(url: &str) -> bool {
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

fn detect_issues_url(cwd: &Path) -> Option<String> {
    git_command_output(cwd, &["config", "--get", "remote.origin.url"])
        .and_then(|remote| issues_url_from_remote(&remote))
}

fn issues_url_from_remote(remote: &str) -> Option<String> {
    releases_url_from_remote(remote).map(|url| {
        url.strip_suffix("/releases")
            .map_or(url.clone(), |prefix| format!("{prefix}/issues"))
    })
}

fn count_files(path: impl AsRef<Path>, extension: Option<&str>) -> Result<usize> {
    match fs::read_dir(path.as_ref()) {
        Ok(entries) => Ok(entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_type()
                    .map(|file_type| file_type.is_file())
                    .unwrap_or(false)
            })
            .filter(|entry| {
                extension.is_none_or(|extension| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some(extension)
                })
            })
            .count()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use wonder_of_u_core::{AppState, FeatureSet, PermissionMode, SessionId, TokenUsage};
    use wonder_of_u_storage::{SessionMetadata, SessionSnapshot};
    use wonder_of_u_test_support::unique_test_dir;

    fn command_context(cwd: &Path, session_id: SessionId) -> CommandContext {
        CommandContext {
            session_id,
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
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        }
    }

    #[test]
    fn cost_command_reports_current_and_aggregate_totals() {
        let storage_dir = unique_test_dir("status-cost-command");
        let store = TranscriptStore::new(storage_dir.clone());

        let mut current = AppState::new(storage_dir.join("workspace"));
        current.record_cost_usage(
            TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                cache_creation_tokens: 2,
                cache_read_tokens: 1,
            },
            Some(0.25),
        );
        store
            .write_snapshot(&SessionSnapshot::from_app_state(
                &current,
                current.messages.len(),
                0,
            ))
            .expect("write current snapshot");
        store
            .write_metadata(&SessionMetadata::from_app_state(&current))
            .expect("write current metadata");

        let mut previous = AppState::new(storage_dir.join("other"));
        previous.record_cost_usage(
            TokenUsage {
                input_tokens: 4,
                output_tokens: 3,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            Some(0.10),
        );
        store
            .write_metadata(&SessionMetadata::from_app_state(&previous))
            .expect("write previous metadata");

        let output = block_on(CostCommand::new(Some(storage_dir.clone())).execute(
            command_context(&current.session.cwd, current.session.id),
            CommandInvocation {
                name: "cost".into(),
                args: String::new(),
                raw: "/cost".into(),
            },
        ))
        .expect("run cost command");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("session_total_tokens=18"));
        assert!(text.contains("session_estimated_cost_usd=0.2500"));
        assert!(text.contains("tracked_sessions=2"));
        assert!(text.contains("aggregate_total_tokens=25"));
        assert!(text.contains("aggregate_estimated_cost_usd=0.3500"));
    }

    #[test]
    fn stats_command_reports_session_activity() {
        let storage_dir = unique_test_dir("status-stats-command");
        let store = TranscriptStore::new(storage_dir.clone());

        let mut first = AppState::new(storage_dir.join("workspace-a"));
        first.session.git_branch = Some("main".into());
        first.session.updated_at = first.session.created_at;
        first.session.title = "workspace-a".into();
        first.set_provider_context(
            Some("openai".into()),
            Some("gpt-4.1".into()),
            wonder_of_u_core::AuthState::missing(wonder_of_u_core::AuthMaterialKind::ApiKey),
        );
        store
            .write_metadata(&SessionMetadata::from_app_state(&first))
            .expect("write first metadata");

        let mut second = AppState::new(storage_dir.join("workspace-b"));
        second.session.git_branch = Some("feature/demo".into());
        second.session.title = "workspace-b".into();
        second.set_provider_context(
            Some("anthropic".into()),
            Some("claude-3-5-haiku-latest".into()),
            wonder_of_u_core::AuthState::missing(wonder_of_u_core::AuthMaterialKind::ApiKey),
        );
        store
            .write_metadata(&SessionMetadata::from_app_state(&second))
            .expect("write second metadata");

        let output = block_on(StatsCommand::new(Some(storage_dir.clone())).execute(
            command_context(&first.session.cwd, first.session.id),
            CommandInvocation {
                name: "stats".into(),
                args: String::new(),
                raw: "/stats".into(),
            },
        ))
        .expect("run stats command");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("## Activity Stats"));
        assert!(text.contains("tracked_sessions=2"));
        assert!(text.contains("providers=2"));
        assert!(text.contains("provider[openai]=1"));
        assert!(text.contains("provider[anthropic]=1"));
        assert!(text.contains("unique_branches=2"));
    }

    #[test]
    fn version_summary_reports_runtime_metadata() {
        let rendered = render_version_summary();

        assert!(rendered.contains("## Version"));
        assert!(rendered.contains("application=wonder-of-u"));
        assert!(rendered.contains(&format!("version={}", env!("CARGO_PKG_VERSION"))));
        assert!(rendered.contains("message_schema_version="));
        assert!(rendered.contains("storage_schema_version="));
    }

    #[test]
    fn usage_summary_reports_local_cost_totals() {
        let storage_dir = unique_test_dir("status-usage-command");
        let store = TranscriptStore::new(storage_dir.clone());
        let mut current = AppState::new(storage_dir.join("workspace"));
        current.record_cost_usage(
            TokenUsage {
                input_tokens: 4,
                output_tokens: 2,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            Some(0.0123),
        );
        store
            .write_metadata(&SessionMetadata::from_app_state(&current))
            .expect("write metadata");
        store
            .write_snapshot(&SessionSnapshot::from_app_state(&current, 0, 0))
            .expect("write snapshot");

        let rendered = render_usage_summary(
            Some(storage_dir.as_path()),
            &command_context(&current.session.cwd, current.session.id),
        )
        .expect("usage summary");

        assert!(rendered.contains("## Usage"));
        assert!(rendered.contains("current_session_total_tokens=6"));
        assert!(rendered.contains("aggregate_total_tokens=6"));
        assert!(rendered.contains("Use /cost for raw token and estimated-cost details."));
    }

    #[test]
    fn release_notes_summary_reads_local_changelog() {
        let dir = unique_test_dir("status-release-notes-local");
        let nested = dir.join("nested/project");
        fs::create_dir_all(&nested).expect("create nested");
        fs::write(
            dir.join("CHANGELOG.md"),
            "# Changelog\n\n## 0.2.0 - 2026-04-26\n- Added /color\n- Added /release-notes\n\n## 0.1.0\n- Initial release\n",
        )
        .expect("write changelog");

        let rendered = render_release_notes_summary(&nested);

        assert!(rendered.contains("## Release Notes"));
        assert!(rendered.contains("source=changelog"));
        assert!(rendered.contains("Version 0.2.0:"));
        assert!(rendered.contains("· Added /color"));
    }

    #[test]
    fn release_notes_remote_urls_normalize_to_releases_pages() {
        assert_eq!(
            releases_url_from_remote("git@github.com:ratchanonth60/wonderofu.git"),
            Some("https://github.com/ratchanonth60/wonderofu/releases".into())
        );
        assert_eq!(
            releases_url_from_remote("https://github.com/ratchanonth60/wonderofu.git"),
            Some("https://github.com/ratchanonth60/wonderofu/releases".into())
        );
    }

    #[test]
    fn feedback_remote_urls_normalize_to_issues_pages() {
        assert_eq!(
            issues_url_from_remote("git@github.com:ratchanonth60/wonderofu.git"),
            Some("https://github.com/ratchanonth60/wonderofu/issues".into())
        );
    }

    #[test]
    fn feedback_summary_includes_draft_text() {
        let rendered =
            render_feedback_summary(Path::new("/tmp"), "TUI picker flow still feels off");

        assert!(rendered.contains("## Feedback"));
        assert!(rendered.contains("Draft report:"));
        assert!(rendered.contains("TUI picker flow still feels off"));
    }

    #[test]
    fn upgrade_summary_points_to_upgrade_flow() {
        let rendered = render_upgrade_summary(false);

        assert!(rendered.contains("## Upgrade"));
        assert!(rendered.contains("upgrade_url=https://claude.ai/upgrade/max"));
        assert!(rendered.contains("browser_launch_attempted=false"));
        assert!(rendered.contains("rerun /login"));
        assert!(rendered.contains("subscription introspection flow"));
    }

    #[test]
    fn desktop_summary_points_to_docs_and_download_flow() {
        let rendered = render_desktop_summary(false);

        assert!(rendered.contains("## Desktop"));
        assert!(rendered.contains("desktop_docs_url=https://clau.de/desktop"));
        assert!(rendered.contains("platform_download_url="));
        assert!(rendered.contains("browser_launch_attempted=false"));
        assert!(rendered.contains("does not yet transfer the live session"));
    }

    #[test]
    fn mobile_summary_exposes_store_links_without_qr() {
        let rendered = render_mobile_summary();

        assert!(rendered.contains("## Mobile"));
        assert!(
            rendered
                .contains("ios_url=https://apps.apple.com/app/claude-by-anthropic/id6473753684")
        );
        assert!(rendered.contains(
            "android_url=https://play.google.com/store/apps/details?id=com.anthropic.claude"
        ));
        assert!(rendered.contains("qr_rendered=false"));
        assert!(rendered.contains("does not yet render QR handoff"));
    }

    #[test]
    fn mobile_command_spec_includes_platform_aliases() {
        let spec = MobileCommand::command_spec();

        assert_eq!(spec.name, "mobile");
        assert_eq!(spec.aliases, vec!["ios".to_string(), "android".to_string()]);
    }

    #[test]
    fn chrome_summary_points_to_extension_setup_links() {
        let rendered = render_chrome_summary(false);

        assert!(rendered.contains("## Chrome"));
        assert!(rendered.contains("extension_url=https://claude.ai/chrome"));
        assert!(rendered.contains("permissions_url=https://clau.de/chrome/permissions"));
        assert!(rendered.contains("docs_url=https://code.claude.com/docs/en/chrome"));
        assert!(rendered.contains("browser_launch_attempted=false"));
        assert!(rendered.contains("extension status picker"));
    }

    #[test]
    fn insights_summary_requires_storage() {
        let rendered = render_insights_enqueue(None, "").expect("insights");

        assert!(rendered.contains("## Insights"));
        assert!(rendered.contains("storage_dir=disabled"));
    }

    #[test]
    fn insights_summary_queues_prompt_from_metadata() {
        let storage_dir = unique_test_dir("status-insights-command");
        let store = TranscriptStore::new(storage_dir.clone());

        let mut first = AppState::new(storage_dir.join("workspace-a"));
        first.session.title = "Fix TUI parity".into();
        first.session.git_branch = Some("feat/tui-command-parity".into());
        first.provider = Some("openai".into());
        first.model = Some("gpt-5.4".into());
        first.set_session_tags(vec!["parity".into()]);
        first.record_cost_usage(
            TokenUsage {
                input_tokens: 20,
                output_tokens: 7,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            Some(0.33),
        );
        store
            .write_metadata(&SessionMetadata::from_app_state(&first))
            .expect("write first metadata");

        let mut second = AppState::new(storage_dir.join("workspace-b"));
        second.session.title = "Add review command".into();
        second.provider = Some("openai".into());
        second.model = Some("gpt-5.4".into());
        second.record_cost_usage(
            TokenUsage {
                input_tokens: 10,
                output_tokens: 3,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            None,
        );
        store
            .write_metadata(&SessionMetadata::from_app_state(&second))
            .expect("write second metadata");

        let rendered =
            render_insights_enqueue(Some(storage_dir.as_path()), "focus on TUI workflow")
                .expect("render insights");

        assert!(rendered.contains("insights_prompt_ready=true"));
        assert!(rendered.contains("tracked_sessions=2"));
        assert!(rendered.contains(
            "enqueue_prompt=Generate a report analyzing my wonder-of-u session history."
        ));
        assert!(rendered.contains("focus on TUI workflow"));
        assert!(rendered.contains("\"Fix TUI parity\""));
    }

    #[test]
    fn output_style_command_returns_deprecation_notice() {
        let output = block_on(OutputStyleCommand::new().execute(
            command_context(Path::new("/workspace"), SessionId::new()),
            CommandInvocation {
                name: "output-style".into(),
                args: String::new(),
                raw: "/output-style".into(),
            },
        ))
        .expect("run output-style command");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("/output-style has been deprecated"));
    }
}
