use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
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
use wonder_of_u_storage::{SessionMemoryIndexStore, SyncStatusReport};

use super::plugin::load_catalogs;
use super::task_runtime::TaskManager;

/// Represents status command
pub struct StatusCommand {
    storage_dir: Option<PathBuf>,
}

/// Represents cost command
pub struct CostCommand {
    storage_dir: Option<PathBuf>,
}
/// Represents stats command
pub struct StatsCommand {
    storage_dir: Option<PathBuf>,
}

/// Represents version command
pub struct VersionCommand;
/// Represents insights command
pub struct InsightsCommand {
    storage_dir: Option<PathBuf>,
}

/// Represents output style command
pub struct OutputStyleCommand;

impl StatusCommand {
    /// Creates a new value
    pub fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "status",
            "Show runtime, storage, MCP, plugin, skill, and task status",
            CommandKind::NonInteractive,
        )
    }
}

fn provider_id_list(providers: &[wonder_of_u_agent::ProviderDescriptor]) -> String {
    providers
        .iter()
        .map(|provider| provider.id.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

fn provider_inventory_lines(report: &wonder_of_u_agent::ProviderStatusReport) -> Vec<String> {
    let configured_ids: BTreeSet<&str> = report
        .configured_providers
        .iter()
        .map(|provider| provider.id.as_str())
        .collect();
    let authenticated_ids: BTreeSet<&str> = report
        .authenticated_providers
        .iter()
        .map(|provider| provider.id.as_str())
        .collect();
    let ready_ids: BTreeSet<&str> = report
        .ready_providers
        .iter()
        .map(|provider| provider.id.as_str())
        .collect();

    let mut lines = vec![
        format!(
            "registered_providers={}",
            provider_id_list(&report.available_providers)
        ),
        format!(
            "configured_providers={}",
            provider_id_list(&report.configured_providers)
        ),
        format!(
            "authenticated_providers={}",
            provider_id_list(&report.authenticated_providers)
        ),
        format!(
            "ready_providers={}",
            provider_id_list(&report.ready_providers)
        ),
    ];

    for provider in &report.available_providers {
        let mut hint = format!(
            "provider_status[{}]=configured={};authenticated={};ready={};auth={}",
            provider.id,
            configured_ids.contains(provider.id.as_str()),
            authenticated_ids.contains(provider.id.as_str()),
            ready_ids.contains(provider.id.as_str()),
            auth_kind_label(provider.auth_kind),
        );
        if let Some(env) = &provider.api_key_env {
            hint.push_str(&format!(";api_key_env={env}"));
        }
        if let Some(env) = &provider.endpoint_env {
            hint.push_str(&format!(";endpoint_env={env}"));
        }
        lines.push(hint);
    }

    lines
}

fn auth_kind_label(kind: wonder_of_u_core::AuthMaterialKind) -> &'static str {
    match kind {
        wonder_of_u_core::AuthMaterialKind::None => "none",
        wonder_of_u_core::AuthMaterialKind::ApiKey => "api_key",
        wonder_of_u_core::AuthMaterialKind::OAuth => "oauth",
        wonder_of_u_core::AuthMaterialKind::AwsSigV4 => "aws_sigv4",
        wonder_of_u_core::AuthMaterialKind::AwsBearer => "aws_bearer",
        wonder_of_u_core::AuthMaterialKind::AwsProfile => "aws_profile",
        wonder_of_u_core::AuthMaterialKind::GcpOAuth2 => "gcp_oauth2",
    }
}

impl CostCommand {
    /// Creates a new value
    pub fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "cost",
            "Show current-session and aggregate token/cost totals",
            CommandKind::Local,
        )
    }
}
impl StatsCommand {
    /// Creates a new value
    pub fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "stats",
            "Show aggregate session activity statistics",
            CommandKind::Local,
        )
    }
}

