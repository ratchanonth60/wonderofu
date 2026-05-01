use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{Arc, RwLock},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use crate::{
    AdditionalWorkingDirectory, AuthState, FeatureSet, MessageEnvelope, PermissionMode,
    ProviderReadiness, Result, SessionId, TaskId, ToolUseId, WonderError,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
}

impl TokenUsage {
    #[must_use]
    pub const fn total_tokens(self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_creation_tokens + self.cache_read_tokens
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.total_tokens() == 0
    }

    pub fn add(&mut self, other: Self) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_tokens += other.cache_creation_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostState {
    pub usage: TokenUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl CostState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            usage: TokenUsage::default(),
            estimated_cost_usd: None,
            updated_at: OffsetDateTime::now_utc(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.usage.is_zero() && self.estimated_cost_usd.is_none()
    }

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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub id: SessionId,
    pub title: String,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
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
            entrypoint: None,
            app_version: None,
            tags: Vec::new(),
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingProviderToolCall {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PendingProviderToolResult {
    pub call_id: String,
    pub content: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PendingToolConversationRound {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_text: Option<String>,
    #[serde(default)]
    pub calls: Vec<PendingProviderToolCall>,
    #[serde(default)]
    pub results: Vec<PendingProviderToolResult>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingLocalToolCall {
    pub provider_call: PendingProviderToolCall,
    pub use_id: ToolUseId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingToolApprovalState {
    pub request_prompt: String,
    #[serde(default)]
    pub rounds: Vec<PendingToolConversationRound>,
    pub current_round: PendingToolConversationRound,
    pub pending_call: PendingLocalToolCall,
    #[serde(default)]
    pub remaining_calls: Vec<PendingLocalToolCall>,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    #[default]
    LocalShell,
    LocalAgent,
    RemoteAgent,
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

impl TaskStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Killed | Self::Cancelled
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntime {
    MetadataOnly,
    PromptSubprocess,
    Deferred,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskBackendSupport {
    Supported,
    Unsupported,
    Deferred,
}

impl TaskBackendSupport {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Unsupported => "unsupported",
            Self::Deferred => "deferred",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskBackendFlow {
    Transport,
    Monitor,
    Checker,
}

impl TaskBackendFlow {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::Monitor => "monitor",
            Self::Checker => "checker",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskBackendState {
    pub flow: TaskBackendFlow,
    pub support: TaskBackendSupport,
    pub reason: String,
}

impl TaskBackendState {
    #[must_use]
    pub fn supported(flow: TaskBackendFlow, reason: impl Into<String>) -> Self {
        Self {
            flow,
            support: TaskBackendSupport::Supported,
            reason: reason.into(),
        }
    }

    #[must_use]
    pub fn unsupported(flow: TaskBackendFlow, reason: impl Into<String>) -> Self {
        Self {
            flow,
            support: TaskBackendSupport::Unsupported,
            reason: reason.into(),
        }
    }

    #[must_use]
    pub fn deferred(flow: TaskBackendFlow, reason: impl Into<String>) -> Self {
        Self {
            flow,
            support: TaskBackendSupport::Deferred,
            reason: reason.into(),
        }
    }

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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RemoteTaskType {
    RemoteAgent,
    BackgroundPr,
    AutofixPr,
    Ultraplan,
    Ultrareview,
}

impl RemoteTaskType {
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteTaskMetadata {
    PullRequest {
        owner: String,
        repo: String,
        pr_number: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteTaskState {
    pub task_type: RemoteTaskType,
    pub transport: TaskBackendState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<TaskBackendState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checker: Option<TaskBackendState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<RemoteTaskMetadata>,
    #[serde(default)]
    pub long_running: bool,
}

impl RemoteTaskState {
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

    #[must_use]
    pub fn start_error_message(&self) -> String {
        format!(
            "cannot start {} task: {}",
            self.task_type.label(),
            self.status_summary()
        )
    }

    #[must_use]
    pub fn stop_error_message(&self) -> String {
        format!(
            "cannot stop {} task: {}",
            self.task_type.label(),
            self.status_summary()
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentTaskState {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub runtime: AgentRuntime,
}

impl AgentTaskState {
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskState {
    pub id: TaskId,
    pub kind: TaskKind,
    pub description: String,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_identity: Option<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub last_heartbeat_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentTaskState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteTaskState>,
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
            kind: TaskKind::LocalShell,
            description: description.into(),
            status: TaskStatus::Pending,
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
            started_at: OffsetDateTime::now_utc(),
            finished_at: None,
        }
    }

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

    #[must_use]
    pub fn recorded_remote(description: impl Into<String>, remote: RemoteTaskState) -> Self {
        let mut task = Self::pending(description);
        task.kind = TaskKind::RemoteAgent;
        task.status_message = Some(remote.status_summary());
        task.remote = Some(remote);
        task
    }

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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    pub session: SessionState,
    pub features: FeatureSet,
    pub permission_mode: PermissionMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_working_directories: Vec<AdditionalWorkingDirectory>,
    pub input_mode: InputMode,
    pub messages: Vec<MessageEnvelope>,
    pub queued_commands: VecDeque<QueuedCommand>,
    pub background_tasks: BTreeMap<TaskId, TaskState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_level: Option<String>,
    #[serde(default)]
    pub brief_mode: bool,
    #[serde(default)]
    pub fast_mode: bool,
    #[serde(default)]
    pub auth: AuthState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_tool_approval: Option<PendingToolApprovalState>,
    pub costs: CostState,
}

impl AppState {
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
            provider: None,
            model: None,
            theme: None,
            session_color: None,
            effort_level: None,
            brief_mode: false,
            fast_mode: false,
            auth: AuthState::default(),
            pending_tool_approval: None,
            costs: CostState::new(),
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

    pub fn record_cost_usage(&mut self, usage: TokenUsage, estimated_cost_usd: Option<f64>) {
        self.costs.record_usage(usage, estimated_cost_usd);
        self.session.updated_at = self.costs.updated_at;
    }

    pub fn set_provider_context(
        &mut self,
        provider: Option<String>,
        model: Option<String>,
        auth: AuthState,
    ) {
        self.provider = provider;
        self.model = model;
        self.auth = auth;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    pub fn set_theme(&mut self, theme: Option<String>) {
        self.theme = theme;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    pub fn set_session_color(&mut self, session_color: Option<String>) {
        self.session_color = session_color;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    pub fn set_effort_level(&mut self, effort_level: Option<String>) {
        self.effort_level = effort_level;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    pub fn set_brief_mode(&mut self, brief_mode: bool) {
        self.brief_mode = brief_mode;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    pub fn set_fast_mode(&mut self, fast_mode: bool) {
        self.fast_mode = fast_mode;
        self.session.updated_at = OffsetDateTime::now_utc();
    }

    #[must_use]
    pub fn effective_system_prompt(&self, explicit: Option<String>) -> Option<String> {
        const BRIEF_MODE_PROMPT: &str =
            "Be brief. Keep user-facing output concise and direct unless the user asks for detail.";

        let explicit = explicit
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        match (self.brief_mode, explicit) {
            (false, None) => None,
            (false, Some(prompt)) => Some(prompt.to_string()),
            (true, None) => Some(BRIEF_MODE_PROMPT.into()),
            (true, Some(prompt)) => Some(format!("{prompt}\n\n{BRIEF_MODE_PROMPT}")),
        }
    }

    pub fn set_session_tags(&mut self, tags: Vec<String>) {
        if self.session.tags != tags {
            self.session.tags = tags;
            self.session.updated_at = OffsetDateTime::now_utc();
        }
    }

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

    pub fn upsert_task(&mut self, task: TaskState) {
        self.background_tasks.insert(task.id, task);
        self.session.updated_at = OffsetDateTime::now_utc();
    }

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

    #[must_use]
    pub fn is_authenticated(&self) -> bool {
        matches!(self.provider_readiness(), ProviderReadiness::Ready)
    }
}

#[must_use]
pub const fn input_mode_label(mode: InputMode) -> &'static str {
    match mode {
        InputMode::Prompt => "prompt",
        InputMode::Bash => "bash",
        InputMode::PermissionPending => "permission",
        InputMode::TaskNotification => "tasks",
    }
}

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
}
