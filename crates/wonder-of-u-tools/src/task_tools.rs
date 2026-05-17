//! Source-compatible task list tool specs plus background task runtime access.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    thread,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use wonder_of_u_core::{
    FeatureFlag, RemoteTaskState, Result, TaskId, TaskState, TaskStatus, Tool, ToolContext,
    ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId, WonderError,
};
use wonder_of_u_storage::TaskStore;

use crate::{app_root, base_spec, parse_input, require_non_empty_text};

const DEFAULT_TASK_OUTPUT_LINES: u32 = 50;
const DEFAULT_TASK_OUTPUT_TIMEOUT_MS: u64 = 30_000;
const TASK_OUTPUT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const SOURCE_TASK_RUNTIME_UNAVAILABLE: &str = concat!(
    "todo-v2 task tools are blocked in the Rust port: ToolContext does not expose a logical ",
    "task-list state, and TaskStore persists background runtime tasks rather than source-compatible ",
    "TaskCreate/TaskUpdate entries"
);
const SOURCE_TASK_RUNTIME_BLOCKERS: [&str; 2] = [
    "ToolContext does not expose a logical task-list state for todo-v2 tools",
    "TaskStore persists background runtime tasks rather than source-compatible task-list entries",
];
/// Represents task create input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCreateInput {
    /// Stores the subject
    pub subject: String,
    /// Stores the description
    pub description: String,
    /// Stores the active form
    #[serde(default, rename = "activeForm", alias = "active_form")]
    pub active_form: Option<String>,
    /// Stores the metadata
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

impl TaskCreateInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("task_create", "subject", &self.subject)?;
        require_non_empty_text("task_create", "description", &self.description)?;
        if let Some(active_form) = &self.active_form {
            require_non_empty_text("task_create", "activeForm", active_form)?;
        }
        Ok(())
    }
}
/// Represents task get input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskGetInput {
    /// Stores the task identifier
    #[serde(rename = "taskId", alias = "task_id")]
    pub task_id: String,
}

impl TaskGetInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("task_get", "taskId", &self.task_id)
    }
}
/// Represents task list input
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskListInput {}
/// Enumerates task update status
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskUpdateStatus {
    /// Represents pending
    Pending,
    /// Represents in progress
    InProgress,
    /// Represents completed
    Completed,
    /// Represents deleted
    Deleted,
}
/// Represents task update input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskUpdateInput {
    /// Stores the task identifier
    #[serde(rename = "taskId", alias = "task_id")]
    pub task_id: String,
    /// Stores the subject
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Stores the description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Stores the active form
    #[serde(default, rename = "activeForm", alias = "active_form")]
    pub active_form: Option<String>,
    /// Stores the status
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskUpdateStatus>,
    /// Stores the add blocks
    #[serde(default, rename = "addBlocks", alias = "add_blocks")]
    pub add_blocks: Option<Vec<String>>,
    /// Stores the add blocked by
    #[serde(default, rename = "addBlockedBy", alias = "add_blocked_by")]
    pub add_blocked_by: Option<Vec<String>>,
    /// Stores the owner
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Stores the metadata
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

impl TaskUpdateInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("task_update", "taskId", &self.task_id)?;

        let mut updated = false;
        if let Some(subject) = &self.subject {
            require_non_empty_text("task_update", "subject", subject)?;
            updated = true;
        }
        if let Some(description) = &self.description {
            require_non_empty_text("task_update", "description", description)?;
            updated = true;
        }
        if let Some(active_form) = &self.active_form {
            require_non_empty_text("task_update", "activeForm", active_form)?;
            updated = true;
        }
        if self.status.is_some() {
            updated = true;
        }
        if let Some(add_blocks) = &self.add_blocks {
            require_non_empty_string_list("task_update", "addBlocks", add_blocks)?;
            updated = true;
        }
        if let Some(add_blocked_by) = &self.add_blocked_by {
            require_non_empty_string_list("task_update", "addBlockedBy", add_blocked_by)?;
            updated = true;
        }
        if let Some(owner) = &self.owner {
            require_non_empty_text("task_update", "owner", owner)?;
            updated = true;
        }
        if self.metadata.is_some() {
            updated = true;
        }

        if !updated {
            return Err(WonderError::validation(
                "task_update requires at least one field to update",
            ));
        }

        Ok(())
    }
}
/// Represents task output input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskOutputInput {
    /// Stores the task identifier
    pub task_id: String,
    /// Stores the lines
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<u32>,
    /// Stores the block
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<bool>,
    /// Stores the timeout
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

