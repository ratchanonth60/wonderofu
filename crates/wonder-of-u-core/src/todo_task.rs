//! Logical todo-v2 task list models.
//!
//! These types are intentionally separate from the background task runtime
//! ([`crate::TaskState`], [`crate::TaskStatus`]) which tracks subprocess
//! execution.  The logical task list is a per-session, model-facing todo
//! board persisted under `tasks/lists/{session_id}.json`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::SessionId;

/// Schema version stored in every [`TodoTaskList`] file.
pub const TODO_TASK_LIST_SCHEMA_VERSION: u16 = 1;

fn default_schema_version() -> u16 {
    TODO_TASK_LIST_SCHEMA_VERSION
}

/// Generates a new logical task id that cannot be parsed as a background [`crate::TaskId`].
///
/// The `todo-` prefix ensures the id is rejected by [`crate::TaskId::parse`],
/// keeping the two id spaces disjoint.
#[must_use]
pub fn new_todo_task_id() -> String {
    format!("todo-{}", Uuid::new_v4())
}

/// Status of a logical todo-v2 task.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoTaskStatus {
    /// Waiting to be started.
    Pending,
    /// Currently being worked on.
    InProgress,
    /// Finished successfully.
    Completed,
    /// Soft-deleted; retained on disk for audit but hidden from list output.
    Deleted,
}

/// A single entry in a session's todo-v2 task list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TodoTaskEntry {
    /// Unique task identifier, prefixed with `todo-` so it cannot be parsed as
    /// a background [`crate::TaskId`].
    pub task_id: String,
    /// Brief title.
    pub subject: String,
    /// Full description of what needs to be done.
    pub description: String,
    /// Present-continuous form displayed while the task is in-progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
    /// Current status.
    pub status: TodoTaskStatus,
    /// Task ids that cannot start until this task completes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<String>,
    /// Task ids that must complete before this task starts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
    /// Optional owner label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Arbitrary key-value metadata attached to the task.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub metadata: Map<String, Value>,
    /// Creation timestamp (RFC 3339).
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Last-updated timestamp (RFC 3339).
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl TodoTaskEntry {
    /// Creates a new pending task with the given id, subject, and description.
    #[must_use]
    pub fn new(
        task_id: impl Into<String>,
        subject: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            task_id: task_id.into(),
            subject: subject.into(),
            description: description.into(),
            active_form: None,
            status: TodoTaskStatus::Pending,
            blocks: Vec::new(),
            blocked_by: Vec::new(),
            owner: None,
            metadata: Map::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Updates the `updated_at` field to now.
    pub fn touch(&mut self) {
        self.updated_at = OffsetDateTime::now_utc();
    }
}

/// Per-session todo-v2 task list persisted at `tasks/lists/{session_id}.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TodoTaskList {
    /// Storage schema guard – always [`TODO_TASK_LIST_SCHEMA_VERSION`].
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    /// Owning session.
    pub session_id: SessionId,
    /// `task_id` → entry map; `BTreeMap` gives deterministic serialization order.
    #[serde(default)]
    pub tasks: BTreeMap<String, TodoTaskEntry>,
    /// Last-updated timestamp (RFC 3339).
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl TodoTaskList {
    /// Returns an empty list for `session_id`.
    #[must_use]
    pub fn empty(session_id: SessionId) -> Self {
        Self {
            schema_version: TODO_TASK_LIST_SCHEMA_VERSION,
            session_id,
            tasks: BTreeMap::new(),
            updated_at: OffsetDateTime::now_utc(),
        }
    }

    /// Updates the `updated_at` field to now.
    pub fn touch(&mut self) {
        self.updated_at = OffsetDateTime::now_utc();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_todo_task_id_has_todo_prefix() {
        let id = new_todo_task_id();
        assert!(id.starts_with("todo-"), "expected 'todo-' prefix, got {id}");
    }

    #[test]
    fn new_todo_task_id_is_not_a_valid_background_task_id() {
        use crate::TaskId;
        let id = new_todo_task_id();
        assert!(
            TaskId::parse(&id).is_err(),
            "todo id should not parse as TaskId"
        );
    }

    #[test]
    fn todo_task_list_empty_roundtrips_via_json() {
        let session_id = SessionId::new();
        let list = TodoTaskList::empty(session_id);
        let json = serde_json::to_string(&list).expect("serialize");
        let decoded: TodoTaskList = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.session_id, session_id);
        assert_eq!(decoded.schema_version, TODO_TASK_LIST_SCHEMA_VERSION);
        assert!(decoded.tasks.is_empty());
    }

    #[test]
    fn todo_task_status_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&TodoTaskStatus::InProgress).unwrap(),
            r#""in_progress""#
        );
        assert_eq!(
            serde_json::to_string(&TodoTaskStatus::Deleted).unwrap(),
            r#""deleted""#
        );
    }
}
