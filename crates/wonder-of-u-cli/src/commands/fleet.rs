//! Fleet command — native multi-agent task orchestration.
//!
//! This module implements the `/fleet` slash-command surface, which lets users
//! and model-callable tools coordinate groups of local-agent tasks under a
//! single [`FleetRunState`] record.
//!
//! # Subcommands
//!
//! | Subcommand | Description |
//! |---|---|
//! | `dispatch` | Drain pending `FleetMemberRequest` files into real agent tasks. |
//! | `start` | Start a fleet run and immediately launch one or more agent tasks. |
//! | `list` | List known fleet runs. |
//! | `status` | Show aggregate fleet statistics. |
//! | `show <fleet_id>` | Show a fleet run's header and member task statuses. |
//! | `stop-all <fleet_id>` | Stop or cancel all non-terminal tasks in a fleet. |
//!
//! # Design notes
//!
//! * `fleet dispatch` does **not** abort on the first failure; it drains all
//!   pending requests and reports per-request errors at the end.
//! * `fleet stop-all` does not implement force-kill; it calls the existing
//!   [`TaskManager::stop_task`] helper which sends SIGTERM and waits briefly.
//!   Tasks that cannot be stopped within that window are reported as failures
//!   without affecting the remaining members.

use std::{collections::BTreeSet, path::PathBuf};

use async_trait::async_trait;
use clap::{Args, Parser, Subcommand};
use wonder_of_u_agent::{ProviderResolver, ProviderStatusReport};
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec,
    FeatureFlag, FleetAgentRole, FleetId, FleetMemberRequest, FleetRoleCatalog, FleetRunState,
    FleetRunStatus, Result, TaskId, TaskStatus, WonderError,
};
use wonder_of_u_storage::FleetStore;

use super::{
    parse_command_args,
    task_runtime::{AgentTaskLaunch, TaskManager},
};

/// The `/fleet` slash-command.
pub struct FleetCommand {
    storage_dir: Option<PathBuf>,
}

impl FleetCommand {
    /// Creates a new [`FleetCommand`].
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Returns the [`CommandSpec`] used to register this command.
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "fleet",
            "Manage fleet runs (multi-agent task groups)",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::Fleet, FeatureFlag::Agents]);
        spec
    }
}

// ── Clap argument types ───────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct FleetArgs {
    #[command(subcommand)]
    command: Option<FleetSubcommand>,
}

#[derive(Debug, Subcommand)]
enum FleetSubcommand {
    /// Drain pending fleet member requests into live agent tasks.
    Dispatch,
    /// Start a fleet run and immediately launch one or more agent tasks.
    Start(FleetStartArgs),
    /// List fleet runs.
    List(FleetListArgs),
    /// Show aggregate fleet statistics.
    Status,
    /// Show a specific fleet run and its member task statuses.
    Show(FleetShowArgs),
    /// Stop (cancel) all non-terminal member tasks in a fleet run.
    StopAll(FleetStopAllArgs),
    /// List available fleet agent roles.
    Roles,
}

#[derive(Debug, Args)]
struct FleetStartArgs {
    /// Human-readable description of what this fleet run will accomplish.
    #[arg(long)]
    description: String,
    /// Agent prompt (used when `--member-name` is provided to create an
    /// immediate single-member fleet).
    #[arg(long)]
    prompt: Option<String>,
    /// Optional member name for the immediate agent task.
    #[arg(long)]
    member_name: Option<String>,
    /// Optional fleet agent role id (e.g. `rust-engineer`).
    ///
    /// When set, the role's prompt preamble is prepended to `--prompt` before
    /// launching the agent task.
    #[arg(long)]
    role: Option<String>,
    /// Optional model override for the immediate agent task.
    #[arg(long)]
    model: Option<String>,
    /// Optional provider override for the immediate agent task.
    #[arg(long)]
    provider: Option<String>,
    /// Optional working directory for the immediate agent task.
    #[arg(long)]
    cwd: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct FleetListArgs {
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

#[derive(Debug, Args)]
struct FleetShowArgs {
    fleet_id: String,
    /// Number of log tail lines to show per member task.
    #[arg(long = "tail", default_value_t = 5)]
    tail_lines: usize,
}

#[derive(Debug, Args)]
struct FleetStopAllArgs {
    fleet_id: String,
}

// ── Command trait impl ────────────────────────────────────────────────────────

#[async_trait]
impl Command for FleetCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<FleetArgs>("fleet", &invocation)?;
        match args.command.unwrap_or(FleetSubcommand::Status) {
            FleetSubcommand::Dispatch => self.dispatch(context),
            FleetSubcommand::Start(args) => self.start(context, args),
            FleetSubcommand::List(args) => self.list(args),
            FleetSubcommand::Status => self.status(),
            FleetSubcommand::Show(args) => self.show(args),
            FleetSubcommand::StopAll(args) => self.stop_all(args),
            FleetSubcommand::Roles => Self::roles(),
        }
    }
}

