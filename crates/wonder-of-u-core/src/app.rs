use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    sync::{Arc, RwLock},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use crate::{
    AdditionalWorkingDirectory, AuthState, FeatureSet, FleetId, MessageEnvelope, PermissionMode,
    ProviderReadiness, Result, SessionId, TaskId, ToolUseId, WonderError,
    agent_name_registry::{AgentNameRegistry, NameConflictError, RegisterOutcome},
};
/// Represents token usage
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Stores the input tokens
    pub input_tokens: u64,
    /// Stores the output tokens
    pub output_tokens: u64,
    /// Stores the cache creation tokens
    pub cache_creation_tokens: u64,
    /// Stores the cache read tokens
    pub cache_read_tokens: u64,
}

impl TokenUsage {
    /// Constant fn
    #[must_use]
    pub const fn total_tokens(self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_creation_tokens + self.cache_read_tokens
    }
    /// Constant fn
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.total_tokens() == 0
    }

    /// Adds a value
    pub fn add(&mut self, other: Self) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_tokens += other.cache_creation_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
    }
}
/// Represents cost state
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostState {
    /// Stores the usage
    pub usage: TokenUsage,
    /// Stores the estimated cost usd
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
    /// Stores the updated at
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl CostState {
    /// Creates a new value
    #[must_use]
    pub fn new() -> Self {
        Self {
            usage: TokenUsage::default(),
            estimated_cost_usd: None,
            updated_at: OffsetDateTime::now_utc(),
        }
    }
    /// Returns whether empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.usage.is_zero() && self.estimated_cost_usd.is_none()
    }

    /// Records usage
    pub fn record_usage(&mut self, usage: TokenUsage, estimated_cost_usd: Option<f64>) {
        self.usage.add(usage);
        if let Some(cost) = estimated_cost_usd {
            self.estimated_cost_usd = Some(self.estimated_cost_usd.unwrap_or_default() + cost);
        }
        self.updated_at = OffsetDateTime::now_utc();
    }
}

impl Default for CostState {
    fn default() -> Self {
        Self::new()
    }
}

/// Session-scoped git worktree state persisted across tool calls.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeWorktreeState {
    /// Stores the original cwd before entering the worktree.
    pub original_cwd: PathBuf,
    /// Stores the repository root used to create the worktree.
    pub repository_root: PathBuf,
    /// Stores the linked worktree path.
    pub worktree_path: PathBuf,
    /// Stores the linked worktree branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_branch: Option<String>,
    /// Stores the original branch at enter time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_branch: Option<String>,
    /// Stores the original head commit at enter time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_head_commit: Option<String>,
    /// Stores an optional tmux session name associated with the worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmux_session_name: Option<String>,
}
/// Represents session state
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    /// Stores the id
    pub id: SessionId,
    /// Stores the title
    pub title: String,
    /// Stores the cwd
    pub cwd: PathBuf,
    /// Stores the git branch
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    /// Stores the entrypoint
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    /// Stores the app version
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    /// Stores the active EnterWorktree session state when the runtime switched into one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<RuntimeWorktreeState>,
    /// Stores the tags
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Stores the created at
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Stores the updated at
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl SessionState {
    /// Creates a new value
    #[must_use]
    pub fn new(cwd: PathBuf) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            id: SessionId::new(),
            title: "Untitled session".into(),
            cwd,
            git_branch: None,
            entrypoint: None,
            app_version: None,
            worktree: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}
/// Enumerates input mode
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    /// Represents prompt
    #[default]
    Prompt,
    /// Represents bash
    Bash,
    /// Represents permission pending
    PermissionPending,
    /// Represents task notification
    TaskNotification,
}
/// Enumerates queue placement
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueuePlacement {
    /// Represents now
    Now,
    /// Represents next
    Next,
    /// Represents later
    Later,
}

/// Configures how much extra reasoning effort the model should spend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ThinkingEffort {
    /// Use the lowest available thinking effort.
    Low,
    /// Use the default balanced thinking effort.
    Medium,
    /// Use the highest available thinking effort.
    High,
}

#[allow(clippy::derivable_impls)]
impl Default for ThinkingEffort {
    fn default() -> Self {
        Self::Medium
    }
}

/// Represents queued command
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueuedCommand {
    /// Stores the command
    pub command: String,
    /// Stores the placement
    pub placement: QueuePlacement,
}
/// Represents pending provider tool call
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingProviderToolCall {
    /// Stores the call identifier
    pub call_id: String,
    /// Stores the tool name
    pub tool_name: String,
    /// Stores the arguments
    pub arguments: Value,
}
/// Represents pending provider tool result
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PendingProviderToolResult {
    /// Stores the call identifier
    pub call_id: String,
    /// Stores the content
    pub content: String,
}
/// Represents pending tool conversation round
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PendingToolConversationRound {
    /// Stores the assistant text
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_text: Option<String>,
    /// Stores the calls
    #[serde(default)]
    pub calls: Vec<PendingProviderToolCall>,
    /// Stores the results
    #[serde(default)]
    pub results: Vec<PendingProviderToolResult>,
}
/// Represents pending local tool call
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingLocalToolCall {
    /// Stores the provider call
    pub provider_call: PendingProviderToolCall,
    /// Stores the use identifier
    pub use_id: ToolUseId,
}
/// Represents pending tool approval state
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingToolApprovalState {
    /// Stores the request prompt
    pub request_prompt: String,
    /// Stores the rounds
    #[serde(default)]
    pub rounds: Vec<PendingToolConversationRound>,
    /// Stores the current round
    pub current_round: PendingToolConversationRound,
    /// Stores the pending call
    pub pending_call: PendingLocalToolCall,
    /// Stores the remaining calls
    #[serde(default)]
    pub remaining_calls: Vec<PendingLocalToolCall>,
    /// Stores the reason
    pub reason: String,
}
/// Enumerates task kind
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// Represents local shell
    #[default]
    LocalShell,
    /// Represents local agent
    LocalAgent,
    /// Represents remote agent
    RemoteAgent,
}
/// Enumerates task status
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Represents pending
    Pending,
    /// Represents running
    Running,
    /// Represents completed
    Completed,
    /// Represents failed
    Failed,
    /// Represents killed
    Killed,
    /// Represents cancelled
    Cancelled,
}

impl TaskStatus {
    /// Constant fn
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Killed | Self::Cancelled
        )
    }
}
/// Enumerates agent runtime
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntime {
    /// Represents metadata only
    MetadataOnly,
    /// Represents prompt subprocess
    PromptSubprocess,
    /// Represents deferred
    Deferred,
}
/// Enumerates task backend support
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskBackendSupport {
    /// Represents supported
    Supported,
    /// Represents unsupported
    Unsupported,
    /// Represents deferred
    Deferred,
}

