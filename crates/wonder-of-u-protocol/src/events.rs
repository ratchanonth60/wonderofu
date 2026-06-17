//! `Event` envelope and every `EventMsg` payload.
//!
//! `Event` correlates a stream of `EventMsg`s back to the originating
//! `Submission`. `EventMsg` is the sum type the agent emits on the EQ; every
//! variant here must round-trip through serde JSON (enforced by the
//! `event_roundtrip` test below).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::approvals::ExitedReviewModeEvent;
use crate::session::ThreadId;

// ---------------------------------------------------------------------------
// Event envelope
// ---------------------------------------------------------------------------

/// Event Queue entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Submission `id` this event correlates with.
    pub id: String,
    /// Payload.
    pub msg: EventMsg,
}

// ---------------------------------------------------------------------------
// EventMsg
// ---------------------------------------------------------------------------

/// Response event from the agent.
///
/// Wire format mirrors codex: `{"type": "agent_message", ...}`. The `type` tag
/// is the snake_case variant name; serde enforces this via the
/// `#[serde(tag = "type", rename_all = "snake_case")]` attribute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventMsg {
    /// Error while executing a submission.
    Error(ErrorEvent),

    /// Warning issued while processing a submission; the turn continued.
    Warning(WarningEvent),

    /// Ack the client's configure message.
    SessionConfigured(SessionConfiguredEvent),

    /// Agent has started a turn.
    TurnStarted(TurnStartedEvent),

    /// Persistent thread-settings overrides applied to the session.
    ThreadSettingsApplied(ThreadSettingsAppliedEvent),

    /// Agent has completed all actions.
    TurnComplete(TurnCompleteEvent),

    /// Usage update for the current session (totals + last turn).
    TokenCount(TokenCountEvent),

    /// Agent text output message.
    AgentMessage(AgentMessageEvent),

    /// User/system input message (what was sent to the model).
    UserMessage(UserMessageEvent),

    /// Incremental chunk of an `AgentMessage` — streamed token-by-token.
    AgentMessageContentDelta(AgentMessageContentDeltaEvent),

    /// Reasoning event from agent.
    AgentReasoning(AgentReasoningEvent),

    /// Incremental chunk of `AgentReasoning`.
    ReasoningContentDelta(ReasoningContentDeltaEvent),

    /// Conversation history was compacted.
    ContextCompacted(ContextCompactedEvent),

    /// Last N user turns were dropped from in-memory context.
    ThreadRolledBack(ThreadRolledBackEvent),

    /// Incremental MCP startup progress update.
    McpStartupUpdate(McpStartupUpdateEvent),

    /// Aggregate MCP startup completion summary.
    McpStartupComplete(McpStartupCompleteEvent),

    /// MCP tool call start.
    McpToolCallBegin(McpToolCallBeginEvent),

    /// MCP tool call end.
    McpToolCallEnd(McpToolCallEndEvent),

    /// Agent is about to execute a command.
    ExecCommandBegin(ExecCommandBeginEvent),

    /// Incremental chunk of output from a running command. (Named to match
    /// codex even though the message carries delta data — see
    /// [`ExecCommandOutputDeltaEvent`].)
    ExecCommandOutputDelta(ExecCommandOutputDeltaEvent),

    /// Terminal interaction for an in-progress command (stdin sent / stdout observed).
    TerminalInteraction(TerminalInteractionEvent),

    /// Command execution finished.
    ExecCommandEnd(ExecCommandEndEvent),

    /// Inline approval request from the executor.
    ExecApprovalRequest(crate::approvals::ExecApprovalRequestEvent),

    /// Request-user-input tool call.
    RequestUserInput(RequestUserInputEvent),

    /// Request-permissions tool call.
    RequestPermissions(RequestPermissionsEvent),

    /// Notification that the agent is about to apply a code patch.
    PatchApplyBegin(crate::approvals::PatchApplyBeginEvent),

    /// Patch application finished.
    PatchApplyEnd(crate::approvals::PatchApplyEndEvent),

    /// ApplyPatch approval request.
    ApplyPatchApprovalRequest(crate::approvals::ApplyPatchApprovalRequestEvent),

    /// Structured plan update from the agent.
    PlanUpdate(PlanUpdateEvent),

    /// Incremental change to an existing plan.
    PlanDelta(PlanDeltaEvent),

    /// Turn aborted (via `Op::Interrupt` or fatal error).
    TurnAborted(TurnAbortedEvent),

    /// Turn-level diff summary (per file added/removed/modified lines).
    TurnDiff(TurnDiffEvent),

    /// Agent is shutting down.
    ShutdownComplete,

    /// Entered review mode.
    EnteredReviewMode(crate::approvals::ReviewRequest),

    /// Exited review mode.
    ExitedReviewMode(ExitedReviewModeEvent),

    /// Raw model item for debugging / replay.
    RawResponseItem(RawResponseItemEvent),
}

