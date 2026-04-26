use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use crate::{MessageId, SessionId, TaskId, ToolUseId, app::TaskStatus};

pub const MESSAGE_SCHEMA_VERSION: u16 = 1;

/// Stable serialized envelope for JSONL transcripts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageEnvelope {
    pub schema_version: u16,
    pub id: MessageId,
    pub session_id: SessionId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    pub payload: MessagePayload,
}

impl MessageEnvelope {
    #[must_use]
    pub fn new(session_id: SessionId, payload: MessagePayload) -> Self {
        Self {
            schema_version: MESSAGE_SCHEMA_VERSION,
            id: MessageId::new(),
            session_id,
            timestamp: OffsetDateTime::now_utc(),
            cwd: None,
            git_branch: None,
            entrypoint: None,
            app_version: None,
            payload,
        }
    }

    #[must_use]
    pub fn with_context(mut self, cwd: Option<PathBuf>, git_branch: Option<String>) -> Self {
        self.cwd = cwd;
        self.git_branch = git_branch;
        self
    }

    #[must_use]
    pub fn with_runtime(mut self, entrypoint: Option<String>, app_version: Option<String>) -> Self {
        self.entrypoint = entrypoint;
        self.app_version = app_version;
        self
    }

    #[must_use]
    pub fn user_text(session_id: SessionId, content: impl Into<String>) -> Self {
        Self::new(
            session_id,
            MessagePayload::UserText {
                content: content.into(),
            },
        )
    }

    #[must_use]
    pub fn system(session_id: SessionId, content: impl Into<String>) -> Self {
        Self::new(
            session_id,
            MessagePayload::System {
                content: content.into(),
            },
        )
    }
}

/// First-slice message variants. Later renderers can add display-specific data
/// without changing the envelope fields above.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum MessagePayload {
    UserText {
        content: String,
    },
    UserAttachment {
        label: String,
        uri: String,
    },
    AssistantText {
        content: String,
    },
    AssistantThinking {
        content: String,
        collapsed: bool,
    },
    AssistantToolUse {
        tool: String,
        use_id: ToolUseId,
        input: Value,
    },
    ToolResult {
        tool: String,
        use_id: ToolUseId,
        success: bool,
        content: String,
    },
    BashOutput {
        stdout: String,
        stderr: String,
        exit_code: Option<i32>,
    },
    System {
        content: String,
    },
    Progress {
        label: String,
        detail: Option<String>,
    },
    Command {
        input: String,
        output: Option<String>,
    },
    HookResult {
        hook: String,
        success: bool,
        output: String,
    },
    CompactBoundary {
        summary: String,
    },
    Task {
        task_id: TaskId,
        status: TaskStatus,
        message: String,
    },
    Permission {
        tool: String,
        decision: String,
        reason: String,
    },
    PlanApproval {
        summary: String,
        approved: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_envelope_json_round_trips() {
        let session_id = SessionId::new();
        let message = MessageEnvelope::user_text(session_id, "hello")
            .with_context(Some(PathBuf::from("/workspace")), Some("main".into()))
            .with_runtime(Some("doctor".into()), Some("0.1.0".into()));

        let json = serde_json::to_string(&message).expect("serialize message");
        let decoded: MessageEnvelope = serde_json::from_str(&json).expect("deserialize message");

        assert_eq!(decoded.schema_version, MESSAGE_SCHEMA_VERSION);
        assert_eq!(decoded.session_id, session_id);
        assert_eq!(
            decoded.payload,
            MessagePayload::UserText {
                content: "hello".into()
            }
        );
        assert_eq!(decoded.entrypoint.as_deref(), Some("doctor"));
        assert_eq!(decoded.app_version.as_deref(), Some("0.1.0"));
    }
}