impl TaskBackendSupport {
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Unsupported => "unsupported",
            Self::Deferred => "deferred",
        }
    }
}
/// Enumerates task backend flow
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskBackendFlow {
    /// Represents transport
    Transport,
    /// Represents monitor
    Monitor,
    /// Represents checker
    Checker,
}

impl TaskBackendFlow {
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::Monitor => "monitor",
            Self::Checker => "checker",
        }
    }
}
/// Represents task backend state
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskBackendState {
    /// Stores the flow
    pub flow: TaskBackendFlow,
    /// Stores the support
    pub support: TaskBackendSupport,
    /// Stores the reason
    pub reason: String,
}

impl TaskBackendState {
    /// Handles supported
    #[must_use]
    pub fn supported(flow: TaskBackendFlow, reason: impl Into<String>) -> Self {
        Self {
            flow,
            support: TaskBackendSupport::Supported,
            reason: reason.into(),
        }
    }
    /// Handles unsupported
    #[must_use]
    pub fn unsupported(flow: TaskBackendFlow, reason: impl Into<String>) -> Self {
        Self {
            flow,
            support: TaskBackendSupport::Unsupported,
            reason: reason.into(),
        }
    }
    /// Handles deferred
    #[must_use]
    pub fn deferred(flow: TaskBackendFlow, reason: impl Into<String>) -> Self {
        Self {
            flow,
            support: TaskBackendSupport::Deferred,
            reason: reason.into(),
        }
    }
    /// Handles summary
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{} is {}: {}",
            self.flow.label(),
            self.support.label(),
            self.reason
        )
    }
}
/// Enumerates remote task type
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RemoteTaskType {
    /// Represents remote agent
    RemoteAgent,
    /// Represents background pr
    BackgroundPr,
    /// Represents autofix pr
    AutofixPr,
    /// Represents ultraplan
    Ultraplan,
    /// Represents ultrareview
    Ultrareview,
}

impl RemoteTaskType {
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::RemoteAgent => "remote-agent",
            Self::BackgroundPr => "background-pr",
            Self::AutofixPr => "autofix-pr",
            Self::Ultraplan => "ultraplan",
            Self::Ultrareview => "ultrareview",
        }
    }
}
/// Enumerates remote task metadata
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteTaskMetadata {
    /// Represents pull request
    PullRequest {
        /// Stores the owner
        owner: String,
        /// Stores the repo
        repo: String,
        /// Stores the pr number
        pr_number: u64,
    },
}
/// Represents remote task state
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteTaskState {
    /// Stores the task type
    pub task_type: RemoteTaskType,
    /// Stores the transport
    pub transport: TaskBackendState,
    /// Stores the monitor
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<TaskBackendState>,
    /// Stores the checker
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checker: Option<TaskBackendState>,
    /// Stores the session identifier
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Stores the metadata
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<RemoteTaskMetadata>,
    /// Stores the long running
    #[serde(default)]
    pub long_running: bool,
}

impl RemoteTaskState {
    /// Handles deferred
    #[must_use]
    pub fn deferred(task_type: RemoteTaskType, metadata: Option<RemoteTaskMetadata>) -> Self {
        let task_label = task_type.label();
        let transport = TaskBackendState::deferred(
            TaskBackendFlow::Transport,
            format!(
                "{task_label} tasks require a remote session transport that is not implemented in this Rust runtime"
            ),
        );
        let monitor = match task_type {
            RemoteTaskType::AutofixPr | RemoteTaskType::Ultraplan | RemoteTaskType::Ultrareview => {
                Some(TaskBackendState::unsupported(
                    TaskBackendFlow::Monitor,
                    format!("{task_label} monitor flow is not implemented in this Rust runtime"),
                ))
            }
            RemoteTaskType::RemoteAgent | RemoteTaskType::BackgroundPr => None,
        };
        let checker = match task_type {
            RemoteTaskType::BackgroundPr | RemoteTaskType::AutofixPr => {
                Some(TaskBackendState::unsupported(
                    TaskBackendFlow::Checker,
                    format!(
                        "{task_label} completion checker flow is not implemented in this Rust runtime"
                    ),
                ))
            }
            RemoteTaskType::RemoteAgent
            | RemoteTaskType::Ultraplan
            | RemoteTaskType::Ultrareview => None,
        };
        Self {
            task_type,
            transport,
            monitor,
            checker,
            session_id: None,
            metadata,
            long_running: matches!(task_type, RemoteTaskType::AutofixPr),
        }
    }
    /// Handles status summary
    #[must_use]
    pub fn status_summary(&self) -> String {
        let mut parts = vec![self.transport.summary()];
        if let Some(monitor) = &self.monitor {
            parts.push(monitor.summary());
        }
        if let Some(checker) = &self.checker {
            parts.push(checker.summary());
        }
        format!("{} backend: {}", self.task_type.label(), parts.join("; "))
    }
    /// Handles start error message
    #[must_use]
    pub fn start_error_message(&self) -> String {
        format!(
            "cannot start {} task: {}",
            self.task_type.label(),
            self.status_summary()
        )
    }
    /// Handles stop error message
    #[must_use]
    pub fn stop_error_message(&self) -> String {
        format!(
            "cannot stop {} task: {}",
            self.task_type.label(),
            self.status_summary()
        )
    }
}
/// Represents agent task state
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentTaskState {
    /// Stores the name
    pub name: String,
    /// Stores the prompt
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Stores the provider
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Stores the model
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Stores the runtime
    pub runtime: AgentRuntime,
}

impl AgentTaskState {
    /// Handles metadata only
    #[must_use]
    pub fn metadata_only(
        name: impl Into<String>,
        prompt: impl Into<String>,
        provider: Option<String>,
        model: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            prompt: Some(prompt.into()),
            provider,
            model,
            runtime: AgentRuntime::MetadataOnly,
        }
    }
    /// Handles prompt subprocess
    #[must_use]
    pub fn prompt_subprocess(
        name: impl Into<String>,
        prompt: impl Into<String>,
        provider: Option<String>,
        model: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            prompt: Some(prompt.into()),
            provider,
            model,
            runtime: AgentRuntime::PromptSubprocess,
        }
    }
}
/// Lightweight progress counters written into [`TaskState`] by the agent runtime.
///
/// All fields default to zero / `None` so that existing persisted task files
/// (which have no `progress` key) deserialize correctly without errors.
///
/// # Examples
///
/// ```
/// use wonder_of_u_core::TaskProgress;
///
/// let mut p = TaskProgress::default();
/// p.record_tool_use("bash");
/// assert_eq!(p.tool_use_count, 1);
/// assert_eq!(p.last_tool_name.as_deref(), Some("bash"));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskProgress {
    /// Number of tool calls made by this task so far.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub tool_use_count: u32,
    /// Cumulative token count (input + output + cache) consumed by this task.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub token_count: u64,
    /// Name of the most-recently invoked tool, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tool_name: Option<String>,
    /// Wall-clock time of the most-recent tool invocation.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub last_tool_at: Option<OffsetDateTime>,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}
fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

impl TaskProgress {
    /// Records a single tool invocation.
    ///
    /// Increments [`tool_use_count`](Self::tool_use_count) and updates
    /// [`last_tool_name`](Self::last_tool_name) and
    /// [`last_tool_at`](Self::last_tool_at).
    pub fn record_tool_use(&mut self, tool_name: &str) {
        self.tool_use_count = self.tool_use_count.saturating_add(1);
        self.last_tool_name = Some(tool_name.to_owned());
        self.last_tool_at = Some(OffsetDateTime::now_utc());
    }

    /// Adds `tokens` to [`token_count`](Self::token_count).
    pub fn record_tokens(&mut self, tokens: u64) {
        self.token_count = self.token_count.saturating_add(tokens);
    }

    /// Returns `true` when at least one metric has been recorded.
    #[must_use]
    pub fn has_data(&self) -> bool {
        self.tool_use_count > 0 || self.token_count > 0
    }
}

/// Represents task state
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskState {
    /// Stores the id
    pub id: TaskId,
    /// Stores the kind
    pub kind: TaskKind,
    /// Stores the description
    pub description: String,
    /// Stores the status
    pub status: TaskStatus,
    /// Optional fleet run this task belongs to.
    ///
    /// Absent for tasks created before fleet support; old task files remain
    /// deserializable because this field carries `#[serde(default)]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fleet_id: Option<FleetId>,
    /// Fleet member request id that triggered this task, if any.
    ///
    /// Populated when the task is launched by `fleet dispatch` / `fleet reconcile`
    /// or the `agent` tool.  Used to correlate result sidecars back to the
    /// originating request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fleet_request_id: Option<String>,
    /// Stores the parent identifier
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<TaskId>,
    /// Stores the cwd
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// Stores the command
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Stores the status message
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    /// Stores the pid
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Stores the process identity
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_identity: Option<String>,
    /// Stores the last heartbeat at
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub last_heartbeat_at: Option<OffsetDateTime>,
    /// Stores the exit code
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Stores the agent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentTaskState>,
    /// Stores the remote
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteTaskState>,
    /// Stores the output log
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_log: Option<PathBuf>,
    /// The git worktree branch this agent task is running inside, if any.
    ///
    /// Set when the task was launched with worktree isolation; absent for
    /// tasks created before worktree isolation was introduced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_branch: Option<String>,
    /// Filesystem path to the git worktree this agent task is running inside.
    ///
    /// Set alongside `worktree_branch` at launch time. After task completion
    /// the runtime attempts to remove the worktree if it is clean; the field
    /// is retained even after cleanup so the path can be surfaced in
    /// `/tasks show` output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<PathBuf>,
    /// HEAD commit hash captured at worktree creation time.
    ///
    /// Used during post-task cleanup to determine whether the agent made new
    /// commits on the worktree branch. A `None` value forces the cleanup logic
    /// to count only uncommitted files (no commit comparison).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_head_commit: Option<String>,
    /// Live progress counters (tool calls, tokens, last tool activity).
    ///
    /// Absent in task files written before progress tracking was introduced;
    /// those files deserialize with all-zero / `None` values via `#[serde(default)]`.
    #[serde(default, skip_serializing_if = "skip_progress")]
    pub progress: TaskProgress,
    /// Short (3-5 word) summary of what the agent subprocess is currently doing.
    ///
    /// Generated every 30 s by the parent process reading the agent's transcript
    /// and asking the API for a present-tense label (e.g. "Reading runAgent.ts").
    /// Absent for tasks created before agent-summary was introduced, or when no
    /// summary has been generated yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_summary: Option<String>,
    /// Stores the started at
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    /// Stores the finished at
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
}

fn skip_progress(p: &TaskProgress) -> bool {
    !p.has_data()
}

impl TaskState {
    /// Handles pending
    #[must_use]
    pub fn pending(description: impl Into<String>) -> Self {
        Self {
            id: TaskId::new(),
            kind: TaskKind::LocalShell,
            description: description.into(),
            status: TaskStatus::Pending,
            fleet_id: None,
            fleet_request_id: None,
            parent_id: None,
            cwd: None,
            command: None,
            status_message: None,
            pid: None,
            process_identity: None,
            last_heartbeat_at: None,
            exit_code: None,
            agent: None,
            remote: None,
            output_log: None,
            worktree_branch: None,
            worktree_path: None,
            worktree_head_commit: None,
            progress: TaskProgress::default(),
            agent_summary: None,
            started_at: OffsetDateTime::now_utc(),
            finished_at: None,
        }
    }
    /// Handles pending shell
    #[must_use]
    pub fn pending_shell(
        description: impl Into<String>,
        command: impl Into<String>,
        cwd: impl Into<PathBuf>,
    ) -> Self {
        let mut task = Self::pending(description);
        task.kind = TaskKind::LocalShell;
        task.command = Some(command.into());
        task.cwd = Some(cwd.into());
        task
    }
    /// Handles pending agent
    #[must_use]
    pub fn pending_agent(description: impl Into<String>, agent: AgentTaskState) -> Self {
        let mut task = Self::pending(description);
        task.kind = TaskKind::LocalAgent;
        task.status_message = Some(
            match agent.runtime {
                AgentRuntime::MetadataOnly => "agent metadata recorded for catalog/status tracking",
                AgentRuntime::PromptSubprocess => {
                    "local agent prompt subprocess is queued for launch"
                }
                AgentRuntime::Deferred => {
                    "legacy agent record; relaunch with prompt_subprocess runtime"
                }
            }
            .into(),
        );
        task.agent = Some(agent);
        task
    }
    /// Handles recorded remote
    #[must_use]
    pub fn recorded_remote(description: impl Into<String>, remote: RemoteTaskState) -> Self {
        let mut task = Self::pending(description);
        task.kind = TaskKind::RemoteAgent;
        task.status_message = Some(remote.status_summary());
        task.remote = Some(remote);
        task
    }

    /// Handles mark running
    pub fn mark_running(
        &mut self,
        pid: Option<u32>,
        process_identity: Option<String>,
        last_heartbeat_at: Option<OffsetDateTime>,
        status_message: Option<String>,
    ) {
        self.status = TaskStatus::Running;
        self.pid = pid;
        self.process_identity = process_identity;
        self.last_heartbeat_at = last_heartbeat_at;
        self.status_message = status_message;
        self.exit_code = None;
        self.finished_at = None;
    }

