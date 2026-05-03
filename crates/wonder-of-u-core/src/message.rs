use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use crate::{MessageId, SessionId, TaskId, ToolUseId, app::TaskStatus};

/// Schema version for message
pub const MESSAGE_SCHEMA_VERSION: u16 = 1;

/// Stable serialized envelope for JSONL transcripts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageEnvelope {
    /// Stores the schema version
    pub schema_version: u16,
    /// Stores the id
    pub id: MessageId,
    /// Stores the session identifier
    pub session_id: SessionId,
    /// Stores the timestamp
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
    /// Stores the cwd
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// Stores the git branch
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    /// Stores the entrypoint
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    /// Stores the app version
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    /// Stores the payload
    pub payload: MessagePayload,
}

impl MessageEnvelope {
    /// Creates a new value
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
    /// Handles with context
    #[must_use]
    pub fn with_context(mut self, cwd: Option<PathBuf>, git_branch: Option<String>) -> Self {
        self.cwd = cwd;
        self.git_branch = git_branch;
        self
    }
    /// Handles with runtime
    #[must_use]
    pub fn with_runtime(mut self, entrypoint: Option<String>, app_version: Option<String>) -> Self {
        self.entrypoint = entrypoint;
        self.app_version = app_version;
        self
    }
    /// Handles user text
    #[must_use]
    pub fn user_text(session_id: SessionId, content: impl Into<String>) -> Self {
        Self::new(
            session_id,
            MessagePayload::UserText {
                content: content.into(),
            },
        )
    }
    /// Handles user paste reference
    #[must_use]
    pub fn user_paste_reference(
        session_id: SessionId,
        sha256: impl Into<String>,
        bytes: usize,
    ) -> Self {
        Self::new(
            session_id,
            MessagePayload::UserPasteReference {
                sha256: sha256.into(),
                bytes,
            },
        )
    }
    /// Handles system
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
    /// Represents user text
    UserText {
        /// Stores the content
        content: String,
    },
    /// Represents user attachment
    UserAttachment {
        /// Stores the label
        label: String,
        /// Stores the uri
        uri: String,
    },
    /// Represents user paste reference
    UserPasteReference {
        /// Stores the sha256
        sha256: String,
        /// Stores the bytes
        bytes: usize,
    },
    /// Represents assistant text
    AssistantText {
        /// Stores the content
        content: String,
    },
    /// Represents assistant thinking
    AssistantThinking {
        /// Stores the content
        content: String,
        /// Stores the collapsed
        collapsed: bool,
    },
    /// Represents assistant tool use
    AssistantToolUse {
        /// Stores the tool
        tool: String,
        /// Stores the use id
        use_id: ToolUseId,
        /// Stores the input
        input: Value,
    },
    /// Represents tool result
    ToolResult {
        /// Stores the tool
        tool: String,
        /// Stores the use id
        use_id: ToolUseId,
        /// Stores the success
        success: bool,
        /// Stores the content
        content: String,
    },
    /// Represents bash output
    BashOutput {
        /// Stores the stdout
        stdout: String,
        /// Stores the stderr
        stderr: String,
        /// Stores the exit code
        exit_code: Option<i32>,
    },
    /// Represents system
    System {
        /// Stores the content
        content: String,
    },
    /// Represents progress
    Progress {
        /// Stores the label
        label: String,
        /// Stores the detail
        detail: Option<String>,
    },
    /// Represents command
    Command {
        /// Stores the input
        input: String,
        /// Stores the output
        output: Option<String>,
    },
    /// Represents hook result
    HookResult {
        /// Stores the hook
        hook: String,
        /// Stores the success
        success: bool,
        /// Stores the output
        output: String,
    },
    /// Represents compact boundary
    CompactBoundary {
        /// Stores the summary
        summary: String,
    },
    /// Represents task
    Task {
        /// Stores the task id
        task_id: TaskId,
        /// Stores the status
        status: TaskStatus,
        /// Stores the message
        message: String,
    },
    /// Represents permission
    Permission {
        /// Stores the tool
        tool: String,
        /// Stores the decision
        decision: String,
        /// Stores the reason
        reason: String,
    },
    /// Represents plan approval
    PlanApproval {
        /// Stores the summary
        summary: String,
        /// Stores the approved
        approved: bool,
    },
    /// A sanitized provider or runtime error persisted into the transcript.
    ///
    /// The `message` field contains display-safe text with credentials
    /// already redacted. The `kind` field is a short machine-readable tag
    /// (e.g. `"provider"`, `"runtime"`, `"timeout"`) that callers may use
    /// for styling or filtering.
    ProviderError {
        /// Machine-readable error kind.
        kind: String,
        /// Sanitized, display-safe error message.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_error_payload_round_trips_json() {
        let session_id = SessionId::new();
        let msg = MessageEnvelope::new(
            session_id,
            MessagePayload::ProviderError {
                kind: "provider".into(),
                message: "connection refused".into(),
            },
        );
        let json = serde_json::to_string(&msg).expect("serialize");
        let decoded: MessageEnvelope = serde_json::from_str(&json).expect("deserialize");
        assert!(
            matches!(&decoded.payload, MessagePayload::ProviderError { kind, message }
                if kind == "provider" && message == "connection refused")
        );
    }

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
