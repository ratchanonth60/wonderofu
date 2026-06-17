//! Identifier types for submissions, sessions, and threads.
//!
//! `SubmissionId` correlates a `Submission` with the `Event`s it produces.
//! `SessionId` / `ThreadId` will be threaded into the storage layer in Phase 5
//! (currently just opaque wrappers so the wire types compile end-to-end).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Opaque id for a single `Submission` entry. The id is echoed on every
/// `Event` the agent emits in response.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SubmissionId(pub String);

impl SubmissionId {
    /// Build a `SubmissionId` from an arbitrary string.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl From<String> for SubmissionId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SubmissionId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl AsRef<str> for SubmissionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Persistent session id — survives across thread forks / resumes (Phase 5).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub Uuid);

impl SessionId {
    /// Generate a fresh random `SessionId`.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-thread id — a session can spawn multiple threads (Phase 5).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ThreadId(pub Uuid);

impl ThreadId {
    /// Generate a fresh random `ThreadId`.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ThreadId {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submission_id_roundtrip() {
        let id = SubmissionId::new("sub_abc");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"sub_abc\"");
        let parsed: SubmissionId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn session_id_roundtrip() {
        let id = SessionId::new();
        let json = serde_json::to_string(&id).unwrap();
        let parsed: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn thread_id_roundtrip() {
        let id = ThreadId::new();
        let json = serde_json::to_string(&id).unwrap();
        let parsed: ThreadId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
    }
}