impl EventMsg {
    /// Snake-case variant name. Matches the wire `type` tag.
    pub fn kind(&self) -> &'static str {
        match self {
            EventMsg::Error(_) => "error",
            EventMsg::Warning(_) => "warning",
            EventMsg::SessionConfigured(_) => "session_configured",
            EventMsg::TurnStarted(_) => "turn_started",
            EventMsg::ThreadSettingsApplied(_) => "thread_settings_applied",
            EventMsg::TurnComplete(_) => "turn_complete",
            EventMsg::TokenCount(_) => "token_count",
            EventMsg::AgentMessage(_) => "agent_message",
            EventMsg::UserMessage(_) => "user_message",
            EventMsg::AgentMessageContentDelta(_) => "agent_message_content_delta",
            EventMsg::AgentReasoning(_) => "agent_reasoning",
            EventMsg::ReasoningContentDelta(_) => "reasoning_content_delta",
            EventMsg::ContextCompacted(_) => "context_compacted",
            EventMsg::ThreadRolledBack(_) => "thread_rolled_back",
            EventMsg::McpStartupUpdate(_) => "mcp_startup_update",
            EventMsg::McpStartupComplete(_) => "mcp_startup_complete",
            EventMsg::McpToolCallBegin(_) => "mcp_tool_call_begin",
            EventMsg::McpToolCallEnd(_) => "mcp_tool_call_end",
            EventMsg::ExecCommandBegin(_) => "exec_command_begin",
            EventMsg::ExecCommandOutputDelta(_) => "exec_command_output_delta",
            EventMsg::TerminalInteraction(_) => "terminal_interaction",
            EventMsg::ExecCommandEnd(_) => "exec_command_end",
            EventMsg::ExecApprovalRequest(_) => "exec_approval_request",
            EventMsg::RequestUserInput(_) => "request_user_input",
            EventMsg::RequestPermissions(_) => "request_permissions",
            EventMsg::PatchApplyBegin(_) => "patch_apply_begin",
            EventMsg::PatchApplyEnd(_) => "patch_apply_end",
            EventMsg::ApplyPatchApprovalRequest(_) => "apply_patch_approval_request",
            EventMsg::PlanUpdate(_) => "plan_update",
            EventMsg::PlanDelta(_) => "plan_delta",
            EventMsg::TurnAborted(_) => "turn_aborted",
            EventMsg::TurnDiff(_) => "turn_diff",
            EventMsg::ShutdownComplete => "shutdown_complete",
            EventMsg::EnteredReviewMode(_) => "entered_review_mode",
            EventMsg::ExitedReviewMode(_) => "exited_review_mode",
            EventMsg::RawResponseItem(_) => "raw_response_item",
        }
    }
}

// ---------------------------------------------------------------------------
// Session / turn lifecycle
// ---------------------------------------------------------------------------

/// `EventMsg::SessionConfigured` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionConfiguredEvent {
    /// Session id.
    pub session_id: ThreadId,
    /// Initial model identifier.
    pub model: String,
    /// Approval policy in effect.
    pub approval_policy: crate::config::AskForApproval,
    /// Sandbox policy in effect.
    pub sandbox_policy: crate::config::SandboxPolicy,
    /// Reasoning effort override (if any).
    pub reasoning_effort: Option<ReasoningEffort>,
}

/// `EventMsg::TurnStarted` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnStartedEvent {
    /// Turn id (uuid).
    pub turn_id: String,
}

/// `EventMsg::TurnComplete` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnCompleteEvent {
    /// Turn id (uuid).
    pub turn_id: String,
    /// Token usage for the turn that just completed.
    pub usage: TokenUsage,
}

