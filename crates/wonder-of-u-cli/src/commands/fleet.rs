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
use time::OffsetDateTime;
use wonder_of_u_agent::{ProviderResolver, ProviderStatusReport};
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec,
    FeatureFlag, FleetAgentRole, FleetId, FleetMemberRequest, FleetRoleCatalog, FleetRunState,
    FleetRunStatus, Result, TaskId, TaskStatus, WonderError, WorktreeIsolation,
    WorktreeIsolationMode, get_git_root,
};
use wonder_of_u_storage::FleetStore;
use wonder_of_u_tools::{
    create_fleet_agent_worktree_with_branch, fleet_agent_worktree_slug,
    validate_worktree_branch_name,
};

use super::{
    fleet_plan::parse_plan,
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

// `FleetStartArgs` is inherently large (many CLI fields); boxing it is less
// readable than this one-time allow on the top-level dispatch enum.
#[allow(clippy::large_enum_variant)]
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
    /// Launch ready members of a plan-based fleet run, respecting dependencies.
    Reconcile(FleetReconcileArgs),
}

#[derive(Debug, Args)]
struct FleetStartArgs {
    /// Human-readable description of what this fleet run will accomplish.
    #[arg(long)]
    description: String,
    /// Agent prompt (used when `--member-name` is provided to create an
    /// immediate single-member fleet).
    ///
    /// Conflicts with `--plan`.
    #[arg(long, conflicts_with = "plan")]
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
    /// Path to a JSON plan file (array of member specs) to queue as a batch.
    ///
    /// Conflicts with `--prompt`. Use `fleet reconcile <fleet_id>` to launch
    /// members after creating the run.
    #[arg(long, conflicts_with = "prompt")]
    plan: Option<PathBuf>,
    /// Maximum number of member tasks allowed to run concurrently.
    ///
    /// Only meaningful with `--plan`. Must be > 0.
    #[arg(long)]
    max_concurrency: Option<usize>,
    /// Enable worktree isolation for the immediate single-member agent task.
    ///
    /// Accepted value: `worktree`. Conflicts with `--plan`.
    /// Setting `--worktree-branch` implies `--isolation worktree`.
    #[arg(long, conflicts_with = "plan")]
    isolation: Option<String>,
    /// Explicit git branch name to use for the worktree.
    ///
    /// Implies `--isolation worktree`. Conflicts with `--plan`.
    #[arg(long, conflicts_with = "plan")]
    worktree_branch: Option<String>,
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

#[derive(Debug, Args)]
struct FleetReconcileArgs {
    /// The fleet run id to reconcile.
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
            FleetSubcommand::Reconcile(args) => self.reconcile(context, args),
        }
    }
}

// ── Sub-command implementations ───────────────────────────────────────────────

impl FleetCommand {
    /// Drains all pending `FleetMemberRequest` files into live agent tasks.
    ///
    /// Processes requests in queue order (oldest first).  A per-request
    /// failure is recorded but does **not** abort remaining requests.
    ///
    /// Requests with a non-empty `depends_on` are skipped with a warning;
    /// those must be launched through `fleet reconcile <fleet_id>`.
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
        let mut skipped_deps = 0usize;
        let mut errors: Vec<String> = Vec::new();

        for req in &pending {
            // Skip requests that belong to a plan and carry dependency
            // constraints; those must go through `fleet reconcile`.
            if !req.depends_on.is_empty() {
                skipped_deps += 1;
                errors.push(format!(
                    "skipped[{}]: has depends_on — use `fleet reconcile {}` instead",
                    req.id,
                    req.fleet_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "<fleet_id>".into())
                ));
                continue;
            }

