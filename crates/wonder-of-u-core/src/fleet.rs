//! Fleet (multi-agent) run types.
//!
//! A *fleet run* groups one or more local-agent tasks under a single
//! [`FleetRunState`] record.  Member tasks are dispatched by the CLI
//! `fleet dispatch` command (simple, dependency-free requests) or by
//! `fleet reconcile` (dependency-aware plan execution).

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{FleetId, PermissionMode, TaskId, TaskStatus};

// ── Worktree isolation types ─────────────────────────────────────────────────

/// Isolation mode for a fleet agent member.
///
/// Only `Worktree` is defined today; the enum allows future modes (e.g.
/// container, sandbox) to be added without breaking existing JSON.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeIsolationMode {
    /// Run the agent in a dedicated git worktree branched from `HEAD`.
    Worktree,
}

/// Describes the worktree isolation configuration for a fleet member.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorktreeIsolation {
    /// Which isolation strategy to apply.
    pub mode: WorktreeIsolationMode,
    /// Optional explicit branch name to use / create.
    ///
    /// When absent a deterministic slug is derived from the fleet id and
    /// request id at dispatch time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// Schema version for [`FleetRunState`] JSON files.
pub const FLEET_SCHEMA_VERSION: u16 = 1;

fn default_fleet_schema_version() -> u16 {
    FLEET_SCHEMA_VERSION
}

/// Overall lifecycle status of a fleet run.
///
/// Derived from the collection of member [`TaskStatus`] values; see
/// [`FleetRunStatus::from_member_statuses`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetRunStatus {
    /// No members have been dispatched yet (or the fleet has no members).
    Pending,
    /// At least one member is actively running.
    Running,
    /// All members finished successfully.
    Completed,
    /// All members are terminal and at least one failed or was killed.
    Failed,
    /// All members were explicitly cancelled.
    Cancelled,
}

impl FleetRunStatus {
    /// Returns `true` when the run has reached a terminal state.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Derives fleet status from the slice of member task statuses.
    ///
    /// Rules (evaluated in order):
    /// 1. Any `running` → [`FleetRunStatus::Running`]
    /// 2. All terminal + any `failed`/`killed` → [`FleetRunStatus::Failed`]
    /// 3. All terminal + all `cancelled` → [`FleetRunStatus::Cancelled`]
    /// 4. All terminal → [`FleetRunStatus::Completed`]
    /// 5. Otherwise (all pending, or partially pending with no running) →
    ///    [`FleetRunStatus::Pending`]
    ///
    /// An empty slice returns [`FleetRunStatus::Pending`].
    #[must_use]
    pub fn from_member_statuses(statuses: &[TaskStatus]) -> Self {
        if statuses.is_empty() {
            return Self::Pending;
        }

        if statuses.contains(&TaskStatus::Running) {
            return Self::Running;
        }

        let all_terminal = statuses.iter().all(|s| s.is_terminal());
        if all_terminal {
            if statuses
                .iter()
                .any(|s| matches!(s, TaskStatus::Failed | TaskStatus::Killed))
            {
                return Self::Failed;
            }
            if statuses.iter().all(|s| *s == TaskStatus::Cancelled) {
                return Self::Cancelled;
            }
            return Self::Completed;
        }

        Self::Pending
    }

    /// Human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Persisted state for a fleet run.
///
/// Written to `fleet/runs/{fleet_id}.json` by the `fleet start` command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FleetRunState {
    /// Storage schema guard – always [`FLEET_SCHEMA_VERSION`].
    #[serde(default = "default_fleet_schema_version")]
    pub schema_version: u16,
    /// Unique identifier for this fleet run.
    pub id: FleetId,
    /// Human-readable description of what the fleet is doing.
    pub description: String,
    /// Derived status; callers should recompute via
    /// [`FleetRunStatus::from_member_statuses`] before displaying.
    pub status: FleetRunStatus,
    /// Ordered list of member task ids (set when tasks are dispatched).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub member_task_ids: Vec<TaskId>,
    /// Permission mode in effect when the fleet was started.
    pub permission_mode: PermissionMode,
    /// Working directory from which the fleet was launched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// When the first member task was launched.
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    /// When the last member task reached a terminal state, if any.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
    /// Maximum number of member tasks that may run concurrently.
    ///
    /// `None` means unbounded. Set by `fleet start --plan --max-concurrency N`
    /// and respected by `fleet reconcile`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<usize>,
    /// Maps each plan member request id to the [`TaskId`] that was launched
    /// for it.  Used by `fleet reconcile` to track dependency readiness.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dispatched_requests: BTreeMap<String, TaskId>,
}