impl TaskOutputInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("task_output", "task_id", &self.task_id)?;
        if self.lines == Some(0) {
            return Err(WonderError::validation(
                "task_output lines must be greater than zero",
            ));
        }
        if self.timeout == Some(0) {
            return Err(WonderError::validation(
                "task_output timeout must be greater than zero",
            ));
        }
        if self.timeout.is_some() && self.block != Some(true) {
            return Err(WonderError::validation(
                "task_output timeout requires block=true",
            ));
        }
        Ok(())
    }

    fn lines(&self) -> usize {
        self.lines.unwrap_or(DEFAULT_TASK_OUTPUT_LINES) as usize
    }

    fn block(&self) -> bool {
        self.block.unwrap_or(false)
    }

    fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout.unwrap_or(DEFAULT_TASK_OUTPUT_TIMEOUT_MS))
    }
}
/// Represents task stop input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskStopInput {
    /// Stores the task identifier
    #[serde(alias = "shell_id")]
    pub task_id: String,
}

impl TaskStopInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("task_stop", "task_id", &self.task_id)
    }
}
/// Represents task create tool
#[derive(Debug, Default)]
pub struct TaskCreateTool;
/// Represents task get tool
#[derive(Debug, Default)]
pub struct TaskGetTool;
/// Represents task list tool
#[derive(Debug, Default)]
pub struct TaskListTool;
/// Represents task update tool
#[derive(Debug, Default)]
pub struct TaskUpdateTool;
/// Represents task output tool
#[derive(Debug, Default)]
pub struct TaskOutputTool;
/// Represents task stop tool
#[derive(Debug, Default)]
pub struct TaskStopTool;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskStopOutcome {
    SignalSent,
    LegacyCancelSignal,
    AlreadyTerminal(TaskStatus),
}

impl TaskStopOutcome {
    fn content(self) -> String {
        match self {
            Self::SignalSent => "task stop signal sent".into(),
            Self::LegacyCancelSignal => "task cancel signal sent".into(),
            Self::AlreadyTerminal(status) => format!("task already {}", task_status_label(status)),
        }
    }

    fn metadata(self) -> Value {
        match self {
            Self::SignalSent => json!({
                "requested": true,
                "legacy_fallback": false,
                "already_terminal": false,
            }),
            Self::LegacyCancelSignal => json!({
                "requested": true,
                "legacy_fallback": true,
                "already_terminal": false,
            }),
            Self::AlreadyTerminal(status) => json!({
                "requested": false,
                "legacy_fallback": false,
                "already_terminal": true,
                "status": task_status_label(status),
            }),
        }
    }
}

#[async_trait]
impl Tool for TaskCreateTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "task_create",
            "Create a new task in the task list",
            ToolKind::Task,
        )
        .with_input_schema(
            ToolSchema::object()
                .property("subject", ToolSchema::string("a brief title for the task"))
                .property("description", ToolSchema::string("what needs to be done"))
                .property(
                    "activeForm",
                    ToolSchema::string(
                        "present continuous form shown while the task is in progress",
                    ),
                )
                .property(
                    "metadata",
                    arbitrary_metadata_schema("metadata to attach to the task"),
                )
                .required("subject")
                .required("description"),
        );
        spec.aliases.push("TaskCreate".into());
        spec.concurrency_safe = true;
        spec.required_features.insert(FeatureFlag::TodoV2);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<TaskCreateInput>("task_create", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<TaskCreateInput>("task_create", &input)?;
        input.validate()?;
        Ok(unsupported_task_list_result(use_id, "task_create"))
    }
}