/// `EventMsg::ThreadSettingsApplied` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadSettingsAppliedEvent {
    /// Approval policy after applying the overrides.
    pub approval_policy: crate::config::AskForApproval,
    /// Sandbox policy after applying the overrides.
    pub sandbox_policy: crate::config::SandboxPolicy,
    /// Model after applying the overrides.
    pub model: String,
}

/// `EventMsg::TurnAborted` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnAbortedEvent {
    /// Why the turn was aborted.
    pub reason: TurnAbortReason,
}

/// Reason for `EventMsg::TurnAborted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnAbortReason {
    /// User pressed Ctrl+C / `Op::Interrupt`.
    Interrupted,
    /// Exceeded the configured turn timeout.
    Timeout,
    /// Internal fatal error (see `ErrorEvent` for the underlying cause).
    InternalError,
}

/// Reasoning effort override applied to reasoning-capable models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    /// Lowest effort.
    Low,
    /// Default.
    Medium,
    /// High effort.
    High,
}

// ---------------------------------------------------------------------------
// Token accounting
// ---------------------------------------------------------------------------

/// `EventMsg::TokenCount` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenCountEvent {
    /// Optional information; `None` means the runtime doesn't know yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<TokenUsage>,
}

/// Token usage breakdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Tokens consumed by the model's input (this turn).
    pub input_tokens: u64,
    /// Tokens generated by the model's output (this turn).
    pub output_tokens: u64,
    /// Cached input tokens (read-from-cache) (this turn).
    #[serde(default)]
    pub cached_input_tokens: u64,
    /// Total tokens across the session so far.
    pub total_tokens: u64,
}

// ---------------------------------------------------------------------------
// Agent / user messages
// ---------------------------------------------------------------------------

/// `EventMsg::AgentMessage` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentMessageEvent {
    /// Message id (for de-dup / replay).
    pub id: String,
    /// Plain text content (final, complete).
    pub text: String,
}

/// `EventMsg::AgentMessageContentDelta` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentMessageContentDeltaEvent {
    /// Incremental text chunk.
    pub delta: String,
}

/// `EventMsg::UserMessage` payload — mirrors what was sent to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserMessageEvent {
    /// Resolved user message as the model saw it.
    pub message: String,
    /// Image URLs (after inline expansion).
    #[serde(default)]
    pub images: Vec<String>,
}

/// `EventMsg::AgentReasoning` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentReasoningEvent {
    /// Reasoning id.
    pub id: String,
    /// Final reasoning text (this chunk).
    pub text: String,
}

/// `EventMsg::ReasoningContentDelta` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReasoningContentDeltaEvent {
    /// Incremental reasoning chunk.
    pub delta: String,
    /// Optional reasoning id (deltas after the first chunk share the id).
    pub reasoning_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Compaction / rollback
// ---------------------------------------------------------------------------

/// `EventMsg::ContextCompacted` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextCompactedEvent {
    /// Summary produced by the agent (may be empty for client-driven compacts).
    pub summary: String,
}

/// `EventMsg::ThreadRolledBack` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadRolledBackEvent {
    /// Number of user turns dropped.
    pub num_turns: u32,
}

// ---------------------------------------------------------------------------
// MCP startup
// ---------------------------------------------------------------------------

/// `EventMsg::McpStartupUpdate` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpStartupUpdateEvent {
    /// Server name being initialized.
    pub server: String,
    /// Human-readable status (e.g. "connecting", "tools listed").
    pub status: String,
}

/// `EventMsg::McpStartupComplete` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpStartupCompleteEvent {
    /// Total servers configured.
    pub total: usize,
    /// Servers that initialized successfully.
    pub ready: usize,
    /// Per-server error message (empty if all ready).
    #[serde(default)]
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// MCP tool calls
// ---------------------------------------------------------------------------

/// `EventMsg::McpToolCallBegin` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpToolCallBeginEvent {
    /// MCP server name.
    pub server: String,
    /// Tool name.
    pub tool: String,
    /// Call id (correlates with end event).
    pub call_id: String,
    /// JSON-encoded arguments.
    pub arguments: Value,
}

