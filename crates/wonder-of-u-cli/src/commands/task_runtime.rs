#[cfg(unix)]
use std::os::unix::process::CommandExt;

use std::{
    fs::{self, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    thread,
    time::Duration,
};

use time::OffsetDateTime;
use wonder_of_u_core::{
    AgentRuntime, AgentTaskState, CommandContext, FleetId, PermissionDecision, PermissionMode,
    PermissionRequest, RemoteTaskMetadata, RemoteTaskState, RemoteTaskType, Result, TaskId,
    TaskKind, TaskState, TaskStatus, ToolPermissionContext, WonderError, resolve_path,
};
use wonder_of_u_storage::TaskStore;

const HEARTBEAT_INTERVAL_SECS: u64 = 2;
const STALE_HEARTBEAT_AFTER_SECS: i64 = 8;
const CLI_BIN_OVERRIDE_ENV: &str = "WONDER_OF_U_CLI_BIN";

/// Environment variable injected into every named agent subprocess so the
/// subprocess can poll its own mailbox inbox at turn start.
pub(crate) const WONDER_OF_U_AGENT_NAME_ENV: &str = "WONDER_OF_U_AGENT_NAME";

#[derive(Clone, Debug)]
pub(crate) struct TaskManager {
    store: TaskStore,
}

#[derive(Clone, Debug)]
pub(crate) struct ShellTaskLaunch {
    pub description: String,
    pub command: String,
    pub cwd: Option<PathBuf>,
    pub permission_mode: PermissionMode,
    pub read_only: bool,
    pub destructive: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct AgentTaskLaunch {
    pub name: String,
    pub description: Option<String>,
    pub prompt: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub cwd: PathBuf,
    pub fleet_id: Option<FleetId>,
    /// Fleet member request id that triggered this launch.
    pub fleet_request_id: Option<String>,
    /// Parent agent task id (i.e. the task that queued this one via the `agent` tool).
    pub parent_task_id: Option<TaskId>,
    pub allowed_tools: Option<Vec<String>>,
    /// Git worktree branch the agent will run inside, if any.
    pub worktree_branch: Option<String>,
    /// Composed child system prompt forwarded from the parent session via fork-lite.
    /// Passed as `--system <value>` in the child subprocess command when present.
    pub system_prompt: Option<String>,
    /// Depth of this fork (parent depth + 1), injected as `WONDER_OF_U_FORK_DEPTH`
    /// env var in the child subprocess so recursive forks can be detected.
    pub fork_depth: Option<u32>,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug)]
pub(crate) struct RemoteTaskLaunch {
    pub description: String,
    pub task_type: RemoteTaskType,
    pub metadata: Option<RemoteTaskMetadata>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TaskSummary {
    pub total: usize,
    pub active: usize,
    pub terminal: usize,
    pub pending: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
    pub killed: usize,
    pub cancelled: usize,
    pub shell: usize,
    pub agents: usize,
    pub remote: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct TaskReconcileReport {
    pub reconciled_at: OffsetDateTime,
    pub tasks: Vec<TaskState>,
    pub changed: usize,
    pub finished: usize,
    pub fresh_heartbeats: usize,
    pub stale_heartbeats: usize,
    pub missing_heartbeats: usize,
}

impl TaskReconcileReport {
    fn new(reconciled_at: OffsetDateTime) -> Self {
        Self {
            reconciled_at,
            tasks: Vec::new(),
            changed: 0,
            finished: 0,
            fresh_heartbeats: 0,
            stale_heartbeats: 0,
            missing_heartbeats: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskHeartbeatState {
    Fresh,
    Stale,
    Missing,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TaskPruneReport {
    /// Tasks that were successfully removed.
    pub removed: Vec<TaskState>,
    /// Active (non-terminal) tasks that were skipped.
    pub skipped_active: Vec<TaskState>,
    /// Terminal tasks skipped by a narrower prune filter.
    pub skipped_terminal: Vec<TaskState>,
}

#[derive(Clone, Copy, Debug, Default)]
struct TaskReconcileOutcome {
    changed: bool,
    finished: bool,
}

impl TaskSummary {
    /// Handles from tasks
    #[must_use]
    pub fn from_tasks(tasks: &[TaskState]) -> Self {
        let mut summary = Self::default();
        for task in tasks {
            summary.total += 1;
            if task.status.is_terminal() {
                summary.terminal += 1;
            } else {
                summary.active += 1;
            }
            match task.status {
                TaskStatus::Pending => summary.pending += 1,
                TaskStatus::Running => summary.running += 1,
                TaskStatus::Completed => summary.completed += 1,
                TaskStatus::Failed => summary.failed += 1,
                TaskStatus::Killed => summary.killed += 1,
                TaskStatus::Cancelled => summary.cancelled += 1,
            }
            match task.kind {
                TaskKind::LocalShell => summary.shell += 1,
                TaskKind::LocalAgent => summary.agents += 1,
                TaskKind::RemoteAgent => summary.remote += 1,
            }
        }
        summary
    }
}

impl TaskManager {
    /// Creates a new value
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            store: TaskStore::new(base_dir),
        }
    }
    /// Handles storage dir
    #[must_use]
    pub fn storage_dir(&self) -> &Path {
        self.store.paths().base_dir()
    }
    /// Handles logs dir
    #[must_use]
    pub fn logs_dir(&self) -> PathBuf {
        self.store.paths().task_logs_dir()
    }

    /// Handles reconcile tasks
    pub fn reconcile_tasks(&self, kind: Option<TaskKind>) -> Result<TaskReconcileReport> {
        let reconciled_at = OffsetDateTime::now_utc();
        let mut report = TaskReconcileReport::new(reconciled_at);
        for task in self.store.list_tasks()? {
            let (task, outcome) = self.reconcile_task(task, reconciled_at)?;
            if kind.is_some_and(|kind| task.kind != kind) {
                continue;
            }
            if outcome.changed {
                report.changed += 1;
            }
            if outcome.finished {
                report.finished += 1;
            }
            match task_heartbeat_state(&task, reconciled_at) {
                Some(TaskHeartbeatState::Fresh) => report.fresh_heartbeats += 1,
                Some(TaskHeartbeatState::Stale) => report.stale_heartbeats += 1,
                Some(TaskHeartbeatState::Missing) => report.missing_heartbeats += 1,
                None => {}
            }
            report.tasks.push(task);
        }
        Ok(report)
    }

    /// Returns the task
    pub fn get_task(&self, task_id: TaskId) -> Result<TaskState> {
        let task = self.store.read_task(task_id)?;
        Ok(self.reconcile_task(task, OffsetDateTime::now_utc())?.0)
    }

    /// Reads log tail
    pub fn read_log_tail(&self, task_id: TaskId, tail_lines: usize) -> Result<Vec<String>> {
        self.store.read_log_tail(task_id, tail_lines)
    }

    /// Handles start shell task
    pub fn start_shell_task(
        &self,
        context: &CommandContext,
        launch: ShellTaskLaunch,
    ) -> Result<TaskState> {
        let cwd = launch
            .cwd
            .as_deref()
            .map(|path| resolve_path(path, &context.cwd))
            .unwrap_or_else(|| context.cwd.clone());
        ensure_directory(&cwd)?;

        let permission = ToolPermissionContext::new(&context.cwd, launch.permission_mode).evaluate(
            &PermissionRequest::new("bash")
                .with_path(cwd.clone())
                .with_shell_command(launch.command.clone())
                .read_only(launch.read_only)
                .destructive(launch.destructive),
        );
        ensure_permission_allowed(permission)?;

        let mut task = TaskState::pending_shell(launch.description, launch.command.clone(), cwd);
        task.output_log = Some(self.store.paths().task_log_path(task.id));
        self.store.write_task(&task)?;
        self.store
            .append_log(task.id, render_shell_task_header(&task))?;

        let log_path = task
            .output_log
            .clone()
            .ok_or_else(|| WonderError::internal("task log path missing after initialization"))?;
        let pid = spawn_background_task(
            task.command.as_deref().unwrap_or_default(),
            task.cwd.as_deref().unwrap_or(self.storage_dir()),
            &log_path,
            &self.store.paths().task_exit_path(task.id),
            &self.store.paths().task_heartbeat_path(task.id),
            &[],
        )?;
        let process_identity = capture_process_identity(pid)?;

        task.mark_running(
            Some(pid),
            process_identity,
            Some(OffsetDateTime::now_utc()),
            Some("background shell task is running".into()),
        );
        self.store.write_task(&task)?;
        Ok(task)
    }

    /// Handles start agent task
    pub fn start_agent_task(&self, launch: AgentTaskLaunch) -> Result<TaskState> {
        ensure_directory(&launch.cwd)?;
        let description = launch
            .description
            .clone()
            .unwrap_or_else(|| launch.name.clone());
        let mut task = TaskState::pending_agent(
            description,
            AgentTaskState::prompt_subprocess(
                launch.name.clone(),
                launch.prompt.clone(),
                launch.provider.clone(),
                launch.model.clone(),
            ),
        );
        task.fleet_id = launch.fleet_id;
        task.fleet_request_id = launch.fleet_request_id.clone();
        task.parent_id = launch.parent_task_id;
        task.cwd = Some(launch.cwd.clone());
        task.worktree_branch = launch.worktree_branch.clone();
        task.output_log = Some(self.store.paths().task_log_path(task.id));
        // Build the subprocess command now that task.id is known (needed for env injection).
        task.command = Some(agent_prompt_command(self.storage_dir(), &launch)?);
        self.store.write_task(&task)?;
        self.store
            .append_log(task.id, render_agent_task_header(&task))?;

        let log_path = task
            .output_log
            .clone()
            .ok_or_else(|| WonderError::internal("task log path missing after initialization"))?;

        // Inject fleet context env vars so the agent subprocess can write
        // result sidecars and queue child requests with correct lineage.
        let mut env_vars: Vec<(String, String)> =
            vec![("WONDER_OF_U_TASK_ID".into(), task.id.to_string())];
        if let Some(fleet_id) = launch.fleet_id {
            env_vars.push(("WONDER_OF_U_FLEET_ID".into(), fleet_id.to_string()));
        }
        if let Some(ref req_id) = launch.fleet_request_id {
            env_vars.push(("WONDER_OF_U_FLEET_REQUEST_ID".into(), req_id.clone()));
        }
        // Propagate fork depth so the child subprocess can enforce the
        // recursive fork guard in AgentTool::execute().
        if let Some(depth) = launch.fork_depth {
            env_vars.push(("WONDER_OF_U_FORK_DEPTH".into(), depth.to_string()));
        }
        // Inject agent name so the child subprocess can locate its own mailbox
        // inbox when polling for unread messages at turn start.
        if !launch.name.is_empty() {
            env_vars.push((WONDER_OF_U_AGENT_NAME_ENV.into(), launch.name.clone()));
        }

        let pid = spawn_background_task(
            task.command.as_deref().unwrap_or_default(),
            task.cwd.as_deref().unwrap_or(self.storage_dir()),
            &log_path,
            &self.store.paths().task_exit_path(task.id),
            &self.store.paths().task_heartbeat_path(task.id),
            &env_vars,
        )?;
        let process_identity = capture_process_identity(pid)?;

        task.mark_running(
            Some(pid),
            process_identity,
            Some(OffsetDateTime::now_utc()),
            Some(running_status_message(task.kind, TaskHeartbeatState::Fresh).into()),
        );
        self.store.write_task(&task)?;
        Ok(task)
    }
    /// Handles start remote task
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn start_remote_task(&self, launch: RemoteTaskLaunch) -> Result<TaskState> {
        let remote = RemoteTaskState::deferred(launch.task_type, launch.metadata);
        Err(WonderError::validation(format!(
            "{} [{}]",
            remote.start_error_message(),
            launch.description
        )))
    }

    /// Handles stop task
    pub fn stop_task(&self, task_id: TaskId, force: bool) -> Result<TaskState> {
        let task = self.get_task(task_id)?;
        if task.status.is_terminal() {
            return Ok(task);
        }

        match task.kind {
            TaskKind::LocalShell | TaskKind::LocalAgent => self.stop_process_task(task, force),
            TaskKind::RemoteAgent => Err(WonderError::validation(
                task.remote
                    .as_ref()
                    .map(RemoteTaskState::stop_error_message)
                    .unwrap_or_else(|| {
                        "cannot stop remote task: remote task transport is unavailable in this Rust runtime"
                            .into()
                    }),
            )),
        }
    }

    /// Removes a single task's persisted artifacts (state, log, heartbeat, exit
    /// files).
    ///
    /// * If the task is in a terminal state the artifacts are deleted and the
    ///   task state snapshot is returned.
    /// * If the task is still active (pending/running) the call returns an
    ///   error *unless* `force` is `true`, in which case the artifacts are
    ///   removed unconditionally.
    pub fn remove_task(&self, task_id: TaskId, force: bool) -> Result<TaskState> {
        // Reconcile so the state reflects any process exits since the last write.
        let task = self.get_task(task_id)?;
        if !task.status.is_terminal() && !force {
            return Err(WonderError::validation(format!(
                "task {} is still active (status={}); pass --force to remove it anyway",
                task.id,
                task_status_label(task.status),
            )));
        }
        self.store.delete_task_artifacts(task_id)?;
        Ok(task)
    }

    /// Removes all terminal tasks in bulk.
    ///
    /// When `completed_only` is `true` only `Completed` tasks are pruned;
    /// otherwise every terminal status (Completed, Failed, Killed, Cancelled)
    /// is pruned.  Active (Pending/Running) tasks are always skipped.
    ///
    /// Returns a [`TaskPruneReport`] with the removed and skipped lists so
    /// callers can produce stable `key=value` output.
    pub fn prune_tasks(&self, completed_only: bool) -> Result<TaskPruneReport> {
        // Reconcile all tasks first so any that finished since the last
        // heartbeat are marked terminal before we decide whether to remove them.
        let report = self.reconcile_tasks(None)?;
        let mut result = TaskPruneReport::default();
        for task in report.tasks {
            if task.status.is_terminal() {
                let should_prune = !completed_only || task.status == TaskStatus::Completed;
                if should_prune {
                    self.store.delete_task_artifacts(task.id)?;
                    result.removed.push(task);
                } else {
                    // Terminal but excluded by the completed-only filter.
                    result.skipped_terminal.push(task);
                }
            } else {
                // Active task – never touched.
                result.skipped_active.push(task);
            }
        }
        Ok(result)
    }

    fn stop_process_task(&self, mut task: TaskState, force: bool) -> Result<TaskState> {
        let pid = task.pid.ok_or_else(|| {
            WonderError::internal(format!("task {} is missing a process id", task.id))
        })?;
        send_signal(pid, "TERM")?;
        if !wait_for_exit(pid, Duration::from_secs(2))? && force {
            send_signal(pid, "KILL")?;
            let _ = wait_for_exit(pid, Duration::from_secs(2))?;
        }
        if process_is_alive(pid)? {
            return Err(WonderError::internal(format!(
                "task {} is still running after stop request; retry with --force",
                task.id
            )));
        }

        let (status, note) = if force {
            (TaskStatus::Killed, "force-stopped by stop request")
        } else {
            (TaskStatus::Cancelled, "cancelled by stop request")
        };
        task.mark_finished(status, None, Some(note.into()));
        self.store.write_task(&task)?;
        self.store.append_log(
            task.id,
            format!(
                "[wonder-of-u {} stop] at={} status={}\n",
                task_kind_label(task.kind),
                OffsetDateTime::now_utc(),
                note
            ),
        )?;
        Ok(task)
    }

    fn reconcile_task(
        &self,
        mut task: TaskState,
        reconciled_at: OffsetDateTime,
    ) -> Result<(TaskState, TaskReconcileOutcome)> {
        let mut outcome = TaskReconcileOutcome::default();

        if !task_supports_process_runtime(task.kind) || task.status.is_terminal() {
            return Ok((task, outcome));
        }
        let kind = task.kind;

        if let Some(heartbeat_at) = self.store.read_heartbeat_at(task.id)? {
            if task.last_heartbeat_at != Some(heartbeat_at) {
                task.last_heartbeat_at = Some(heartbeat_at);
                outcome.changed = true;
            }
        }

        if let Some(exit_code) = self.store.read_exit_code(task.id)? {
            let status = if exit_code == 0 {
                TaskStatus::Completed
            } else {
                TaskStatus::Failed
            };
            if task.status != status
                || task.exit_code != Some(exit_code)
                || task.finished_at.is_none()
            {
                self.finish_reconciled_task(
                    &mut task,
                    status,
                    Some(exit_code),
                    format!("exited with code {exit_code}"),
                    format!("reconciled persisted exit code {exit_code}"),
                )?;
                outcome.changed = true;
                outcome.finished = true;
            }
            return Ok((task, outcome));
        }

        let Some(pid) = task.pid else {
            match task_heartbeat_state(&task, reconciled_at) {
                Some(TaskHeartbeatState::Fresh) => {
                    task.status = TaskStatus::Running;
                    task.status_message = Some(
                        missing_pid_status_message(task.kind, TaskHeartbeatState::Fresh).into(),
                    );
                    self.store.write_task(&task)?;
                    outcome.changed = true;
                    return Ok((task, outcome));
                }
                Some(TaskHeartbeatState::Stale) if task.status == TaskStatus::Running => {
                    self.finish_reconciled_task(
                        &mut task,
                        TaskStatus::Killed,
                        None,
                        format!(
                            "{} process id is missing and heartbeat is stale",
                            task_runtime_subject(kind)
                        ),
                        format!(
                            "reconciled stale running {} without a persisted process id",
                            task_kind_label(kind)
                        ),
                    )?;
                    outcome.changed = true;
                    outcome.finished = true;
                }
                _ if task.status == TaskStatus::Running => {
                    self.finish_reconciled_task(
                        &mut task,
                        TaskStatus::Killed,
                        None,
                        format!(
                            "running {} metadata is missing a process id",
                            task_kind_label(kind)
                        ),
                        format!(
                            "reconciled stale running {} metadata without a process id",
                            task_kind_label(kind)
                        ),
                    )?;
                    outcome.changed = true;
                    outcome.finished = true;
                }
                _ if task.status == TaskStatus::Pending
                    && reconciled_at - task.started_at > time::Duration::seconds(5) =>
                {
                    self.finish_reconciled_task(
                        &mut task,
                        TaskStatus::Failed,
                        None,
                        format!(
                            "{} launch never recorded a process id",
                            task_runtime_subject(kind)
                        ),
                        format!(
                            "reconciled stale pending {} launch that never captured a process id",
                            task_kind_label(kind)
                        ),
                    )?;
                    outcome.changed = true;
                    outcome.finished = true;
                }
                _ => {}
            }
            return Ok((task, outcome));
        };

        let expected_process_identity = task.process_identity.clone();
        if let (Some(expected), Some(actual)) =
            (expected_process_identity, capture_process_identity(pid)?)
        {
            if actual != expected {
                self.finish_reconciled_task(
                    &mut task,
                    TaskStatus::Killed,
                    None,
                    "original task process is gone; pid now belongs to a different process".into(),
                    format!("reconciled stale pid reuse (expected {expected}, observed {actual})"),
                )?;
                outcome.changed = true;
                outcome.finished = true;
                return Ok((task, outcome));
            }
        }

        if !process_is_alive(pid)? {
            self.finish_reconciled_task(
                &mut task,
                TaskStatus::Killed,
                None,
                format!(
                    "{} process exited without writing an exit marker",
                    task_runtime_subject(kind)
                ),
                format!(
                    "reconciled missing exit marker after {} process exit",
                    task_kind_label(kind)
                ),
            )?;
            outcome.changed = true;
            outcome.finished = true;
            return Ok((task, outcome));
        }

        if task.status != TaskStatus::Running {
            task.mark_running(
                task.pid,
                task.process_identity.clone(),
                task.last_heartbeat_at,
                Some(running_status_message(task.kind, TaskHeartbeatState::Fresh).into()),
            );
            outcome.changed = true;
        }

        let desired_status_message = match task_heartbeat_state(&task, reconciled_at) {
            Some(state) => running_status_message(task.kind, state),
            None => running_status_message(task.kind, TaskHeartbeatState::Fresh),
        };
        if task.status_message.as_deref() != Some(desired_status_message) {
            task.status_message = Some(desired_status_message.into());
            outcome.changed = true;
        }

        if outcome.changed {
            self.store.write_task(&task)?;
            if matches!(
                task_heartbeat_state(&task, reconciled_at),
                Some(TaskHeartbeatState::Stale)
            ) {
                self.store.append_log(
                    task.id,
                    format!(
                        "[wonder-of-u task heartbeat] at={} state=stale detail={}\n",
                        reconciled_at,
                        sanitize_log_value(
                            "heartbeat file has stopped advancing while process is still alive"
                        )
                    ),
                )?;
            }
        }

        Ok((task, outcome))
    }

    fn finish_reconciled_task(
        &self,
        task: &mut TaskState,
        status: TaskStatus,
        exit_code: Option<i32>,
        status_message: String,
        log_message: String,
    ) -> Result<()> {
        task.mark_finished(status, exit_code, Some(status_message.clone()));
        self.store.write_task(task)?;
        self.store.append_log(
            task.id,
            format!(
                "[wonder-of-u task reconcile] at={} status={} detail={}\n",
                OffsetDateTime::now_utc(),
                task_status_label(status),
                sanitize_log_value(&log_message)
            ),
        )?;
        Ok(())
    }
}

fn ensure_directory(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path).map_err(|error| match error.kind() {
        ErrorKind::NotFound => WonderError::not_found("directory", path.display().to_string()),
        _ => error.into(),
    })?;
    if metadata.is_dir() {
        Ok(())
    } else {
        Err(WonderError::validation(format!(
            "task cwd is not a directory: {}",
            path.display()
        )))
    }
}

fn ensure_permission_allowed(decision: PermissionDecision) -> Result<()> {
    if decision.is_allowed() {
        return Ok(());
    }

    Err(WonderError::permission_denied(format!(
        "task launch requires an allowed permission decision: {}",
        decision.reason()
    )))
}

fn render_shell_task_header(task: &TaskState) -> String {
    format!(
        concat!(
            "[wonder-of-u task start]\n",
            "task_id={}\n",
            "kind=local_shell\n",
            "description={}\n",
            "cwd={}\n",
            "command={}\n",
            "started_at={}\n\n"
        ),
        task.id,
        task.description,
        task.cwd
            .as_deref()
            .unwrap_or_else(|| Path::new("."))
            .display(),
        task.command.as_deref().unwrap_or_default(),
        task.started_at,
    )
}

fn render_agent_task_header(task: &TaskState) -> String {
    let agent = task.agent.as_ref();
    format!(
        concat!(
            "[wonder-of-u agent start]\n",
            "task_id={}\n",
            "kind=local_agent\n",
            "description={}\n",
            "runtime={}\n",
            "provider={}\n",
            "model={}\n",
            "prompt={}\n",
            "started_at={}\n\n"
        ),
        task.id,
        task.description,
        agent
            .map(|agent| agent_runtime_label(agent.runtime))
            .unwrap_or("metadata_only"),
        agent
            .and_then(|agent| agent.provider.as_deref())
            .unwrap_or("unconfigured"),
        agent
            .and_then(|agent| agent.model.as_deref())
            .unwrap_or("unconfigured"),
        agent
            .and_then(|agent| agent.prompt.as_deref())
            .unwrap_or_default(),
        task.started_at,
    )
}

fn sanitize_log_value(value: &str) -> String {
    value.replace(['\n', '\r'], " ")
}

fn task_supports_process_runtime(kind: TaskKind) -> bool {
    matches!(kind, TaskKind::LocalShell | TaskKind::LocalAgent)
}

fn task_runtime_subject(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::LocalShell => "task",
        TaskKind::LocalAgent => "local agent",
        TaskKind::RemoteAgent => "remote task",
    }
}

fn task_kind_label(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::LocalShell => "local_shell",
        TaskKind::LocalAgent => "local_agent",
        TaskKind::RemoteAgent => "remote_agent",
    }
}

fn running_status_message(kind: TaskKind, heartbeat_state: TaskHeartbeatState) -> &'static str {
    match (kind, heartbeat_state) {
        (TaskKind::LocalShell, TaskHeartbeatState::Fresh) => "background shell task is running",
        (TaskKind::LocalShell, TaskHeartbeatState::Missing) => {
            "background shell task is running; waiting for first heartbeat"
        }
        (TaskKind::LocalShell, TaskHeartbeatState::Stale) => {
            "background shell task is running but heartbeat is stale"
        }
        (TaskKind::LocalAgent, TaskHeartbeatState::Fresh) => {
            "background local agent prompt subprocess is running"
        }
        (TaskKind::LocalAgent, TaskHeartbeatState::Missing) => {
            "background local agent prompt subprocess is running; waiting for first heartbeat"
        }
        (TaskKind::LocalAgent, TaskHeartbeatState::Stale) => {
            "background local agent prompt subprocess is running but heartbeat is stale"
        }
        (TaskKind::RemoteAgent, _) => {
            "remote task status is metadata-only; no local heartbeat is available"
        }
    }
}

fn missing_pid_status_message(kind: TaskKind, heartbeat_state: TaskHeartbeatState) -> &'static str {
    match (kind, heartbeat_state) {
        (TaskKind::LocalShell, TaskHeartbeatState::Fresh) => {
            "background shell task heartbeat is present but process id was not persisted"
        }
        (TaskKind::LocalAgent, TaskHeartbeatState::Fresh) => {
            "background local agent prompt heartbeat is present but process id was not persisted"
        }
        (TaskKind::LocalShell, TaskHeartbeatState::Missing)
        | (TaskKind::LocalShell, TaskHeartbeatState::Stale) => {
            "background shell task metadata is missing a process id"
        }
        (TaskKind::LocalAgent, TaskHeartbeatState::Missing)
        | (TaskKind::LocalAgent, TaskHeartbeatState::Stale) => {
            "background local agent prompt metadata is missing a process id"
        }
        (TaskKind::RemoteAgent, _) => "remote task metadata is managed without a local process id",
    }
}