#[async_trait]
impl Tool for TaskGetTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "task_get",
            "Get a task by ID from the task list",
            ToolKind::Task,
        )
        .with_input_schema(
            ToolSchema::object()
                .property("taskId", ToolSchema::string("the task id to retrieve"))
                .required("taskId"),
        );
        spec.aliases.push("TaskGet".into());
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec.required_features.insert(FeatureFlag::TodoV2);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<TaskGetInput>("task_get", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<TaskGetInput>("task_get", &input)?;
        input.validate()?;
        Ok(unsupported_task_list_result(use_id, "task_get"))
    }
}

#[async_trait]
impl Tool for TaskListTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "task_list",
            "List all tasks in the task list",
            ToolKind::Task,
        )
        .with_input_schema(ToolSchema::object());
        spec.aliases.push("TaskList".into());
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec.required_features.insert(FeatureFlag::TodoV2);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<TaskListInput>("task_list", input).map(|_| ())
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        parse_input::<TaskListInput>("task_list", &input)?;
        Ok(unsupported_task_list_result(use_id, "task_list"))
    }
}

#[async_trait]
impl Tool for TaskUpdateTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "task_update",
            "Update a task in the task list",
            ToolKind::Task,
        )
        .with_input_schema(
            ToolSchema::object()
                .property("taskId", ToolSchema::string("the task id to update"))
                .property("subject", ToolSchema::string("new subject for the task"))
                .property(
                    "description",
                    ToolSchema::string("new description for the task"),
                )
                .property(
                    "activeForm",
                    ToolSchema::string(
                        "present continuous form shown while the task is in progress",
                    ),
                )
                .property(
                    "status",
                    ToolSchema::enumeration(
                        "new task status",
                        ["pending", "in_progress", "completed", "deleted"],
                    ),
                )
                .property(
                    "addBlocks",
                    string_array_schema("task ids that cannot start until this one completes"),
                )
                .property(
                    "addBlockedBy",
                    string_array_schema("task ids that must complete before this one starts"),
                )
                .property("owner", ToolSchema::string("new owner for the task"))
                .property(
                    "metadata",
                    arbitrary_metadata_schema(
                        "metadata keys to merge into the task; null values remove keys",
                    ),
                )
                .required("taskId"),
        );
        spec.aliases.push("TaskUpdate".into());
        spec.concurrency_safe = true;
        spec.destructive = true;
        spec.required_features.insert(FeatureFlag::TodoV2);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<TaskUpdateInput>("task_update", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<TaskUpdateInput>("task_update", &input)?;
        input.validate()?;
        Ok(unsupported_task_list_result(use_id, "task_update"))
    }
}

#[async_trait]
impl Tool for TaskOutputTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "task_output",
            "Read recent output from a background task",
            ToolKind::Task,
        )
        .with_input_schema(
            ToolSchema::object()
                .property("task_id", ToolSchema::string("background task id"))
                .property(
                    "lines",
                    ToolSchema::integer("number of trailing log lines to return"),
                )
                .property(
                    "block",
                    ToolSchema::boolean(
                        "when true, wait up to timeout for running tasks that have not produced output yet",
                    ),
                )
                .property(
                    "timeout",
                    ToolSchema::integer(
                        "optional wait timeout in milliseconds when block=true",
                    ),
                )
                .required("task_id"),
        );
        spec.aliases.push("TaskOutput".into());
        spec.aliases.push("AgentOutputTool".into());
        spec.aliases.push("BashOutputTool".into());
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec.required_features.insert(FeatureFlag::BackgroundTasks);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<TaskOutputInput>("task_output", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<TaskOutputInput>("task_output", &input)?;
        input.validate()?;

        let output = read_task_output(&app_root()?, &input)?;
        let mut result = ToolResult::success(use_id, output.content);
        result.metadata = output.metadata;
        Ok(result)
    }
}

#[async_trait]
impl Tool for TaskStopTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec("task_stop", "Request task cancellation", ToolKind::Task)
            .with_input_schema(
            ToolSchema::object()
                .property("task_id", ToolSchema::string("background task id"))
                .property(
                    "shell_id",
                    ToolSchema::string(
                        "source-compatible legacy task id field accepted as an alias for task_id",
                    ),
                )
                .required("task_id"),
        );
        spec.aliases.push("TaskStop".into());
        spec.aliases.push("KillShell".into());
        spec.destructive = true;
        spec.required_features.insert(FeatureFlag::BackgroundTasks);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<TaskStopInput>("task_stop", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<TaskStopInput>("task_stop", &input)?;
        input.validate()?;

        let outcome = request_task_stop(&app_root()?, &input.task_id)?;
        let mut result = ToolResult::success(use_id, outcome.content());
        result.metadata = outcome.metadata();
        Ok(result)
    }
}