            match dispatch_one(&manager, &store, &context, req, &report) {
                Ok(task_id) => {
                    dispatched += 1;
                    // Best-effort: if the request belongs to a fleet run, link
                    // the new task to that run.
                    if let Some(fleet_id) = req.fleet_id {
                        let _ = link_task_to_fleet(&store, fleet_id, &req.id, task_id);
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
            format!("fleet_skipped_deps={skipped_deps}"),
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

        // ── Plan-based start ──────────────────────────────────────────────────
        if let Some(plan_path) = args.plan {
            return self.start_plan(
                context,
                &store,
                &args.description,
                &plan_path,
                args.max_concurrency,
            );
        }

        // Validate max_concurrency when present (even without --plan it's a user error).
        if let Some(mc) = args.max_concurrency {
            if mc == 0 {
                return Err(WonderError::validation(
                    "--max-concurrency must be greater than 0",
                ));
            }
        }

        // ── Validate and resolve isolation mode ───────────────────────────────
        // `--worktree-branch` implies `--isolation worktree`.
        let isolation: Option<WorktreeIsolation> =
            parse_isolation_args(args.isolation.as_deref(), args.worktree_branch.as_deref())?;

        // ── Single-member / no-member start (original behaviour) ─────────────

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

        // If --prompt is given, launch an immediate agent task.
        if let Some(prompt) = &args.prompt {
            let report = ProviderResolver::builtin().load_report(self.storage_dir.as_deref())?;
            let cwd = args.cwd.clone().unwrap_or_else(|| context.cwd.clone());

            // Compose full prompt: prepend role preamble when a role is set.
            let full_prompt = if let Some(ref role) = resolved_role {
                role.compose_prompt(prompt)
            } else {
                prompt.clone()
            };

            // Apply worktree isolation if requested.
            let (effective_cwd, worktree_branch) = if let Some(ref iso) = isolation {
                resolve_worktree_cwd(&cwd, iso, &run.id.to_string(), &run.id.to_string())?
            } else {
                (cwd, None)
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
                cwd: effective_cwd,
                fleet_id: Some(run.id),
                fleet_request_id: None,
                parent_task_id: None,
                allowed_tools: resolved_role
                    .as_ref()
                    .and_then(role_allowed_tools_for_launch),
                worktree_branch,
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
        if let Some(ref iso) = isolation {
            lines.push(format!("isolation_mode={}", isolation_mode_label(iso.mode)));
        }
        for (i, task_id) in run.member_task_ids.iter().enumerate() {
            lines.push(format!("member[{i}].task_id={task_id}"));
        }
        lines.push("note=use `fleet dispatch` to launch queued pending requests".into());
        Ok(CommandOutput::Text(lines.join("\n")))
    }

    /// Handles `fleet start --plan <path>`: parse the plan, queue all members
    /// as [`FleetMemberRequest`]s, and write the [`FleetRunState`].
    ///
    /// Does **not** launch any tasks; use `fleet reconcile <fleet_id>`.
    fn start_plan(
        &self,
        context: CommandContext,
        store: &FleetStore,
        description: &str,
        plan_path: &std::path::Path,
        max_concurrency: Option<usize>,
    ) -> Result<CommandOutput> {
        if let Some(mc) = max_concurrency {
            if mc == 0 {
                return Err(WonderError::validation(
                    "--max-concurrency must be greater than 0",
                ));
            }
        }

        let json = std::fs::read_to_string(plan_path).map_err(|e| {
            WonderError::validation(format!(
                "cannot read plan file `{}`: {e}",
                plan_path.display()
            ))
        })?;
        let specs = parse_plan(&json)?;

        let mut run = FleetRunState::new(
            description,
            context.permission_mode,
            Some(context.cwd.clone()),
        );
        run.max_concurrency = max_concurrency;
        store.write_run(&run)?;

        // Queue all members as pending requests in topological order.
        for spec in &specs {
            let mut req = FleetMemberRequest::new(spec.prompt.clone());
            req.id = spec.id.clone(); // use the plan id as the request id
            req.fleet_id = Some(run.id);
            req.name = spec.name.clone();
            req.description = spec.description.clone();
            req.role = spec.role.clone();
            req.model = spec.model.clone();
            req.provider = spec.provider.clone();
            req.cwd = spec.cwd.clone();
            req.depends_on = spec.depends_on.clone();
            req.isolation = spec.isolation.clone();
            store.queue_member_request(&req)?;
        }

        let mut lines = vec![
            format!("fleet_id={}", run.id),
            format!("description={}", sanitize_line(description)),
            format!("status={}", run.status.label()),
            format!("members={}", specs.len()),
        ];
        if let Some(mc) = max_concurrency {
            lines.push(format!("max_concurrency={mc}"));
        }
        for (i, spec) in specs.iter().enumerate() {
            lines.push(format!("member[{i}].id={}", spec.id));
            if !spec.depends_on.is_empty() {
                lines.push(format!(
                    "member[{i}].depends_on={}",
                    spec.depends_on.join(",")
                ));
            }
            if spec.isolation.is_some() {
                lines.push(format!("member[{i}].isolation=worktree"));
            }
        }
        lines.push(format!(
            "note=run `fleet reconcile {}` to launch ready members",
            run.id
        ));
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
                    if let Some(ref cwd) = task.cwd {
                        lines.push(format!("member[{i}].cwd={}", cwd.display()));
                    }
                    if let Some(ref branch) = task.worktree_branch {
                        lines.push(format!(
                            "member[{i}].worktree_branch={}",
                            sanitize_line(branch)
                        ));
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

    /// Launches ready members of a plan-based fleet run, respecting deps.
    ///
    /// This is a **stateless re-entrant** operation: calling it multiple times
    /// is safe and idempotent — already-dispatched members (tracked in
    /// [`FleetRunState::dispatched_requests`]) are never re-launched.
    ///
    /// # Classification
    ///
    /// For each pending request belonging to `fleet_id`, the reconciler
    /// classifies it as:
    ///
    /// - **ready** — all `depends_on` ids have been dispatched and their tasks
    ///   are in [`TaskStatus::Completed`].
    /// - **waiting** — all deps have been dispatched but at least one is still
    ///   non-terminal (pending/running).
    /// - **blocked** — at least one dep's task is terminal but not
    ///   [`TaskStatus::Completed`] (failed/killed/cancelled).
    ///
    /// Ready requests are launched up to the `max_concurrency` cap.
    fn reconcile(
        &self,
        context: CommandContext,
        args: FleetReconcileArgs,
    ) -> Result<CommandOutput> {
        let (manager, store) = match self.stores() {
            Some(pair) => pair,
            None => {
                return Ok(CommandOutput::Text(runtime_disabled_message(
                    "fleet reconcile",
                )));
            }
        };

        let fleet_id = parse_fleet_id(&args.fleet_id)?;
        let mut run = store.read_run(fleet_id)?;

        // Load all pending requests that belong to this fleet.
        let all_pending = store.list_pending_requests()?;
        let pending: Vec<_> = all_pending
            .iter()
            .filter(|r| r.fleet_id == Some(fleet_id))
            .collect();

        // Count active (non-terminal) member tasks for concurrency cap.
        let active_count = run
            .member_task_ids
            .iter()
            .filter(|&&tid| {
                manager
                    .get_task(tid)
                    .map(|t| !t.status.is_terminal())
                    .unwrap_or(false)
            })
            .count();

        let report = ProviderResolver::builtin().load_report(self.storage_dir.as_deref())?;

        let mut newly_launched = 0usize;
        let mut ready_count = 0usize;
        let mut waiting_count = 0usize;
        let mut blocked_count = 0usize;
        let mut launch_errors: Vec<String> = Vec::new();

        // Compute remaining concurrency slots (None = unbounded).
        let mut slots_remaining: Option<usize> = run
            .max_concurrency
            .map(|cap| cap.saturating_sub(active_count));

        for req in &pending {
            // Skip already-dispatched (idempotency guard).
            if run.dispatched_requests.contains_key(&req.id) {
                continue;
            }

            // Classify the request based on its dependency statuses.
            let classification = classify_request(req, &run, &manager);

            match classification {
                DepClassification::Ready => {
                    ready_count += 1;
                    // Honour max_concurrency cap.
                    if slots_remaining == Some(0) {
                        // No slots left this pass; leave as ready for next call.
                        continue;
                    }

                    match dispatch_one(&manager, &store, &context, req, &report) {
                        Ok(task_id) => {
                            newly_launched += 1;
                            run.record_dispatch(req.id.clone(), task_id);
                            if run.status == FleetRunStatus::Pending {
                                run.status = FleetRunStatus::Running;
                            }
                            if let Some(ref mut slots) = slots_remaining {
                                *slots = slots.saturating_sub(1);
                            }
                        }
                        Err(err) => {
                            launch_errors.push(format!(
                                "launch_error[{}]: {}",
                                req.id,
                                sanitize_line(&err.to_string())
                            ));
                        }
                    }
                }
                DepClassification::Waiting => waiting_count += 1,
                DepClassification::Blocked => blocked_count += 1,
            }
        }

        // Recompute completed/failed counts from member tasks.
        let mut completed_count = 0usize;
        let mut failed_count = 0usize;
        let mut active_after = 0usize;
        let mut member_statuses: Vec<TaskStatus> = Vec::new();
        for &tid in &run.member_task_ids {
            match manager.get_task(tid) {
                Ok(t) => {
                    member_statuses.push(t.status);
                    if t.status.is_terminal() {
                        if t.status == TaskStatus::Completed {
                            completed_count += 1;
                        } else {
                            failed_count += 1;
                        }
                    } else {
                        active_after += 1;
                    }
                }
                Err(_) => member_statuses.push(TaskStatus::Pending),
            }
        }

        // Determine new fleet status.
        //
        // Pending-queue items that are still waiting/ready/unblocked count as
        // "work remaining" and keep the fleet Running rather than prematurely
        // marking it terminal.
        let remaining_work = pending
            .iter()
            .filter(|r| !run.dispatched_requests.contains_key(&r.id))
            .count();

        if blocked_count > 0 && active_after == 0 && remaining_work == blocked_count {
            // All remaining work is blocked and nothing is active → Failed.
            run.status = FleetRunStatus::Failed;
            if run.finished_at.is_none() {
                run.finished_at = Some(OffsetDateTime::now_utc());
            }
        } else if active_after > 0 || remaining_work > 0 {
            run.status = FleetRunStatus::Running;
        } else if !member_statuses.is_empty() {
            // All dispatched, all terminal.
            run.reconcile_status(&member_statuses);
        }

        store.write_run(&run)?;

        let mut lines = vec![
            format!("fleet_id={}", run.id),
            format!("fleet_status={}", run.status.label()),
            format!("active={active_after}"),
            format!("completed={completed_count}"),
            format!("failed={failed_count}"),
            format!("ready={ready_count}"),
            format!("waiting={waiting_count}"),
            format!("blocked={blocked_count}"),
            format!("newly_launched={newly_launched}"),
            format!("launch_errors={}", launch_errors.len()),
        ];
        lines.extend(launch_errors);
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
///
/// If the request carries an `isolation` field with mode `Worktree`, a git
/// worktree is created (or resumed) before launching the agent.  A failure to
/// create the worktree returns an error and leaves the pending request intact.
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
        // Fall back to per-request allowed_tools (set by the `agent` tool when
        // the caller passes an explicit `tools` list).
        (req.prompt.clone(), req.allowed_tools.clone())
    };

    // Apply worktree isolation if requested.
    let fleet_id_str = req
        .fleet_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| req.id.clone());
    let (effective_cwd, worktree_branch) = if let Some(ref iso) = req.isolation {
        resolve_worktree_cwd(&cwd, iso, &fleet_id_str, &req.id)?
    } else {
        (cwd, None)
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
        cwd: effective_cwd,
        fleet_id: req.fleet_id,
        fleet_request_id: Some(req.id.clone()),
        parent_task_id: req.parent_task_id,
        allowed_tools,
        worktree_branch,
    })?;

    store.delete_pending_request_for(req)?;
    Ok(task.id)
}

/// Links a newly-dispatched task to an existing fleet run.
///
/// Records the `request_id → task_id` mapping via [`FleetRunState::record_dispatch`],
/// which also appends to `member_task_ids` (deduped).  Marks the run as at
/// least `Running` if it was still `Pending`.
fn link_task_to_fleet(
    store: &FleetStore,
    fleet_id: FleetId,
    request_id: &str,
    task_id: TaskId,
) -> Result<()> {
    let mut run = store.read_run(fleet_id)?;
    run.record_dispatch(request_id.to_owned(), task_id);
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

/// How a pending request's dependencies are classified during reconcile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DepClassification {
    /// All deps dispatched and completed → may launch now.
    Ready,
    /// All deps dispatched but at least one is still non-terminal.
    Waiting,
    /// At least one dep's task is terminal but not Completed.
    Blocked,
}

/// Classifies a pending request's dependency state.
fn classify_request(
    req: &FleetMemberRequest,
    run: &FleetRunState,
    manager: &TaskManager,
) -> DepClassification {
    if req.depends_on.is_empty() {
        return DepClassification::Ready;
    }

    let mut all_completed = true;
    for dep_id in &req.depends_on {
        match run.dispatched_requests.get(dep_id) {
            None => {
                // Dependency not yet dispatched → waiting (can't be blocked).
                return DepClassification::Waiting;
            }
            Some(&dep_task_id) => match manager.get_task(dep_task_id) {
                Ok(task) => {
                    if task.status == TaskStatus::Completed {
                        // This dep is done; continue checking the rest.
                    } else if task.status.is_terminal() {
                        // Terminal but not Completed → blocked.
                        return DepClassification::Blocked;
                    } else {
                        // Still running/pending → waiting.
                        all_completed = false;
                    }
                }
                Err(_) => {
                    // Can't read the task; treat conservatively as waiting.
                    all_completed = false;
                }
            },
        }
    }

    if all_completed {
        DepClassification::Ready
    } else {
        DepClassification::Waiting
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

/// Parses the `--isolation` / `--worktree-branch` CLI args into a
/// [`WorktreeIsolation`] value, or returns an error for unknown mode strings.
///
/// `--worktree-branch` alone implies `worktree` mode.
fn parse_isolation_args(
    isolation: Option<&str>,
    worktree_branch: Option<&str>,
) -> Result<Option<WorktreeIsolation>> {
    // `--worktree-branch` implies worktree mode.
    if worktree_branch.is_some() && isolation.is_none() {
        let branch = worktree_branch.map(str::to_string);
        if let Some(ref b) = branch {
            validate_worktree_branch_name(b)?;
        }
        return Ok(Some(WorktreeIsolation {
            mode: WorktreeIsolationMode::Worktree,
            branch,
        }));
    }

    let Some(mode_str) = isolation else {
        return Ok(None);
    };

    match mode_str {
        "worktree" => {
            let branch = worktree_branch.map(str::to_string);
            if let Some(ref b) = branch {
                validate_worktree_branch_name(b)?;
            }
            Ok(Some(WorktreeIsolation {
                mode: WorktreeIsolationMode::Worktree,
                branch,
            }))
        }
        other => Err(WonderError::validation(format!(
            "unknown isolation mode `{other}`; accepted: worktree"
        ))),
    }
}

fn isolation_mode_label(mode: WorktreeIsolationMode) -> &'static str {
    match mode {
        WorktreeIsolationMode::Worktree => "worktree",
    }
}

/// Resolves the effective cwd and worktree branch for an isolated agent.
///
/// Verifies the cwd is inside a git repo, derives a stable member slug for the
/// worktree path, and delegates worktree creation to the tools crate. Explicit
/// branch names are passed as branch overrides, not as path slugs.
fn resolve_worktree_cwd(
    base_cwd: &std::path::Path,
    iso: &WorktreeIsolation,
    fleet_id_str: &str,
    request_id: &str,
) -> Result<(PathBuf, Option<String>)> {
    let repository_root = get_git_root(base_cwd).map_err(|_| {
        WonderError::validation(format!(
            "worktree isolation requires a git repository; `{}` is not inside one",
            base_cwd.display()
        ))
    })?;

    let slug = fleet_agent_worktree_slug(fleet_id_str, request_id);
    let (worktree_path, branch) = create_fleet_agent_worktree_with_branch(
        base_cwd,
        &repository_root,
        &slug,
        iso.branch.as_deref(),
    )?;
    Ok((worktree_path, Some(branch)))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::Path;

    use wonder_of_u_core::{
        FeatureSet, FleetId, FleetMemberRequest, FleetRunState, FleetRunStatus, PermissionMode,
        SessionId, WorktreeIsolationMode,
    };
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

    #[test]
    fn fleet_start_isolation_mode_output_is_stable_snake_case() {
        let dir = unique_test_dir("fleet-cmd-start-isolation-output");
        let cmd = make_fleet_command(&dir);
        // No --prompt means no task launch and no worktree creation.
        let output = futures::executor::block_on(cmd.execute(
            stub_context(dir.clone()),
            invocation("start --description isolated --isolation worktree"),
        ))
        .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("isolation_mode=worktree"), "got: {text}");
        assert!(
            !text.contains("isolation_mode=worktreeisolationmode"),
            "got: {text}"
        );
    }

    // ── fleet start --plan tests ─────────────────────────────────────────────

    fn write_plan(dir: &std::path::Path, name: &str, json: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, json).expect("write plan");
        path
    }

    #[test]
    fn fleet_start_plan_queues_members_and_writes_run() {
        let dir = unique_test_dir("fleet-start-plan");
        let plan_json = serde_json::json!([
            {"id": "step-a", "prompt": "do a"},
            {"id": "step-b", "prompt": "do b", "depends_on": ["step-a"]}
        ])
        .to_string();
        let plan_path = write_plan(&dir, "plan.json", &plan_json);

        let cmd = make_fleet_command(&dir);
        let output = futures::executor::block_on(cmd.execute(
            stub_context(dir.clone()),
            invocation(&format!(
                "start --description \"two-step plan\" --plan {} --max-concurrency 1",
                plan_path.display()
            )),
        ))
        .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("members=2"), "got: {text}");
        assert!(text.contains("max_concurrency=1"), "got: {text}");
        assert!(text.contains("step-a"), "got: {text}");
        assert!(text.contains("step-b"), "got: {text}");
        assert!(text.contains("fleet reconcile"), "got: {text}");

        // Both pending requests must be stored.
        let store = FleetStore::new(&dir);
        let pending = store.list_pending_requests().expect("list");
        assert_eq!(pending.len(), 2);
        let ids: Vec<&str> = pending.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"step-a"), "step-a missing from pending");
        assert!(ids.contains(&"step-b"), "step-b missing from pending");

        // step-b must carry depends_on.
        let step_b = pending.iter().find(|r| r.id == "step-b").unwrap();
        assert_eq!(step_b.depends_on, vec!["step-a"]);
    }

