//! Submission Queue entry and the `Op` sum type.
//!
//! - [`Submission`] wraps an `Op` with a correlation id.
//! - [`Op`] enumerates every operation a client (TUI / CLI / app-server) can
//!   send into the agent. The wire format mirrors codex `Op` minus the
//!   realtime-audio / voice variants we will not implement.
//!
//! Serde round-trips for every variant live in [`tests`].

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::approvals::{ReviewDecision, ReviewRequest};
use crate::config::AskForApproval;
use crate::session::SubmissionId;
use crate::user_input::UserInput;

// ---------------------------------------------------------------------------
// Submission
// ---------------------------------------------------------------------------

/// Submission Queue entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Submission {
    /// Correlation id echoed on every `Event` the agent emits in response.
    pub id: SubmissionId,
    /// Operation.
    pub op: Op,
    /// Client-provided id for the user message represented by
    /// `Op::UserInput`. Echoed on the resulting `UserMessage` event for
    /// de-duplication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_user_message_id: Option<String>,
}

impl Submission {
    /// Convenience constructor.
    pub fn new(id: impl Into<SubmissionId>, op: Op) -> Self {
        Self {
            id: id.into(),
            op,
            client_user_message_id: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Op
// ---------------------------------------------------------------------------

/// Submission operation.
///
/// Wire format: `{"type": "<snake_case variant name>", ...}`. Serde enforces
/// this via `#[serde(tag = "type", rename_all = "snake_case")]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Op {
    /// Abort the current task without terminating background terminal
    /// processes. The agent replies with `EventMsg::TurnAborted`.
    Interrupt,

    /// Terminate all running background terminal processes for this thread.
    CleanBackgroundTerminals,

    /// User input.
    UserInput {
        /// Input items the user submitted.
        items: Vec<UserInput>,
        /// Optional JSON Schema constraining the final assistant message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        final_output_json_schema: Option<Value>,
        /// Per-turn overrides applied before the input is sent.
        #[serde(default)]
        thread_settings: ThreadSettingsOverrides,
    },

    /// Apply thread-settings overrides without starting a turn.
    ThreadSettings {
        /// Overrides to apply.
        thread_settings: ThreadSettingsOverrides,
    },

    /// Approve a command execution.
    ExecApproval {
        /// Submission id of the approval request this resolves.
        id: String,
        /// Turn id, if the approval was tied to a turn.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        /// The user's decision.
        decision: ReviewDecision,
    },

    /// Approve a code patch.
    PatchApproval {
        /// Submission id of the approval request this resolves.
        id: String,
        /// The user's decision.
        decision: ReviewDecision,
    },

    /// Resolve a `request_user_input` tool call.
    UserInputAnswer {
        /// Call id of the in-flight request.
        id: String,
        /// User-provided answers.
        response: crate::events::UserInputAnswer,
    },

    /// Resolve a `request_permissions` tool call.
    RequestPermissionsResponse {
        /// Call id of the in-flight request.
        id: String,
        /// User decision.
        response: crate::events::RequestPermissionsResponse,
    },

    /// Reload user config layer overrides for the active session.
    ReloadUserConfig,

    /// Refresh MCP server connections and re-list tools.
    RefreshMcpServers,

    /// Request the agent to summarize the current context.
    Compact,

    /// Request a code review from the agent.
    Review {
        /// Review request payload.
        review_request: ReviewRequest,
    },

    /// Drop the last N user turns from in-memory context.
    ThreadRollback {
        /// Number of user turns to drop.
        num_turns: u32,
    },

    /// Request to shut down the agent.
    Shutdown,

    /// Execute a user-initiated one-off shell command (triggered by `!cmd`).
    RunUserShellCommand {
        /// The raw command string after the leading `!`.
        command: String,
    },
}

impl Op {
    /// Snake-case variant name. Matches the wire `type` tag.
    pub fn kind(&self) -> &'static str {
        match self {
            Op::Interrupt => "interrupt",
            Op::CleanBackgroundTerminals => "clean_background_terminals",
            Op::UserInput { .. } => "user_input",
            Op::ThreadSettings { .. } => "thread_settings",
            Op::ExecApproval { .. } => "exec_approval",
            Op::PatchApproval { .. } => "patch_approval",
            Op::UserInputAnswer { .. } => "user_input_answer",
            Op::RequestPermissionsResponse { .. } => "request_permissions_response",
            Op::ReloadUserConfig => "reload_user_config",
            Op::RefreshMcpServers => "refresh_mcp_servers",
            Op::Compact => "compact",
            Op::Review { .. } => "review",
            Op::ThreadRollback { .. } => "thread_rollback",
            Op::Shutdown => "shutdown",
            Op::RunUserShellCommand { .. } => "run_user_shell_command",
        }
    }
}

/// Persistent thread-settings overrides applied before a turn (or standalone).
///
/// Mirrors codex's `ThreadSettingsOverrides`. Phase 1 will route this through
/// the session configuration; Phase 5 persists it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThreadSettingsOverrides {
    /// Updated approval policy.
    pub approval_policy: Option<AskForApproval>,
    /// Updated sandbox policy.
    pub sandbox_policy: Option<crate::config::SandboxPolicy>,
    /// Updated model slug.
    pub model: Option<String>,
    /// Updated reasoning effort override.
    pub reasoning_effort: Option<crate::events::ReasoningEffort>,
    /// Free-form metadata (session tags, labels). Phase 5 persists this.
    pub metadata: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(op: Op) {
        let sub = Submission::new("sub_1", op.clone());
        let json = serde_json::to_string(&sub).unwrap();
        let parsed: Submission = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, sub, "round-trip mismatch for op {}", op.kind());
    }

    #[test]
    fn interrupt_roundtrip() {
        roundtrip(Op::Interrupt);
    }

    #[test]
    fn clean_background_terminals_roundtrip() {
        roundtrip(Op::CleanBackgroundTerminals);
    }

    #[test]
    fn shutdown_roundtrip() {
        roundtrip(Op::Shutdown);
    }

    #[test]
    fn reload_user_config_roundtrip() {
        roundtrip(Op::ReloadUserConfig);
    }

    #[test]
    fn refresh_mcp_servers_roundtrip() {
        roundtrip(Op::RefreshMcpServers);
    }

    #[test]
    fn compact_roundtrip() {
        roundtrip(Op::Compact);
    }

    #[test]
    fn thread_rollback_roundtrip() {
        roundtrip(Op::ThreadRollback { num_turns: 3 });
    }

    #[test]
    fn run_user_shell_command_roundtrip() {
        roundtrip(Op::RunUserShellCommand {
            command: "ls -la".to_string(),
        });
    }

    #[test]
    fn user_input_roundtrip() {
        roundtrip(Op::UserInput {
            items: vec![
                UserInput::Text {
                    text: "hi".to_string(),
                },
                UserInput::LocalImage {
                    path: "/tmp/x.png".to_string(),
                },
            ],
            final_output_json_schema: None,
            thread_settings: ThreadSettingsOverrides::default(),
        });
    }

    #[test]
    fn thread_settings_roundtrip() {
        roundtrip(Op::ThreadSettings {
            thread_settings: ThreadSettingsOverrides {
                approval_policy: Some(AskForApproval::OnRequest),
                sandbox_policy: Some(crate::config::SandboxPolicy::new_workspace_write_policy()),
                model: Some("claude-opus-4".to_string()),
                reasoning_effort: Some(crate::events::ReasoningEffort::High),
                metadata: HashMap::from([("tag".to_string(), "smoke".to_string())]),
            },
        });
    }

    #[test]
    fn exec_approval_roundtrip() {
        roundtrip(Op::ExecApproval {
            id: "sub_99".to_string(),
            turn_id: Some("turn_1".to_string()),
            decision: ReviewDecision::ApprovedForSession,
        });
    }

    #[test]
    fn patch_approval_roundtrip() {
        roundtrip(Op::PatchApproval {
            id: "sub_99".to_string(),
            decision: ReviewDecision::Denied,
        });
    }

    #[test]
    fn user_input_answer_roundtrip() {
        roundtrip(Op::UserInputAnswer {
            id: "rq_1".to_string(),
            response: crate::events::UserInputAnswer {
                call_id: "rq_1".to_string(),
                answers: vec![crate::events::UserInputAnswerEntry {
                    id: "q1".to_string(),
                    answer: "yes".to_string(),
                }],
            },
        });
    }

    #[test]
    fn request_permissions_response_roundtrip() {
        roundtrip(Op::RequestPermissionsResponse {
            id: "rp_1".to_string(),
            response: crate::events::RequestPermissionsResponse {
                call_id: "rp_1".to_string(),
                granted: false,
                reason: Some("too broad".to_string()),
            },
        });
    }

    #[test]
    fn review_roundtrip() {
        roundtrip(Op::Review {
            review_request: ReviewRequest {
                prompt: "review this PR".to_string(),
                focus_paths: vec!["/tmp/x.rs".to_string()],
                thread_id: None,
            },
        });
    }

    #[test]
    fn kind_is_stable() {
        let op = Op::Interrupt;
        let json = serde_json::to_string(&op).unwrap();
        assert!(
            json.contains("\"type\":\"interrupt\""),
            "unexpected wire format: {json}"
        );
        assert_eq!(op.kind(), "interrupt");
    }

    #[test]
    fn submission_id_correlates() {
        let sub = Submission::new("sub_xyz", Op::Shutdown);
        assert_eq!(sub.id.as_ref(), "sub_xyz");
        assert_eq!(sub.op.kind(), "shutdown");
    }
}