impl FleetRunState {
    /// Creates a new pending fleet run.
    #[must_use]
    pub fn new(
        description: impl Into<String>,
        permission_mode: PermissionMode,
        cwd: Option<PathBuf>,
    ) -> Self {
        Self {
            schema_version: FLEET_SCHEMA_VERSION,
            id: FleetId::new(),
            description: description.into(),
            status: FleetRunStatus::Pending,
            member_task_ids: Vec::new(),
            permission_mode,
            cwd,
            started_at: OffsetDateTime::now_utc(),
            finished_at: None,
            max_concurrency: None,
            dispatched_requests: BTreeMap::new(),
        }
    }

    /// Recomputes and updates [`Self::status`] from the given member statuses.
    ///
    /// Also sets [`Self::finished_at`] to `now` when the run becomes terminal.
    pub fn reconcile_status(&mut self, member_statuses: &[TaskStatus]) {
        let derived = FleetRunStatus::from_member_statuses(member_statuses);
        self.status = derived;
        if derived.is_terminal() && self.finished_at.is_none() {
            self.finished_at = Some(OffsetDateTime::now_utc());
        }
    }

    /// Records that `request_id` was dispatched as `task_id`.
    ///
    /// Inserts the mapping into [`Self::dispatched_requests`] and appends
    /// `task_id` to [`Self::member_task_ids`], deduplicating on the latter.
    pub fn record_dispatch(&mut self, request_id: String, task_id: TaskId) {
        self.dispatched_requests.insert(request_id, task_id);
        if !self.member_task_ids.contains(&task_id) {
            self.member_task_ids.push(task_id);
        }
    }
}

/// A pending agent dispatch request written by the `agent` tool bridge.
///
/// Files are stored at `fleet/pending/{id}.json` and drained by
/// `fleet dispatch`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FleetMemberRequest {
    /// Unique id for this pending request (UUID v4).
    pub id: String,
    /// Optional fleet run this request belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fleet_id: Option<FleetId>,
    /// Prompt to forward to the agent subprocess.
    pub prompt: String,
    /// Optional human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional agent name used as the task name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional fleet agent role id (e.g. `"rust-engineer"`).
    ///
    /// When set, the dispatcher prepends the role's preamble to [`Self::prompt`]
    /// before launching the agent task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Optional model override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Optional provider override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Optional working directory override (resolved at dispatch time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// Ids of other plan members that must complete before this one launches.
    ///
    /// Set by `fleet start --plan`; respected by `fleet reconcile`.
    /// `fleet dispatch` skips requests with a non-empty `depends_on`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// Optional worktree isolation configuration for this member.
    ///
    /// When set, `fleet dispatch` / `fleet reconcile` creates (or resumes) a
    /// dedicated git worktree before launching the agent subprocess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<WorktreeIsolation>,
    /// When the request was queued.
    #[serde(with = "time::serde::rfc3339")]
    pub queued_at: OffsetDateTime,
}

impl FleetMemberRequest {
    /// Creates a new request with a fresh UUID and `queued_at = now`.
    #[must_use]
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            fleet_id: None,
            prompt: prompt.into(),
            description: None,
            name: None,
            role: None,
            model: None,
            provider: None,
            cwd: None,
            depends_on: Vec::new(),
            isolation: None,
            queued_at: OffsetDateTime::now_utc(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PermissionMode, TaskId};

    // ── FleetRunStatus::from_member_statuses ─────────────────────────────────

    #[test]
    fn empty_members_is_pending() {
        assert_eq!(
            FleetRunStatus::from_member_statuses(&[]),
            FleetRunStatus::Pending
        );
    }

    #[test]
    fn any_running_makes_run_running() {
        let statuses = [
            TaskStatus::Pending,
            TaskStatus::Running,
            TaskStatus::Completed,
        ];
        assert_eq!(
            FleetRunStatus::from_member_statuses(&statuses),
            FleetRunStatus::Running
        );
    }

    #[test]
    fn all_completed_makes_fleet_completed() {
        let statuses = [TaskStatus::Completed, TaskStatus::Completed];
        assert_eq!(
            FleetRunStatus::from_member_statuses(&statuses),
            FleetRunStatus::Completed
        );
    }

    #[test]
    fn any_failed_makes_fleet_failed() {
        let statuses = [TaskStatus::Completed, TaskStatus::Failed];
        assert_eq!(
            FleetRunStatus::from_member_statuses(&statuses),
            FleetRunStatus::Failed
        );
    }

