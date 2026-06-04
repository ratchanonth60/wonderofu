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
    FeatureFlag, RemoteTaskState, Result, TaskId, TaskState, TaskStatus, TodoTaskEntry,
    TodoTaskStatus, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId,
    WonderError, new_todo_task_id,
};
use wonder_of_u_storage::{TaskStore, TodoTaskStore};

use crate::{app_root, base_spec, parse_input, require_non_empty_text};

const DEFAULT_TASK_OUTPUT_LINES: u32 = 50;
const DEFAULT_TASK_OUTPUT_TIMEOUT_MS: u64 = 30_000;
const TASK_OUTPUT_POLL_INTERVAL: Duration = Duration::from_millis(100);
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
    /// Task ids to remove from this task's `blocks` list (bidirectional).
    #[serde(default, rename = "removeBlocks", alias = "remove_blocks")]
    pub remove_blocks: Option<Vec<String>>,
    /// Task ids to remove from this task's `blocked_by` list (bidirectional).
    #[serde(default, rename = "removeBlockedBy", alias = "remove_blocked_by")]
    pub remove_blocked_by: Option<Vec<String>>,
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
            reject_self_dependency("addBlocks", &self.task_id, add_blocks)?;
            updated = true;
        }
        if let Some(add_blocked_by) = &self.add_blocked_by {
            require_non_empty_string_list("task_update", "addBlockedBy", add_blocked_by)?;
            reject_self_dependency("addBlockedBy", &self.task_id, add_blocked_by)?;
            updated = true;
        }
        if let Some(remove_blocks) = &self.remove_blocks {
            require_non_empty_string_list("task_update", "removeBlocks", remove_blocks)?;
            updated = true;
        }
        if let Some(remove_blocked_by) = &self.remove_blocked_by {
            require_non_empty_string_list("task_update", "removeBlockedBy", remove_blocked_by)?;
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
        // Not concurrency_safe: writes to per-session file without a lock.
        spec.required_features.insert(FeatureFlag::TodoV2);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<TaskCreateInput>("task_create", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<TaskCreateInput>("task_create", &input)?;
        input.validate()?;

        let app_root = app_root()?;
        let store = TodoTaskStore::new(&app_root);
        let mut list = store.read_or_default(context.session_id)?;

        let task_id = new_todo_task_id();
        let mut entry = TodoTaskEntry::new(&task_id, &input.subject, &input.description);
        if let Some(active_form) = input.active_form {
            entry.active_form = Some(active_form);
        }
        if let Some(metadata) = input.metadata {
            entry.metadata = metadata;
        }

        list.tasks.insert(task_id.clone(), entry.clone());
        list.touch();
        store.write(&list)?;

        let content = format!("Task #{task_id} created successfully: {}", entry.subject);
        let mut result = ToolResult::success(use_id, content);
        result.metadata = json!({
            "supported": true,
            "tool": "task_create",
            "tool_family": "todo_v2_task_list",
            "task_id": task_id,
            "session_id": context.session_id.to_string(),
            "task": serde_json::to_value(&entry).unwrap_or_default(),
        });
        Ok(result)
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
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<TaskGetInput>("task_get", &input)?;
        input.validate()?;

        let app_root = app_root()?;
        let store = TodoTaskStore::new(&app_root);
        let list = store.read_or_default(context.session_id)?;

        // Return a model-friendly success (not an error) when the task is not
        // found — this prevents the runtime from cancelling sibling tool calls.
        let Some(entry) = list.tasks.get(&input.task_id) else {
            let mut result =
                ToolResult::success(use_id, format!("Task '{}' not found.", input.task_id));
            result.metadata = json!({
                "supported": true,
                "tool": "task_get",
                "tool_family": "todo_v2_task_list",
                "task_id": input.task_id,
                "session_id": context.session_id.to_string(),
                "not_found": true,
            });
            return Ok(result);
        };

        let content = format_task_entry(entry);
        let mut result = ToolResult::success(use_id, content);
        result.metadata = json!({
            "supported": true,
            "tool": "task_get",
            "tool_family": "todo_v2_task_list",
            "task_id": input.task_id,
            "session_id": context.session_id.to_string(),
            "not_found": false,
            "task": serde_json::to_value(entry).unwrap_or_default(),
        });
        Ok(result)
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
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        parse_input::<TaskListInput>("task_list", &input)?;

        let app_root = app_root()?;
        let store = TodoTaskStore::new(&app_root);
        let list = store.read_or_default(context.session_id)?;

        // Deleted tasks and internal (`metadata._internal = true`) tasks are
        // hidden from model-facing output; they stay on disk for audit.
        let visible: Vec<&TodoTaskEntry> = list
            .tasks
            .values()
            .filter(|e| e.status != TodoTaskStatus::Deleted)
            .filter(|e| !is_internal_task(e))
            .collect();

        let content = format_task_list(&visible, &list.tasks);
        let mut result = ToolResult::success(use_id, content);
        result.metadata = json!({
            "supported": true,
            "tool": "task_list",
            "tool_family": "todo_v2_task_list",
            "count": visible.len(),
            "session_id": context.session_id.to_string(),
        });
        Ok(result)
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
                .property(
                    "removeBlocks",
                    string_array_schema("task ids to remove from this task's blocks list"),
                )
                .property(
                    "removeBlockedBy",
                    string_array_schema("task ids to remove from this task's blocked_by list"),
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
        // Not concurrency_safe: writes to per-session file without a lock.
        spec.destructive = true;
        spec.required_features.insert(FeatureFlag::TodoV2);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<TaskUpdateInput>("task_update", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<TaskUpdateInput>("task_update", &input)?;
        input.validate()?;

        // Collect the names of fields being changed before we consume `input`.
        let changed_fields = collect_changed_fields(&input);

        let app_root = app_root()?;
        let store = TodoTaskStore::new(&app_root);
        let mut list = store.read_or_default(context.session_id)?;

        // Validate that the target task exists.
        if !list.tasks.contains_key(&input.task_id) {
            return Err(WonderError::not_found("todo task", &input.task_id));
        }

        // Collect all dependency ids that must already exist in the list.
        let dep_ids_to_check: Vec<&str> = [
            input.add_blocks.as_deref().unwrap_or(&[]),
            input.add_blocked_by.as_deref().unwrap_or(&[]),
            input.remove_blocks.as_deref().unwrap_or(&[]),
            input.remove_blocked_by.as_deref().unwrap_or(&[]),
        ]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect();

        for dep_id in &dep_ids_to_check {
            if !list.tasks.contains_key(*dep_id) {
                return Err(WonderError::not_found("todo task (dependency)", *dep_id));
            }
        }

        // Apply scalar field updates to the target task.
        {
            let entry = list.tasks.get_mut(&input.task_id).expect("checked above");

            if let Some(subject) = input.subject {
                entry.subject = subject;
            }
            if let Some(description) = input.description {
                entry.description = description;
            }
            if let Some(active_form) = input.active_form {
                entry.active_form = Some(active_form);
            }
            if let Some(status) = input.status {
                entry.status = match status {
                    TaskUpdateStatus::Pending => TodoTaskStatus::Pending,
                    TaskUpdateStatus::InProgress => TodoTaskStatus::InProgress,
                    TaskUpdateStatus::Completed => TodoTaskStatus::Completed,
                    TaskUpdateStatus::Deleted => TodoTaskStatus::Deleted,
                };
            }
            if let Some(owner) = input.owner {
                entry.owner = Some(owner);
            }
            // Merge metadata; null values remove keys.
            if let Some(new_meta) = input.metadata {
                for (key, value) in new_meta {
                    if value.is_null() {
                        entry.metadata.remove(&key);
                    } else {
                        entry.metadata.insert(key, value);
                    }
                }
            }
            entry.touch();
        }

        // Bidirectional addBlocks: A.blocks += B, B.blocked_by += A.
        if let Some(add_blocks) = &input.add_blocks {
            for other_id in add_blocks {
                let entry = list.tasks.get_mut(&input.task_id).expect("checked above");
                if !entry.blocks.contains(other_id) {
                    entry.blocks.push(other_id.clone());
                }
                let other = list.tasks.get_mut(other_id).expect("checked above");
                if !other.blocked_by.contains(&input.task_id) {
                    other.blocked_by.push(input.task_id.clone());
                    other.touch();
                }
            }
        }

        // Bidirectional addBlockedBy: A.blocked_by += B, B.blocks += A.
        if let Some(add_blocked_by) = &input.add_blocked_by {
            for other_id in add_blocked_by {
                let entry = list.tasks.get_mut(&input.task_id).expect("checked above");
                if !entry.blocked_by.contains(other_id) {
                    entry.blocked_by.push(other_id.clone());
                }
                let other = list.tasks.get_mut(other_id).expect("checked above");
                if !other.blocks.contains(&input.task_id) {
                    other.blocks.push(input.task_id.clone());
                    other.touch();
                }
            }
        }

        // Bidirectional removeBlocks: A.blocks -= B, B.blocked_by -= A.
        if let Some(remove_blocks) = &input.remove_blocks {
            for other_id in remove_blocks {
                let entry = list.tasks.get_mut(&input.task_id).expect("checked above");
                entry.blocks.retain(|id| id != other_id);
                let other = list.tasks.get_mut(other_id).expect("checked above");
                other.blocked_by.retain(|id| id != &input.task_id);
                other.touch();
            }
        }

        // Bidirectional removeBlockedBy: A.blocked_by -= B, B.blocks -= A.
        if let Some(remove_blocked_by) = &input.remove_blocked_by {
            for other_id in remove_blocked_by {
                let entry = list.tasks.get_mut(&input.task_id).expect("checked above");
                entry.blocked_by.retain(|id| id != other_id);
                let other = list.tasks.get_mut(other_id).expect("checked above");
                other.blocks.retain(|id| id != &input.task_id);
                other.touch();
            }
        }

        list.touch();
        store.write(&list)?;

        let updated_entry = list.tasks.get(&input.task_id).expect("entry still present");
        let fields_display = changed_fields.join(", ");
        let content = format!("Updated task #{}: {fields_display}", input.task_id);
        let mut result = ToolResult::success(use_id, content);
        result.metadata = json!({
            "supported": true,
            "tool": "task_update",
            "tool_family": "todo_v2_task_list",
            "task_id": input.task_id,
            "session_id": context.session_id.to_string(),
            "changed_fields": changed_fields,
            "task": serde_json::to_value(updated_entry).unwrap_or_default(),
        });
        Ok(result)
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

fn reject_self_dependency(field: &str, task_id: &str, values: &[String]) -> Result<()> {
    if values.iter().any(|value| value == task_id) {
        return Err(WonderError::validation(format!(
            "task_update {field} cannot reference the task itself"
        )));
    }
    Ok(())
}

// ── Human-readable result text helpers ────────────────────────────────────────

/// Returns `true` if the entry carries `metadata._internal = true`.
///
/// Internal tasks are used for bookkeeping and should never appear in
/// model-facing output.
fn is_internal_task(entry: &TodoTaskEntry) -> bool {
    entry
        .metadata
        .get("_internal")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Returns `true` if the blocker task is resolved (completed or deleted).
fn is_blocker_resolved(
    blocker_id: &str,
    all_tasks: &std::collections::BTreeMap<String, TodoTaskEntry>,
) -> bool {
    match all_tasks.get(blocker_id) {
        Some(t) => matches!(
            t.status,
            TodoTaskStatus::Completed | TodoTaskStatus::Deleted
        ),
        // Unknown blocker — treat as unresolved to be conservative.
        None => false,
    }
}

/// Formats a single [`TodoTaskEntry`] as a human-readable multi-line string.
fn format_task_entry(entry: &TodoTaskEntry) -> String {
    let status_label = todo_status_label(entry.status);
    let mut lines = vec![
        format!("Task #{}", entry.task_id),
        format!("  Subject:     {}", entry.subject),
        format!("  Status:      {status_label}"),
        format!("  Description: {}", entry.description),
    ];
    if let Some(form) = &entry.active_form {
        lines.push(format!("  Active form: {form}"));
    }
    if let Some(owner) = &entry.owner {
        lines.push(format!("  Owner:       {owner}"));
    }
    if !entry.blocks.is_empty() {
        lines.push(format!("  Blocks:      {}", entry.blocks.join(", ")));
    }
    if !entry.blocked_by.is_empty() {
        lines.push(format!("  Blocked by:  {}", entry.blocked_by.join(", ")));
    }
    lines.join("\n")
}

/// Formats the visible task list as human-readable text.
///
/// `blocked_by` entries whose blocker is already completed or deleted are
/// omitted — they are resolved and no longer meaningful to the model.
fn format_task_list(
    visible: &[&TodoTaskEntry],
    all_tasks: &std::collections::BTreeMap<String, TodoTaskEntry>,
) -> String {
    if visible.is_empty() {
        return "No tasks.".to_string();
    }

    let mut lines: Vec<String> = Vec::new();
    for entry in visible {
        let status_label = todo_status_label(entry.status);
        let mut line = format!("[{}] #{} — {}", status_label, entry.task_id, entry.subject);

        // Show active blockers only — skip resolved ones.
        let active_blockers: Vec<&str> = entry
            .blocked_by
            .iter()
            .filter(|id| !is_blocker_resolved(id, all_tasks))
            .map(String::as_str)
            .collect();

        if !active_blockers.is_empty() {
            line.push_str(&format!(" (blocked by: {})", active_blockers.join(", ")));
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// Collects the names of fields being changed in a [`TaskUpdateInput`].
fn collect_changed_fields(input: &TaskUpdateInput) -> Vec<String> {
    let mut fields = Vec::new();
    if input.subject.is_some() {
        fields.push("subject".to_string());
    }
    if input.description.is_some() {
        fields.push("description".to_string());
    }
    if input.active_form.is_some() {
        fields.push("active_form".to_string());
    }
    if input.status.is_some() {
        fields.push("status".to_string());
    }
    if input.add_blocks.is_some() {
        fields.push("blocks".to_string());
    }
    if input.add_blocked_by.is_some() {
        fields.push("blocked_by".to_string());
    }
    if input.remove_blocks.is_some() {
        fields.push("blocks".to_string());
    }
    if input.remove_blocked_by.is_some() {
        fields.push("blocked_by".to_string());
    }
    if input.owner.is_some() {
        fields.push("owner".to_string());
    }
    if input.metadata.is_some() {
        fields.push("metadata".to_string());
    }
    // Deduplicate while preserving order (e.g. both addBlocks and removeBlocks).
    let mut seen = std::collections::HashSet::new();
    fields.retain(|f| seen.insert(f.clone()));
    fields
}

const fn todo_status_label(status: TodoTaskStatus) -> &'static str {
    match status {
        TodoTaskStatus::Pending => "pending",
        TodoTaskStatus::InProgress => "in_progress",
        TodoTaskStatus::Completed => "completed",
        TodoTaskStatus::Deleted => "deleted",
    }
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
    // Include progress metrics only when data exists so legacy consumers
    // that check for key absence are unaffected.
    let progress = if task.progress.has_data() {
        Some(serde_json::json!({
            "tool_use_count": task.progress.tool_use_count,
            "token_count": task.progress.token_count,
            "last_tool_name": task.progress.last_tool_name,
        }))
    } else {
        None
    };
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
        "progress": progress,
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
    use std::fs;

    use futures::executor::block_on;
    use serde_json::json;
    use wonder_of_u_core::{FeatureSet, PermissionMode, SessionId, TaskState, TodoTaskStatus};
    use wonder_of_u_core::{RemoteTaskState, RemoteTaskType};
    use wonder_of_u_storage::TodoTaskStore;
    use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

    use super::*;

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
    fn task_update_validation_rejects_self_dependencies() {
        let tool = TaskUpdateTool;
        let add_blocks_error = tool
            .validate_input(&json!({
                "taskId": "task-1",
                "addBlocks": ["task-1"],
            }))
            .expect_err("self block should be rejected");
        assert!(add_blocks_error.to_string().contains("task itself"));

        let add_blocked_by_error = tool
            .validate_input(&json!({
                "taskId": "task-1",
                "addBlockedBy": ["task-1"],
            }))
            .expect_err("self blocked-by should be rejected");
        assert!(add_blocked_by_error.to_string().contains("task itself"));
    }

    #[test]
    fn task_update_validation_rejects_blank_remove_dependency_ids() {
        let tool = TaskUpdateTool;
        let error = tool
            .validate_input(&json!({
                "taskId": "task-1",
                "removeBlocks": [" "],
            }))
            .expect_err("blank remove dependency");

        assert!(error.to_string().contains("removeBlocks"));
    }

    // ── Happy-path execute tests (require WONDER_OF_U_STORAGE_DIR) ─────────

    fn tool_context_with_dir(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd: dir.to_path_buf(),
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            provider: None,
            model: None,
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: None,
            progress_tx: None,
            interaction_rx: None,
            fork_context: None,
        }
    }

    fn set_storage_dir(dir: &std::path::Path) -> EnvVarGuard {
        EnvVarGuard::set("WONDER_OF_U_STORAGE_DIR", dir.as_os_str())
    }

    #[tokio::test]
    async fn task_create_execute_persists_task_and_returns_success() {
        let dir = unique_test_dir("tools-task-create-happy");
        let _storage_guard = set_storage_dir(&dir);
        let ctx = tool_context_with_dir(&dir);
        let session_id = ctx.session_id;

        let result = TaskCreateTool
            .execute(
                ctx,
                ToolUseId::new(),
                json!({
                    "subject": "Ship release",
                    "description": "Run the release workflow",
                }),
            )
            .await
            .expect("execute");

        assert!(result.success, "expected success, got: {}", result.content);
        assert_eq!(result.metadata["supported"], true);
        assert_eq!(result.metadata["tool"], "task_create");
        assert_eq!(result.metadata["tool_family"], "todo_v2_task_list");

        let task_id = result.metadata["task_id"]
            .as_str()
            .expect("task_id in metadata");
        assert!(task_id.starts_with("todo-"), "id should have todo- prefix");

        // Verify it was persisted.
        let store = TodoTaskStore::new(&dir);
        let list = store.read_or_default(session_id).expect("read list");
        assert!(
            list.tasks.contains_key(task_id),
            "task not found in persisted list"
        );
        let entry = &list.tasks[task_id];
        assert_eq!(entry.subject, "Ship release");
        assert_eq!(entry.description, "Run the release workflow");
        assert_eq!(entry.status, TodoTaskStatus::Pending);
    }

    #[tokio::test]
    async fn task_list_execute_returns_visible_tasks_and_hides_deleted() {
        let dir = unique_test_dir("tools-task-list-filter");
        let _storage_guard = set_storage_dir(&dir);
        let ctx = tool_context_with_dir(&dir);
        let session_id = ctx.session_id;

        // Seed two tasks: one normal, one deleted.
        let store = TodoTaskStore::new(&dir);
        let mut list = wonder_of_u_core::TodoTaskList::empty(session_id);
        let e1 = wonder_of_u_core::TodoTaskEntry::new("todo-aaaa", "Task A", "desc A");
        let mut e2 = wonder_of_u_core::TodoTaskEntry::new("todo-bbbb", "Task B", "desc B");
        e2.status = TodoTaskStatus::Deleted;
        list.tasks.insert("todo-aaaa".into(), e1.clone());
        list.tasks.insert("todo-bbbb".into(), e2.clone());
        store.write(&list).expect("seed list");

        let result = TaskListTool
            .execute(ctx, ToolUseId::new(), json!({}))
            .await
            .expect("execute");

        assert!(result.success);
        assert_eq!(result.metadata["count"], 1);
        assert!(
            result.content.contains("Task A"),
            "visible task should appear"
        );
        assert!(
            !result.content.contains("Task B"),
            "deleted task should be hidden"
        );
    }

    #[tokio::test]
    async fn task_get_execute_returns_task_or_not_found() {
        let dir = unique_test_dir("tools-task-get");
        let _storage_guard = set_storage_dir(&dir);
        let ctx = tool_context_with_dir(&dir);
        let session_id = ctx.session_id;

        let store = TodoTaskStore::new(&dir);
        let mut list = wonder_of_u_core::TodoTaskList::empty(session_id);
        list.tasks.insert(
            "todo-get-1".into(),
            wonder_of_u_core::TodoTaskEntry::new("todo-get-1", "Get me", "by id"),
        );
        store.write(&list).expect("seed");

        // Happy path.
        let ctx2 = ToolContext {
            session_id,
            ..tool_context_with_dir(&dir)
        };
        let result = TaskGetTool
            .execute(ctx2, ToolUseId::new(), json!({ "taskId": "todo-get-1" }))
            .await
            .expect("execute get");
        assert!(result.success);
        assert!(result.content.contains("Get me"));

        // Not found — must succeed (model-friendly) with not_found=true in metadata.
        let ctx3 = ToolContext {
            session_id,
            ..tool_context_with_dir(&dir)
        };
        let not_found_result = TaskGetTool
            .execute(ctx3, ToolUseId::new(), json!({ "taskId": "todo-missing" }))
            .await
            .expect("not-found should be a success result, not an error");
        assert!(
            not_found_result.success,
            "not-found result must be success to avoid cancelling sibling calls"
        );
        assert_eq!(
            not_found_result.metadata["not_found"], true,
            "metadata must carry not_found=true"
        );
        assert!(
            not_found_result.content.contains("not found"),
            "content should mention 'not found'"
        );
    }

    #[tokio::test]
    async fn task_update_execute_applies_scalar_changes() {
        let dir = unique_test_dir("tools-task-update-scalar");
        let _storage_guard = set_storage_dir(&dir);
        let store = TodoTaskStore::new(&dir);
        let session_id = SessionId::new();
        let mut list = wonder_of_u_core::TodoTaskList::empty(session_id);
        list.tasks.insert(
            "todo-u1".into(),
            wonder_of_u_core::TodoTaskEntry::new("todo-u1", "Old subject", "Old desc"),
        );
        store.write(&list).expect("seed");

        let ctx = ToolContext {
            session_id,
            ..tool_context_with_dir(&dir)
        };
        let result = TaskUpdateTool
            .execute(
                ctx,
                ToolUseId::new(),
                json!({
                    "taskId": "todo-u1",
                    "subject": "New subject",
                    "status": "in_progress",
                }),
            )
            .await
            .expect("execute update");

        assert!(result.success);
        let updated = store.read_or_default(session_id).expect("read");
        let entry = &updated.tasks["todo-u1"];
        assert_eq!(entry.subject, "New subject");
        assert_eq!(entry.status, TodoTaskStatus::InProgress);
    }

    #[tokio::test]
    async fn task_update_execute_null_metadata_removes_keys() {
        let dir = unique_test_dir("tools-task-update-meta-null");
        let _storage_guard = set_storage_dir(&dir);
        let store = TodoTaskStore::new(&dir);
        let session_id = SessionId::new();
        let mut list = wonder_of_u_core::TodoTaskList::empty(session_id);
        let mut entry = wonder_of_u_core::TodoTaskEntry::new("todo-mn1", "S", "D");
        entry
            .metadata
            .insert("keep".into(), serde_json::json!("yes"));
        entry
            .metadata
            .insert("drop".into(), serde_json::json!("no"));
        list.tasks.insert("todo-mn1".into(), entry);
        store.write(&list).expect("seed");

        let ctx = ToolContext {
            session_id,
            ..tool_context_with_dir(&dir)
        };
        block_on(TaskUpdateTool.execute(
            ctx,
            ToolUseId::new(),
            json!({
                "taskId": "todo-mn1",
                "metadata": { "drop": null, "added": "value" },
            }),
        ))
        .expect("execute");

        let updated = store.read_or_default(session_id).expect("read");
        let meta = &updated.tasks["todo-mn1"].metadata;
        assert!(meta.contains_key("keep"), "keep should remain");
        assert!(!meta.contains_key("drop"), "drop should be removed");
        assert_eq!(meta["added"], serde_json::json!("value"));
    }

    #[test]
    fn task_update_execute_bidirectional_add_blocks() {
        let dir = unique_test_dir("tools-task-update-add-blocks");
        let _storage_guard = set_storage_dir(&dir);
        let store = TodoTaskStore::new(&dir);
        let session_id = SessionId::new();
        let mut list = wonder_of_u_core::TodoTaskList::empty(session_id);
        list.tasks.insert(
            "todo-a".into(),
            wonder_of_u_core::TodoTaskEntry::new("todo-a", "A", "task A"),
        );
        list.tasks.insert(
            "todo-b".into(),
            wonder_of_u_core::TodoTaskEntry::new("todo-b", "B", "task B"),
        );
        store.write(&list).expect("seed");

        let ctx = ToolContext {
            session_id,
            ..tool_context_with_dir(&dir)
        };
        block_on(TaskUpdateTool.execute(
            ctx,
            ToolUseId::new(),
            json!({ "taskId": "todo-a", "addBlocks": ["todo-b"] }),
        ))
        .expect("execute");

        let updated = store.read_or_default(session_id).expect("read");
        assert!(
            updated.tasks["todo-a"]
                .blocks
                .contains(&"todo-b".to_string())
        );
        assert!(
            updated.tasks["todo-b"]
                .blocked_by
                .contains(&"todo-a".to_string())
        );
    }

    #[test]
    fn task_update_execute_bidirectional_remove_blocks_deduplicates() {
        let dir = unique_test_dir("tools-task-update-remove-blocks");
        let _storage_guard = set_storage_dir(&dir);
        let store = TodoTaskStore::new(&dir);
        let session_id = SessionId::new();
        let mut list = wonder_of_u_core::TodoTaskList::empty(session_id);
        let mut a = wonder_of_u_core::TodoTaskEntry::new("todo-c", "C", "task C");
        let mut b = wonder_of_u_core::TodoTaskEntry::new("todo-d", "D", "task D");
        a.blocks.push("todo-d".into());
        b.blocked_by.push("todo-c".into());
        list.tasks.insert("todo-c".into(), a);
        list.tasks.insert("todo-d".into(), b);
        store.write(&list).expect("seed");

        let ctx = ToolContext {
            session_id,
            ..tool_context_with_dir(&dir)
        };
        block_on(TaskUpdateTool.execute(
            ctx,
            ToolUseId::new(),
            json!({ "taskId": "todo-c", "removeBlocks": ["todo-d"] }),
        ))
        .expect("execute");

        let updated = store.read_or_default(session_id).expect("read");
        assert!(
            !updated.tasks["todo-c"]
                .blocks
                .contains(&"todo-d".to_string())
        );
        assert!(
            !updated.tasks["todo-d"]
                .blocked_by
                .contains(&"todo-c".to_string())
        );
    }

    #[test]
    fn task_update_execute_errors_on_missing_dependency_id() {
        let dir = unique_test_dir("tools-task-update-missing-dep");
        let _storage_guard = set_storage_dir(&dir);
        let store = TodoTaskStore::new(&dir);
        let session_id = SessionId::new();
        let mut list = wonder_of_u_core::TodoTaskList::empty(session_id);
        list.tasks.insert(
            "todo-e".into(),
            wonder_of_u_core::TodoTaskEntry::new("todo-e", "E", "task E"),
        );
        store.write(&list).expect("seed");

        let ctx = ToolContext {
            session_id,
            ..tool_context_with_dir(&dir)
        };
        let err = block_on(TaskUpdateTool.execute(
            ctx,
            ToolUseId::new(),
            json!({ "taskId": "todo-e", "addBlocks": ["todo-nonexistent"] }),
        ))
        .expect_err("should error on missing dep");
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn task_create_is_not_concurrency_safe() {
        let spec = TaskCreateTool.spec();
        assert!(
            !spec.concurrency_safe,
            "task_create must not be concurrency_safe"
        );
    }

    #[test]
    fn task_update_is_not_concurrency_safe() {
        let spec = TaskUpdateTool.spec();
        assert!(
            !spec.concurrency_safe,
            "task_update must not be concurrency_safe"
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

    // ── Result-text tests ──────────────────────────────────────────────────────

    #[test]
    fn task_create_result_text_contains_id_and_subject() {
        let dir = unique_test_dir("tools-task-create-text");
        let _storage_guard = set_storage_dir(&dir);
        let ctx = tool_context_with_dir(&dir);

        let result = block_on(TaskCreateTool.execute(
            ctx,
            ToolUseId::new(),
            json!({ "subject": "Deploy hotfix", "description": "Push the patch" }),
        ))
        .expect("execute");

        let task_id = result.metadata["task_id"].as_str().expect("task_id");
        assert!(
            result.content.contains(task_id),
            "content should contain the task id"
        );
        assert!(
            result.content.contains("Deploy hotfix"),
            "content should contain the subject"
        );
        assert!(
            result.content.contains("created successfully"),
            "content should say 'created successfully'"
        );
        // Structured task is still accessible via metadata.
        assert!(
            result.metadata["task"].is_object(),
            "metadata must carry structured task"
        );
    }

    #[test]
    fn task_get_result_text_is_human_readable() {
        let dir = unique_test_dir("tools-task-get-text");
        let _storage_guard = set_storage_dir(&dir);
        let ctx = tool_context_with_dir(&dir);
        let session_id = ctx.session_id;

        let store = TodoTaskStore::new(&dir);
        let mut list = wonder_of_u_core::TodoTaskList::empty(session_id);
        list.tasks.insert(
            "todo-hr1".into(),
            wonder_of_u_core::TodoTaskEntry::new("todo-hr1", "Write docs", "All public APIs"),
        );
        store.write(&list).expect("seed");

        let result = block_on(TaskGetTool.execute(
            ToolContext {
                session_id,
                ..tool_context_with_dir(&dir)
            },
            ToolUseId::new(),
            json!({ "taskId": "todo-hr1" }),
        ))
        .expect("execute");

        assert!(result.success);
        assert!(result.content.contains("todo-hr1"), "id in content");
        assert!(result.content.contains("Write docs"), "subject in content");
        assert!(result.content.contains("pending"), "status in content");
        // The raw JSON blob must NOT appear as the top-level content.
        assert!(
            !result.content.starts_with('{'),
            "content must not be raw JSON"
        );
        // Structured task is in metadata.
        assert!(result.metadata["task"].is_object(), "task in metadata");
        assert_eq!(result.metadata["not_found"], false);
    }

    #[test]
    fn task_list_result_text_is_human_readable() {
        let dir = unique_test_dir("tools-task-list-text");
        let _storage_guard = set_storage_dir(&dir);
        let ctx = tool_context_with_dir(&dir);
        let session_id = ctx.session_id;

        let store = TodoTaskStore::new(&dir);
        let mut list = wonder_of_u_core::TodoTaskList::empty(session_id);
        list.tasks.insert(
            "todo-lt1".into(),
            wonder_of_u_core::TodoTaskEntry::new("todo-lt1", "Write tests", "all the tests"),
        );
        store.write(&list).expect("seed");

        let result =
            block_on(TaskListTool.execute(ctx, ToolUseId::new(), json!({}))).expect("execute");

        assert!(result.success);
        // Must be a line-based summary, not a raw JSON array.
        // Our format starts with "[status] #id — subject", not "[{…}]".
        assert!(!result.content.starts_with("[{"), "not raw JSON array");
        assert!(result.content.contains("Write tests"), "subject present");
        assert!(result.content.contains("todo-lt1"), "id present");
    }

    #[test]
    fn task_list_filters_internal_tasks() {
        let dir = unique_test_dir("tools-task-list-internal");
        let _storage_guard = set_storage_dir(&dir);
        let ctx = tool_context_with_dir(&dir);
        let session_id = ctx.session_id;

        let store = TodoTaskStore::new(&dir);
        let mut list = wonder_of_u_core::TodoTaskList::empty(session_id);
        let visible = wonder_of_u_core::TodoTaskEntry::new("todo-vis1", "Visible task", "public");
        let mut internal =
            wonder_of_u_core::TodoTaskEntry::new("todo-int1", "Internal task", "hidden");
        internal
            .metadata
            .insert("_internal".into(), serde_json::json!(true));
        list.tasks.insert("todo-vis1".into(), visible);
        list.tasks.insert("todo-int1".into(), internal);
        store.write(&list).expect("seed");

        let result =
            block_on(TaskListTool.execute(ctx, ToolUseId::new(), json!({}))).expect("execute");

        assert!(result.success);
        assert_eq!(result.metadata["count"], 1, "only visible task counted");
        assert!(
            result.content.contains("Visible task"),
            "visible task in output"
        );
        assert!(
            !result.content.contains("Internal task"),
            "internal task must be hidden"
        );
    }

    #[test]
    fn task_list_filters_resolved_blocked_by_entries() {
        let dir = unique_test_dir("tools-task-list-blockers");
        let _storage_guard = set_storage_dir(&dir);
        let ctx = tool_context_with_dir(&dir);
        let session_id = ctx.session_id;

        let store = TodoTaskStore::new(&dir);
        let mut list = wonder_of_u_core::TodoTaskList::empty(session_id);

        // Blocker A is completed.
        let mut blocker_done =
            wonder_of_u_core::TodoTaskEntry::new("todo-blk-done", "Blocker done", "finished");
        blocker_done.status = TodoTaskStatus::Completed;

        // Blocker B is still pending.
        let blocker_pending =
            wonder_of_u_core::TodoTaskEntry::new("todo-blk-pending", "Blocker pending", "wait");

        // Task blocked by both.
        let mut blocked =
            wonder_of_u_core::TodoTaskEntry::new("todo-blocked", "Blocked task", "waiting");
        blocked.blocked_by.push("todo-blk-done".into());
        blocked.blocked_by.push("todo-blk-pending".into());

        list.tasks.insert("todo-blk-done".into(), blocker_done);
        list.tasks
            .insert("todo-blk-pending".into(), blocker_pending);
        list.tasks.insert("todo-blocked".into(), blocked);
        store.write(&list).expect("seed");

        let result =
            block_on(TaskListTool.execute(ctx, ToolUseId::new(), json!({}))).expect("execute");

        assert!(result.success);
        // Only the active (pending) blocker should appear in the "blocked by:"
        // annotation for the blocked task itself.  The completed blocker
        // appears as its own visible line (completed tasks remain visible) but
        // must NOT be listed in the "(blocked by: …)" annotation.
        let blocked_line = result
            .content
            .lines()
            .find(|l| l.contains("Blocked task"))
            .expect("line for Blocked task");
        assert!(
            blocked_line.contains("todo-blk-pending"),
            "active blocker must appear in annotation"
        );
        assert!(
            !blocked_line.contains("todo-blk-done"),
            "resolved blocker must be omitted from annotation"
        );
    }

    #[test]
    fn task_update_result_text_lists_changed_fields() {
        let dir = unique_test_dir("tools-task-update-text");
        let _storage_guard = set_storage_dir(&dir);
        let store = TodoTaskStore::new(&dir);
        let session_id = SessionId::new();
        let mut list = wonder_of_u_core::TodoTaskList::empty(session_id);
        list.tasks.insert(
            "todo-upd-txt".into(),
            wonder_of_u_core::TodoTaskEntry::new("todo-upd-txt", "Old", "desc"),
        );
        store.write(&list).expect("seed");

        let ctx = ToolContext {
            session_id,
            ..tool_context_with_dir(&dir)
        };
        let result = block_on(TaskUpdateTool.execute(
            ctx,
            ToolUseId::new(),
            json!({
                "taskId": "todo-upd-txt",
                "subject": "New subject",
                "status": "completed",
            }),
        ))
        .expect("execute");

        assert!(result.success);
        assert!(
            result.content.contains("todo-upd-txt"),
            "content should reference the task id"
        );
        assert!(
            result.content.contains("subject"),
            "subject in changed list"
        );
        assert!(result.content.contains("status"), "status in changed list");
        // Structured task still accessible via metadata.
        assert!(result.metadata["task"].is_object(), "task in metadata");
        let changed: Vec<&str> = result.metadata["changed_fields"]
            .as_array()
            .expect("changed_fields array")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(changed.contains(&"subject"));
        assert!(changed.contains(&"status"));
    }

    #[test]
    fn is_internal_task_detects_internal_flag() {
        let mut entry = wonder_of_u_core::TodoTaskEntry::new("todo-int-flag", "S", "D");
        assert!(!is_internal_task(&entry), "no flag → not internal");

        entry
            .metadata
            .insert("_internal".into(), serde_json::json!(true));
        assert!(is_internal_task(&entry), "_internal=true → internal");

        *entry.metadata.get_mut("_internal").unwrap() = serde_json::json!(false);
        assert!(!is_internal_task(&entry), "_internal=false → not internal");
    }

    #[test]
    fn is_blocker_resolved_treats_unknown_id_as_unresolved() {
        let all: std::collections::BTreeMap<String, wonder_of_u_core::TodoTaskEntry> =
            std::collections::BTreeMap::new();
        assert!(
            !is_blocker_resolved("todo-ghost", &all),
            "unknown blocker is unresolved"
        );
    }
}