fn agent_prompt_command(storage_dir: &Path, launch: &AgentTaskLaunch) -> Result<String> {
    let executable = resolve_cli_executable()?;
    let mut tokens = vec![
        executable.display().to_string(),
        "--storage-dir".into(),
        storage_dir.display().to_string(),
        "prompt".into(),
        "--tools".into(),
    ];
    if let Some(provider) = &launch.provider {
        tokens.push("--provider".into());
        tokens.push(provider.clone());
    }
    if let Some(model) = &launch.model {
        tokens.push("--model".into());
        tokens.push(model.clone());
    }
    if let Some(allowed_tools) = &launch.allowed_tools
        && !allowed_tools.is_empty()
    {
        tokens.push("--allowed-tools".into());
        tokens.push(allowed_tools.join(","));
    }
    // Pass the composed fork system prompt to the child subprocess.
    if let Some(ref system_prompt) = launch.system_prompt {
        tokens.push("--system".into());
        tokens.push(system_prompt.clone());
    }
    tokens.push(launch.prompt.clone());
    Ok(shell_words::join(tokens.iter().map(String::as_str)))
}

fn resolve_cli_executable() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(CLI_BIN_OVERRIDE_ENV) {
        return Ok(PathBuf::from(path));
    }

    let current = std::env::current_exe()?;
    if current.file_stem().and_then(|value| value.to_str()) == Some("wonder-of-u") {
        return Ok(current);
    }

    Err(WonderError::internal(format!(
        "unable to locate the wonder-of-u executable; set {CLI_BIN_OVERRIDE_ENV}"
    )))
}

