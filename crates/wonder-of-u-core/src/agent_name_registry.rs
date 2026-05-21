//! Deterministic agent name → task-id registry for local `SendMessage` routing.
//!
//! The registry is an in-memory, runtime-only map; it is **not** persisted to
//! disk.  Its purpose is to let `SendMessage` resolve a bare teammate name
//! (e.g. `"reviewer"`) to the [`TaskId`] of the currently-running agent task
//! that was launched under that name, so the CLI runtime can route the message
//! to the correct task's inbox without requiring a fleet-level broadcast.
//!
//! # Design constraints
//!
//! * Names are treated case-sensitively (matching the source-compatible
//!   behaviour of the upstream `agent` tool's `name` field).
//! * A [`BTreeMap`] is used so iteration order is deterministic in tests and
//!   diagnostic output.
//! * Only one task may hold a given name at any time.  Attempting to register a
//!   name that is already occupied returns
//!   [`RegistrationError::NameAlreadyRegistered`] carrying the conflicting
//!   [`TaskId`].

use std::collections::{BTreeMap, btree_map};

use serde::{Deserialize, Serialize};

use crate::TaskId;

/// Outcome of a successful [`AgentNameRegistry::register`] call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegisterOutcome {
    /// The name was freshly inserted (no prior entry).
    Inserted,
    /// The name was already mapped to this exact task id; the existing entry
    /// was left unchanged.
    AlreadyRegistered,
}

/// Error returned when attempting to register a name that is already held by a
/// *different* task.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("agent name `{name}` is already registered to task {existing_task_id}")]
pub struct NameConflictError {
    /// The name that caused the conflict.
    pub name: String,
    /// The task that currently holds the name.
    pub existing_task_id: TaskId,
}

/// In-memory registry mapping deterministic agent names to their current
/// [`TaskId`].
///
/// # Lifecycle
///
/// * **Register** at task launch time via [`register`][Self::register].
/// * **Deregister** when the task reaches a terminal state via
///   [`deregister_by_task_id`][Self::deregister_by_task_id].
/// * **Lookup** at `SendMessage` dispatch time via [`lookup`][Self::lookup].
///
/// # Serialization
///
/// The registry derives `Serialize`/`Deserialize` only to satisfy the blanket
/// derive on [`AppState`][crate::AppState].  The field is tagged with
/// `#[serde(skip)]` on `AppState` because the registry is purely runtime
/// state — it is not meaningful to persist between process invocations.
///
/// [`AppState`]: crate::AppState
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentNameRegistry {
    /// Deterministic map from agent name → active task id.
    ///
    /// `BTreeMap` ensures stable iteration order in tests and diagnostics.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    entries: BTreeMap<String, TaskId>,
}