    #[test]
    fn killed_member_makes_fleet_failed() {
        let statuses = [TaskStatus::Killed];
        assert_eq!(
            FleetRunStatus::from_member_statuses(&statuses),
            FleetRunStatus::Failed
        );
    }

    #[test]
    fn all_cancelled_makes_fleet_cancelled() {
        let statuses = [TaskStatus::Cancelled, TaskStatus::Cancelled];
        assert_eq!(
            FleetRunStatus::from_member_statuses(&statuses),
            FleetRunStatus::Cancelled
        );
    }

    #[test]
    fn all_pending_stays_pending() {
        let statuses = [TaskStatus::Pending, TaskStatus::Pending];
        assert_eq!(
            FleetRunStatus::from_member_statuses(&statuses),
            FleetRunStatus::Pending
        );
    }

    // ── Serde round-trips ─────────────────────────────────────────────────────

    #[test]
    fn fleet_run_state_serde_round_trip() {
        let run = FleetRunState::new("test fleet", PermissionMode::Default, None);
        let json = serde_json::to_string(&run).expect("serialize");
        let decoded: FleetRunState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(run, decoded);
    }

    #[test]
    fn fleet_member_request_serde_round_trip() {
        let mut req = FleetMemberRequest::new("do the thing");
        req.description = Some("review".into());
        req.model = Some("claude-3-5-sonnet".into());

        let json = serde_json::to_string(&req).expect("serialize");
        let decoded: FleetMemberRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(req, decoded);
    }