    /// Handles mark finished
    pub fn mark_finished(
        &mut self,
        status: TaskStatus,
        exit_code: Option<i32>,
        status_message: Option<String>,
    ) {
        self.status = status;
        self.exit_code = exit_code;
        self.status_message = status_message;
        self.finished_at = Some(OffsetDateTime::now_utc());
    }

    /// Records a single tool call in this task's progress counters.
    ///
    /// Delegates to [`TaskProgress::record_tool_use`]; call site should
    /// persist the updated `TaskState` to storage afterwards.
    pub fn record_tool_use(&mut self, tool_name: &str) {
        self.progress.record_tool_use(tool_name);
    }

    /// Adds `tokens` to this task's cumulative token counter.
    ///
    /// Delegates to [`TaskProgress::record_tokens`]; call site should
    /// persist the updated `TaskState` to storage afterwards.
    pub fn record_tokens(&mut self, tokens: u64) {
        self.progress.record_tokens(tokens);
    }

    /// Returns a compact one-line progress summary suitable for status displays,
    /// or `None` when no progress has been recorded yet.
    ///
    /// # Examples
    ///
    /// ```
    /// use wonder_of_u_core::TaskState;
    ///
    /// let mut task = TaskState::pending("demo");
    /// assert!(task.progress_summary().is_none());
    ///
    /// task.record_tool_use("bash");
    /// task.record_tokens(512);
    /// let summary = task.progress_summary().unwrap();
    /// assert!(summary.contains("tools=1"));
    /// assert!(summary.contains("tokens=512"));
    /// ```
    #[must_use]
    pub fn progress_summary(&self) -> Option<String> {
        if !self.progress.has_data() {
            return None;
        }
        let mut parts = Vec::new();
        if self.progress.tool_use_count > 0 {
            parts.push(format!("tools={}", self.progress.tool_use_count));
        }
        if self.progress.token_count > 0 {
            parts.push(format!("tokens={}", self.progress.token_count));
        }
        if let Some(ref name) = self.progress.last_tool_name {
            parts.push(format!("last_tool={name}"));
        }
        Some(parts.join(" "))
    }
}
/// Represents app state
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    /// Stores the session
    pub session: SessionState,
    /// Stores the features
    pub features: FeatureSet,
    /// Stores the permission mode
    pub permission_mode: PermissionMode,
    /// Stores the additional working directories
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_working_directories: Vec<AdditionalWorkingDirectory>,
    /// Stores the input mode
    pub input_mode: InputMode,
    /// Stores the messages
    pub messages: Vec<MessageEnvelope>,
    /// Stores the queued commands
    pub queued_commands: VecDeque<QueuedCommand>,
    /// Stores the background tasks
    pub background_tasks: BTreeMap<TaskId, TaskState>,
    /// Runtime-only name → task-id registry for local `SendMessage` routing.
    ///
    /// Never persisted: populated at task launch time by
    /// [`register_agent_name`][Self::register_agent_name] and cleared
    /// automatically when a task reaches a terminal state via
    /// [`upsert_task`][Self::upsert_task].
    #[serde(skip)]
    pub(crate) agent_name_registry: AgentNameRegistry,
    /// Stores the provider
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Stores the model
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Stores the theme
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Stores the session color
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_color: Option<String>,
    /// Stores the effort level
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_level: Option<String>,
    /// Stores the brief mode
    #[serde(default)]
    pub brief_mode: bool,
    /// Stores the fast mode
    #[serde(default)]
    pub fast_mode: bool,
    /// Stores whether extended thinking is enabled for future provider requests.
    #[serde(default)]
    pub thinking_enabled: bool,
    /// Stores the configured effort level for thinking-capable models.
    #[serde(default)]
    pub thinking_effort: ThinkingEffort,
    /// When enabled, instructs the model to minimise token usage in every
    /// reply.  Injected as a system-prompt prefix so no user message is needed.
    #[serde(default)]
    pub optimize_token_mode: bool,
    /// Optional advisor/secondary model for multi-model reasoning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advisor_model: Option<String>,
    /// Stores the auth
    #[serde(default)]
    pub auth: AuthState,
    /// Stores the pending tool approval
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_tool_approval: Option<PendingToolApprovalState>,
    /// Maximum context window size for the current model (tokens).
    /// Set from provider response or model config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_size: Option<u64>,
    /// Stores the costs
    pub costs: CostState,
    /// Task IDs for which a `<task-notification>` XML message has already been
    /// injected into this session's model-facing transcript.
    ///
    /// Persisted so that re-opening the same session (or a crash-recovery load)
    /// never re-injects notifications for tasks whose terminal state was already
    /// recorded.  Serialisation omits the field when empty to keep state files
    /// compact.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub injected_task_notifications: BTreeSet<TaskId>,
}