fn arbitrary_metadata_schema(description: &str) -> Value {
    json!({
        "type": "object",
        "description": description,
        "additionalProperties": true,
    })
}

fn string_array_schema(description: &str) -> Value {
    json!({
        "type": "array",
        "description": description,
        "items": {
            "type": "string",
        }
    })
}

fn require_non_empty_string_list(tool_name: &str, field: &str, values: &[String]) -> Result<()> {
    for value in values {
        require_non_empty_text(tool_name, field, value)?;
    }
    Ok(())
}

fn unsupported_task_list_result(use_id: ToolUseId, tool_name: &str) -> ToolResult {
    let mut result = ToolResult::failure(
        use_id,
        format!("{tool_name} is unavailable: {SOURCE_TASK_RUNTIME_UNAVAILABLE}"),
    );
    result.metadata = json!({
        "supported": false,
        "tool": tool_name,
        "tool_family": "todo_v2_task_list",
        "reason": SOURCE_TASK_RUNTIME_UNAVAILABLE,
        "blocked_by": SOURCE_TASK_RUNTIME_BLOCKERS,
    });
    result
}

#[derive(Clone, Debug, PartialEq)]
struct TaskOutputRead {
    content: String,
    metadata: Value,
}

fn read_task_output(app_root: &Path, input: &TaskOutputInput) -> Result<TaskOutputRead> {
    if let Some(output) = read_persisted_task_output(app_root, input)? {
        return Ok(output);
    }
    if let Some(content) = read_legacy_task_output(app_root, &input.task_id, input.lines())? {
        return Ok(TaskOutputRead {
            content,
            metadata: json!({
                "task_id": input.task_id,
                "lines": input.lines(),
                "block": input.block(),
                "timed_out": false,
                "not_ready": false,
                "legacy": true,
            }),
        });
    }
    Err(WonderError::not_found("task", &input.task_id))
}

fn read_persisted_task_output(
    app_root: &Path,
    input: &TaskOutputInput,
) -> Result<Option<TaskOutputRead>> {
    let Ok(task_id) = TaskId::parse(&input.task_id) else {
        return Ok(None);
    };
    let store = TaskStore::new(app_root);
    let line_limit = input.lines().max(1);

    let mut snapshot = match read_persisted_task_snapshot(&store, task_id, line_limit)? {
        Some(snapshot) => snapshot,
        None => return Ok(None),
    };

    if input.block() && snapshot.content.is_empty() && !snapshot.task.status.is_terminal() {
        let deadline = Instant::now() + input.timeout();
        loop {
            if Instant::now() >= deadline {
                let metadata =
                    task_output_metadata(&snapshot.task, task_id, line_limit, true, true, false);
                return Ok(Some(TaskOutputRead {
                    content: snapshot.content,
                    metadata,
                }));
            }

            thread::sleep(TASK_OUTPUT_POLL_INTERVAL);
            snapshot = read_persisted_task_snapshot(&store, task_id, line_limit)?
                .ok_or_else(|| WonderError::not_found("task", task_id.to_string()))?;
            if !snapshot.content.is_empty() || snapshot.task.status.is_terminal() {
                break;
            }
        }
    }

    let metadata = task_output_metadata(
        &snapshot.task,
        task_id,
        line_limit,
        false,
        snapshot.content.is_empty() && !snapshot.task.status.is_terminal(),
        false,
    );
    Ok(Some(TaskOutputRead {
        content: snapshot.content,
        metadata,
    }))
}

#[derive(Clone, Debug, PartialEq)]
struct PersistedTaskOutputSnapshot {
    task: TaskState,
    content: String,
}