// ── Sub-command implementations ───────────────────────────────────────────────

impl FleetCommand {
    /// Drains all pending `FleetMemberRequest` files into live agent tasks.
    ///
    /// Processes requests in queue order (oldest first).  A per-request
    /// failure is recorded but does **not** abort remaining requests.
    fn dispatch(&self, context: CommandContext) -> Result<CommandOutput> {
        let (manager, store) = match self.stores() {
            Some(pair) => pair,
            None => {
                return Ok(CommandOutput::Text(runtime_disabled_message(
                    "fleet dispatch",
                )));
            }
        };

        let pending = store.list_pending_requests()?;
        if pending.is_empty() {
            return Ok(CommandOutput::Text(
                "fleet_pending=0\nnote=no pending fleet requests to dispatch".into(),
            ));
        }

        let report = ProviderResolver::builtin().load_report(self.storage_dir.as_deref())?;
        let mut dispatched = 0usize;
        let mut errors: Vec<String> = Vec::new();

        for req in &pending {
            match dispatch_one(&manager, &store, &context, req, &report) {
                Ok(task_id) => {
                    dispatched += 1;
                    // Best-effort: if the request belongs to a fleet run, link
                    // the new task to that run.
                    if let Some(fleet_id) = req.fleet_id {
                        let _ = link_task_to_fleet(&store, fleet_id, task_id);
                    }
                }
                Err(error) => {
                    errors.push(format!(
                        "request_error[{}]: {}",
                        req.id,
                        sanitize_line(&error.to_string())
                    ));
                }
            }
        }

        let mut lines = vec![
            format!("fleet_pending_total={}", pending.len()),
            format!("fleet_dispatched={dispatched}"),
            format!("fleet_dispatch_errors={}", errors.len()),
        ];
        lines.extend(errors);
        Ok(CommandOutput::Text(lines.join("\n")))
    }

    /// Starts a new fleet run, optionally launching an immediate member task.
    fn start(&self, context: CommandContext, args: FleetStartArgs) -> Result<CommandOutput> {
        let (manager, store) = match self.stores() {
            Some(pair) => pair,
            None => {
                return Ok(CommandOutput::Text(runtime_disabled_message("fleet start")));
            }
        };

        // Resolve role early so we can include it in the output.
        let resolved_role = if let Some(ref role_id) = args.role {
            let catalog = FleetRoleCatalog::builtin();
            let role = catalog
                .get(role_id)
                .or_else(|| catalog.resolve_alias(role_id));
            match role {
                Some(r) => Some(r.clone()),
                None => {
                    return Err(WonderError::validation(format!(
                        "unknown fleet role `{role_id}`; known roles: {}",
                        catalog.known_ids_display()
                    )));
                }
            }
        } else {
            None
        };

        let mut run = FleetRunState::new(
            &args.description,
            context.permission_mode,
            Some(context.cwd.clone()),
        );

        // If both --prompt and --member-name are given, launch an immediate
        // agent task and link it to this fleet run.
        if let Some(prompt) = &args.prompt {
            let report = ProviderResolver::builtin().load_report(self.storage_dir.as_deref())?;
            let cwd = args.cwd.clone().unwrap_or_else(|| context.cwd.clone());

            // Compose full prompt: prepend role preamble when a role is set.
            let full_prompt = if let Some(ref role) = resolved_role {
                role.compose_prompt(prompt)
            } else {
                prompt.clone()
            };

            let task = manager.start_agent_task(AgentTaskLaunch {
                name: args
                    .member_name
                    .clone()
                    .unwrap_or_else(|| run.id.to_string()),
                description: Some(args.description.clone()),
                prompt: full_prompt,
                provider: args.provider.or(report.provider),
                model: args.model.or(report.model),
                cwd,
                fleet_id: Some(run.id),
                allowed_tools: resolved_role
                    .as_ref()
                    .and_then(role_allowed_tools_for_launch),
            })?;
            run.member_task_ids.push(task.id);
            run.status = FleetRunStatus::Running;
        }

        store.write_run(&run)?;

        let mut lines = vec![
            format!("fleet_id={}", run.id),
            format!("description={}", sanitize_line(&run.description)),
            format!("status={}", run.status.label()),
            format!("members={}", run.member_task_ids.len()),
        ];
        if let Some(ref role) = resolved_role {
            lines.push(format!("role={}", role.id));
            lines.push(format!("role_name={}", sanitize_line(&role.name)));
        }
        for (i, task_id) in run.member_task_ids.iter().enumerate() {
            lines.push(format!("member[{i}].task_id={task_id}"));
        }
        lines.push("note=use `fleet dispatch` to launch queued pending requests".into());
        Ok(CommandOutput::Text(lines.join("\n")))
    }

