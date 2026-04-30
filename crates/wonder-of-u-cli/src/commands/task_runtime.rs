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
    AgentRuntime, AgentTaskState, CommandContext, PermissionDecision, PermissionMode,
    PermissionRequest, Result, TaskId, TaskKind, TaskState, TaskStatus, ToolPermissionContext,
    WonderError, resolve_path,
};
use wonder_of_u_storage::TaskStore;

const HEARTBEAT_INTERVAL_SECS: u64 = 2;
const STALE_HEARTBEAT_AFTER_SECS: i64 = 8;
const CLI_BIN_OVERRIDE_ENV: &str = "WONDER_OF_U_CLI_BIN";

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

#[derive(Clone, Copy, Debug, Default)]
struct TaskReconcileOutcome {
    changed: bool,
    finished: bool,
}

impl TaskSummary {
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
            }
        }
        summary
    }
}

impl TaskManager {
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            store: TaskStore::new(base_dir),
        }
    }

    #[must_use]
    pub fn storage_dir(&self) -> &Path {
        self.store.paths().base_dir()
    }

    #[must_use]
    pub fn logs_dir(&self) -> PathBuf {
        self.store.paths().task_logs_dir()
    }

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

    pub fn get_task(&self, task_id: TaskId) -> Result<TaskState> {
        let task = self.store.read_task(task_id)?;
        Ok(self.reconcile_task(task, OffsetDateTime::now_utc())?.0)
    }

    pub fn read_log_tail(&self, task_id: TaskId, tail_lines: usize) -> Result<Vec<String>> {
        self.store.read_log_tail(task_id, tail_lines)
    }

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

    pub fn start_agent_task(&self, launch: AgentTaskLaunch) -> Result<TaskState> {
        ensure_directory(&launch.cwd)?;
        let command = agent_prompt_command(self.storage_dir(), &launch)?;
        let description = launch
            .description
            .clone()
            .unwrap_or_else(|| launch.name.clone());
        let mut task = TaskState::pending_agent(
            description,
            AgentTaskState::prompt_subprocess(
                launch.name,
                launch.prompt,
                launch.provider,
                launch.model,
            ),
        );
        task.cwd = Some(launch.cwd.clone());
        task.command = Some(command);
        task.output_log = Some(self.store.paths().task_log_path(task.id));
        self.store.write_task(&task)?;
        self.store
            .append_log(task.id, render_agent_task_header(&task))?;

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

    pub fn stop_task(&self, task_id: TaskId, force: bool) -> Result<TaskState> {
        let task = self.get_task(task_id)?;
        if task.status.is_terminal() {
            return Ok(task);
        }

        match task.kind {
            TaskKind::LocalShell | TaskKind::LocalAgent => self.stop_process_task(task, force),
        }
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
    value.replace('\n', " ").replace('\r', " ")
}

fn task_supports_process_runtime(kind: TaskKind) -> bool {
    matches!(kind, TaskKind::LocalShell | TaskKind::LocalAgent)
}

fn task_runtime_subject(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::LocalShell => "task",
        TaskKind::LocalAgent => "local agent",
    }
}

fn task_kind_label(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::LocalShell => "local_shell",
        TaskKind::LocalAgent => "local_agent",
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
        AgentRuntime::Deferred => "deferred",
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
            if libc::setsid() == -1 {
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
            "heartbeat() {{ date -u +\"%Y-%m-%dT%H:%M:%SZ\" > \"$heartbeat_file\"; }}; ",
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

#[cfg(unix)]
fn process_is_zombie(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    stat.rsplit_once(") ")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        == Some("Z")
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

    use wonder_of_u_core::{CommandContext, FeatureSet, PermissionMode, SessionId};
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
            })
            .expect("start agent task");

        assert_eq!(task.kind, TaskKind::LocalAgent);
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
}