fn agent_runtime_label(runtime: AgentRuntime) -> &'static str {
    match runtime {
        AgentRuntime::MetadataOnly => "metadata_only",
        AgentRuntime::PromptSubprocess => "prompt_subprocess",
        AgentRuntime::Deferred => "legacy_relaunch_required",
    }
}

pub(crate) fn task_heartbeat_state(
    task: &TaskState,
    reconciled_at: OffsetDateTime,
) -> Option<TaskHeartbeatState> {
    if !task_supports_process_runtime(task.kind) || task.status.is_terminal() {
        return None;
    }

    let Some(last_heartbeat_at) = task.last_heartbeat_at else {
        return Some(TaskHeartbeatState::Missing);
    };
    if reconciled_at - last_heartbeat_at > time::Duration::seconds(STALE_HEARTBEAT_AFTER_SECS) {
        Some(TaskHeartbeatState::Stale)
    } else {
        Some(TaskHeartbeatState::Fresh)
    }
}

pub(crate) const fn task_heartbeat_state_label(state: TaskHeartbeatState) -> &'static str {
    match state {
        TaskHeartbeatState::Fresh => "fresh",
        TaskHeartbeatState::Stale => "stale",
        TaskHeartbeatState::Missing => "missing",
    }
}

fn spawn_background_task(
    command: &str,
    cwd: &Path,
    log_path: &Path,
    exit_path: &Path,
    heartbeat_path: &Path,
    env_vars: &[(String, String)],
) -> Result<u32> {
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    let stderr = stdout.try_clone()?;
    let mut child = background_shell_command(command, exit_path, heartbeat_path);
    child
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    for (key, val) in env_vars {
        child.env(key, val);
    }
    Ok(child.spawn()?.id())
}