    /// Lists fleet runs (most recent first).
    fn list(&self, args: FleetListArgs) -> Result<CommandOutput> {
        let (_, store) = match self.stores() {
            Some(pair) => pair,
            None => {
                return Ok(CommandOutput::Text(runtime_disabled_message("fleet list")));
            }
        };

        let runs = store.list_runs()?;
        let limit = args.limit.max(1);
        let mut lines = vec![
            format!("fleet_runs={}", runs.len()),
            format!("fleet_runs_shown={}", runs.len().min(limit)),
        ];
        for (i, run) in runs.iter().take(limit).enumerate() {
            lines.push(format!("fleet[{i}].id={}", run.id));
            lines.push(format!(
                "fleet[{i}].description={}",
                sanitize_line(&run.description)
            ));
            lines.push(format!("fleet[{i}].status={}", run.status.label()));
            lines.push(format!("fleet[{i}].members={}", run.member_task_ids.len()));
            lines.push(format!("fleet[{i}].started_at={}", run.started_at));
        }
        if runs.len() > limit {
            lines.push(format!("truncated=true ({} shown)", limit));
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }

    /// Shows aggregate statistics for all fleet runs.
    fn status(&self) -> Result<CommandOutput> {
        let (_, store) = match self.stores() {
            Some(pair) => pair,
            None => {
                return Ok(CommandOutput::Text(runtime_disabled_message(
                    "fleet status",
                )));
            }
        };

        let runs = store.list_runs()?;
        let pending_requests = store.list_pending_requests()?;

        let mut pending = 0usize;
        let mut running = 0usize;
        let mut completed = 0usize;
        let mut failed = 0usize;
        let mut cancelled = 0usize;
        for run in &runs {
            match run.status {
                FleetRunStatus::Pending => pending += 1,
                FleetRunStatus::Running => running += 1,
                FleetRunStatus::Completed => completed += 1,
                FleetRunStatus::Failed => failed += 1,
                FleetRunStatus::Cancelled => cancelled += 1,
            }
        }

        let lines: &[String] = &[
            format!("fleet_runs={}", runs.len()),
            format!("fleet_pending={pending}"),
            format!("fleet_running={running}"),
            format!("fleet_completed={completed}"),
            format!("fleet_failed={failed}"),
            format!("fleet_cancelled={cancelled}"),
            format!("pending_dispatch_requests={}", pending_requests.len()),
            "note=run `fleet dispatch` to launch pending agent requests".into(),
        ];
        Ok(CommandOutput::Text(lines.join("\n")))
    }

    /// Shows a specific fleet run with member task statuses and log tails.
    fn show(&self, args: FleetShowArgs) -> Result<CommandOutput> {
        let (manager, store) = match self.stores() {
            Some(pair) => pair,
            None => {
                return Ok(CommandOutput::Text(runtime_disabled_message("fleet show")));
            }
        };

        let fleet_id = parse_fleet_id(&args.fleet_id)?;
        let run = store.read_run(fleet_id)?;

        let mut lines = vec![
            format!("fleet_id={}", run.id),
            format!("description={}", sanitize_line(&run.description)),
            format!("status={}", run.status.label()),
            format!("permission_mode={:?}", run.permission_mode),
            format!("started_at={}", run.started_at),
        ];
        if let Some(finished_at) = run.finished_at {
            lines.push(format!("finished_at={finished_at}"));
        }
        if let Some(cwd) = &run.cwd {
            lines.push(format!("cwd={}", cwd.display()));
        }
        lines.push(format!("members={}", run.member_task_ids.len()));

        for (i, task_id) in run.member_task_ids.iter().enumerate() {
            match manager.get_task(*task_id) {
                Ok(task) => {
                    lines.push(format!("member[{i}].task_id={task_id}"));
                    lines.push(format!(
                        "member[{i}].status={}",
                        task_status_label(task.status)
                    ));
                    lines.push(format!(
                        "member[{i}].description={}",
                        sanitize_line(&task.description)
                    ));
                    if let Some(agent) = &task.agent {
                        lines.push(format!("member[{i}].name={}", sanitize_line(&agent.name)));
                    }
                    if args.tail_lines > 0 {
                        if let Ok(tail) = manager.read_log_tail(*task_id, args.tail_lines) {
                            for (j, log_line) in tail.iter().enumerate() {
                                lines.push(format!(
                                    "member[{i}].log[{j}]={}",
                                    sanitize_line(log_line)
                                ));
                            }
                        }
                    }
                }
                Err(_) => {
                    lines.push(format!("member[{i}].task_id={task_id}"));
                    lines.push(format!("member[{i}].status=not_found"));
                }
            }
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }

    /// Stops (cancels) all non-terminal member tasks in a fleet run.
    ///
    /// Failures to stop individual tasks are reported without aborting the
    /// remaining members.  After attempting all stops the fleet run status
    /// is recomputed and persisted.
    ///
    /// # Limitations
    ///
    /// Uses SIGTERM only (no force-kill flag); tasks that survive the 2-second
    /// wait window will be reported as errors.
    fn stop_all(&self, args: FleetStopAllArgs) -> Result<CommandOutput> {
        let (manager, store) = match self.stores() {
            Some(pair) => pair,
            None => {
                return Ok(CommandOutput::Text(runtime_disabled_message(
                    "fleet stop-all",
                )));
            }
        };

        let fleet_id = parse_fleet_id(&args.fleet_id)?;
        let mut run = store.read_run(fleet_id)?;

        let mut stopped = 0usize;
        let mut already_terminal = 0usize;
        let mut errors: Vec<String> = Vec::new();
        let mut final_statuses: Vec<TaskStatus> = Vec::new();

        for task_id in &run.member_task_ids {
            match manager.get_task(*task_id) {
                Ok(task) if task.status.is_terminal() => {
                    already_terminal += 1;
                    final_statuses.push(task.status);
                }
                Ok(_) => match manager.stop_task(*task_id, false) {
                    Ok(task) => {
                        stopped += 1;
                        final_statuses.push(task.status);
                    }
                    Err(error) => {
                        errors.push(format!(
                            "stop_error[{task_id}]: {}",
                            sanitize_line(&error.to_string())
                        ));
                        // Use Running as a pessimistic placeholder for tasks
                        // we failed to stop, so the fleet doesn't incorrectly
                        // become terminal.
                        final_statuses.push(TaskStatus::Running);
                    }
                },
                Err(_) => {
                    // Task record missing — treat as cancelled for status calc.
                    final_statuses.push(TaskStatus::Cancelled);
                }
            }
        }

        run.reconcile_status(&final_statuses);
        store.write_run(&run)?;

        let mut lines = vec![
            format!("fleet_id={}", run.id),
            format!("fleet_status={}", run.status.label()),
            format!("members={}", run.member_task_ids.len()),
            format!("stopped={stopped}"),
            format!("already_terminal={already_terminal}"),
            format!("stop_errors={}", errors.len()),
        ];
        if !errors.is_empty() {
            lines.extend(errors);
            lines.push("note=some tasks could not be stopped; retry or check process state".into());
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn task_manager(&self) -> Option<TaskManager> {
        self.storage_dir.clone().map(TaskManager::new)
    }

    fn fleet_store(&self) -> Option<FleetStore> {
        self.storage_dir.clone().map(FleetStore::new)
    }

    fn stores(&self) -> Option<(TaskManager, FleetStore)> {
        Some((self.task_manager()?, self.fleet_store()?))
    }

    /// Lists all built-in fleet agent roles.
    fn roles() -> Result<CommandOutput> {
        let catalog = FleetRoleCatalog::builtin();
        let roles = catalog.list();
        let mut lines = vec![format!("fleet_roles={}", roles.len())];
        for (i, role) in roles.iter().enumerate() {
            lines.push(format!("role[{i}].id={}", role.id));
            lines.push(format!("role[{i}].name={}", sanitize_line(&role.name)));
            lines.push(format!(
                "role[{i}].description={}",
                sanitize_line(&role.description)
            ));
            if !role.tags.is_empty() {
                lines.push(format!("role[{i}].tags={}", role.tags.join(",")));
            }
        }
        lines.push("note=pass --role <id> to `fleet start` to apply a role preamble".into());
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

// ── Free helpers ──────────────────────────────────────────────────────────────

/// Dispatches a single pending [`FleetMemberRequest`] to a live agent task.
///
/// On success the pending file is deleted and the new [`TaskId`] is returned.
///
/// If the request carries a `role` id, the role's preamble is prepended to the
/// request prompt before the agent is launched.
fn dispatch_one(
    manager: &TaskManager,
    store: &FleetStore,
    context: &CommandContext,
    req: &FleetMemberRequest,
    report: &ProviderStatusReport,
) -> Result<TaskId> {
    let cwd = req.cwd.clone().unwrap_or_else(|| context.cwd.clone());

    // Apply role preamble when a role id is stored on the request.
    let (prompt, allowed_tools) = if let Some(ref role_id) = req.role {
        let catalog = FleetRoleCatalog::builtin();
        let role = catalog
            .get(role_id)
            .or_else(|| catalog.resolve_alias(role_id));
        let role = role.ok_or_else(|| {
            WonderError::validation(format!(
                "unknown fleet role `{role_id}` in pending request {}; known roles: {}",
                req.id,
                catalog.known_ids_display()
            ))
        })?;
        (
            role.compose_prompt(&req.prompt),
            role_allowed_tools_for_launch(role),
        )
    } else {
        (req.prompt.clone(), None)
    };

    let task = manager.start_agent_task(AgentTaskLaunch {
        name: req
            .name
            .clone()
            .unwrap_or_else(|| request_name_fallback(&req.id)),
        description: req.description.clone(),
        prompt,
        provider: req.provider.clone().or_else(|| report.provider.clone()),
        model: req.model.clone().or_else(|| report.model.clone()),
        cwd,
        fleet_id: req.fleet_id,
        allowed_tools,
    })?;

    store.delete_pending_request(&req.id)?;
    Ok(task.id)
}

/// Links a newly-dispatched task to an existing fleet run by appending to
/// `member_task_ids` and recomputing the run's status.
fn link_task_to_fleet(store: &FleetStore, fleet_id: FleetId, task_id: TaskId) -> Result<()> {
    let mut run = store.read_run(fleet_id)?;
    if !run.member_task_ids.contains(&task_id) {
        run.member_task_ids.push(task_id);
    }
    // At least one member is now running.
    if run.status == FleetRunStatus::Pending {
        run.status = FleetRunStatus::Running;
    }
    store.write_run(&run)?;
    Ok(())
}

fn parse_fleet_id(value: &str) -> Result<FleetId> {
    FleetId::parse(value)
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

fn request_name_fallback(request_id: &str) -> String {
    let prefix: String = request_id.chars().take(8).collect();
    if prefix.is_empty() {
        "agent".into()
    } else {
        prefix
    }
}

fn role_allowed_tools_for_launch(role: &FleetAgentRole) -> Option<Vec<String>> {
    (!role.allowed_tools.is_empty()).then(|| role.allowed_tools.clone())
}

fn sanitize_line(value: &str) -> String {
    value.lines().collect::<Vec<_>>().join("\\n")
}

fn runtime_disabled_message(subcommand: &str) -> String {
    format!(
        "fleet_{}_disabled=true\nnote={} requires --storage-dir",
        subcommand.replace(' ', "_"),
        subcommand
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::Path;

    use wonder_of_u_core::{FeatureSet, FleetRunState, PermissionMode, SessionId};
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn make_fleet_command(dir: &Path) -> FleetCommand {
        FleetCommand::new(Some(dir.to_path_buf()))
    }

    fn stub_context(cwd: PathBuf) -> CommandContext {
        CommandContext {
            session_id: SessionId::new(),
            cwd,
            features: FeatureSet::first_release(),
            authenticated: false,
            interactive: false,
            permission_mode: PermissionMode::Default,
            theme: None,
            session_color: None,
            effort_level: None,
            brief_mode: false,
            optimize_token_mode: false,
            fast_mode: false,
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        }
    }

    fn invocation(args: &str) -> CommandInvocation {
        CommandInvocation {
            name: "fleet".into(),
            args: args.into(),
            raw: format!("/fleet {args}"),
        }
    }

    #[test]
    fn fleet_status_no_storage_returns_disabled() {
        let cmd = FleetCommand::new(None);
        let output = futures::executor::block_on(cmd.execute(
            stub_context(std::env::current_dir().unwrap()),
            invocation("status"),
        ))
        .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("requires --storage-dir"), "got: {text}");
    }

    #[test]
    fn fleet_status_empty_storage_shows_zeros() {
        let dir = unique_test_dir("fleet-cmd-status");
        let cmd = make_fleet_command(&dir);
        let output = futures::executor::block_on(
            cmd.execute(stub_context(dir.clone()), invocation("status")),
        )
        .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("fleet_runs=0"), "got: {text}");
        assert!(text.contains("pending_dispatch_requests=0"), "got: {text}");
    }

    #[test]
    fn fleet_dispatch_no_pending_reports_zero() {
        let dir = unique_test_dir("fleet-cmd-dispatch-empty");
        let cmd = make_fleet_command(&dir);
        let output = futures::executor::block_on(
            cmd.execute(stub_context(dir.clone()), invocation("dispatch")),
        )
        .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("fleet_pending=0"), "got: {text}");
    }

    #[test]
    fn fleet_dispatch_unknown_role_reports_error_without_deleting_request() {
        let dir = unique_test_dir("fleet-cmd-dispatch-bad-role");
        let store = FleetStore::new(&dir);
        let mut request = FleetMemberRequest::new("inspect the repo");
        request.role = Some("missing-role".into());
        let request_id = request.id.clone();
        store.queue_member_request(&request).expect("queue");

        let cmd = make_fleet_command(&dir);
        let output = futures::executor::block_on(
            cmd.execute(stub_context(dir.clone()), invocation("dispatch")),
        )
        .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("fleet_dispatch_errors=1"), "got: {text}");
        assert!(
            text.contains("unknown fleet role `missing-role`"),
            "got: {text}"
        );
        assert!(
            store.read_pending_request(&request_id).is_ok(),
            "failed dispatch should leave pending request for inspection or retry"
        );
    }

    #[test]
    fn fleet_list_shows_stored_runs() {
        let dir = unique_test_dir("fleet-cmd-list");
        let store = FleetStore::new(&dir);
        let run = FleetRunState::new("my fleet", PermissionMode::Default, None);
        let run_id = run.id;
        store.write_run(&run).expect("write");

        let cmd = make_fleet_command(&dir);
        let output =
            futures::executor::block_on(cmd.execute(stub_context(dir.clone()), invocation("list")))
                .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains(&run_id.to_string()), "got: {text}");
        assert!(text.contains("my fleet"), "got: {text}");
    }

    #[test]
    fn fleet_show_missing_returns_error() {
        let dir = unique_test_dir("fleet-cmd-show-missing");
        let store = FleetStore::new(&dir);
        store.ensure_layout().expect("layout");
        let cmd = make_fleet_command(&dir);
        let missing_id = FleetId::new();
        let result = futures::executor::block_on(cmd.execute(
            stub_context(dir.clone()),
            invocation(&format!("show {missing_id}")),
        ));
        // Should propagate a not-found error.
        assert!(result.is_err(), "expected error for missing fleet run");
    }

    #[test]
    fn fleet_command_spec_requires_fleet_feature() {
        let spec = FleetCommand::command_spec();
        assert!(spec.required_features.contains(&FeatureFlag::Fleet));
        assert!(spec.required_features.contains(&FeatureFlag::Agents));
    }

    #[test]
    fn request_name_fallback_handles_short_or_empty_ids() {
        assert_eq!(request_name_fallback("abcdef123456"), "abcdef12");
        assert_eq!(request_name_fallback("abc"), "abc");
        assert_eq!(request_name_fallback(""), "agent");
    }

    #[test]
    fn role_allowed_tools_for_launch_uses_builtin_tool_names() {
        let catalog = FleetRoleCatalog::builtin();
        let role = catalog.get("rust-architect").expect("role");
        let tools = role_allowed_tools_for_launch(role).expect("allowed tools");

        assert!(tools.contains(&"bash".to_string()));
        assert!(tools.contains(&"file_read".to_string()));
        assert!(tools.contains(&"glob".to_string()));
        assert!(tools.contains(&"grep".to_string()));
        assert!(!tools.contains(&"file_write".to_string()));
    }

    // ── Role catalog CLI tests ────────────────────────────────────────────────

    #[test]
    fn fleet_roles_lists_builtin_role_ids() {
        let cmd = FleetCommand::new(None);
        let output = futures::executor::block_on(cmd.execute(
            stub_context(std::env::current_dir().unwrap()),
            invocation("roles"),
        ))
        .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };

        // All nine built-in ids must appear.
        for id in &[
            "rust-engineer",
            "rust-tester",
            "rust-architect",
            "rust-refactor",
            "rust-optimizer",
            "rust-documenter",
            "tui-designer",
            "rubber-duck",
            "code-reviewer",
        ] {
            assert!(text.contains(id), "missing role id `{id}`; got: {text}");
        }
        assert!(
            text.contains("fleet_roles=9"),
            "expected 9 roles; got: {text}"
        );
    }

    #[test]
    fn fleet_roles_output_is_stable_order() {
        let cmd = FleetCommand::new(None);
        let output = futures::executor::block_on(cmd.execute(
            stub_context(std::env::current_dir().unwrap()),
            invocation("roles"),
        ))
        .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            _ => panic!("expected text"),
        };

        // code-reviewer (c) must appear before rust-engineer (r) in alphabetical order.
        let pos_code = text.find("code-reviewer").expect("code-reviewer");
        let pos_rust = text.find("rust-engineer").expect("rust-engineer");
        assert!(
            pos_code < pos_rust,
            "expected alphabetical order; code-reviewer should precede rust-engineer"
        );
    }

    #[test]
    fn fleet_start_unknown_role_returns_error() {
        let dir = unique_test_dir("fleet-cmd-start-bad-role");
        let cmd = make_fleet_command(&dir);
        let result = futures::executor::block_on(cmd.execute(
            stub_context(dir.clone()),
            invocation("start --description test --role totally-unknown-role"),
        ));
        let err = result.expect_err("expected error for unknown role");
        assert!(err.to_string().contains("unknown fleet role"), "got: {err}");
    }

    #[test]
    fn fleet_start_with_role_includes_role_in_output() {
        let dir = unique_test_dir("fleet-cmd-start-with-role");
        let cmd = make_fleet_command(&dir);
        // No --prompt means no actual task launch (no process spawned).
        let output = futures::executor::block_on(cmd.execute(
            stub_context(dir.clone()),
            invocation("start --description \"fix auth\" --role rust-engineer"),
        ))
        .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("role=rust-engineer"), "got: {text}");
        assert!(text.contains("role_name=Rust Engineer"), "got: {text}");
    }
}