/// `EventMsg::McpToolCallEnd` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpToolCallEndEvent {
    /// Call id.
    pub call_id: String,
    /// Whether the call succeeded.
    pub success: bool,
    /// JSON-encoded result or error message.
    pub result: Value,
}

// ---------------------------------------------------------------------------
// Exec command streaming
// ---------------------------------------------------------------------------

/// `EventMsg::ExecCommandBegin` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecCommandBeginEvent {
    /// Call id (correlates with `ExecCommandOutputDelta` / `ExecCommandEnd`).
    pub call_id: String,
    /// The parsed argv that will run (Phase 1 fills this from
    /// `parse_command`; for now it's `command` split on whitespace).
    pub command: Vec<String>,
    /// Raw command string for display (joined argv, or shell expression).
    pub command_display: String,
    /// Working directory the command runs in.
    pub cwd: String,
    /// Optional process id once the child has spawned (set lazily in
    /// Phase 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

/// `EventMsg::ExecCommandOutputDelta` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecCommandOutputDeltaEvent {
    /// Call id (correlates with begin/end).
    pub call_id: String,
    /// Stream identifier ("stdout" or "stderr").
    pub stream: ExecOutputStream,
    /// Chunk of output.
    pub chunk: String,
}

/// Stdout vs stderr for `ExecCommandOutputDeltaEvent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecOutputStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// `EventMsg::TerminalInteraction` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalInteractionEvent {
    /// Call id.
    pub call_id: String,
    /// Stdin bytes sent to the process.
    pub stdin: String,
    /// Stdout bytes observed since the last interaction event.
    pub stdout: String,
}

/// `EventMsg::ExecCommandEnd` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecCommandEndEvent {
    /// Call id.
    pub call_id: String,
    /// Exit code (None if killed by signal).
    pub exit_code: Option<i32>,
    /// Final stdout (full content, not just the last delta).
    #[serde(default)]
    pub stdout: String,
    /// Final stderr.
    #[serde(default)]
    pub stderr: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Tool-side flows (request_user_input / request_permissions)
// ---------------------------------------------------------------------------

/// `EventMsg::RequestUserInput` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestUserInputEvent {
    /// Call id (the user answers via `Op::UserInputAnswer` with the same id).
    pub call_id: String,
    /// Question prompts to present to the user.
    pub questions: Vec<UserInputQuestion>,
}

/// A single question inside `RequestUserInputEvent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserInputQuestion {
    /// Stable question id used in the response.
    pub id: String,
    /// Question header (short, e.g. "Confirm").
    pub header: String,
    /// Full question text.
    pub question: String,
    /// Whether the user can submit free-form text in addition to selecting
    /// options (always allowed regardless of this flag).
    #[serde(default)]
    pub multi_select: bool,
    /// Selectable options (empty = free-form only).
    pub options: Vec<UserInputOption>,
}

/// One selectable option in a `UserInputQuestion`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserInputOption {
    /// Display label.
    pub label: String,
    /// Description (shown beneath the label).
    pub description: String,
}

/// `Op::UserInputAnswer` payload — what the user answered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserInputAnswer {
    /// Call id this answer is for.
    pub call_id: String,
    /// Per-question answers.
    pub answers: Vec<UserInputAnswerEntry>,
}

/// One answer entry inside `UserInputAnswer`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserInputAnswerEntry {
    /// Question id this answer addresses.
    pub id: String,
    /// Selected option labels (or free-form text if no options were presented).
    pub answer: String,
}

/// `EventMsg::RequestPermissions` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestPermissionsEvent {
    /// Call id (answered via `Op::RequestPermissionsResponse`).
    pub call_id: String,
    /// Permission descriptor (serialized JSON, schema TBD in Phase 1).
    pub permissions: Value,
}

/// `Op::RequestPermissionsResponse` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestPermissionsResponse {
    /// Call id this response is for.
    pub call_id: String,
    /// Whether the user granted the requested permissions.
    pub granted: bool,
    /// Optional reason (denial reason / granted scope notes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Diff / plan
// ---------------------------------------------------------------------------

/// `EventMsg::TurnDiff` payload — per-file line-change summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnDiffEvent {
    /// Per-file diff summary.
    pub diff: Vec<FileDiff>,
}

/// Per-file diff summary inside `TurnDiffEvent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    /// File path.
    pub path: String,
    /// Lines added.
    pub added: u64,
    /// Lines removed.
    pub removed: u64,
}

