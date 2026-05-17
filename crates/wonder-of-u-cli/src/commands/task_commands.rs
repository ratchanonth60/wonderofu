//! Implements the agent-task and shell-task management commands:
//! `/agents` and `/tasks`.

use std::{collections::BTreeSet, path::PathBuf, thread, time::Duration};

use async_trait::async_trait;
use clap::{Args, Parser, Subcommand};
use wonder_of_u_agent::ProviderResolver;
use wonder_of_u_core::{
    AgentRuntime, Command, CommandContext, CommandInvocation, CommandKind, CommandOutput,
    CommandSpec, FeatureFlag, Result, TaskId, TaskKind, TaskState, TaskStatus, WonderError,
};

use super::parse_command_args;
use super::task_runtime::{
    AgentTaskLaunch, ShellTaskLaunch, TaskManager, TaskPruneReport, TaskReconcileReport,
    TaskSummary, task_heartbeat_state, task_heartbeat_state_label,
};
use super::workflow::{parse_permission_mode, sanitize_single_line};

// ── Command structs ───────────────────────────────────────────────────────────

/// Manages persisted local agent tasks.
pub struct AgentsCommand {
    storage_dir: Option<PathBuf>,
}

impl AgentsCommand {
    /// Creates a new `AgentsCommand` with an optional persistent storage directory.
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Returns the `CommandSpec` for `/agents`.
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

/// Monitors and manages background shell and agent tasks.
pub struct TasksCommand {
    storage_dir: Option<PathBuf>,
}

impl TasksCommand {
    /// Creates a new `TasksCommand` with an optional persistent storage directory.
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Returns the `CommandSpec` for `/tasks`.
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
        // Upstream alias: /bashes resolves to /tasks.
        spec.aliases = vec!["bashes".into()];
        spec
    }
}

// ── Clap arg structs ──────────────────────────────────────────────────────────

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
    cwd: Option<std::path::PathBuf>,
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

// ── Command impls ─────────────────────────────────────────────────────────────

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

    pub(super) fn status(&self) -> Result<CommandOutput> {
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

// ── Render helpers ────────────────────────────────────────────────────────────

fn render_runtime_disabled(kind: &str, note: &str) -> String {
    format!("{kind}=0\nstorage_dir=disabled\nnote={note}")
}

fn render_task_summary_lines(
    label: &str,
    manager: &TaskManager,
    tasks: &[TaskState],
) -> Vec<String> {
    let summary = TaskSummary::from_tasks(tasks);
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

pub(super) fn render_task_list(
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

pub(super) fn render_task_detail(
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

fn render_task_prune_report(report: &TaskPruneReport) -> String {
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

fn agent_runtime_label(runtime: AgentRuntime) -> &'static str {
    match runtime {
        AgentRuntime::MetadataOnly => "metadata_only",
        AgentRuntime::PromptSubprocess => "prompt_subprocess",
        AgentRuntime::Deferred => "legacy_relaunch_required",
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use wonder_of_u_core::{AgentTaskState, CommandOutput, FleetId, TaskId, TaskState};
    use wonder_of_u_test_support::unique_test_dir;

    use super::{AgentsCommand, TaskManager, render_task_detail, render_task_list};
    use crate::commands::task_runtime::TaskReconcileReport;

    #[test]
    fn task_list_includes_fleet_metadata_when_present() {
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
        // Verify /agents status still works correctly after
        // the definitions sub-surface was extracted.
        let cmd = AgentsCommand::new(None);
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
