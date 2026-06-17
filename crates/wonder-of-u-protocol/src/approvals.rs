//! Approval / review types.
//!
//! These flow across the SQ/EQ in both directions:
//! - `Op::ExecApproval` / `Op::PatchApproval` carry the user's [`ReviewDecision`]
//!   back to a pending request.
//! - [`ExecApprovalRequestEvent`] / [`ApplyPatchApprovalRequestEvent`] are
//!   emitted on the EQ to ask the user for a decision.
//!
//! Phase 1's executor policy engine will route these through
//! `wonder-of-u-exec`; Phase 4 wires the TUI's inline approval modal to the
//! event side.

use serde::{Deserialize, Serialize};

/// The user's decision in response to an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    /// Approve this one execution only.
    Approved,
    /// Approve and remember for the remainder of the session.
    ApprovedForSession,
    /// Approve and persist into the policy file (sandbox / execpolicy).
    ApprovedPermanent,
    /// Deny the request; the executor fails the call.
    Denied,
    /// Defer to the user's higher-level answer (e.g. via the
    /// `request_user_input` tool).
    Abstain,
}

/// `EventMsg::ExecApprovalRequest` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecApprovalRequestEvent {
    /// Submission id this approval is for.
    pub submission_id: String,
    /// Optional turn id (set when the approval is associated with a turn).
    pub turn_id: Option<String>,
    /// Identifier for the proposed command (exec tool call id).
    pub call_id: String,
    /// The proposed shell command string (may be empty for non-shell exec).
    pub command: String,
    /// Working directory the command would run in.
    pub cwd: String,
    /// Human-readable reason for the approval request.
    pub reason: Option<String>,
    /// Risk classification surfaced for the TUI's approval modal.
    pub risk_level: RiskLevel,
}

/// `EventMsg::ApplyPatchApprovalRequest` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplyPatchApprovalRequestEvent {
    /// Submission id this approval is for.
    pub submission_id: String,
    /// Optional turn id.
    pub turn_id: Option<String>,
    /// Identifier for the proposed patch call.
    pub call_id: String,
    /// Files changed by the proposed patch, with a unified diff per file.
    pub changes: Vec<PatchChange>,
}

/// A single file change in an `ApplyPatchApprovalRequestEvent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchChange {
    /// Path of the file (relative to cwd or absolute, per caller convention).
    pub path: String,
    /// Unified diff produced for this change.
    pub diff: String,
}

/// `EventMsg::PatchApplyBegin` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchApplyBeginEvent {
    /// Call id for the patch invocation.
    pub call_id: String,
    /// Auto-approval reason if the executor pre-approved the patch.
    pub auto_approved: bool,
}

/// `EventMsg::PatchApplyEnd` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchApplyEndEvent {
    /// Call id for the patch invocation.
    pub call_id: String,
    /// Per-file success / failure summary.
    pub results: Vec<PatchApplyResult>,
    /// Final status (Success, Failure, or Cancelled).
    pub status: PatchApplyStatus,
}

/// Per-file patch-apply outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchApplyResult {
    /// Path of the file.
    pub path: String,
    /// Whether the patch was applied cleanly to this file.
    pub success: bool,
    /// Optional error message if `success` is false.
    pub error: Option<String>,
}

/// Top-level patch-apply outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchApplyStatus {
    /// All file patches applied successfully.
    Success,
    /// One or more file patches failed.
    Failure,
    /// The user cancelled the patch mid-application.
    Cancelled,
}

/// Coarse risk classification surfaced by `ExecApprovalRequestEvent` so the
/// TUI can pick an appropriate icon and default action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// Read-only or otherwise known-safe operation.
    Low,
    /// Writes to disk within the workspace.
    #[default]
    Medium,
    /// Writes outside the workspace, network calls, or destructive ops.
    High,
    /// Irreversible / catastrophic. Should require typed confirmation.
    Critical,
}

/// `Op::Review` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewRequest {
    /// The prompt to feed the reviewer agent.
    pub prompt: String,
    /// Files / paths to focus on (empty = entire working tree).
    #[serde(default)]
    pub focus_paths: Vec<String>,
    /// Optional thread id to associate this review with.
    pub thread_id: Option<String>,
}

/// `EventMsg::ExitedReviewMode` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExitedReviewModeEvent {
    /// The review output, if the user accepted the review.
    pub review_output: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_decision_roundtrip() {
        for value in [
            ReviewDecision::Approved,
            ReviewDecision::ApprovedForSession,
            ReviewDecision::ApprovedPermanent,
            ReviewDecision::Denied,
            ReviewDecision::Abstain,
        ] {
            let json = serde_json::to_string(&value).unwrap();
            let parsed: ReviewDecision = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, value);
        }
    }

    #[test]
    fn exec_approval_request_roundtrip() {
        let evt = ExecApprovalRequestEvent {
            submission_id: "sub_1".to_string(),
            turn_id: Some("turn_1".to_string()),
            call_id: "call_1".to_string(),
            command: "rm -rf build".to_string(),
            cwd: "/home/u".to_string(),
            reason: Some("destructive".to_string()),
            risk_level: RiskLevel::High,
        };
        let json = serde_json::to_string(&evt).unwrap();
        let parsed: ExecApprovalRequestEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, evt);
    }

    #[test]
    fn patch_apply_status_roundtrip() {
        for value in [
            PatchApplyStatus::Success,
            PatchApplyStatus::Failure,
            PatchApplyStatus::Cancelled,
        ] {
            let json = serde_json::to_string(&value).unwrap();
            let parsed: PatchApplyStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, value);
        }
    }
}