impl AppState {
    /// Creates a new value
    #[must_use]
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            session: SessionState::new(cwd),
            features: FeatureSet::first_release(),
            permission_mode: PermissionMode::default(),
            additional_working_directories: Vec::new(),
            input_mode: InputMode::default(),
            messages: Vec::new(),
            queued_commands: VecDeque::new(),
            background_tasks: BTreeMap::new(),
            agent_name_registry: AgentNameRegistry::new(),
            provider: None,
            model: None,
            theme: None,
            session_color: None,
            effort_level: None,
            brief_mode: false,
            fast_mode: false,
            thinking_enabled: false,
            thinking_effort: ThinkingEffort::default(),
            optimize_token_mode: false,
            advisor_model: None,
            auth: AuthState::default(),
            pending_tool_approval: None,
            context_window_size: None,
            costs: CostState::new(),
            injected_task_notifications: BTreeSet::new(),
        }
    }

    /// Handles push message
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

    /// Handles queue command
    pub fn queue_command(&mut self, command: impl Into<String>, placement: QueuePlacement) {
        self.queued_commands.push_back(QueuedCommand {
            command: command.into(),
            placement,
        });
    }

    /// Records cost usage
    pub fn record_cost_usage(&mut self, usage: TokenUsage, estimated_cost_usd: Option<f64>) {
        self.costs.record_usage(usage, estimated_cost_usd);
        self.session.updated_at = self.costs.updated_at;
    }

    /// Handles set provider context
    pub fn set_provider_context(
        &mut self,
        provider: Option<String>,
        model: Option<String>,
        auth: AuthState,
    ) {
        self.provider = provider;
        self.model = model;
        self.auth = auth;
        self.context_window_size = None;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    /// Handles set context window size
    pub fn set_context_window_size(&mut self, context_window_size: Option<u64>) {
        self.context_window_size = context_window_size;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    /// Handles set theme
    pub fn set_theme(&mut self, theme: Option<String>) {
        self.theme = theme;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    /// Handles set session color
    pub fn set_session_color(&mut self, session_color: Option<String>) {
        self.session_color = session_color;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    /// Handles set effort level
    pub fn set_effort_level(&mut self, effort_level: Option<String>) {
        self.effort_level = effort_level;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    /// Handles set brief mode
    pub fn set_brief_mode(&mut self, brief_mode: bool) {
        self.brief_mode = brief_mode;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    /// Handles set fast mode
    pub fn set_fast_mode(&mut self, fast_mode: bool) {
        self.fast_mode = fast_mode;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    /// Handles set thinking enabled
    pub fn set_thinking_enabled(&mut self, thinking_enabled: bool) {
        self.thinking_enabled = thinking_enabled;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    /// Handles set thinking effort
    pub fn set_thinking_effort(&mut self, effort: ThinkingEffort) {
        self.thinking_effort = effort;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    /// Handles set advisor model
    pub fn set_advisor_model(&mut self, model: Option<String>) {
        self.advisor_model = model;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    /// Toggles or sets the token-optimisation mode for this session.
    pub fn set_optimize_token_mode(&mut self, enabled: bool) {
        self.optimize_token_mode = enabled;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    /// Handles effective system prompt
    #[must_use]
    pub fn effective_system_prompt(&self, explicit: Option<String>) -> Option<String> {
        const BRIEF_MODE_PROMPT: &str =
            "Be brief. Keep user-facing output concise and direct unless the user asks for detail.";
        // Instruct the model to minimise token usage when optimize-token mode is on.
        // Placed before the brief hint so both can coexist in priority order.
        const OPTIMIZE_TOKEN_PROMPT: &str = "Minimize token usage. Omit preambles, filler words, \
            padding, and unnecessary repetition. Respond with only the essential information \
            requested, in the most compact form possible.";

        let explicit = explicit
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());

        // Build up injected prefixes; optimize-token comes first, brief appended after.
        let mut injected: Vec<&str> = Vec::new();
        if self.optimize_token_mode {
            injected.push(OPTIMIZE_TOKEN_PROMPT);
        }
        if self.brief_mode {
            injected.push(BRIEF_MODE_PROMPT);
        }

        match (injected.is_empty(), explicit) {
            (true, None) => None,
            (true, Some(prompt)) => Some(prompt.to_string()),
            (false, None) => Some(injected.join("\n\n")),
            (false, Some(prompt)) => Some(format!("{prompt}\n\n{}", injected.join("\n\n"))),
        }
    }

    /// Handles set session tags
    pub fn set_session_tags(&mut self, tags: Vec<String>) {
        if self.session.tags != tags {
            self.session.tags = tags;
            self.session.updated_at = OffsetDateTime::now_utc();
        }
    }

    /// Handles add additional working directory
    pub fn add_additional_working_directory(&mut self, directory: AdditionalWorkingDirectory) {
        if !self
            .additional_working_directories
            .iter()
            .any(|existing| existing.path == directory.path)
        {
            self.additional_working_directories.push(directory);
            self.session.updated_at = OffsetDateTime::now_utc();
        }
    }

    /// Handles upsert task
    pub fn upsert_task(&mut self, task: TaskState) {
        // Auto-deregister the name when the task reaches a terminal state so
        // the slot becomes available for a future task with the same name.
        if task.status.is_terminal() {
            self.agent_name_registry.deregister_by_task_id(task.id);
        }
        self.background_tasks.insert(task.id, task);
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    /// Registers `name` as owned by `task_id` in the runtime name registry.
    ///
    /// Call this at agent task launch time (after [`upsert_task`][Self::upsert_task])
    /// to make the task addressable by name for local `SendMessage` routing.
    ///
    /// # Errors
    ///
    /// Returns [`NameConflictError`] when `name` is already held by a different
    /// active task.
    pub fn register_agent_name(
        &mut self,
        name: String,
        task_id: TaskId,
    ) -> std::result::Result<RegisterOutcome, NameConflictError> {
        self.agent_name_registry.register(name, task_id)
    }

    /// Looks up the task id currently registered under `name`, or `None`.
    ///
    /// Used by `SendMessage` routing to resolve a bare teammate name to a live
    /// [`TaskId`].
    #[must_use]
    pub fn lookup_agent_by_name(&self, name: &str) -> Option<TaskId> {
        self.agent_name_registry.lookup(name)
    }

    /// Removes any name entry associated with `task_id` from the registry.
    ///
    /// Returns the name that was removed, or `None` if no entry existed.
    /// Normally called automatically by [`upsert_task`][Self::upsert_task] when
    /// a task transitions to a terminal status; this method is provided for
    /// explicit cleanup (e.g. task prune / force-remove paths).
    pub fn deregister_agent_task(&mut self, task_id: TaskId) -> Option<String> {
        self.agent_name_registry.deregister_by_task_id(task_id)
    }

    /// Returns a read-only reference to the agent name registry.
    ///
    /// Useful in tests and diagnostic commands that need to inspect the full
    /// registry contents without mutating it.
    #[must_use]
    pub fn agent_name_registry(&self) -> &AgentNameRegistry {
        &self.agent_name_registry
    }
    /// Handles provider readiness
    #[must_use]
    pub fn provider_readiness(&self) -> ProviderReadiness {
        if self.provider.is_none() || self.model.is_none() {
            return ProviderReadiness::Unconfigured;
        }

        if self.auth.is_ready() {
            ProviderReadiness::Ready
        } else {
            ProviderReadiness::MissingAuth
        }
    }
    /// Returns whether authenticated
    #[must_use]
    pub fn is_authenticated(&self) -> bool {
        matches!(self.provider_readiness(), ProviderReadiness::Ready)
    }
}
/// Constant fn
#[must_use]
pub const fn input_mode_label(mode: InputMode) -> &'static str {
    match mode {
        InputMode::Prompt => "prompt",
        InputMode::Bash => "bash",
        InputMode::PermissionPending => "permission",
        InputMode::TaskNotification => "tasks",
    }
}
/// Constant fn
#[must_use]
pub const fn permission_mode_label(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "accept-edits",
        PermissionMode::BypassPermissions => "bypass",
        PermissionMode::DontAsk => "dont-ask",
        PermissionMode::Plan => "plan",
    }
}
/// Returns the session status text
#[must_use]
pub fn session_status_text(app: &AppState) -> String {
    let running = app
        .background_tasks
        .values()
        .filter(|task| matches!(task.status, TaskStatus::Running))
        .count();
    let pending = app
        .background_tasks
        .values()
        .filter(|task| matches!(task.status, TaskStatus::Pending))
        .count();
    let total_tasks = app.background_tasks.len();

    let mut parts = vec![
        input_mode_label(app.input_mode).to_string(),
        format!("{} messages", app.messages.len()),
    ];

    if total_tasks > 0 {
        let agents = app
            .background_tasks
            .values()
            .filter(|task| matches!(task.kind, TaskKind::LocalAgent))
            .count();
        let remote = app
            .background_tasks
            .values()
            .filter(|task| matches!(task.kind, TaskKind::RemoteAgent))
            .count();
        parts.push(format!("tasks {running}r/{pending}p/{total_tasks}t"));
        if agents > 0 {
            parts.push(format!("agents={agents}"));
        }
        if remote > 0 {
            parts.push(format!("remote={remote}"));
        }
    }

    if let (Some(provider), Some(model)) = (&app.provider, &app.model) {
        parts.push(format!("{provider}:{model}"));
        parts.push(format!("auth={}", app.auth.status_label()));
    }

    if !app.costs.usage.is_zero() {
        parts.push(format!("{} tok", app.costs.usage.total_tokens()));
    }

    parts.join(" | ")
}
/// Returns the session footer text
#[must_use]
pub fn session_footer_text(app: &AppState) -> String {
    let mut parts = vec![
        format!("cwd={}", app.session.cwd.display()),
        format!("permission={}", permission_mode_label(app.permission_mode)),
    ];

    if let Some(branch) = &app.session.git_branch {
        parts.push(format!("branch={branch}"));
    }

    if !app.session.tags.is_empty() {
        parts.push(format!(
            "tags={}",
            app.session
                .tags
                .iter()
                .map(|tag| format!("#{tag}"))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }

    if !app.additional_working_directories.is_empty() {
        parts.push(format!("dirs={}", app.additional_working_directories.len()));
    }

    if !app.queued_commands.is_empty() {
        parts.push(format!("queued={}", app.queued_commands.len()));
    }

    parts.push("ctrl-c interrupt".into());

    parts.join(" | ")
}

/// Small lock-backed store for non-TUI tests and early services. The eventual
/// TUI loop can replace this with channel-owned state where appropriate.
#[derive(Clone, Debug)]
pub struct StateStore {
    inner: Arc<RwLock<AppState>>,
}

impl StateStore {
    /// Creates a new value
    #[must_use]
    pub fn new(state: AppState) -> Self {
        Self {
            inner: Arc::new(RwLock::new(state)),
        }
    }

    /// Handles get
    pub fn get(&self) -> Result<AppState> {
        self.inner
            .read()
            .map(|guard| guard.clone())
            .map_err(|_| WonderError::internal("app state lock poisoned"))
    }

    /// Handles update
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
    use crate::{FeatureFlag, MessageEnvelope};

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

    #[test]
    fn app_state_tracks_cost_usage() {
        let mut state = AppState::new(PathBuf::from("/workspace"));

        state.record_cost_usage(
            TokenUsage {
                input_tokens: 128,
                output_tokens: 32,
                cache_creation_tokens: 16,
                cache_read_tokens: 8,
            },
            Some(0.42),
        );

        assert_eq!(state.costs.usage.total_tokens(), 184);
        assert_eq!(state.costs.estimated_cost_usd, Some(0.42));
        assert_eq!(state.session.updated_at, state.costs.updated_at);
    }

    #[test]
    fn app_state_tracks_context_window_size() {
        let mut state = AppState::new(PathBuf::from("/workspace"));

        assert_eq!(state.context_window_size, None);
        state.set_context_window_size(Some(200_000));
        assert_eq!(state.context_window_size, Some(200_000));

        state.set_provider_context(
            Some("anthropic".into()),
            Some("claude-3-7-sonnet-latest".into()),
            AuthState::default(),
        );
        assert_eq!(state.context_window_size, None);
    }

    #[test]
    fn app_state_defaults_thinking_to_disabled() {
        let mut state = AppState::new(PathBuf::from("/workspace"));
        assert!(!state.thinking_enabled);
        assert_eq!(state.thinking_effort, ThinkingEffort::Medium);

        state.set_thinking_enabled(true);

        assert!(state.thinking_enabled);
    }

    #[test]
    fn app_state_stores_thinking_effort() {
        let mut state = AppState::new(PathBuf::from("/workspace"));

        state.set_thinking_effort(ThinkingEffort::High);

        assert_eq!(state.thinking_effort, ThinkingEffort::High);
    }

    #[test]
    fn app_state_reports_provider_readiness() {
        let mut state = AppState::new(PathBuf::from("/workspace"));
        assert_eq!(state.provider_readiness(), ProviderReadiness::Unconfigured);

        state.set_provider_context(
            Some("openai".into()),
            Some("gpt-4.1".into()),
            AuthState::missing(crate::AuthMaterialKind::ApiKey),
        );
        assert_eq!(state.provider_readiness(), ProviderReadiness::MissingAuth);

        state.set_provider_context(
            Some("openai".into()),
            Some("gpt-4.1".into()),
            AuthState::ready(
                crate::AuthMaterialKind::ApiKey,
                crate::AuthSource::Environment,
            ),
        );
        assert!(state.is_authenticated());
    }

    #[test]
    fn task_helpers_capture_shell_and_agent_metadata() {
        let shell = TaskState::pending_shell("run tests", "cargo test", "/workspace");
        assert_eq!(shell.kind, TaskKind::LocalShell);
        assert_eq!(shell.command.as_deref(), Some("cargo test"));
        assert_eq!(
            shell.cwd.as_deref(),
            Some(std::path::Path::new("/workspace"))
        );

        let agent = TaskState::pending_agent(
            "release planner",
            AgentTaskState::metadata_only(
                "release planner",
                "Summarize release blockers",
                Some("openai".into()),
                Some("gpt-4.1".into()),
            ),
        );
        assert_eq!(agent.kind, TaskKind::LocalAgent);
        assert_eq!(agent.status, TaskStatus::Pending);
        assert_eq!(
            agent.agent.as_ref().map(|agent| agent.runtime),
            Some(AgentRuntime::MetadataOnly)
        );

        let prompt_agent = TaskState::pending_agent(
            "prompt planner",
            AgentTaskState::prompt_subprocess(
                "prompt planner",
                "Summarize release blockers",
                Some("openai".into()),
                Some("gpt-4.1".into()),
            ),
        );
        assert_eq!(
            prompt_agent.agent.as_ref().map(|agent| agent.runtime),
            Some(AgentRuntime::PromptSubprocess)
        );
        assert_eq!(
            prompt_agent.status_message.as_deref(),
            Some("local agent prompt subprocess is queued for launch")
        );

        let remote = TaskState::recorded_remote(
            "background review",
            RemoteTaskState::deferred(
                RemoteTaskType::Ultrareview,
                Some(RemoteTaskMetadata::PullRequest {
                    owner: "wonder".into(),
                    repo: "of-u".into(),
                    pr_number: 42,
                }),
            ),
        );
        assert_eq!(remote.kind, TaskKind::RemoteAgent);
        assert!(
            remote.status_message.as_deref().is_some_and(
                |message| message.contains("ultrareview backend: transport is deferred")
            )
        );
        assert!(
            remote
                .remote
                .as_ref()
                .and_then(|remote| remote.monitor.as_ref())
                .is_some()
        );
    }

    #[test]
    fn session_chrome_helpers_track_provider_tokens_and_tasks() {
        let mut state = AppState::new(PathBuf::from("/workspace"));
        state.provider = Some("copilot".into());
        state.model = Some("gpt-5".into());
        state.set_session_tags(vec!["wip".into()]);
        state.queue_command("/status", QueuePlacement::Later);
        state.record_cost_usage(
            TokenUsage {
                input_tokens: 12,
                output_tokens: 3,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            None,
        );
        state.background_tasks.insert(
            Default::default(),
            TaskState {
                description: "index workspace".into(),
                status: TaskStatus::Running,
                ..TaskState::pending("index workspace")
            },
        );

        assert!(session_status_text(&state).contains("copilot:gpt-5"));
        assert!(session_status_text(&state).contains("15 tok"));
        assert!(session_footer_text(&state).contains("cwd=/workspace"));
        assert!(session_footer_text(&state).contains("tags=#wip"));
        assert!(session_footer_text(&state).contains("queued=1"));
        assert_eq!(input_mode_label(InputMode::Prompt), "prompt");
        assert_eq!(
            permission_mode_label(PermissionMode::AcceptEdits),
            "accept-edits"
        );
    }

    #[test]
    fn remote_task_state_serializes_backend_support_and_metadata() {
        let remote = RemoteTaskState::deferred(
            RemoteTaskType::AutofixPr,
            Some(RemoteTaskMetadata::PullRequest {
                owner: "wonder".into(),
                repo: "of-u".into(),
                pr_number: 7,
            }),
        );

        let value = serde_json::to_value(&remote).expect("serialize remote task");

        assert_eq!(value["task_type"], "autofix-pr");
        assert_eq!(value["transport"]["flow"], "transport");
        assert_eq!(value["transport"]["support"], "deferred");
        assert_eq!(value["monitor"]["flow"], "monitor");
        assert_eq!(value["monitor"]["support"], "unsupported");
        assert_eq!(value["checker"]["flow"], "checker");
        assert_eq!(value["checker"]["support"], "unsupported");
        assert_eq!(value["metadata"]["kind"], "pull_request");
        assert_eq!(value["metadata"]["pr_number"], 7);
        assert!(
            remote
                .start_error_message()
                .contains("cannot start autofix-pr task")
        );
    }

    #[test]
    fn session_status_text_reports_remote_task_counts() {
        let mut state = AppState::new(PathBuf::from("/workspace"));
        state.background_tasks.insert(
            TaskId::new(),
            TaskState::recorded_remote(
                "cloud review",
                RemoteTaskState::deferred(RemoteTaskType::Ultrareview, None),
            ),
        );

        let status = session_status_text(&state);

        assert!(status.contains("tasks 0r/1p/1t"));
        assert!(status.contains("remote=1"));
    }

    #[test]
    fn effective_system_prompt_adds_brief_mode_guidance() {
        let mut state = AppState::new(PathBuf::from("/workspace"));
        assert_eq!(state.effective_system_prompt(None), None);

        state.set_brief_mode(true);
        let brief_prompt = state.effective_system_prompt(None).expect("brief prompt");
        assert!(brief_prompt.contains("Be brief."));

        let combined = state
            .effective_system_prompt(Some("Follow the project's Rust style guide.".into()))
            .expect("combined prompt");
        assert!(combined.contains("Follow the project's Rust style guide."));
        assert!(combined.contains("Keep user-facing output concise and direct"));
    }

    #[test]
    fn set_fast_mode_updates_state() {
        let mut state = AppState::new(PathBuf::from("/workspace"));
        assert!(!state.fast_mode);

        state.set_fast_mode(true);

        assert!(state.fast_mode);
    }

    #[test]
    fn set_optimize_token_mode_updates_state() {
        let mut state = AppState::new(PathBuf::from("/workspace"));
        assert!(!state.optimize_token_mode);

        state.set_optimize_token_mode(true);

        assert!(state.optimize_token_mode);
    }

    #[test]
    fn effective_system_prompt_adds_optimize_token_guidance() {
        let mut state = AppState::new(PathBuf::from("/workspace"));
        assert_eq!(state.effective_system_prompt(None), None);

        state.set_optimize_token_mode(true);
        let prompt = state
            .effective_system_prompt(None)
            .expect("optimize-token prompt");
        assert!(
            prompt.contains("Minimize token usage"),
            "should contain token-minimisation instruction"
        );

        // Explicit system prompt is preserved and the hint is appended.
        let combined = state
            .effective_system_prompt(Some("You are a helpful assistant.".into()))
            .expect("combined");
        assert!(combined.contains("You are a helpful assistant."));
        assert!(combined.contains("Minimize token usage"));
    }

    #[test]
    fn effective_system_prompt_combines_optimize_token_and_brief() {
        let mut state = AppState::new(PathBuf::from("/workspace"));
        state.set_optimize_token_mode(true);
        state.set_brief_mode(true);

        let prompt = state
            .effective_system_prompt(None)
            .expect("combined modes prompt");

        // Both hints must be present; optimize-token comes first.
        let opt_pos = prompt.find("Minimize token usage").expect("opt pos");
        let brief_pos = prompt.find("Be brief.").expect("brief pos");
        assert!(
            opt_pos < brief_pos,
            "optimize-token hint should precede brief hint"
        );
    }

    // ── Remote-cloud-transport deferred-guarantee anchors ─────────────────────
    // These tests encode the decision recorded in docs/adr-remote-cloud-transport.md.
    // They must stay green until all implementation gates in the ADR are cleared.

    /// `RemoteTaskState::deferred` for `RemoteAgent` must surface
    /// `TaskBackendSupport::Deferred` on the transport leg and include the
    /// phrase "not implemented" in the reason — the canonical signal that no
    /// live CCR transport exists yet.
    #[test]
    fn remote_task_state_remote_agent_deferred_transport_is_deferred_and_reason_mentions_not_implemented()
     {
        let state = RemoteTaskState::deferred(RemoteTaskType::RemoteAgent, None);

        assert_eq!(
            state.transport.support,
            TaskBackendSupport::Deferred,
            "RemoteAgent transport support must be Deferred (CCR transport absent)"
        );
        assert!(
            state.transport.reason.contains("not implemented"),
            "transport reason must mention 'not implemented'; got: {:?}",
            state.transport.reason
        );
        // RemoteAgent has no monitor or checker legs — assert those stay absent
        // so callers never accidentally treat missing legs as supported.
        assert!(
            state.monitor.is_none(),
            "RemoteAgent must not have a monitor leg"
        );
        assert!(
            state.checker.is_none(),
            "RemoteAgent must not have a checker leg"
        );
    }

    /// Every `RemoteTaskType::label()` value must be unique and must not
    /// change — downstream systems (serialisation, parity ledger, log
    /// parsers) depend on these strings being stable identifiers.
    #[test]
    fn remote_task_type_labels_are_unique_and_stable() {
        let variants = [
            RemoteTaskType::RemoteAgent,
            RemoteTaskType::BackgroundPr,
            RemoteTaskType::AutofixPr,
            RemoteTaskType::Ultraplan,
            RemoteTaskType::Ultrareview,
        ];
        let labels: Vec<&'static str> = variants.iter().map(|v| v.label()).collect();

        // Uniqueness: no two variants share the same label.
        let mut seen = std::collections::HashSet::new();
        for label in &labels {
            assert!(
                seen.insert(*label),
                "duplicate RemoteTaskType label: {label:?}"
            );
        }

        // Stability: hard-code the expected strings so any accidental rename
        // causes a test failure before it reaches the parity ledger.
        assert_eq!(RemoteTaskType::RemoteAgent.label(), "remote-agent");
        assert_eq!(RemoteTaskType::BackgroundPr.label(), "background-pr");
        assert_eq!(RemoteTaskType::AutofixPr.label(), "autofix-pr");
        assert_eq!(RemoteTaskType::Ultraplan.label(), "ultraplan");
        assert_eq!(RemoteTaskType::Ultrareview.label(), "ultrareview");
    }

    /// `FeatureSet::first_release()` must **not** include
    /// `FeatureFlag::RemoteTriggers`.  This flag is the canonical gate for
    /// activating live CCR transport; it must stay absent until every blocker
    /// listed in docs/adr-remote-cloud-transport.md is resolved.
    #[test]
    fn first_release_feature_set_does_not_contain_remote_triggers() {
        let features = FeatureSet::first_release();
        assert!(
            !features.contains(FeatureFlag::RemoteTriggers),
            "FeatureFlag::RemoteTriggers must not be in first_release() \
             until CCR transport blockers are cleared (see docs/adr-remote-cloud-transport.md)"
        );
    }

    // ── TaskProgress tests ────────────────────────────────────────────────────

    #[test]
    fn task_progress_default_has_no_data() {
        let p = TaskProgress::default();
        assert!(!p.has_data());
        assert_eq!(p.tool_use_count, 0);
        assert_eq!(p.token_count, 0);
        assert!(p.last_tool_name.is_none());
        assert!(p.last_tool_at.is_none());
    }

    #[test]
    fn task_progress_record_tool_use_increments_counter() {
        let mut p = TaskProgress::default();
        p.record_tool_use("bash");
        assert_eq!(p.tool_use_count, 1);
        assert_eq!(p.last_tool_name.as_deref(), Some("bash"));
        assert!(p.last_tool_at.is_some());
        assert!(p.has_data());

        p.record_tool_use("read_file");
        assert_eq!(p.tool_use_count, 2);
        assert_eq!(p.last_tool_name.as_deref(), Some("read_file"));
    }

    #[test]
    fn task_progress_record_tokens_accumulates() {
        let mut p = TaskProgress::default();
        p.record_tokens(100);
        p.record_tokens(250);
        assert_eq!(p.token_count, 350);
        assert!(p.has_data());
    }

    #[test]
    fn task_progress_saturates_instead_of_overflowing() {
        let mut p = TaskProgress {
            tool_use_count: u32::MAX,
            ..TaskProgress::default()
        };
        p.record_tool_use("x");
        // saturating_add keeps us at MAX rather than wrapping.
        assert_eq!(p.tool_use_count, u32::MAX);

        p.token_count = u64::MAX;
        p.record_tokens(1);
        assert_eq!(p.token_count, u64::MAX);
    }

    #[test]
    fn task_progress_serializes_only_nonzero_fields() {
        // Default (all-zero) should omit tool_use_count and token_count.
        let p = TaskProgress::default();
        let json = serde_json::to_string(&p).unwrap();
        assert!(
            !json.contains("tool_use_count"),
            "zero count omitted: {json}"
        );
        assert!(!json.contains("token_count"), "zero count omitted: {json}");

        // After recording, fields appear.
        let mut p2 = TaskProgress::default();
        p2.record_tokens(42);
        let json2 = serde_json::to_string(&p2).unwrap();
        assert!(
            json2.contains("token_count"),
            "token_count present: {json2}"
        );
    }

    #[test]
    fn task_state_deserializes_without_progress_field() {
        // Simulate a persisted task file from before progress tracking existed.
        let json = serde_json::json!({
            "id": crate::TaskId::new(),
            "kind": "local_shell",
            "description": "old task",
            "status": "completed",
            "started_at": "2024-01-01T00:00:00Z",
        });
        let task: TaskState = serde_json::from_value(json).expect("deserialize legacy task");
        // Must default to zero/None without error.
        assert!(!task.progress.has_data());
        assert_eq!(task.progress.tool_use_count, 0);
        assert_eq!(task.progress.token_count, 0);
    }

    #[test]
    fn task_state_progress_roundtrips_via_json() {
        let mut task = TaskState::pending("progress roundtrip test");
        task.record_tool_use("bash");
        task.record_tool_use("read_file");
        task.record_tokens(1024);

        let json = serde_json::to_string(&task).unwrap();
        let decoded: TaskState = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.progress.tool_use_count, 2);
        assert_eq!(decoded.progress.token_count, 1024);
        assert_eq!(
            decoded.progress.last_tool_name.as_deref(),
            Some("read_file")
        );
        assert!(decoded.progress.last_tool_at.is_some());
    }

    #[test]
    fn task_state_progress_summary_none_when_no_data() {
        let task = TaskState::pending("empty progress");
        assert!(task.progress_summary().is_none());
    }

    #[test]
    fn task_state_progress_summary_contains_metrics() {
        let mut task = TaskState::pending("with progress");
        task.record_tool_use("bash");
        task.record_tokens(512);
        let summary = task.progress_summary().expect("summary present");
        assert!(summary.contains("tools=1"), "summary: {summary}");
        assert!(summary.contains("tokens=512"), "summary: {summary}");
        assert!(summary.contains("last_tool=bash"), "summary: {summary}");
    }

    #[test]
    fn task_state_no_progress_field_in_json_when_empty() {
        // Verify old task files are not polluted with a `progress` key.
        let task = TaskState::pending("no progress");
        let json = serde_json::to_string(&task).unwrap();
        assert!(
            !json.contains("\"progress\""),
            "progress key absent when no data: {json}"
        );
    }
}