    #[test]
    fn fleet_start_plan_cycle_is_rejected_without_writing() {
        let dir = unique_test_dir("fleet-start-plan-cycle");
        let plan_json = serde_json::json!([
            {"id": "a", "prompt": "x", "depends_on": ["b"]},
            {"id": "b", "prompt": "y", "depends_on": ["a"]}
        ])
        .to_string();
        let plan_path = write_plan(&dir, "cycle.json", &plan_json);

        let cmd = make_fleet_command(&dir);
        let result = futures::executor::block_on(cmd.execute(
            stub_context(dir.clone()),
            invocation(&format!(
                "start --description cycle --plan {}",
                plan_path.display()
            )),
        ));

        let err = result.expect_err("should reject cycle");
        assert!(err.to_string().contains("cycle"), "got: {err}");

        // No run must have been persisted.
        let store = FleetStore::new(&dir);
        let runs = store.list_runs().expect("list");
        assert!(
            runs.is_empty(),
            "no run should be stored for a rejected plan"
        );
    }

    #[test]
    fn fleet_start_plan_zero_max_concurrency_is_rejected() {
        let dir = unique_test_dir("fleet-start-mc-zero");
        let plan_json = r#"[{"id":"step-a","prompt":"do a"}]"#;
        let plan_path = write_plan(&dir, "plan.json", plan_json);

        let cmd = make_fleet_command(&dir);
        let result = futures::executor::block_on(cmd.execute(
            stub_context(dir.clone()),
            invocation(&format!(
                "start --description x --plan {} --max-concurrency 0",
                plan_path.display()
            )),
        ));
        let err = result.expect_err("should reject max-concurrency=0");
        assert!(err.to_string().contains("greater than 0"), "got: {err}");
    }