impl VersionCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "version",
            "Show application and schema version details",
            CommandKind::Local,
        )
    }
}
impl InsightsCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
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
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
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
        lines.extend(provider_inventory_lines(&report));

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
                let memory_indexes = SessionMemoryIndexStore::new(storage_dir.clone()).list()?;
                lines.push(format!("session_memory_indexes={}", memory_indexes.len()));
                lines.push(format!(
                    "session_memory_entries={}",
                    memory_indexes
                        .iter()
                        .map(|index| index.entries.len())
                        .sum::<usize>()
                ));
                let sync_status = SyncStatusReport::inspect(store.paths());
                lines.push(format!(
                    "settings_sync={}",
                    sync_status.settings_sync.status.label()
                ));
                lines.push(format!(
                    "settings_sync_cloud={}",
                    sync_status.settings_sync.cloud_status.label()
                ));
                lines.push(format!(
                    "settings_sync_cloud_attempted={}",
                    sync_status.settings_sync.cloud_attempted
                ));
                lines.push(format!(
                    "settings_sync_reason={}",
                    sync_status.settings_sync.reason
                ));
                lines.push(format!(
                    "remote_managed_settings={}",
                    sync_status.remote_managed_settings.status.label()
                ));
                lines.push(format!(
                    "remote_managed_settings_cloud_attempted={}",
                    sync_status.remote_managed_settings.cloud_attempted
                ));
                lines.push(format!(
                    "remote_managed_settings_reason={}",
                    sync_status.remote_managed_settings.reason
                ));
                lines.push(format!(
                    "team_memory_sync={}",
                    sync_status.team_memory_sync.status.label()
                ));
                lines.push(format!(
                    "team_memory_sync_cloud_attempted={}",
                    sync_status.team_memory_sync.cloud_attempted
                ));
                lines.push(format!(
                    "team_memory_sync_reason={}",
                    sync_status.team_memory_sync.reason
                ));
                lines.push(format!(
                    "analytics={}",
                    sync_status.analytics.status.label()
                ));
                lines.push(format!(
                    "experiments={}",
                    sync_status.experiments.status.label()
                ));
                let mcp_store = McpConfigStore::new(storage_dir.clone());
                let (mcp_config, project_config_path) =
                    mcp_store.read_with_project(&context.cwd, None)?;
                let mcp_report =
                    McpStatusReport::inspect(mcp_store.paths().mcp_servers_path(), &mcp_config);
                lines.push(format!(
                    "mcp_project_config={}",
                    project_config_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "none".into())
                ));
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
fn render_insights_enqueue(storage_dir: Option<&Path>, focus: &str) -> Result<String> {
    let Some(storage_dir) = storage_dir else {
        return Ok([
            "## Insights".to_string(),
            "storage_dir=disabled".to_string(),
            "The Rust port needs --storage-dir session history before it can generate an insights report.".to_string(),
        ]
        .join("\n"));
    };
    let store = TranscriptStore::new(storage_dir);
    let metadata = store.list_metadata()?;
    if metadata.is_empty() {
        return Ok([
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
        "note=local insights use stored session metadata and can be extended by the queued provider prompt".into(),
        format!("enqueue_prompt={prompt}"),
    ]
    .join("\n"))
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
    use wonder_of_u_agent::ProviderResolver;
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
            optimize_token_mode: false,
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
    fn provider_inventory_lines_keep_registered_providers_distinct_from_ready_ones() {
        let report = ProviderResolver::builtin()
            .resolve_with_env(
                &wonder_of_u_agent::AgentSettings::default(),
                &wonder_of_u_agent::StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
            )
            .expect("resolve report");

        let rendered = provider_inventory_lines(&report).join("\n");

        assert!(rendered.contains("registered_providers="));
        assert!(rendered.contains("ready_providers=local"));
        assert!(rendered.contains("configured_providers="));
        assert!(rendered.contains(
            "provider_status[openai]=configured=false;authenticated=false;ready=false;auth=api_key"
        ));
        assert!(rendered.contains(
            "provider_status[local]=configured=false;authenticated=false;ready=true;auth=none"
        ));
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
    fn output_style_command_spec_is_hidden() {
        // Claude Code reference behavior: /output-style is hidden so it does
        // not surface in user-visible autocompletion or help listings.
        let spec = OutputStyleCommand::command_spec();
        assert!(
            spec.hidden,
            "/output-style CommandSpec must be hidden to match Claude Code reference behavior"
        );
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
