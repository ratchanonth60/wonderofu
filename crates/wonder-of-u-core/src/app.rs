use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{Arc, RwLock},
};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{FeatureSet, MessageEnvelope, PermissionMode, Result, SessionId, TaskId, WonderError};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub id: SessionId,
    pub title: String,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl SessionState {
    #[must_use]
    pub fn new(cwd: PathBuf) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            id: SessionId::new(),
            title: "Untitled session".into(),
            cwd,
            git_branch: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    #[default]
    Prompt,
    Bash,
    PermissionPending,
    TaskNotification,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueuePlacement {
    Now,
    Next,
    Later,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueuedCommand {
    pub command: String,
    pub placement: QueuePlacement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Killed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskState {
    pub id: TaskId,
    pub description: String,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_log: Option<PathBuf>,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
}

impl TaskState {
    #[must_use]
    pub fn pending(description: impl Into<String>) -> Self {
        Self {
            id: TaskId::new(),
            description: description.into(),
            status: TaskStatus::Pending,
            parent_id: None,
            output_log: None,
            started_at: OffsetDateTime::now_utc(),
            finished_at: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    pub session: SessionState,
    pub features: FeatureSet,
    pub permission_mode: PermissionMode,
    pub input_mode: InputMode,
    pub messages: Vec<MessageEnvelope>,
    pub queued_commands: VecDeque<QueuedCommand>,
    pub background_tasks: BTreeMap<TaskId, TaskState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl AppState {
    #[must_use]
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            session: SessionState::new(cwd),
            features: FeatureSet::first_release(),
            permission_mode: PermissionMode::default(),
            input_mode: InputMode::default(),
            messages: Vec::new(),
            queued_commands: VecDeque::new(),
            background_tasks: BTreeMap::new(),
            provider: None,
            model: None,
        }
    }

    pub fn push_message(&mut self, message: MessageEnvelope) -> Result<()> {
        if message.session_id != self.session.id {
            return Err(WonderError::validation(
                "message belongs to a different session",
            ));
        }
        self.session.updated_at = message.timestamp;
        self.messages.push(message);
        Ok(())
    }

    pub fn queue_command(&mut self, command: impl Into<String>, placement: QueuePlacement) {
        self.queued_commands.push_back(QueuedCommand {
            command: command.into(),
            placement,
        });
    }
}

/// Small lock-backed store for non-TUI tests and early services. The eventual
/// TUI loop can replace this with channel-owned state where appropriate.
#[derive(Clone, Debug)]
pub struct StateStore {
    inner: Arc<RwLock<AppState>>,
}

impl StateStore {
    #[must_use]
    pub fn new(state: AppState) -> Self {
        Self {
            inner: Arc::new(RwLock::new(state)),
        }
    }

    pub fn get(&self) -> Result<AppState> {
        self.inner
            .read()
            .map(|guard| guard.clone())
            .map_err(|_| WonderError::internal("app state lock poisoned"))
    }

    pub fn update<R>(&self, update: impl FnOnce(&mut AppState) -> Result<R>) -> Result<R> {
        let mut guard = self
            .inner
            .write()
            .map_err(|_| WonderError::internal("app state lock poisoned"))?;
        update(&mut guard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MessageEnvelope;

    #[test]
    fn app_state_accepts_messages_for_current_session() {
        let cwd = PathBuf::from("/workspace");
        let mut state = AppState::new(cwd);
        let message = MessageEnvelope::user_text(state.session.id, "hello");

        state.push_message(message).expect("push message");

        assert_eq!(state.messages.len(), 1);
    }

    #[test]
    fn app_state_rejects_messages_from_other_sessions() {
        let mut state = AppState::new(PathBuf::from("/workspace"));
        let message = MessageEnvelope::user_text(SessionId::new(), "wrong session");

        let error = state.push_message(message).expect_err("wrong session");

        assert!(error.to_string().contains("different session"));
    }

    #[test]
    fn state_store_updates_state() {
        let store = StateStore::new(AppState::new(PathBuf::from("/workspace")));
        store
            .update(|state| {
                state.queue_command("/help", QueuePlacement::Later);
                Ok(())
            })
            .expect("update state");

        assert_eq!(store.get().expect("state").queued_commands.len(), 1);
    }
}