fn read_persisted_task_snapshot(
    store: &TaskStore,
    task_id: TaskId,
    lines: usize,
) -> Result<Option<PersistedTaskOutputSnapshot>> {
    let task = match store.read_task(task_id) {
        Ok(task) => task,
        Err(WonderError::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(error),
    };
    let content = match store.read_log_tail(task_id, lines.max(1)) {
        Ok(lines) => lines.join("\n"),
        Err(WonderError::NotFound { .. }) => String::new(),
        Err(error) => return Err(error),
    };
    Ok(Some(PersistedTaskOutputSnapshot { task, content }))
}

fn task_output_metadata(
    task: &TaskState,
    task_id: TaskId,
    lines: usize,
    timed_out: bool,
    not_ready: bool,
    legacy: bool,
) -> Value {
    json!({
        "task_id": task_id,
        "status": task_status_label(task.status),
        "pid": task.pid,
        "exit_code": task.exit_code,
        "running": !task.status.is_terminal(),
        "completed": task.status.is_terminal(),
        "timed_out": timed_out,
        "not_ready": not_ready,
        "lines": lines,
        "legacy": legacy,
        "cwd": task.cwd.as_ref().map(|cwd| cwd.display().to_string()),
    })
}

fn read_legacy_task_output(app_root: &Path, task_id: &str, lines: usize) -> Result<Option<String>> {
    let path = legacy_task_dir(app_root, task_id).join("output.log");
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)?;
    let mut content_lines = content.lines().collect::<Vec<_>>();
    let line_limit = lines.max(1);
    if content_lines.len() > line_limit {
        content_lines.drain(0..content_lines.len() - line_limit);
    }
    Ok(Some(content_lines.join("\n")))
}

fn request_task_stop(app_root: &Path, task_id: &str) -> Result<TaskStopOutcome> {
    if let Some(outcome) = request_persisted_task_stop(app_root, task_id)? {
        return Ok(outcome);
    }
    request_legacy_task_stop(app_root, task_id)
}

fn request_persisted_task_stop(app_root: &Path, task_id: &str) -> Result<Option<TaskStopOutcome>> {
    let Ok(task_id) = TaskId::parse(task_id) else {
        return Ok(None);
    };

    let store = TaskStore::new(app_root);
    let task = match store.read_task(task_id) {
        Ok(task) => task,
        Err(WonderError::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(error),
    };

    if task.status.is_terminal() {
        return Ok(Some(TaskStopOutcome::AlreadyTerminal(task.status)));
    }

    if task.kind == wonder_of_u_core::TaskKind::RemoteAgent {
        return Err(WonderError::validation(
            task.remote
                .as_ref()
                .map(RemoteTaskState::stop_error_message)
                .unwrap_or_else(|| {
                    "cannot stop remote task: remote task transport is unavailable in this Rust runtime"
                        .into()
                }),
        ));
    }

    let pid = task.pid.ok_or_else(|| {
        WonderError::validation(format!(
            "task_stop cannot stop task {} because it has no process id",
            task.id
        ))
    })?;
    send_stop_signal(pid)?;
    Ok(Some(TaskStopOutcome::SignalSent))
}

fn request_legacy_task_stop(app_root: &Path, task_id: &str) -> Result<TaskStopOutcome> {
    let dir = legacy_task_dir(app_root, task_id);
    if !dir.exists() {
        return Err(WonderError::not_found("task", task_id));
    }
    fs::write(dir.join("cancel"), "cancel\n")?;
    Ok(TaskStopOutcome::LegacyCancelSignal)
}

fn send_stop_signal(pid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        let output = ProcessCommand::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .output()?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(WonderError::internal(if stderr.is_empty() {
            format!("failed to send stop signal to task process {pid}")
        } else {
            format!("failed to send stop signal to task process {pid}: {stderr}")
        }))
    }

    #[cfg(windows)]
    {
        let output = ProcessCommand::new("taskkill")
            .arg("/PID")
            .arg(pid.to_string())
            .output()?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(WonderError::internal(if stderr.is_empty() {
            format!("failed to request task termination for process {pid}")
        } else {
            format!("failed to request task termination for process {pid}: {stderr}")
        }))
    }

    #[cfg(not(any(unix, windows)))]
    {
        Err(WonderError::internal(format!(
            "task_stop is not supported on this platform for process {pid}"
        )))
    }
}

fn legacy_task_dir(app_root: &Path, task_id: &str) -> PathBuf {
    app_root.join("tasks").join(task_id)
}

