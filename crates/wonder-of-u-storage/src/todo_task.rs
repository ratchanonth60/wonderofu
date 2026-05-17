//! Persistent store for logical todo-v2 task lists (`TodoTaskStore`).
//!
//! Each Claude session has exactly one per-session todo list file stored
//! under `tasks/lists/{session_id}.json`.  This is **separate** from the
//! background agent-task runtime managed by [`super::TaskStore`].

use std::{fs, path::PathBuf};

use wonder_of_u_core::{Result, SessionId, TodoTaskList};

use super::{StoragePaths, ensure_supported_schema, write_json_atomically};

// ── TodoTaskStore ─────────────────────────────────────────────────────────────

/// Persistent store for logical todo-v2 task lists.
///
/// These lists are **separate** from the background task runtime managed by
/// [`super::TaskStore`].  Each session has exactly one list file.
///
/// Layout under the storage base directory:
///
/// ```text
/// tasks/
///   lists/{session_id}.json   ← per-session TodoTaskList
/// ```
#[derive(Clone, Debug)]
pub struct TodoTaskStore {
    paths: StoragePaths,
}

impl TodoTaskStore {
    /// Creates a new `TodoTaskStore` rooted at `base_dir`.
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
        }
    }

    /// Returns the underlying path helper.
    #[must_use]
    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    /// Ensures `tasks/lists/` directory exists.
    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.paths.task_lists_dir())?;
        Ok(())
    }

    /// Reads the todo task list for `session_id`, returning an empty list when
    /// no file exists yet.
    pub fn read_or_default(&self, session_id: SessionId) -> Result<TodoTaskList> {
        let path = self.paths.task_list_path(session_id);
        if !path.exists() {
            return Ok(TodoTaskList::empty(session_id));
        }
        let list: TodoTaskList = serde_json::from_str(&fs::read_to_string(path)?)?;
        ensure_supported_schema("todo task list", list.schema_version)?;
        Ok(list)
    }

    /// Atomically writes a [`TodoTaskList`] to disk.
    pub fn write(&self, list: &TodoTaskList) -> Result<()> {
        self.ensure_layout()?;
        write_json_atomically(&self.paths.task_list_path(list.session_id), list)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wonder_of_u_core::SessionId;
    use wonder_of_u_test_support::unique_test_dir;

    use super::TodoTaskStore;

    #[test]
    fn todo_task_store_read_or_default_returns_empty_list_when_no_file() {
        let dir = unique_test_dir("todo-store-empty");
        let store = TodoTaskStore::new(&dir);
        let session_id = SessionId::new();

        let list = store.read_or_default(session_id).expect("read or default");
        assert_eq!(list.session_id, session_id);
        assert!(list.tasks.is_empty());
        assert_eq!(
            list.schema_version,
            wonder_of_u_core::TODO_TASK_LIST_SCHEMA_VERSION
        );
    }

    #[test]
    fn todo_task_store_write_and_read_roundtrip() {
        let dir = unique_test_dir("todo-store-roundtrip");
        let store = TodoTaskStore::new(&dir);
        let session_id = SessionId::new();
        let mut list = wonder_of_u_core::TodoTaskList::empty(session_id);
        list.tasks.insert(
            "todo-abc".into(),
            wonder_of_u_core::TodoTaskEntry::new("todo-abc", "Do the thing", "description here"),
        );

        store.write(&list).expect("write");
        let read_back = store.read_or_default(session_id).expect("read back");

        assert_eq!(read_back.session_id, session_id);
        assert_eq!(read_back.tasks.len(), 1);
        assert_eq!(read_back.tasks["todo-abc"].subject, "Do the thing");
    }

    #[test]
    fn todo_task_store_schema_mismatch_returns_error() {
        use std::io::Write;

        let dir = unique_test_dir("todo-store-schema");
        let store = TodoTaskStore::new(&dir);
        let session_id = SessionId::new();

        // Write a file with an unsupported schema version.
        store.ensure_layout().expect("layout");
        let path = store.paths().task_list_path(session_id);
        let bad = json!({
            "schema_version": 9999_u32,
            "session_id": session_id.to_string(),
            "tasks": {},
            "updated_at": "2024-01-01T00:00:00Z",
        });
        let mut f = std::fs::File::create(&path).expect("create");
        f.write_all(serde_json::to_string(&bad).unwrap().as_bytes())
            .expect("write");

        let err = store
            .read_or_default(session_id)
            .expect_err("schema mismatch");
        assert!(
            err.to_string().contains("9999"),
            "error should mention bad version: {err}"
        );
    }
}
