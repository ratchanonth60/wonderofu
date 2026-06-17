//! Submission Queue / Event Queue wire contract for the wonder-of-u codex-rs port.
//!
//! This crate holds the pure-data types that flow between the TUI/CLI and the
//! async agent core. It carries no behavior, no I/O, and no async runtime — the
//! async path is opt-in via the `wonder-of-u-async` feature (see `CLAUDE.md`
//! pitfall #1 for the rationale).
//!
//! # Topology
//!
//! ```text
//! TUI / CLI ─── Submission { id, op } ──▶ core agent loop
//!                  (mpsc / SQ)
//! TUI / CLI ◀── Event    { id, msg } ─── core agent loop
//!                  (mpsc / EQ)
//! ```
//!
//! - [`Op`] enumerates everything the user can request (`UserInput`,
//!   `Interrupt`, `ExecApproval`, `Compact`, ...).
//! - [`EventMsg`] enumerates everything the agent can stream back
//!   (`AgentMessage`, `ExecCommandBegin`, `ExecCommandOutputDelta`,
//!   `ExecApprovalRequest`, `TokenCount`, ...).
//! - [`Submission`] wraps an `Op` with a correlation id; [`Event`] wraps an
//!   `EventMsg` with the same id.
//!
//! Phase 0 of the port establishes this contract. Subsequent phases (1, 3, 5)
//! wire it into a tokio event loop, the TUI, and the storage thread-store.

#![deny(rust_2018_idioms)]
#![warn(missing_docs)]

pub mod approvals;
pub mod config;
pub mod events;
pub mod protocol;
pub mod session;
pub mod user_input;

pub use approvals::{
    ApplyPatchApprovalRequestEvent, ExecApprovalRequestEvent, ExitedReviewModeEvent,
    PatchApplyBeginEvent, PatchApplyEndEvent, ReviewDecision, ReviewRequest,
};
pub use config::{AskForApproval, GranularApprovalConfig, NetworkAccess, SandboxPolicy};
pub use events::{
    AgentMessageContentDeltaEvent, AgentMessageEvent, AgentReasoningEvent, ContextCompactedEvent,
    ErrorEvent, Event, ExecCommandBeginEvent, ExecCommandEndEvent, ExecCommandOutputDeltaEvent,
    ExecOutputStream, FileDiff, McpStartupCompleteEvent, McpStartupUpdateEvent,
    McpToolCallBeginEvent, McpToolCallEndEvent, PlanDeltaEvent, PlanItem, PlanItemStatus,
    PlanUpdateEvent, RawResponseItemEvent, ReasoningContentDeltaEvent, RequestPermissionsEvent,
    RequestPermissionsResponse, RequestUserInputEvent, SessionConfiguredEvent,
    TerminalInteractionEvent, ThreadRolledBackEvent, ThreadSettingsAppliedEvent, TokenCountEvent,
    TokenUsage, TurnAbortReason, TurnAbortedEvent, TurnCompleteEvent, TurnDiffEvent,
    TurnStartedEvent, UserInputAnswer, UserInputAnswerEntry, UserInputOption, UserInputQuestion,
    UserMessageEvent, WarningEvent,
};
pub use protocol::{Op, Submission, ThreadSettingsOverrides};
pub use session::{SessionId, SubmissionId, ThreadId};
pub use user_input::{UserInput, UserInputItem};