/// `EventMsg::PlanUpdate` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanUpdateEvent {
    /// Full plan.
    pub plan: Vec<PlanItem>,
}

/// `EventMsg::PlanDelta` payload — incremental plan change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanDeltaEvent {
    /// Plan items whose state changed since the last update.
    pub delta: Vec<PlanItem>,
}

/// A single step inside a plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanItem {
    /// Stable step id.
    pub id: String,
    /// Human-readable description of the step.
    pub title: String,
    /// Current status.
    pub status: PlanItemStatus,
    /// Optional nested sub-tasks.
    #[serde(default)]
    pub children: Vec<PlanItem>,
}

/// Status of a `PlanItem`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanItemStatus {
    /// Not started.
    Pending,
    /// Currently in progress.
    InProgress,
    /// Successfully completed.
    Completed,
    /// Failed / abandoned.
    Failed,
}

// ---------------------------------------------------------------------------
// Diagnostics / debug
// ---------------------------------------------------------------------------

/// `EventMsg::Error` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorEvent {
    /// Short, user-facing message.
    pub message: String,
    /// Optional richer context (e.g. underlying error chain).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    /// Whether the turn was aborted as a result.
    #[serde(default)]
    pub fatal: bool,
}

/// `EventMsg::Warning` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WarningEvent {
    /// Warning message.
    pub message: String,
}