impl AgentNameRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `name` as owned by `task_id`.
    ///
    /// # Errors
    ///
    /// Returns [`NameConflictError`] when `name` is already registered to a
    /// *different* task id.  Re-registering the same `(name, task_id)` pair is
    /// idempotent and returns [`RegisterOutcome::AlreadyRegistered`].
    ///
    /// # Examples
    ///
    /// ```
    /// use wonder_of_u_core::{AgentNameRegistry, TaskId};
    ///
    /// let mut reg = AgentNameRegistry::new();
    /// let id = TaskId::new();
    /// reg.register("reviewer".into(), id).unwrap();
    /// assert_eq!(reg.lookup("reviewer"), Some(id));
    /// ```
    pub fn register(
        &mut self,
        name: String,
        task_id: TaskId,
    ) -> Result<RegisterOutcome, NameConflictError> {
        match self.entries.entry(name.clone()) {
            btree_map::Entry::Vacant(slot) => {
                slot.insert(task_id);
                Ok(RegisterOutcome::Inserted)
            }
            btree_map::Entry::Occupied(slot) => {
                let existing = *slot.get();
                if existing == task_id {
                    // Idempotent re-registration — same task, same name.
                    Ok(RegisterOutcome::AlreadyRegistered)
                } else {
                    Err(NameConflictError {
                        name,
                        existing_task_id: existing,
                    })
                }
            }
        }
    }

    /// Returns the [`TaskId`] currently registered under `name`, or `None`.
    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<TaskId> {
        self.entries.get(name).copied()
    }

    /// Removes the name entry owned by `task_id`, if any.
    ///
    /// This is the primary cleanup path: call it when a task reaches a
    /// terminal state (completed, failed, killed, cancelled) or is pruned.
    ///
    /// Returns the name that was removed, or `None` if `task_id` had no entry.
    pub fn deregister_by_task_id(&mut self, task_id: TaskId) -> Option<String> {
        // Linear scan is acceptable: at most a few dozen live named agents.
        let name = self
            .entries
            .iter()
            .find(|&(_, v)| *v == task_id)
            .map(|(k, _)| k.clone())?;
        self.entries.remove(&name);
        Some(name)
    }

    /// Removes the entry for `name` and returns the evicted [`TaskId`], or
    /// `None` if the name was not registered.
    pub fn deregister_by_name(&mut self, name: &str) -> Option<TaskId> {
        self.entries.remove(name)
    }

    /// Returns `true` when no names are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of currently-registered names.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns an iterator over `(name, task_id)` pairs in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, TaskId)> {
        self.entries.iter().map(|(k, &v)| (k.as_str(), v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── register ─────────────────────────────────────────────────────────────

    #[test]
    fn register_inserts_new_entry() {
        let mut reg = AgentNameRegistry::new();
        let id = TaskId::new();
        let outcome = reg.register("reviewer".into(), id).unwrap();
        assert_eq!(outcome, RegisterOutcome::Inserted);
        assert_eq!(reg.lookup("reviewer"), Some(id));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn register_same_name_and_task_is_idempotent() {
        let mut reg = AgentNameRegistry::new();
        let id = TaskId::new();
        reg.register("reviewer".into(), id).unwrap();
        let outcome = reg.register("reviewer".into(), id).unwrap();
        assert_eq!(outcome, RegisterOutcome::AlreadyRegistered);
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn register_same_name_different_task_returns_conflict() {
        let mut reg = AgentNameRegistry::new();
        let id1 = TaskId::new();
        let id2 = TaskId::new();
        reg.register("reviewer".into(), id1).unwrap();

        let err = reg.register("reviewer".into(), id2).unwrap_err();
        assert_eq!(err.name, "reviewer");
        assert_eq!(err.existing_task_id, id1);
        // Original mapping is untouched.
        assert_eq!(reg.lookup("reviewer"), Some(id1));
    }

    #[test]
    fn multiple_distinct_names_coexist() {
        let mut reg = AgentNameRegistry::new();
        let id1 = TaskId::new();
        let id2 = TaskId::new();
        reg.register("alpha".into(), id1).unwrap();
        reg.register("beta".into(), id2).unwrap();
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.lookup("alpha"), Some(id1));
        assert_eq!(reg.lookup("beta"), Some(id2));
    }

    // ── lookup ────────────────────────────────────────────────────────────────

    #[test]
    fn lookup_returns_none_for_unknown_name() {
        let reg = AgentNameRegistry::new();
        assert_eq!(reg.lookup("no-such-agent"), None);
    }

    #[test]
    fn lookup_is_case_sensitive() {
        let mut reg = AgentNameRegistry::new();
        let id = TaskId::new();
        reg.register("Reviewer".into(), id).unwrap();
        assert_eq!(reg.lookup("Reviewer"), Some(id));
        assert_eq!(reg.lookup("reviewer"), None);
    }

    // ── deregister_by_task_id ────────────────────────────────────────────────

    #[test]
    fn deregister_by_task_id_removes_entry() {
        let mut reg = AgentNameRegistry::new();
        let id = TaskId::new();
        reg.register("planner".into(), id).unwrap();

        let removed_name = reg.deregister_by_task_id(id);
        assert_eq!(removed_name, Some("planner".to_string()));
        assert!(reg.is_empty());
        assert_eq!(reg.lookup("planner"), None);
    }

    #[test]
    fn deregister_by_task_id_returns_none_for_unknown_task() {
        let mut reg = AgentNameRegistry::new();
        let id = TaskId::new();
        assert_eq!(reg.deregister_by_task_id(id), None);
    }

    #[test]
    fn deregister_by_task_id_leaves_other_entries_intact() {
        let mut reg = AgentNameRegistry::new();
        let id1 = TaskId::new();
        let id2 = TaskId::new();
        reg.register("alpha".into(), id1).unwrap();
        reg.register("beta".into(), id2).unwrap();

        reg.deregister_by_task_id(id1);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.lookup("beta"), Some(id2));
    }

    // ── deregister_by_name ───────────────────────────────────────────────────

    #[test]
    fn deregister_by_name_removes_entry() {
        let mut reg = AgentNameRegistry::new();
        let id = TaskId::new();
        reg.register("worker".into(), id).unwrap();

        let evicted = reg.deregister_by_name("worker");
        assert_eq!(evicted, Some(id));
        assert!(reg.is_empty());
    }

    #[test]
    fn deregister_by_name_returns_none_for_unknown_name() {
        let mut reg = AgentNameRegistry::new();
        assert_eq!(reg.deregister_by_name("ghost"), None);
    }

    // ── terminal task cleanup (via AppState::upsert_task) ────────────────────

    #[test]
    fn terminal_task_deregistered_on_upsert() {
        use std::path::PathBuf;

        use crate::{AgentTaskState, AppState, TaskState, TaskStatus};

        let mut state = AppState::new(PathBuf::from("/workspace"));
        let id = TaskId::new();

        // Simulate launch: register the name before the task is upserted.
        state
            .register_agent_name("reviewer".into(), id)
            .expect("register");
        assert_eq!(state.lookup_agent_by_name("reviewer"), Some(id));

        // Build a task that starts as Running and then reaches terminal status.
        let mut task = TaskState::pending_agent(
            "review PR",
            AgentTaskState::prompt_subprocess("reviewer", "Review the pull request", None, None),
        );
        task.id = id;
        task.status = TaskStatus::Completed;

        // Upserting a terminal task should auto-deregister the name.
        state.upsert_task(task);
        assert_eq!(
            state.lookup_agent_by_name("reviewer"),
            None,
            "terminal task must be deregistered"
        );
    }

    #[test]
    fn non_terminal_task_remains_registered_on_upsert() {
        use std::path::PathBuf;

        use crate::{AgentTaskState, AppState, TaskState, TaskStatus};

        let mut state = AppState::new(PathBuf::from("/workspace"));
        let id = TaskId::new();

        state
            .register_agent_name("planner".into(), id)
            .expect("register");

        let mut task = TaskState::pending_agent(
            "plan work",
            AgentTaskState::prompt_subprocess("planner", "Plan the sprint", None, None),
        );
        task.id = id;
        task.status = TaskStatus::Running;

        state.upsert_task(task);
        assert_eq!(
            state.lookup_agent_by_name("planner"),
            Some(id),
            "running task must stay registered"
        );
    }

    // ── unnamed agent has no registry entry ──────────────────────────────────

    #[test]
    fn unnamed_agent_has_no_registry_entry() {
        use std::path::PathBuf;

        use crate::{AgentTaskState, AppState, TaskState, TaskStatus};

        let mut state = AppState::new(PathBuf::from("/workspace"));

        // Launch an agent without calling register_agent_name.
        let mut task = TaskState::pending_agent(
            "unnamed work",
            AgentTaskState::prompt_subprocess("auto-slug-123", "Do some work", None, None),
        );
        task.status = TaskStatus::Running;
        state.upsert_task(task.clone());

        // No name was registered; lookup should return None.
        assert_eq!(state.lookup_agent_by_name("auto-slug-123"), None);
        assert!(
            state.agent_name_registry().is_empty(),
            "registry must remain empty for unnamed agents"
        );
    }

    // ── iter ─────────────────────────────────────────────────────────────────

    #[test]
    fn iter_yields_entries_in_key_order() {
        let mut reg = AgentNameRegistry::new();
        let id_a = TaskId::new();
        let id_b = TaskId::new();
        // Insert in reverse alphabetical order.
        reg.register("zebra".into(), id_b).unwrap();
        reg.register("alpha".into(), id_a).unwrap();

        let names: Vec<&str> = reg.iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec!["alpha", "zebra"], "BTreeMap must sort keys");
    }
}