    #[test]
    fn fleet_member_request_role_field_round_trips() {
        let mut req = FleetMemberRequest::new("implement the feature");
        req.role = Some("rust-engineer".into());

        let json = serde_json::to_string(&req).expect("serialize");
        let decoded: FleetMemberRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.role.as_deref(), Some("rust-engineer"));
    }

    #[test]
    fn fleet_run_state_schema_version_defaults_on_deserialize() {
        // Simulate a stored object without `schema_version`.
        let raw = serde_json::json!({
            "id": FleetId::new().to_string(),
            "description": "old run",
            "status": "pending",
            "permission_mode": "default",
            "started_at": "2024-01-01T00:00:00Z"
        });
        let run: FleetRunState =
            serde_json::from_value(raw).expect("deserialize without schema_version");
        assert_eq!(run.schema_version, FLEET_SCHEMA_VERSION);
    }

    #[test]
    fn fleet_run_reconcile_sets_finished_at_when_terminal() {
        let mut run = FleetRunState::new("x", PermissionMode::Default, None);
        assert!(run.finished_at.is_none());
        run.reconcile_status(&[TaskStatus::Completed]);
        assert_eq!(run.status, FleetRunStatus::Completed);
        assert!(run.finished_at.is_some());
        // Second call must not overwrite.
        let first = run.finished_at;
        run.reconcile_status(&[TaskStatus::Completed]);
        assert_eq!(run.finished_at, first);
    }

    // ── Backward-compatibility: old JSON missing new fields ───────────────────

    #[test]
    fn fleet_run_state_old_json_missing_new_fields_deserializes() {
        // Simulate a FleetRunState written before max_concurrency and
        // dispatched_requests were added; both fields must default cleanly.
        let raw = serde_json::json!({
            "schema_version": 1,
            "id": FleetId::new().to_string(),
            "description": "legacy run",
            "status": "running",
            "permission_mode": "default",
            "started_at": "2024-06-01T12:00:00Z"
        });
        let run: FleetRunState =
            serde_json::from_value(raw).expect("deserialize legacy FleetRunState");
        assert!(run.max_concurrency.is_none());
        assert!(run.dispatched_requests.is_empty());
    }

    #[test]
    fn fleet_member_request_old_json_missing_depends_on_deserializes() {
        // Simulate a FleetMemberRequest written before depends_on was added.
        let raw = serde_json::json!({
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "prompt": "do the thing",
            "queued_at": "2024-06-01T12:00:00Z"
        });
        let req: FleetMemberRequest =
            serde_json::from_value(raw).expect("deserialize legacy FleetMemberRequest");
        assert!(req.depends_on.is_empty());
    }

    // ── record_dispatch ───────────────────────────────────────────────────────

    #[test]
    fn record_dispatch_adds_to_maps_and_deduplicates_member_task_ids() {
        let mut run = FleetRunState::new("dispatch-test", PermissionMode::Default, None);
        let task_a = TaskId::new();
        let task_b = TaskId::new();

        run.record_dispatch("req-a".into(), task_a);
        run.record_dispatch("req-b".into(), task_b);
        // Duplicate: same task_id for same request_id; member_task_ids must not grow.
        run.record_dispatch("req-a".into(), task_a);

        assert_eq!(run.dispatched_requests.get("req-a"), Some(&task_a));
        assert_eq!(run.dispatched_requests.get("req-b"), Some(&task_b));
        assert_eq!(run.member_task_ids.len(), 2);
        assert!(run.member_task_ids.contains(&task_a));
        assert!(run.member_task_ids.contains(&task_b));
    }

    #[test]
    fn record_dispatch_round_trips_via_json() {
        let mut run = FleetRunState::new("roundtrip", PermissionMode::Default, None);
        let tid = TaskId::new();
        run.record_dispatch("step-1".into(), tid);

        let json = serde_json::to_string(&run).expect("serialize");
        let decoded: FleetRunState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.dispatched_requests.get("step-1"), Some(&tid));
    }

    #[test]
    fn fleet_run_state_max_concurrency_round_trips() {
        let mut run = FleetRunState::new("mc-test", PermissionMode::Default, None);
        run.max_concurrency = Some(3);
        let json = serde_json::to_string(&run).expect("serialize");
        let decoded: FleetRunState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.max_concurrency, Some(3));
    }

    #[test]
    fn fleet_member_request_depends_on_round_trips() {
        let mut req = FleetMemberRequest::new("step 2");
        req.depends_on = vec!["step-1".into(), "step-0".into()];
        let json = serde_json::to_string(&req).expect("serialize");
        let decoded: FleetMemberRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.depends_on, vec!["step-1", "step-0"]);
    }

    #[test]
    fn fleet_member_request_empty_depends_on_is_not_serialized() {
        let req = FleetMemberRequest::new("standalone");
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(!json.contains("depends_on"), "should be omitted: {json}");
    }

    #[test]
    fn fleet_run_state_empty_dispatched_requests_is_not_serialized() {
        let run = FleetRunState::new("no-dispatches", PermissionMode::Default, None);
        let json = serde_json::to_string(&run).expect("serialize");
        assert!(
            !json.contains("dispatched_requests"),
            "should be omitted: {json}"
        );
    }

    // ── WorktreeIsolation serde tests ─────────────────────────────────────────

    #[test]
    fn worktree_isolation_mode_round_trips() {
        let mode = WorktreeIsolationMode::Worktree;
        let json = serde_json::to_string(&mode).expect("serialize");
        assert_eq!(json, r#""worktree""#);
        let decoded: WorktreeIsolationMode = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, mode);
    }

    #[test]
    fn worktree_isolation_without_branch_round_trips() {
        let iso = WorktreeIsolation {
            mode: WorktreeIsolationMode::Worktree,
            branch: None,
        };
        let json = serde_json::to_string(&iso).expect("serialize");
        assert!(
            !json.contains("branch"),
            "absent branch should be omitted: {json}"
        );
        let decoded: WorktreeIsolation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, iso);
    }

    #[test]
    fn worktree_isolation_with_explicit_branch_round_trips() {
        let iso = WorktreeIsolation {
            mode: WorktreeIsolationMode::Worktree,
            branch: Some("feat/my-worktree".into()),
        };
        let json = serde_json::to_string(&iso).expect("serialize");
        let decoded: WorktreeIsolation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.branch.as_deref(), Some("feat/my-worktree"));
    }

    #[test]
    fn fleet_member_request_isolation_round_trips() {
        let mut req = FleetMemberRequest::new("isolated task");
        req.isolation = Some(WorktreeIsolation {
            mode: WorktreeIsolationMode::Worktree,
            branch: None,
        });
        let json = serde_json::to_string(&req).expect("serialize");
        let decoded: FleetMemberRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.isolation, req.isolation);
    }

    #[test]
    fn fleet_member_request_no_isolation_not_serialized() {
        let req = FleetMemberRequest::new("plain task");
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(!json.contains("isolation"), "should be omitted: {json}");
    }

    /// Legacy JSON without `isolation` must deserialize cleanly.
    #[test]
    fn fleet_member_request_legacy_json_missing_isolation_deserializes() {
        let raw = serde_json::json!({
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "prompt": "do the thing",
            "queued_at": "2024-06-01T12:00:00Z"
        });
        let req: FleetMemberRequest =
            serde_json::from_value(raw).expect("deserialize legacy request");
        assert!(req.isolation.is_none());
    }
}