/// `EventMsg::RawResponseItem` payload — opaque model item for debug / replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawResponseItemEvent {
    /// JSON-encoded model item.
    pub item: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(msg: EventMsg) {
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: EventMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn event_envelope_roundtrip() {
        let evt = Event {
            id: "sub_1".to_string(),
            msg: EventMsg::ShutdownComplete,
        };
        let json = serde_json::to_string(&evt).unwrap();
        let parsed: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, evt);
    }

    #[test]
    fn session_configured_roundtrip() {
        roundtrip(EventMsg::SessionConfigured(SessionConfiguredEvent {
            session_id: ThreadId::new(),
            model: "claude-opus-4".to_string(),
            approval_policy: crate::config::AskForApproval::OnRequest,
            sandbox_policy: crate::config::SandboxPolicy::new_workspace_write_policy(),
            reasoning_effort: Some(ReasoningEffort::Medium),
        }));
    }

    #[test]
    fn turn_started_roundtrip() {
        roundtrip(EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn_1".to_string(),
        }));
    }

    #[test]
    fn turn_complete_roundtrip() {
        roundtrip(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn_1".to_string(),
            usage: TokenUsage::default(),
        }));
    }

    #[test]
    fn token_count_roundtrip() {
        roundtrip(EventMsg::TokenCount(TokenCountEvent {
            info: Some(TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                cached_input_tokens: 0,
                total_tokens: 150,
            }),
        }));
    }

    #[test]
    fn agent_message_roundtrip() {
        roundtrip(EventMsg::AgentMessage(AgentMessageEvent {
            id: "msg_1".to_string(),
            text: "hello".to_string(),
        }));
    }

    #[test]
    fn agent_message_content_delta_roundtrip() {
        roundtrip(EventMsg::AgentMessageContentDelta(
            AgentMessageContentDeltaEvent {
                delta: "hel".to_string(),
            },
        ));
    }

    #[test]
    fn user_message_roundtrip() {
        roundtrip(EventMsg::UserMessage(UserMessageEvent {
            message: "hi".to_string(),
            images: vec!["/tmp/x.png".to_string()],
        }));
    }

    #[test]
    fn agent_reasoning_roundtrip() {
        roundtrip(EventMsg::AgentReasoning(AgentReasoningEvent {
            id: "r_1".to_string(),
            text: "thinking...".to_string(),
        }));
    }

    #[test]
    fn reasoning_content_delta_roundtrip() {
        roundtrip(EventMsg::ReasoningContentDelta(
            ReasoningContentDeltaEvent {
                delta: "thin".to_string(),
                reasoning_id: Some("r_1".to_string()),
            },
        ));
    }

    #[test]
    fn context_compacted_roundtrip() {
        roundtrip(EventMsg::ContextCompacted(ContextCompactedEvent {
            summary: "summary".to_string(),
        }));
    }

    #[test]
    fn thread_rolled_back_roundtrip() {
        roundtrip(EventMsg::ThreadRolledBack(ThreadRolledBackEvent {
            num_turns: 2,
        }));
    }

    #[test]
    fn thread_settings_applied_roundtrip() {
        roundtrip(EventMsg::ThreadSettingsApplied(
            ThreadSettingsAppliedEvent {
                approval_policy: crate::config::AskForApproval::OnRequest,
                sandbox_policy: crate::config::SandboxPolicy::new_read_only_policy(),
                model: "claude-haiku".to_string(),
            },
        ));
    }

    #[test]
    fn mcp_startup_update_roundtrip() {
        roundtrip(EventMsg::McpStartupUpdate(McpStartupUpdateEvent {
            server: "filesystem".to_string(),
            status: "ready".to_string(),
        }));
    }

    #[test]
    fn mcp_startup_complete_roundtrip() {
        roundtrip(EventMsg::McpStartupComplete(McpStartupCompleteEvent {
            total: 3,
            ready: 2,
            errors: vec!["github".to_string()],
        }));
    }

    #[test]
    fn mcp_tool_call_begin_roundtrip() {
        roundtrip(EventMsg::McpToolCallBegin(McpToolCallBeginEvent {
            server: "filesystem".to_string(),
            tool: "read".to_string(),
            call_id: "mcp_1".to_string(),
            arguments: serde_json::json!({"path": "/tmp/x"}),
        }));
    }

    #[test]
    fn mcp_tool_call_end_roundtrip() {
        roundtrip(EventMsg::McpToolCallEnd(McpToolCallEndEvent {
            call_id: "mcp_1".to_string(),
            success: true,
            result: serde_json::json!("contents"),
        }));
    }

    #[test]
    fn exec_command_output_delta_roundtrip() {
        roundtrip(EventMsg::ExecCommandOutputDelta(
            ExecCommandOutputDeltaEvent {
                call_id: "exec_1".to_string(),
                stream: ExecOutputStream::Stdout,
                chunk: "hello\n".to_string(),
            },
        ));
    }

    #[test]
    fn terminal_interaction_roundtrip() {
        roundtrip(EventMsg::TerminalInteraction(TerminalInteractionEvent {
            call_id: "exec_1".to_string(),
            stdin: "y\n".to_string(),
            stdout: "Are you sure? ".to_string(),
        }));
    }

    #[test]
    fn exec_command_end_roundtrip() {
        roundtrip(EventMsg::ExecCommandEnd(ExecCommandEndEvent {
            call_id: "exec_1".to_string(),
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            duration_ms: 42,
        }));
    }

    #[test]
    fn exec_command_begin_roundtrip() {
        roundtrip(EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
            call_id: "exec_1".to_string(),
            command: vec!["ls".to_string(), "-la".to_string()],
            command_display: "ls -la".to_string(),
            cwd: "/tmp".to_string(),
            pid: Some(1234),
        }));
        roundtrip(EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
            call_id: "exec_2".to_string(),
            command: vec!["echo".to_string()],
            command_display: "echo".to_string(),
            cwd: "/tmp".to_string(),
            pid: None,
        }));
    }

    #[test]
    fn exec_approval_request_roundtrip() {
        let evt = crate::approvals::ExecApprovalRequestEvent {
            submission_id: "sub_1".to_string(),
            turn_id: None,
            call_id: "call_1".to_string(),
            command: "rm -rf build".to_string(),
            cwd: "/tmp".to_string(),
            reason: Some("destructive".to_string()),
            risk_level: crate::approvals::RiskLevel::High,
        };
        roundtrip(EventMsg::ExecApprovalRequest(evt));
    }

    #[test]
    fn request_user_input_roundtrip() {
        roundtrip(EventMsg::RequestUserInput(RequestUserInputEvent {
            call_id: "rq_1".to_string(),
            questions: vec![UserInputQuestion {
                id: "q1".to_string(),
                header: "Confirm".to_string(),
                question: "Proceed?".to_string(),
                multi_select: false,
                options: vec![UserInputOption {
                    label: "Yes".to_string(),
                    description: "Proceed".to_string(),
                }],
            }],
        }));
    }

    #[test]
    fn request_permissions_roundtrip() {
        roundtrip(EventMsg::RequestPermissions(RequestPermissionsEvent {
            call_id: "rp_1".to_string(),
            permissions: serde_json::json!({"network": true}),
        }));
    }

    #[test]
    fn patch_apply_roundtrip() {
        roundtrip(EventMsg::PatchApplyBegin(
            crate::approvals::PatchApplyBeginEvent {
                call_id: "patch_1".to_string(),
                auto_approved: false,
            },
        ));
        roundtrip(EventMsg::PatchApplyEnd(
            crate::approvals::PatchApplyEndEvent {
                call_id: "patch_1".to_string(),
                results: vec![crate::approvals::PatchApplyResult {
                    path: "/tmp/x.rs".to_string(),
                    success: true,
                    error: None,
                }],
                status: crate::approvals::PatchApplyStatus::Success,
            },
        ));
        roundtrip(EventMsg::ApplyPatchApprovalRequest(
            crate::approvals::ApplyPatchApprovalRequestEvent {
                submission_id: "sub_1".to_string(),
                turn_id: None,
                call_id: "patch_1".to_string(),
                changes: vec![crate::approvals::PatchChange {
                    path: "/tmp/x.rs".to_string(),
                    diff: "+ hello".to_string(),
                }],
            },
        ));
    }

    #[test]
    fn plan_roundtrip() {
        roundtrip(EventMsg::PlanUpdate(PlanUpdateEvent {
            plan: vec![PlanItem {
                id: "step_1".to_string(),
                title: "Read file".to_string(),
                status: PlanItemStatus::InProgress,
                children: vec![PlanItem {
                    id: "step_1_a".to_string(),
                    title: "Parse".to_string(),
                    status: PlanItemStatus::Pending,
                    children: Vec::new(),
                }],
            }],
        }));
        roundtrip(EventMsg::PlanDelta(PlanDeltaEvent {
            delta: vec![PlanItem {
                id: "step_1".to_string(),
                title: "Read file".to_string(),
                status: PlanItemStatus::Completed,
                children: Vec::new(),
            }],
        }));
    }

    #[test]
    fn turn_aborted_roundtrip() {
        for reason in [
            TurnAbortReason::Interrupted,
            TurnAbortReason::Timeout,
            TurnAbortReason::InternalError,
        ] {
            roundtrip(EventMsg::TurnAborted(TurnAbortedEvent { reason }));
        }
    }

    #[test]
    fn turn_diff_roundtrip() {
        roundtrip(EventMsg::TurnDiff(TurnDiffEvent {
            diff: vec![FileDiff {
                path: "/tmp/x.rs".to_string(),
                added: 3,
                removed: 1,
            }],
        }));
    }

    #[test]
    fn shutdown_complete_roundtrip() {
        roundtrip(EventMsg::ShutdownComplete);
    }

    #[test]
    fn review_mode_roundtrip() {
        roundtrip(EventMsg::EnteredReviewMode(
            crate::approvals::ReviewRequest {
                prompt: "review this".to_string(),
                focus_paths: vec!["/tmp/x.rs".to_string()],
                thread_id: None,
            },
        ));
        roundtrip(EventMsg::ExitedReviewMode(ExitedReviewModeEvent {
            review_output: Some("looks good".to_string()),
        }));
    }

    #[test]
    fn raw_response_item_roundtrip() {
        roundtrip(EventMsg::RawResponseItem(RawResponseItemEvent {
            item: serde_json::json!({"role": "assistant", "content": "ok"}),
        }));
    }

    #[test]
    fn error_warning_roundtrip() {
        roundtrip(EventMsg::Error(ErrorEvent {
            message: "boom".to_string(),
            details: Some("stacktrace".to_string()),
            fatal: true,
        }));
        roundtrip(EventMsg::Warning(WarningEvent {
            message: "heads up".to_string(),
        }));
    }

    #[test]
    fn kind_is_stable() {
        // Spot-check that `kind()` matches the wire `type` tag.
        let evt = EventMsg::ShutdownComplete;
        let json = serde_json::to_string(&evt).unwrap();
        assert!(
            json.contains("\"type\":\"shutdown_complete\""),
            "unexpected wire format: {json}"
        );
        assert_eq!(evt.kind(), "shutdown_complete");
    }
}