#[cfg(target_os = "linux")]
fn capture_process_identity(pid: u32) -> Result<Option<String>> {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let start_time = stat
        .rsplit_once(") ")
        .and_then(|(_, rest)| rest.split_whitespace().nth(19))
        .ok_or_else(|| {
            WonderError::validation(format!("invalid /proc stat payload for pid {pid}"))
        })?;
    Ok(Some(format!("linux-proc-start:{start_time}")))
}

#[cfg(not(target_os = "linux"))]
fn capture_process_identity(_pid: u32) -> Result<Option<String>> {
    Ok(None)
}

#[cfg(unix)]
fn background_shell_command(
    command: &str,
    exit_path: &Path,
    heartbeat_path: &Path,
) -> ProcessCommand {
    let mut child = ProcessCommand::new("sh");
    child
        .arg("-lc")
        .arg(shell_wrapper(command, exit_path, heartbeat_path));
    unsafe {
        child.pre_exec(|| {
            // On macOS, setsid() places the child in a new session.  The
            // parent process cannot send signals to a process group that
            // belongs to a different session (EPERM), so we use setpgid
            // instead, which creates a new process group within the same
            // session and still lets kill(-pgid, signal) work.
            #[cfg(target_os = "macos")]
            let rc = libc::setpgid(0, 0);
            #[cfg(not(target_os = "macos"))]
            let rc = libc::setsid();
            if rc == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    child
}

#[cfg(not(unix))]
fn background_shell_command(
    command: &str,
    exit_path: &Path,
    heartbeat_path: &Path,
) -> ProcessCommand {
    let mut child = ProcessCommand::new("sh");
    child
        .arg("-lc")
        .arg(shell_wrapper(command, exit_path, heartbeat_path));
    child
}

fn shell_wrapper(command: &str, exit_path: &Path, heartbeat_path: &Path) -> String {
    let exit_path = exit_path.display().to_string();
    let status_file = shell_words::quote(&exit_path);
    let heartbeat_path = heartbeat_path.display().to_string();
    let heartbeat_file = shell_words::quote(&heartbeat_path);
    format!(
        concat!(
            "status_file={status_file}; ",
            "heartbeat_file={heartbeat_file}; ",
            "heartbeat() {{ heartbeat_tmp=\"$heartbeat_file.next.$$\"; ",
            "date -u +\"%Y-%m-%dT%H:%M:%SZ\" > \"$heartbeat_tmp\" && mv \"$heartbeat_tmp\" \"$heartbeat_file\"; }}; ",
            "heartbeat; ",
            "while :; do heartbeat; sleep {heartbeat_interval}; done & heartbeat_pid=$!; ",
            "trap 'status=$?; kill \"$heartbeat_pid\" 2>/dev/null || true; ",
            "wait \"$heartbeat_pid\" 2>/dev/null || true; heartbeat; printf \"%s\\n\" \"$status\" > \"$status_file\"; ",
            "printf \"\\n[task-exit] %s\\n\" \"$status\"' EXIT; ",
            "{command}"
        ),
        status_file = status_file,
        heartbeat_file = heartbeat_file,
        heartbeat_interval = HEARTBEAT_INTERVAL_SECS,
        command = command,
    )
}

fn wait_for_exit(pid: u32, timeout: Duration) -> Result<bool> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if !process_is_alive(pid)? {
            return Ok(true);
        }
        if std::time::Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> Result<bool> {
    let result = unsafe { libc::kill(pid as i32, 0) };
    if result == 0 {
        return Ok(!process_is_zombie(pid));
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        Some(libc::EPERM) => Ok(true),
        _ => Err(error.into()),
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn process_is_zombie(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    stat.rsplit_once(") ")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        == Some("Z")
}

// On macOS /proc does not exist.  Use waitpid(WNOHANG) to detect (and reap)
// zombie children.  When we are not the parent the call returns ECHILD;
// in that case init/launchd reaps the zombie quickly so returning false is
// safe — the next kill(pid, 0) poll will see ESRCH once it is gone.
#[cfg(target_os = "macos")]
fn process_is_zombie(pid: u32) -> bool {
    unsafe {
        libc::waitpid(pid as libc::pid_t, std::ptr::null_mut(), libc::WNOHANG) == pid as libc::pid_t
    }
}

#[cfg(not(unix))]
fn process_is_alive(pid: u32) -> Result<bool> {
    let status = ProcessCommand::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(status.success())
}

#[cfg(unix)]
fn send_signal(pid: u32, signal: &str) -> Result<()> {
    let signal = match signal {
        "TERM" => libc::SIGTERM,
        "KILL" => libc::SIGKILL,
        other => {
            return Err(WonderError::validation(format!(
                "unsupported signal for task control: {other}"
            )));
        }
    };
    let result = unsafe { libc::kill(-(pid as i32), signal) };
    if result == 0 || !process_is_alive(pid)? {
        Ok(())
    } else {
        Err(WonderError::internal(format!(
            "failed to send signal to task process group {pid}"
        )))
    }
}

#[cfg(not(unix))]
fn send_signal(pid: u32, signal: &str) -> Result<()> {
    let status = ProcessCommand::new("kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() || !process_is_alive(pid)? {
        Ok(())
    } else {
        Err(WonderError::internal(format!(
            "failed to send {signal} to task process {pid}"
        )))
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

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use wonder_of_u_core::{
        CommandContext, FeatureSet, PermissionMode, RemoteTaskState, RemoteTaskType, SessionId,
    };
    use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

    use super::*;

    fn command_context(cwd: &Path) -> CommandContext {
        CommandContext {
            session_id: SessionId::new(),
            cwd: cwd.to_path_buf(),
            features: FeatureSet::first_release(),
            authenticated: false,
            interactive: false,
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
    fn manager_starts_shell_task_and_captures_output() {
        let dir = unique_test_dir("task-manager-shell");
        let manager = TaskManager::new(&dir);
        let task = manager
            .start_shell_task(
                &command_context(&dir),
                ShellTaskLaunch {
                    description: "echo hello".into(),
                    command: "printf 'hello from task'".into(),
                    cwd: None,
                    permission_mode: PermissionMode::Default,
                    read_only: true,
                    destructive: false,
                },
            )
            .expect("start shell task");

        let mut final_task = manager.get_task(task.id).expect("task snapshot");
        for _ in 0..20 {
            if final_task.status.is_terminal() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
            final_task = manager.get_task(task.id).expect("task snapshot");
        }

        assert_eq!(final_task.status, TaskStatus::Completed);
        assert_eq!(final_task.exit_code, Some(0));
        assert!(final_task.last_heartbeat_at.is_some());
        let log = manager
            .read_log_tail(task.id, 20)
            .expect("log tail")
            .join("\n");
        assert!(log.contains("hello from task"));
    }

    #[test]
    fn manager_stops_running_shell_task() {
        let dir = unique_test_dir("task-manager-stop");
        let manager = TaskManager::new(&dir);
        let task = manager
            .start_shell_task(
                &command_context(&dir),
                ShellTaskLaunch {
                    description: "sleep".into(),
                    command: "sleep 10".into(),
                    cwd: None,
                    permission_mode: PermissionMode::Default,
                    read_only: true,
                    destructive: false,
                },
            )
            .expect("start shell task");

        let stopped = manager.stop_task(task.id, true).expect("stop task");
        assert_eq!(stopped.status, TaskStatus::Killed);
    }

    #[test]
    fn manager_starts_prompt_subprocess_agent_tasks() {
        let dir = unique_test_dir("task-manager-agent");
        let fleet_id = FleetId::new();
        let script = write_agent_script(
            &dir,
            "agent-run.sh",
            "printf 'agent invocation: %s\\n' \"$*\"\nsleep 0.2\nprintf 'agent done\\n'\n",
        );
        let _env = EnvVarGuard::set(CLI_BIN_OVERRIDE_ENV, script.into_os_string());
        let manager = TaskManager::new(&dir);
        let task = manager
            .start_agent_task(AgentTaskLaunch {
                name: "planner".into(),
                description: Some("release planner".into()),
                prompt: "Summarize remaining release work".into(),
                provider: Some("openai".into()),
                model: Some("gpt-4.1".into()),
                cwd: dir.clone(),
                fleet_id: Some(fleet_id),
                fleet_request_id: None,
                parent_task_id: None,
                allowed_tools: Some(vec!["bash".into(), "file_read".into()]),
                worktree_branch: None,
                system_prompt: None,
                fork_depth: None,
            })
            .expect("start agent task");

        assert_eq!(task.kind, TaskKind::LocalAgent);
        assert_eq!(task.fleet_id, Some(fleet_id));
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(
            task.agent.as_ref().map(|agent| agent.runtime),
            Some(AgentRuntime::PromptSubprocess)
        );
        assert!(
            task.command
                .as_deref()
                .is_some_and(|command| command.contains(" prompt "))
        );
        assert!(
            task.command
                .as_deref()
                .is_some_and(|command| command.contains(" --tools "))
        );
        assert!(
            task.command
                .as_deref()
                .is_some_and(|command| command.contains("--provider openai"))
        );
        assert!(
            task.command
                .as_deref()
                .is_some_and(|command| command.contains("--allowed-tools bash,file_read"))
        );

        let mut final_task = manager.get_task(task.id).expect("agent snapshot");
        for _ in 0..30 {
            if final_task.status.is_terminal() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
            final_task = manager.get_task(task.id).expect("agent snapshot");
        }

        assert_eq!(final_task.status, TaskStatus::Completed);
        assert_eq!(final_task.exit_code, Some(0));
        assert!(
            manager
                .read_log_tail(task.id, 10)
                .expect("agent log")
                .join("\n")
                .contains("agent invocation:")
        );
    }

    #[test]
    fn manager_stops_running_agent_tasks() {
        let dir = unique_test_dir("task-manager-agent-stop");
        let script = write_agent_script(
            &dir,
            "agent-stop.sh",
            "printf 'agent running\\n'\nsleep 10\n",
        );
        let _env = EnvVarGuard::set(CLI_BIN_OVERRIDE_ENV, script.into_os_string());
        let manager = TaskManager::new(&dir);
        let task = manager
            .start_agent_task(AgentTaskLaunch {
                name: "planner".into(),
                description: Some("release planner".into()),
                prompt: "Wait for stop".into(),
                provider: Some("openai".into()),
                model: Some("gpt-4.1".into()),
                cwd: dir.clone(),
                fleet_id: None,
                fleet_request_id: None,
                parent_task_id: None,
                allowed_tools: None,
                worktree_branch: None,
                system_prompt: None,
                fork_depth: None,
            })
            .expect("start agent task");

        let stopped = manager.stop_task(task.id, true).expect("stop agent task");
        assert_eq!(stopped.status, TaskStatus::Killed);
    }

    #[test]
    fn manager_reports_stale_heartbeat_for_alive_shell_task() {
        let dir = unique_test_dir("task-manager-stale-heartbeat");
        let manager = TaskManager::new(&dir);
        let mut task = TaskState::pending_shell("stale heartbeat", "sleep 1", &dir);
        task.status = TaskStatus::Running;
        task.pid = Some(std::process::id());
        task.output_log = Some(manager.logs_dir().join(format!("{}.log", task.id)));
        manager.store.write_task(&task).expect("write task");
        manager
            .store
            .write_heartbeat_at(
                task.id,
                OffsetDateTime::now_utc() - time::Duration::seconds(STALE_HEARTBEAT_AFTER_SECS + 2),
            )
            .expect("write stale heartbeat");

        let report = manager.reconcile_tasks(None).expect("reconcile tasks");
        assert_eq!(report.stale_heartbeats, 1);
        let reconciled = report.tasks.into_iter().next().expect("task");
        assert_eq!(reconciled.status, TaskStatus::Running);
        assert_eq!(
            reconciled.status_message.as_deref(),
            Some("background shell task is running but heartbeat is stale")
        );
    }

    #[test]
    fn manager_preserves_running_task_with_fresh_heartbeat_but_missing_pid() {
        let dir = unique_test_dir("task-manager-fresh-heartbeat-no-pid");
        let manager = TaskManager::new(&dir);
        let mut task = TaskState::pending_shell("fresh heartbeat", "sleep 1", &dir);
        task.output_log = Some(manager.logs_dir().join(format!("{}.log", task.id)));
        manager.store.write_task(&task).expect("write task");
        manager
            .store
            .write_heartbeat_at(task.id, OffsetDateTime::now_utc())
            .expect("write heartbeat");

        let reconciled = manager.get_task(task.id).expect("reconcile task");
        assert_eq!(reconciled.status, TaskStatus::Running);
        assert_eq!(
            reconciled.status_message.as_deref(),
            Some("background shell task heartbeat is present but process id was not persisted")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn manager_reconciles_pid_reuse_via_process_identity() {
        let dir = unique_test_dir("task-manager-pid-reuse");
        let manager = TaskManager::new(&dir);
        let mut task = TaskState::pending_shell("stale task", "sleep 1", &dir);
        task.status = TaskStatus::Running;
        task.pid = Some(std::process::id());
        task.process_identity = Some("linux-proc-start:stale".into());
        task.output_log = Some(manager.logs_dir().join(format!("{}.log", task.id)));
        manager.store.write_task(&task).expect("write task");

        let reconciled = manager.get_task(task.id).expect("reconcile task");
        assert_eq!(reconciled.status, TaskStatus::Killed);
        assert_eq!(
            reconciled.status_message.as_deref(),
            Some("original task process is gone; pid now belongs to a different process")
        );
    }

    #[test]
    fn manager_reconciles_stale_pending_launch_without_pid() {
        let dir = unique_test_dir("task-manager-stale-pending");
        let manager = TaskManager::new(&dir);
        let mut task = TaskState::pending_shell("stale pending", "sleep 1", &dir);
        task.started_at -= time::Duration::seconds(10);
        task.output_log = Some(manager.logs_dir().join(format!("{}.log", task.id)));
        manager.store.write_task(&task).expect("write task");

        let reconciled = manager.get_task(task.id).expect("reconcile task");
        assert_eq!(reconciled.status, TaskStatus::Failed);
        assert_eq!(
            reconciled.status_message.as_deref(),
            Some("task launch never recorded a process id")
        );
    }

    #[test]
    fn manager_rejects_remote_task_launch_without_side_effects() {
        let dir = unique_test_dir("task-manager-remote-launch");
        let manager = TaskManager::new(&dir);

        let error = manager
            .start_remote_task(RemoteTaskLaunch {
                description: "cloud review".into(),
                task_type: RemoteTaskType::Ultrareview,
                metadata: None,
            })
            .expect_err("remote launch unsupported");

        assert!(error.to_string().contains("cannot start ultrareview task"));
        assert!(!manager.storage_dir().join("tasks").exists());
    }

    #[test]
    fn manager_reconcile_leaves_remote_tasks_metadata_only() {
        let dir = unique_test_dir("task-manager-remote-reconcile");
        let manager = TaskManager::new(&dir);
        let task = TaskState::recorded_remote(
            "cloud review",
            RemoteTaskState::deferred(RemoteTaskType::AutofixPr, None),
        );
        manager.store.write_task(&task).expect("write task");

        let report = manager.reconcile_tasks(None).expect("reconcile tasks");
        let reconciled = report.tasks.into_iter().next().expect("task");

        assert_eq!(report.changed, 0);
        assert_eq!(report.finished, 0);
        assert_eq!(report.fresh_heartbeats, 0);
        assert_eq!(report.stale_heartbeats, 0);
        assert_eq!(report.missing_heartbeats, 0);
        assert_eq!(reconciled.kind, TaskKind::RemoteAgent);
        assert_eq!(reconciled.status, TaskStatus::Pending);
        assert_eq!(reconciled.status_message, task.status_message);
        assert!(!manager.logs_dir().join(format!("{}.log", task.id)).exists());
    }

    #[test]
    fn task_summary_counts_remote_tasks() {
        let tasks = vec![
            TaskState::pending_shell("shell", "true", "/workspace"),
            TaskState::pending_agent(
                "agent",
                AgentTaskState::metadata_only("agent", "prompt", None, None),
            ),
            TaskState::recorded_remote(
                "remote",
                RemoteTaskState::deferred(RemoteTaskType::BackgroundPr, None),
            ),
        ];

        let summary = TaskSummary::from_tasks(&tasks);

        assert_eq!(summary.total, 3);
        assert_eq!(summary.shell, 1);
        assert_eq!(summary.agents, 1);
        assert_eq!(summary.remote, 1);
    }

    fn write_agent_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}")).expect("write agent script");
        #[cfg(unix)]
        {
            let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).expect("set agent script permissions");
        }
        path
    }

    // ── remove_task / prune_tasks tests ──────────────────────────────────────

    /// Helper: write a terminal shell task with a log and exit file into the
    /// manager's store so we can test artifact cleanup without spawning a
    /// real process.
    fn seed_completed_task(manager: &TaskManager, label: &str) -> TaskState {
        let dir = manager.storage_dir();
        let mut task = TaskState::pending_shell(label, "true", dir);
        task.mark_finished(TaskStatus::Completed, Some(0), Some("done".into()));
        task.output_log = Some(manager.store.paths().task_log_path(task.id));
        manager.store.write_task(&task).expect("write task");
        manager
            .store
            .append_log(task.id, "output\n")
            .expect("write log");
        manager
            .store
            .write_exit_code(task.id, 0)
            .expect("write exit code");
        task
    }

    /// Helper: write a running shell task (no real process, pid points to the
    /// test process so it appears alive to reconcile).
    fn seed_running_task(manager: &TaskManager, label: &str) -> TaskState {
        let dir = manager.storage_dir();
        let mut task = TaskState::pending_shell(label, "sleep 999", dir);
        task.status = TaskStatus::Running;
        task.pid = Some(std::process::id()); // test process – always alive
        task.output_log = Some(manager.store.paths().task_log_path(task.id));
        manager.store.write_task(&task).expect("write task");
        manager
            .store
            .append_log(task.id, "running\n")
            .expect("write log");
        task
    }

    #[test]
    fn remove_completed_task_deletes_artifacts() {
        let dir = unique_test_dir("task-remove-completed");
        let manager = TaskManager::new(&dir);
        let task = seed_completed_task(&manager, "cleanup-me");

        let state_path = manager.store.paths().task_state_path(task.id);
        let log_path = manager.store.paths().task_log_path(task.id);
        assert!(state_path.exists(), "state file should exist before remove");
        assert!(log_path.exists(), "log file should exist before remove");

        let removed = manager
            .remove_task(task.id, false)
            .expect("remove completed task");
        assert_eq!(removed.id, task.id);

        assert!(
            !state_path.exists(),
            "state file should be gone after remove"
        );
        assert!(!log_path.exists(), "log file should be gone after remove");
    }

    #[test]
    fn remove_running_task_rejected_without_force() {
        let dir = unique_test_dir("task-remove-running-reject");
        let manager = TaskManager::new(&dir);
        let task = seed_running_task(&manager, "still-running");

        let err = manager
            .remove_task(task.id, false)
            .expect_err("should reject active task");
        assert!(
            err.to_string().contains("still active"),
            "error should mention active: {err}"
        );

        // The state file must still be present – nothing was deleted.
        let state_path = manager.store.paths().task_state_path(task.id);
        assert!(
            state_path.exists(),
            "state file must remain when remove is rejected"
        );
    }

    #[test]
    fn remove_running_task_succeeds_with_force() {
        let dir = unique_test_dir("task-remove-running-force");
        let manager = TaskManager::new(&dir);
        let task = seed_running_task(&manager, "force-delete-me");

        manager
            .remove_task(task.id, true)
            .expect("force remove should succeed");

        let state_path = manager.store.paths().task_state_path(task.id);
        assert!(
            !state_path.exists(),
            "state file should be gone after forced remove"
        );
    }

    #[test]
    fn prune_terminal_skips_running_tasks() {
        let dir = unique_test_dir("task-prune-skips-running");
        let manager = TaskManager::new(&dir);

        let completed = seed_completed_task(&manager, "done-task");
        let running = seed_running_task(&manager, "live-task");

        let report = manager.prune_tasks(false).expect("prune all terminal");

        assert_eq!(report.removed.len(), 1, "only the completed task removed");
        assert_eq!(report.removed[0].id, completed.id);
        assert_eq!(report.skipped_active.len(), 1, "running task skipped");
        assert_eq!(report.skipped_active[0].id, running.id);

        // Running task's state file must still be on disk.
        let running_state = manager.store.paths().task_state_path(running.id);
        assert!(
            running_state.exists(),
            "running task state must survive prune"
        );
    }

    #[test]
    fn prune_completed_only_leaves_failed_tasks() {
        let dir = unique_test_dir("task-prune-completed-only");
        let manager = TaskManager::new(&dir);

        // Seed a completed task.
        let completed = seed_completed_task(&manager, "ok-task");

        // Seed a failed task by writing it directly.
        let mut failed = TaskState::pending_shell("failed-task", "false", &dir);
        failed.mark_finished(TaskStatus::Failed, Some(1), Some("error".into()));
        failed.output_log = Some(manager.store.paths().task_log_path(failed.id));
        manager
            .store
            .write_task(&failed)
            .expect("write failed task");

        let report = manager.prune_tasks(true).expect("prune completed only");

        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].id, completed.id);
        // The failed task ends up in terminal-skipped because completed_only=true.
        assert!(
            report.skipped_terminal.iter().any(|t| t.id == failed.id),
            "failed task should be in terminal skipped list under --completed"
        );
        assert!(report.skipped_active.is_empty());

        let failed_state = manager.store.paths().task_state_path(failed.id);
        assert!(
            failed_state.exists(),
            "failed task state must not be deleted under --completed"
        );
    }
}