const fn task_status_label(status: TaskStatus) -> &'static str {
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
    use std::{fs, path::PathBuf};

    use futures::executor::block_on;
    use serde_json::json;
    use wonder_of_u_core::{FeatureSet, PermissionMode, SessionId, TaskState};
    use wonder_of_u_core::{RemoteTaskState, RemoteTaskType};
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn tool_context(cwd: PathBuf) -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd,
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            provider: None,
            model: None,
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: None,
            fork_context: None,
        }
    }

    #[test]
    fn task_create_validation_rejects_blank_subject() {
        let tool = TaskCreateTool;
        let error = tool
            .validate_input(&json!({
                "subject": "   ",
                "description": "ship it",
            }))
            .expect_err("blank subject");

        assert!(error.to_string().contains("subject"));
    }

    #[test]
    fn task_get_validation_rejects_blank_task_id() {
        let tool = TaskGetTool;
        let error = tool
            .validate_input(&json!({ "taskId": "" }))
            .expect_err("blank task id");

        assert!(error.to_string().contains("taskId"));
    }

    #[test]
    fn task_list_validation_rejects_unknown_fields() {
        let tool = TaskListTool;
        let error = tool
            .validate_input(&json!({ "unexpected": true }))
            .expect_err("unexpected field");

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn task_update_validation_requires_a_change() {
        let tool = TaskUpdateTool;
        let error = tool
            .validate_input(&json!({ "taskId": "task-1" }))
            .expect_err("missing update");

        assert!(error.to_string().contains("at least one field"));
    }

    #[test]
    fn task_update_validation_rejects_blank_dependency_ids() {
        let tool = TaskUpdateTool;
        let error = tool
            .validate_input(&json!({
                "taskId": "task-1",
                "addBlockedBy": ["  "],
            }))
            .expect_err("blank dependency");

        assert!(error.to_string().contains("addBlockedBy"));
    }

    #[test]
    fn task_create_execute_returns_explicit_unsupported_failure() {
        let tool = TaskCreateTool;
        let result = block_on(tool.execute(
            tool_context(unique_test_dir("tools-task-create-unsupported")),
            ToolUseId::new(),
            json!({
                "subject": "Ship release",
                "description": "Run the release workflow",
            }),
        ))
        .expect("execute");

        assert!(!result.success);
        assert!(result.content.contains("task_create is unavailable"));
        assert_eq!(result.metadata["supported"], false);
        assert_eq!(result.metadata["tool"], "task_create");
        assert_eq!(result.metadata["tool_family"], "todo_v2_task_list");
        assert_eq!(
            result.metadata["blocked_by"],
            json!([
                "ToolContext does not expose a logical task-list state for todo-v2 tools",
                "TaskStore persists background runtime tasks rather than source-compatible task-list entries",
            ])
        );
    }

    #[test]
    fn task_output_validation_rejects_zero_lines() {
        let tool = TaskOutputTool;
        let error = tool
            .validate_input(&json!({ "task_id": "task-1", "lines": 0 }))
            .expect_err("invalid lines");

        assert!(error.to_string().contains("lines"));
    }

    #[test]
    fn task_output_accepts_block_and_rejects_timeout_without_block() {
        let tool = TaskOutputTool;
        tool.validate_input(&json!({
            "task_id": "task-1",
            "block": true,
        }))
        .expect("block=true should be accepted");

        let error = tool
            .validate_input(&json!({
                "task_id": "task-1",
                "timeout": 10,
            }))
            .expect_err("timeout without block should be rejected");

        assert!(error.to_string().contains("block=true"));
    }

    #[test]
    fn task_output_reads_persisted_runtime_log_tail() {
        let dir = unique_test_dir("tools-task-output-store");
        let store = TaskStore::new(&dir);
        let task = TaskState::pending_shell("run tests", "cargo test", dir.clone());
        store.write_task(&task).expect("task");
        store.append_log(task.id, "one\ntwo\nthree\n").expect("log");

        let output = read_task_output(
            &dir,
            &TaskOutputInput {
                task_id: task.id.to_string(),
                lines: Some(2),
                block: None,
                timeout: None,
            },
        )
        .expect("tail");

        assert_eq!(output.content, "two\nthree");
        assert_eq!(output.metadata["status"], "pending");
        assert_eq!(output.metadata["not_ready"], false);
    }

    #[test]
    fn task_output_reads_legacy_log_tail() {
        let dir = unique_test_dir("tools-task-output-legacy");
        let task_dir = dir.join("tasks").join("task-1");
        fs::create_dir_all(&task_dir).expect("task dir");
        fs::write(task_dir.join("output.log"), "one\ntwo\nthree\n").expect("log");

        let output = read_task_output(
            &dir,
            &TaskOutputInput {
                task_id: "task-1".into(),
                lines: Some(2),
                block: None,
                timeout: None,
            },
        )
        .expect("tail");

        assert_eq!(output.content, "two\nthree");
        assert_eq!(output.metadata["legacy"], true);
    }

    #[test]
    fn task_output_block_waits_until_log_is_available() {
        let dir = unique_test_dir("tools-task-output-block");
        let store = TaskStore::new(&dir);
        let mut task = TaskState::pending_shell("run tests", "cargo test", dir.clone());
        task.mark_running(None, None, None, Some("running".into()));
        store.write_task(&task).expect("task");

        let task_id = task.id;
        let store_for_thread = store.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            store_for_thread
                .append_log(task_id, "ready\n")
                .expect("append delayed log");
        });

        let output = read_task_output(
            &dir,
            &TaskOutputInput {
                task_id: task.id.to_string(),
                lines: Some(10),
                block: Some(true),
                timeout: Some(1_000),
            },
        )
        .expect("blocked output");
        writer.join().expect("writer joins");

        assert_eq!(output.content, "ready");
        assert_eq!(output.metadata["timed_out"], false);
        assert_eq!(output.metadata["not_ready"], false);
    }

    #[test]
    fn task_output_block_times_out_when_running_without_output() {
        let dir = unique_test_dir("tools-task-output-timeout");
        let store = TaskStore::new(&dir);
        let mut task = TaskState::pending_shell("run tests", "cargo test", dir.clone());
        task.mark_running(None, None, None, Some("running".into()));
        store.write_task(&task).expect("task");

        let output = read_task_output(
            &dir,
            &TaskOutputInput {
                task_id: task.id.to_string(),
                lines: Some(10),
                block: Some(true),
                timeout: Some(10),
            },
        )
        .expect("timeout output");

        assert!(output.content.is_empty());
        assert_eq!(output.metadata["timed_out"], true);
        assert_eq!(output.metadata["not_ready"], true);
    }

    #[test]
    fn task_stop_reports_terminal_persisted_tasks_without_signalling() {
        let dir = unique_test_dir("tools-task-stop-terminal");
        let store = TaskStore::new(&dir);
        let mut task = TaskState::pending_shell("run tests", "cargo test", dir.clone());
        task.mark_finished(TaskStatus::Completed, Some(0), Some("done".into()));
        store.write_task(&task).expect("task");

        let outcome = request_task_stop(&dir, &task.id.to_string()).expect("stop outcome");

        assert_eq!(
            outcome,
            TaskStopOutcome::AlreadyTerminal(TaskStatus::Completed)
        );
    }

    #[test]
    fn task_stop_rejects_remote_backends_without_signalling() {
        let dir = unique_test_dir("tools-task-stop-remote");
        let store = TaskStore::new(&dir);
        let task = TaskState::recorded_remote(
            "cloud review",
            RemoteTaskState::deferred(RemoteTaskType::Ultrareview, None),
        );
        store.write_task(&task).expect("task");

        let error = request_task_stop(&dir, &task.id.to_string()).expect_err("remote backend");

        assert!(error.to_string().contains("cannot stop ultrareview task"));
    }

    #[test]
    fn task_stop_legacy_fallback_errors_for_missing_task() {
        let dir = unique_test_dir("tools-task-stop");
        let error = request_legacy_task_stop(&dir, "missing").expect_err("missing task");

        assert!(error.to_string().contains("task not found"));
    }

    #[test]
    fn task_stop_accepts_legacy_shell_id_field() {
        let tool = TaskStopTool;

        tool.validate_input(&json!({ "shell_id": "task-1" }))
            .expect("legacy shell id");
    }
}