    // ── fleet dispatch skips dep-constrained requests ─────────────────────────

    #[test]
    fn fleet_dispatch_skips_dep_constrained_requests() {
        let dir = unique_test_dir("fleet-dispatch-skip-deps");
        let store = FleetStore::new(&dir);

        // Create a fleet run so the request has a valid fleet_id to link to.
        let run = FleetRunState::new("test fleet", PermissionMode::Default, None);
        let fleet_id = run.id;
        store.write_run(&run).expect("write run");

        // Queue a request that has depends_on set.
        let mut req = FleetMemberRequest::new("do the thing");
        req.fleet_id = Some(fleet_id);
        req.depends_on = vec!["some-other-step".into()];
        store.queue_member_request(&req).expect("queue");

        let cmd = make_fleet_command(&dir);
        let output = futures::executor::block_on(
            cmd.execute(stub_context(dir.clone()), invocation("dispatch")),
        )
        .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("fleet_skipped_deps=1"), "got: {text}");
        assert!(text.contains("fleet reconcile"), "got: {text}");
        // The request must still be pending (not deleted).
        assert!(
            store.read_pending_request_for(&req).is_ok(),
            "dep-constrained request should remain pending"
        );
    }

    // ── fleet reconcile tests ─────────────────────────────────────────────────

    fn make_run_with_pending(
        dir: &std::path::Path,
        specs: &[(&str, &str, Vec<&str>)], // (id, prompt, depends_on)
        max_concurrency: Option<usize>,
    ) -> (FleetRunState, FleetStore) {
        let store = FleetStore::new(dir);
        let mut run = FleetRunState::new("test-fleet", PermissionMode::Default, None);
        run.max_concurrency = max_concurrency;
        for (id, prompt, deps) in specs {
            let mut req = FleetMemberRequest::new(*prompt);
            req.id = id.to_string();
            req.fleet_id = Some(run.id);
            req.depends_on = deps.iter().map(|s| s.to_string()).collect();
            store.queue_member_request(&req).expect("queue");
        }
        store.write_run(&run).expect("write run");
        (run, store)
    }

    fn fleet_pending_request(fleet_id: FleetId, request_id: &str) -> FleetMemberRequest {
        let mut request = FleetMemberRequest::new("placeholder");
        request.id = request_id.to_string();
        request.fleet_id = Some(fleet_id);
        request
    }

    #[test]
    fn fleet_reconcile_no_storage_returns_disabled() {
        let cmd = FleetCommand::new(None);
        let output = futures::executor::block_on(cmd.execute(
            stub_context(std::env::current_dir().unwrap()),
            invocation("reconcile 00000000-0000-0000-0000-000000000000"),
        ))
        .expect("execute");
        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("requires --storage-dir"), "got: {text}");
    }

    #[test]
    fn fleet_reconcile_missing_fleet_id_returns_error() {
        let dir = unique_test_dir("fleet-reconcile-missing");
        FleetStore::new(&dir).ensure_layout().expect("layout");
        let cmd = make_fleet_command(&dir);
        let result = futures::executor::block_on(cmd.execute(
            stub_context(dir.clone()),
            invocation(&format!("reconcile {}", FleetId::new())),
        ));
        assert!(result.is_err(), "expected error for missing fleet run");
    }

    #[test]
    fn fleet_reconcile_roots_are_launched_without_deps() {
        // Use the WONDER_OF_U_CLI_BIN override to point at a known-good script
        // that exits 0 immediately so start_agent_task can proceed.
        let dir = unique_test_dir("fleet-reconcile-roots");

        // Write a tiny shell script that acts as the "CLI binary".
        let fake_bin = dir.join("fake-wonder");
        std::fs::write(&fake_bin, "#!/bin/sh\nsleep 0\nexit 0\n").expect("write fake bin");
        // Make it executable.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake_bin, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }

        let (run, _store) = make_run_with_pending(
            &dir,
            &[("root-a", "do a", vec![]), ("root-b", "do b", vec![])],
            None,
        );
        let fleet_id = run.id.to_string();

        // Set the CLI bin override so agent tasks spawn our no-op script.
        // SAFETY: test-only env mutation; single-threaded via --test-threads=1.
        unsafe { std::env::set_var("WONDER_OF_U_CLI_BIN", &fake_bin) };

        let cmd = make_fleet_command(&dir);
        let output = futures::executor::block_on(cmd.execute(
            stub_context(dir.clone()),
            invocation(&format!("reconcile {fleet_id}")),
        ))
        .expect("execute");

        // SAFETY: test-only env mutation; single-threaded via --test-threads=1.
        unsafe { std::env::remove_var("WONDER_OF_U_CLI_BIN") };

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("newly_launched=2"), "got: {text}");
    }

    #[test]
    fn fleet_reconcile_waits_for_running_dep() {
        use wonder_of_u_core::{TaskId, TaskStatus};
        use wonder_of_u_storage::TaskStore;

        let dir = unique_test_dir("fleet-reconcile-wait-running");
        let (run, store) = make_run_with_pending(
            &dir,
            &[
                ("root", "do root", vec![]),
                ("follower", "do after", vec!["root"]),
            ],
            None,
        );

        // Simulate root already dispatched as a running task.
        let fake_task_id = TaskId::new();
        let mut run2 = store.read_run(run.id).expect("read");
        run2.record_dispatch("root".into(), fake_task_id);
        run2.status = FleetRunStatus::Running;
        store.write_run(&run2).expect("write");

        // Write a fake task record with Running status + current PID so that
        // TaskManager::get_task (which reconciles) sees a live process and
        // keeps the task in Running rather than marking it Killed.
        {
            let task_store = TaskStore::new(&dir);
            let mut task = wonder_of_u_core::TaskState::pending_agent(
                "root".to_string(),
                wonder_of_u_core::AgentTaskState::prompt_subprocess(
                    "root".to_string(),
                    "do root".to_string(),
                    None,
                    None,
                ),
            );
            task.id = fake_task_id;
            // Use the test process's own PID — process_is_alive(pid) returns true,
            // so the reconciler keeps the task Running.
            task.pid = Some(std::process::id());
            task.status = TaskStatus::Running;
            task_store.write_task(&task).expect("write task");
        }

        // Also delete the root pending request (it was "dispatched").
        store
            .delete_pending_request_for(&fleet_pending_request(run.id, "root"))
            .expect("delete root");

        let cmd = make_fleet_command(&dir);
        let output = futures::executor::block_on(cmd.execute(
            stub_context(dir.clone()),
            invocation(&format!("reconcile {}", run.id)),
        ))
        .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        // follower can't launch yet: dep is running.
        assert!(text.contains("newly_launched=0"), "got: {text}");
        assert!(text.contains("waiting=1"), "got: {text}");
    }

    #[test]
    fn fleet_reconcile_launches_after_completed_dep() {
        use wonder_of_u_core::{TaskId, TaskStatus};
        use wonder_of_u_storage::TaskStore;

        let dir = unique_test_dir("fleet-reconcile-after-completed");

        // Write a fake CLI script so agent tasks don't fail.
        let fake_bin = dir.join("fake-wonder");
        std::fs::write(&fake_bin, "#!/bin/sh\nsleep 0\nexit 0\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake_bin, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }

        let (run, store) =
            make_run_with_pending(&dir, &[("follower", "do after", vec!["root"])], None);

        // Simulate root dispatched and completed.
        let fake_task_id = TaskId::new();
        let mut run2 = store.read_run(run.id).expect("read");
        run2.record_dispatch("root".into(), fake_task_id);
        store.write_run(&run2).expect("write");

        {
            let task_store = TaskStore::new(&dir);
            let mut task = wonder_of_u_core::TaskState::pending_agent(
                "root".to_string(),
                wonder_of_u_core::AgentTaskState::prompt_subprocess(
                    "root".to_string(),
                    "do root".to_string(),
                    None,
                    None,
                ),
            );
            task.id = fake_task_id;
            task.status = TaskStatus::Completed;
            task_store.write_task(&task).expect("write task");
        }

        // SAFETY: test-only env mutation; single-threaded via --test-threads=1.
        unsafe { std::env::set_var("WONDER_OF_U_CLI_BIN", &fake_bin) };

        let cmd = make_fleet_command(&dir);
        let output = futures::executor::block_on(cmd.execute(
            stub_context(dir.clone()),
            invocation(&format!("reconcile {}", run.id)),
        ))
        .expect("execute");

        // SAFETY: test-only env mutation; single-threaded via --test-threads=1.
        unsafe { std::env::remove_var("WONDER_OF_U_CLI_BIN") };

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("newly_launched=1"), "got: {text}");
    }

    #[test]
    fn fleet_reconcile_blocks_on_failed_dep() {
        use wonder_of_u_core::{TaskId, TaskStatus};
        use wonder_of_u_storage::TaskStore;

        let dir = unique_test_dir("fleet-reconcile-blocked-failed");
        let (run, store) =
            make_run_with_pending(&dir, &[("follower", "do after", vec!["root"])], None);

        // Simulate root dispatched but failed.
        let fake_task_id = TaskId::new();
        let mut run2 = store.read_run(run.id).expect("read");
        run2.record_dispatch("root".into(), fake_task_id);
        store.write_run(&run2).expect("write");

        {
            let task_store = TaskStore::new(&dir);
            let mut task = wonder_of_u_core::TaskState::pending_agent(
                "root".to_string(),
                wonder_of_u_core::AgentTaskState::prompt_subprocess(
                    "root".to_string(),
                    "do root".to_string(),
                    None,
                    None,
                ),
            );
            task.id = fake_task_id;
            task.status = TaskStatus::Failed;
            task_store.write_task(&task).expect("write task");
        }

        let cmd = make_fleet_command(&dir);
        let output = futures::executor::block_on(cmd.execute(
            stub_context(dir.clone()),
            invocation(&format!("reconcile {}", run.id)),
        ))
        .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("blocked=1"), "got: {text}");
        assert!(text.contains("newly_launched=0"), "got: {text}");
        assert!(text.contains("fleet_status=failed"), "got: {text}");
    }

    #[test]
    fn fleet_reconcile_respects_max_concurrency() {
        let dir = unique_test_dir("fleet-reconcile-max-concurrency");

        let fake_bin = dir.join("fake-wonder");
        std::fs::write(&fake_bin, "#!/bin/sh\nsleep 0\nexit 0\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake_bin, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }

        // 3 root members (no deps), max_concurrency=1.
        let (run, _store) = make_run_with_pending(
            &dir,
            &[
                ("a", "do a", vec![]),
                ("b", "do b", vec![]),
                ("c", "do c", vec![]),
            ],
            Some(1),
        );

        // SAFETY: test-only env mutation; single-threaded via --test-threads=1.
        unsafe { std::env::set_var("WONDER_OF_U_CLI_BIN", &fake_bin) };

        let cmd = make_fleet_command(&dir);
        let output = futures::executor::block_on(cmd.execute(
            stub_context(dir.clone()),
            invocation(&format!("reconcile {}", run.id)),
        ))
        .expect("execute");

        // SAFETY: test-only env mutation; single-threaded via --test-threads=1.
        unsafe { std::env::remove_var("WONDER_OF_U_CLI_BIN") };

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        // Only 1 slot, so newly_launched should be 1, with 2 ready left over.
        assert!(text.contains("newly_launched=1"), "got: {text}");
    }

    #[test]
    fn fleet_reconcile_is_idempotent() {
        use wonder_of_u_core::TaskId;

        let dir = unique_test_dir("fleet-reconcile-idempotent");
        let (run, store) = make_run_with_pending(&dir, &[("step-a", "do a", vec![])], None);

        // Pre-mark step-a as already dispatched in the run state.
        let fake_tid = TaskId::new();
        let mut run2 = store.read_run(run.id).expect("read");
        run2.record_dispatch("step-a".into(), fake_tid);
        store.write_run(&run2).expect("write");
        // Also remove the pending file (as dispatch_one would have done).
        store
            .delete_pending_request_for(&fleet_pending_request(run.id, "step-a"))
            .expect("delete pending");

        let cmd = make_fleet_command(&dir);
        let output = futures::executor::block_on(cmd.execute(
            stub_context(dir.clone()),
            invocation(&format!("reconcile {}", run.id)),
        ))
        .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        // Nothing to re-launch.
        assert!(text.contains("newly_launched=0"), "got: {text}");
    }

    // ── dep classification unit tests ─────────────────────────────────────────

    #[test]
    fn classify_request_no_deps_is_ready() {
        let run = FleetRunState::new("x", PermissionMode::Default, None);
        let req = FleetMemberRequest::new("do x");
        let manager = TaskManager::new(unique_test_dir("classify-ready"));
        assert_eq!(
            classify_request(&req, &run, &manager),
            DepClassification::Ready
        );
    }

    // ── Worktree isolation arg parsing ────────────────────────────────────────

    #[test]
    fn parse_isolation_worktree_mode_parses() {
        let iso = parse_isolation_args(Some("worktree"), None).expect("parse");
        let iso = iso.expect("some");
        assert_eq!(iso.mode, WorktreeIsolationMode::Worktree);
        assert!(iso.branch.is_none());
    }

    #[test]
    fn parse_isolation_worktree_branch_implies_mode() {
        let iso = parse_isolation_args(None, Some("feat/my-branch")).expect("parse");
        let iso = iso.expect("some");
        assert_eq!(iso.mode, WorktreeIsolationMode::Worktree);
        assert_eq!(iso.branch.as_deref(), Some("feat/my-branch"));
    }

    #[test]
    fn parse_isolation_unknown_mode_errors() {
        let err = parse_isolation_args(Some("container"), None).unwrap_err();
        assert!(
            err.to_string().contains("unknown isolation mode"),
            "got: {err}"
        );
    }

    #[test]
    fn parse_isolation_invalid_branch_errors() {
        let err = parse_isolation_args(Some("worktree"), Some("feat..bad")).unwrap_err();
        assert!(err.to_string().contains(".."), "got: {err}");
    }

    #[test]
    fn parse_isolation_none_when_absent() {
        let iso = parse_isolation_args(None, None).expect("parse");
        assert!(iso.is_none());
    }

    // ── Dispatch: non-isolated path unchanged ─────────────────────────────────

    #[test]
    fn dispatch_non_isolated_request_uses_original_cwd() {
        let dir = unique_test_dir("fleet-dispatch-no-isolation");
        let store = FleetStore::new(&dir);
        let req = FleetMemberRequest::new("do the thing");
        let req_id = req.id.clone();
        store.queue_member_request(&req).expect("queue");

        let fake_bin = dir.join("fake-wonder");
        std::fs::write(&fake_bin, "#!/bin/sh\nsleep 0\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake_bin, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        unsafe { std::env::set_var("WONDER_OF_U_CLI_BIN", &fake_bin) };

        let cmd = make_fleet_command(&dir);
        let output = futures::executor::block_on(
            cmd.execute(stub_context(dir.clone()), invocation("dispatch")),
        )
        .expect("execute");
        unsafe { std::env::remove_var("WONDER_OF_U_CLI_BIN") };

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("fleet_dispatched=1"), "got: {text}");
        // Pending file must be gone.
        assert!(
            store.read_pending_request(&req_id).is_err(),
            "pending file should be deleted after dispatch"
        );
    }

    // ── Dispatch: worktree isolation in temp git repo ─────────────────────────

    fn init_git_repo_for_fleet(prefix: &str) -> PathBuf {
        let dir = unique_test_dir(prefix);
        let run_git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .expect("git")
        };
        run_git(&["init"]);
        run_git(&["config", "user.name", "wonder-of-u"]);
        run_git(&["config", "user.email", "wonder-of-u@example.com"]);
        std::fs::write(dir.join("README.md"), "# repo\n").expect("write");
        run_git(&["add", "README.md"]);
        run_git(&["commit", "-m", "initial"]);
        dir
    }

    #[test]
    fn dispatch_isolated_request_creates_worktree_and_sets_worktree_branch() {
        let git_repo = init_git_repo_for_fleet("fleet-dispatch-worktree");
        // Use a separate storage dir so task files don't land in the repo.
        let storage_dir = unique_test_dir("fleet-dispatch-worktree-storage");

        let store = FleetStore::new(&storage_dir);
        let run = FleetRunState::new("isolated-fleet", PermissionMode::Default, None);
        let fleet_id = run.id;
        store.write_run(&run).expect("write run");

        let mut req = FleetMemberRequest::new("work in isolation");
        req.fleet_id = Some(fleet_id);
        req.cwd = Some(git_repo.clone());
        req.isolation = Some(wonder_of_u_core::WorktreeIsolation {
            mode: wonder_of_u_core::WorktreeIsolationMode::Worktree,
            branch: None,
        });
        let req_id = req.id.clone();
        store.queue_member_request(&req).expect("queue");

        let fake_bin = storage_dir.join("fake-wonder");
        std::fs::write(&fake_bin, "#!/bin/sh\nsleep 0\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake_bin, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        unsafe { std::env::set_var("WONDER_OF_U_CLI_BIN", &fake_bin) };

        let cmd = make_fleet_command(&storage_dir);
        let output = futures::executor::block_on(
            cmd.execute(stub_context(git_repo.clone()), invocation("dispatch")),
        )
        .expect("execute");
        unsafe { std::env::remove_var("WONDER_OF_U_CLI_BIN") };

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("fleet_dispatched=1"), "got: {text}");
        // Pending file must be gone.
        assert!(store.read_pending_request(&req_id).is_err());

        // The dispatched task should have worktree_branch set.
        let task_store = wonder_of_u_storage::TaskStore::new(&storage_dir);
        let tasks = task_store.list_tasks().expect("list tasks");
        let task = tasks
            .iter()
            .find(|t| t.fleet_id == Some(fleet_id))
            .expect("task should belong to fleet");
        assert!(
            task.worktree_branch.is_some(),
            "task should have worktree_branch set; task cwd={:?}",
            task.cwd
        );
        let branch = task.worktree_branch.as_ref().unwrap();
        assert!(
            branch.starts_with("worktree-fleet-"),
            "branch should be fleet-prefixed: {branch}"
        );
    }

    #[test]
    fn dispatch_isolated_request_preserves_explicit_worktree_branch() {
        let git_repo = init_git_repo_for_fleet("fleet-dispatch-worktree-explicit");
        let storage_dir = unique_test_dir("fleet-dispatch-worktree-explicit-storage");

        let store = FleetStore::new(&storage_dir);
        let run = FleetRunState::new("isolated-fleet", PermissionMode::Default, None);
        let fleet_id = run.id;
        store.write_run(&run).expect("write run");

        let explicit_branch = "feat/fleet-member-explicit";
        let mut req = FleetMemberRequest::new("work in explicit isolation");
        req.fleet_id = Some(fleet_id);
        req.cwd = Some(git_repo.clone());
        req.isolation = Some(wonder_of_u_core::WorktreeIsolation {
            mode: wonder_of_u_core::WorktreeIsolationMode::Worktree,
            branch: Some(explicit_branch.into()),
        });
        store.queue_member_request(&req).expect("queue");

        let fake_bin = storage_dir.join("fake-wonder");
        std::fs::write(&fake_bin, "#!/bin/sh\nsleep 0\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake_bin, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        unsafe { std::env::set_var("WONDER_OF_U_CLI_BIN", &fake_bin) };

        let cmd = make_fleet_command(&storage_dir);
        let output = futures::executor::block_on(
            cmd.execute(stub_context(git_repo.clone()), invocation("dispatch")),
        )
        .expect("execute");
        unsafe { std::env::remove_var("WONDER_OF_U_CLI_BIN") };

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("fleet_dispatched=1"), "got: {text}");

        let task_store = wonder_of_u_storage::TaskStore::new(&storage_dir);
        let tasks = task_store.list_tasks().expect("list tasks");
        let task = tasks
            .iter()
            .find(|t| t.fleet_id == Some(fleet_id))
            .expect("task should belong to fleet");
        assert_eq!(task.worktree_branch.as_deref(), Some(explicit_branch));
    }

    #[test]
    fn dispatch_isolated_request_non_git_cwd_leaves_request_pending() {
        let non_git_dir = unique_test_dir("fleet-dispatch-worktree-non-git");
        // non_git_dir is NOT a git repo.
        let storage_dir = unique_test_dir("fleet-dispatch-worktree-non-git-storage");

        let store = FleetStore::new(&storage_dir);
        let mut req = FleetMemberRequest::new("work in non-git");
        req.cwd = Some(non_git_dir.clone());
        req.isolation = Some(wonder_of_u_core::WorktreeIsolation {
            mode: wonder_of_u_core::WorktreeIsolationMode::Worktree,
            branch: None,
        });
        let req_id = req.id.clone();
        store.queue_member_request(&req).expect("queue");

        let cmd = make_fleet_command(&storage_dir);
        let output = futures::executor::block_on(
            cmd.execute(stub_context(non_git_dir.clone()), invocation("dispatch")),
        )
        .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(text.contains("fleet_dispatch_errors=1"), "got: {text}");
        // Request must still be pending.
        assert!(
            store.read_pending_request(&req_id).is_ok(),
            "pending request should be preserved after failed worktree creation"
        );
    }

    // ── fleet show prints worktree_branch ─────────────────────────────────────

    #[test]
    fn fleet_show_prints_worktree_branch_when_set() {
        use wonder_of_u_core::{AgentRuntime, AgentTaskState, TaskState};
        use wonder_of_u_storage::TaskStore;

        let dir = unique_test_dir("fleet-show-worktree-branch");
        let task_store = TaskStore::new(&dir);
        let fleet_store = FleetStore::new(&dir);

        let mut run = FleetRunState::new("show-test", PermissionMode::Default, None);

        // Create a fake task with worktree_branch set.
        let mut task = TaskState::pending_agent(
            "isolated agent",
            AgentTaskState {
                name: "agent-1".into(),
                prompt: Some("do stuff".into()),
                provider: None,
                model: None,
                runtime: AgentRuntime::PromptSubprocess,
            },
        );
        task.fleet_id = Some(run.id);
        task.worktree_branch = Some("worktree-fleet-aabbccdd-11223344".into());
        task.cwd = Some(dir.clone());
        task_store.write_task(&task).expect("write task");

        run.member_task_ids.push(task.id);
        fleet_store.write_run(&run).expect("write run");

        let cmd = make_fleet_command(&dir);
        let output = futures::executor::block_on(cmd.execute(
            stub_context(dir.clone()),
            invocation(&format!("show {}", run.id)),
        ))
        .expect("execute");

        let text = match output {
            CommandOutput::Text(t) => t,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(
            text.contains("worktree_branch="),
            "expected worktree_branch in show output; got: {text}"
        );
        assert!(
            text.contains("worktree-fleet-aabbccdd-11223344"),
            "expected branch name in show output; got: {text}"
        );
    }
}
