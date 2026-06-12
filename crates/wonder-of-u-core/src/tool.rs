use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    AdditionalWorkingDirectory, FeatureFlag, FeatureSet, FleetMemberRequest, ForkContextSnapshot,
    PermissionDecision, PermissionMode, PermissionRequest, PermissionRule, Result,
    RuntimeWorktreeState, SessionId, ShellSessionStore, TaskId, ToolPermissionContext, ToolUseId,
    WonderError, evaluate_permission,
    network_policy::NetworkPolicyConfig,
};
/// Enumerates tool kind
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// Represents shell
    Shell,
    /// Represents file read
    FileRead,
    /// Represents file write
    FileWrite,
    /// Represents search
    Search,
    /// Represents web
    Web,
    /// Represents planning
    Planning,
    /// Represents task
    Task,
    /// Represents agent
    Agent,
    /// Represents interaction
    Interaction,
    /// Represents skill
    Skill,
    /// Represents mcp
    Mcp,
}
/// Enumerates tool source
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    /// Represents native
    Native,
    /// Represents mcp
    Mcp,
    /// Represents plugin
    Plugin,
    /// Represents skill
    Skill,
    /// Represents agent generated
    AgentGenerated,
}
/// Represents tool spec
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Stores the name
    pub name: String,
    /// Stores the aliases
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Stores the description
    pub description: String,
    /// Stores the kind
    pub kind: ToolKind,
    /// Stores the source
    pub source: ToolSource,
    /// Stores the required features
    #[serde(default)]
    pub required_features: BTreeSet<FeatureFlag>,
    /// Stores the input schema
    #[serde(default)]
    pub input_schema: Value,
    /// Stores the read only
    #[serde(default)]
    pub read_only: bool,
    /// Stores the destructive
    #[serde(default)]
    pub destructive: bool,
    /// Stores the concurrency safe
    #[serde(default)]
    pub concurrency_safe: bool,
}

impl ToolSpec {
    /// Creates a new value
    #[must_use]
    pub fn new(name: impl Into<String>, description: impl Into<String>, kind: ToolKind) -> Self {
        Self {
            name: name.into(),
            aliases: Vec::new(),
            description: description.into(),
            kind,
            source: ToolSource::Native,
            required_features: BTreeSet::new(),
            input_schema: Value::Null,
            read_only: false,
            destructive: false,
            concurrency_safe: false,
        }
    }

    /// Validates the value
    pub fn validate(&self) -> Result<()> {
        let canonical = normalize_name(&self.name)?;
        let mut seen = BTreeSet::from([canonical]);
        for alias in &self.aliases {
            let alias = normalize_name(alias)?;
            if !seen.insert(alias.clone()) {
                return Err(WonderError::validation(format!(
                    "duplicate tool alias: {alias}"
                )));
            }
        }
        Ok(())
    }
    /// Handles with input schema
    #[must_use]
    pub fn with_input_schema(mut self, schema: ToolSchema) -> Self {
        self.input_schema = schema.into();
        self
    }
    /// Returns whether enabled
    #[must_use]
    pub fn is_enabled(&self, features: &FeatureSet) -> bool {
        features.contains_all(&self.required_features)
    }
}
/// Represents tool schema
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolSchema {
    value: Value,
}

impl ToolSchema {
    /// Handles any
    #[must_use]
    pub fn any() -> Self {
        Self { value: Value::Null }
    }
    /// Handles object
    #[must_use]
    pub fn object() -> Self {
        Self {
            value: json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false,
            }),
        }
    }
    /// Handles property
    #[must_use]
    pub fn property(mut self, name: impl Into<String>, schema: Value) -> Self {
        if let Some(properties) = self.properties_mut() {
            properties.insert(name.into(), schema);
        }
        self
    }
    /// Handles required
    #[must_use]
    pub fn required(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        if let Some(required) = self.required_mut() {
            if !required
                .iter()
                .any(|value| value.as_str() == Some(name.as_str()))
            {
                required.push(Value::String(name));
            }
        }
        self
    }
    /// Handles additional properties
    #[must_use]
    pub fn additional_properties(mut self, allowed: bool) -> Self {
        if let Some(object) = self.value.as_object_mut() {
            object.insert("additionalProperties".into(), Value::Bool(allowed));
        }
        self
    }
    /// Handles string
    #[must_use]
    pub fn string(description: impl Into<String>) -> Value {
        json!({
            "type": "string",
            "description": description.into(),
        })
    }
    /// Handles boolean
    #[must_use]
    pub fn boolean(description: impl Into<String>) -> Value {
        json!({
            "type": "boolean",
            "description": description.into(),
        })
    }
    /// Handles integer
    #[must_use]
    pub fn integer(description: impl Into<String>) -> Value {
        json!({
            "type": "integer",
            "description": description.into(),
        })
    }
    /// Handles enumeration
    #[must_use]
    pub fn enumeration(
        description: impl Into<String>,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Value {
        json!({
            "type": "string",
            "description": description.into(),
            "enum": values.into_iter().map(Into::into).collect::<Vec<_>>(),
        })
    }
    /// Handles into value
    #[must_use]
    pub fn into_value(self) -> Value {
        self.value
    }
    /// Handles as value
    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.value
    }

    fn properties_mut(&mut self) -> Option<&mut Map<String, Value>> {
        self.value
            .as_object_mut()?
            .get_mut("properties")?
            .as_object_mut()
    }

    fn required_mut(&mut self) -> Option<&mut Vec<Value>> {
        self.value
            .as_object_mut()?
            .get_mut("required")?
            .as_array_mut()
    }
}

impl From<ToolSchema> for Value {
    fn from(schema: ToolSchema) -> Self {
        schema.into_value()
    }
}
/// Represents tool query
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolQuery {
    /// Stores the features
    pub features: FeatureSet,
}

impl ToolQuery {
    /// Creates a new value
    #[must_use]
    pub fn new(features: FeatureSet) -> Self {
        Self { features }
    }
    /// Returns whether the query allows the item
    #[must_use]
    pub fn allows(&self, spec: &ToolSpec) -> bool {
        spec.is_enabled(&self.features)
    }
}

impl From<&ToolContext> for ToolQuery {
    fn from(context: &ToolContext) -> Self {
        Self {
            features: context.features.clone(),
        }
    }
}
/// Represents tool context
#[derive(Clone, Debug)]
pub struct ToolContext {
    /// Stores the session identifier
    pub session_id: SessionId,
    /// Stores the cwd
    pub cwd: PathBuf,
    /// Stores active worktree session state when the runtime has switched into one.
    pub session_worktree: Option<RuntimeWorktreeState>,
    /// Stores the permission mode
    pub permission_mode: PermissionMode,
    /// Stores the additional working directories
    pub additional_working_directories: Vec<AdditionalWorkingDirectory>,
    /// Stores the selected provider, when the runtime has one.
    pub provider: Option<String>,
    /// Stores the selected model, when the runtime has one.
    pub model: Option<String>,
    /// Stores the permission rules
    pub permission_rules: Vec<PermissionRule>,
    /// Stores the features
    pub features: FeatureSet,
    /// Optional persistent bash session store shared across tool calls.
    ///
    /// When present, [`BashTool`] reuses the same bash process between calls,
    /// preserving `$PWD`, environment variables, and shell functions.
    /// When `None` the tool falls back to the one-shot subprocess behaviour.
    pub bash_session_store: Option<Arc<Mutex<ShellSessionStore>>>,
    /// Fork-lite context snapshot from the parent session.
    ///
    /// Populated by the CLI runtime when the current process is running inside
    /// a prompt or TUI loop.  `None` for all other callers (tests, headless
    /// tool calls without a session, etc.).
    ///
    /// The `agent` tool reads this field when `mode = "fork"` to embed the
    /// snapshot in the [`FleetMemberRequest`] before launching the child
    /// subprocess.  All other tools ignore it.
    pub fork_context: Option<ForkContextSnapshot>,
    /// Optional sender for streaming partial shell output to the TUI during execution.
    ///
    /// When present, shell tools (`bash`, `shell`) send each stdout line to this
    /// channel so the TUI can display live progress while the command runs.
    /// `None` for non-TUI callers (tests, headless prompt runs, etc.).
    pub progress_tx: Option<std::sync::mpsc::SyncSender<String>>,
    /// TUI interaction channel for `ask_user` and similar interaction tools.
    ///
    /// When `Some`, the tool must NOT use `io::stdin` / `io::stdout` (which
    /// conflict with crossterm's raw-mode event loop).  Instead it blocks on
    /// this receiver waiting for the TUI to deliver the user's answer.
    ///
    /// Wrapped in `Arc<Mutex>` so that `ToolContext` remains `Clone + Send`.
    /// Only the executing tool thread should ever call `recv()` on it.
    pub interaction_rx: Option<std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<String>>>>,
    /// Optional pre-modification file snapshot hook for `/rewind` restore.
    ///
    /// When present, file-mutating tools (`file_write`, `file_edit`,
    /// `notebook_edit`) call [`ToolContext::checkpoint_file_before_write`]
    /// before changing the file so the runtime can restore its prior state
    /// when the user rewinds the conversation.  `None` for callers without
    /// persistent storage (tests, ad-hoc tool calls).
    pub file_checkpointer: Option<Arc<dyn FileCheckpointer>>,
    /// Optional network egress policy for web tools.
    ///
    /// When `Some`, web tools (`web_fetch`, `web_search`) consult this policy
    /// to determine whether a host is accessible.  When `None`, web tools
    /// fall back to their built-in preapproved host lists.
    pub network_policy: Option<NetworkPolicyConfig>,
}

/// Captures the pre-modification state of files so `/rewind` can restore them.
///
/// Implemented by the storage layer; tools only see the trait so the
/// foundation crates stay free of persistence concerns.
pub trait FileCheckpointer: Send + Sync + std::fmt::Debug {
    /// Records the current on-disk state of `path` (including "absent") before
    /// a tool mutates it.
    fn checkpoint_file(&self, path: &Path) -> Result<()>;
}

impl ToolContext {
    /// Handles permission context
    #[must_use]
    pub fn permission_context(&self) -> ToolPermissionContext {
        ToolPermissionContext {
            cwd: self.cwd.clone(),
            mode: self.permission_mode,
            additional_working_directories: self.additional_working_directories.clone(),
            rules: self.permission_rules.clone(),
        }
    }

    /// Snapshots `path` for `/rewind` restore before a tool mutates it.
    ///
    /// Best-effort: checkpoint failures must never abort the tool call, so
    /// errors are swallowed here.
    pub fn checkpoint_file_before_write(&self, path: &Path) {
        if let Some(checkpointer) = &self.file_checkpointer {
            let _ = checkpointer.checkpoint_file(path);
        }
    }
}
/// Plain-data spec for launching an agent task, carried as a [`ToolEffect`].
///
/// Wraps a fully-constructed [`FleetMemberRequest`] — including the definition
/// snapshot, lineage, allowed tools, and model override — so the runtime can
/// start a real task without re-reading any files.
///
/// # Output paths
///
/// When `reserved_task_id` is `Some`, the runtime **must** use that id when
/// creating the task record so the output paths published in the tool result
/// metadata remain stable.  The paths are deterministic given the app-root
/// and task-id:
///
/// | field | path |
/// |---|---|
/// | `output_log_path` | `tasks/logs/{task_id}.log` |
/// | `result_path` | `tasks/results/{task_id}.json` |
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentLaunchSpec {
    /// The fully-constructed request ready for the runtime to convert into an
    /// `AgentTaskLaunch` and pass to `TaskManager::start_agent_task`.
    pub request: FleetMemberRequest,

    /// Pre-allocated task id chosen at tool-call time.
    ///
    /// When present the runtime **must** use this id instead of generating a
    /// fresh one so that the output paths published in the accompanying
    /// `ToolResult::metadata` stay stable and can be consumed by the caller
    /// before the task reaches a terminal state.
    ///
    /// `None` is serialized as absent for backward compatibility with runtimes
    /// that predate this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserved_task_id: Option<TaskId>,
}

/// Plain-data spec carried by [`ToolEffect::SendAgentMessage`].
///
/// Produced by `SendMessageTool` for team-recipient calls.  The CLI runtime
/// reads `WONDER_OF_U_FLEET_ID` and persists this as a
/// [`FleetSteeringMessage`](crate::FleetSteeringMessage) with
/// `source = SteeringSource::Agent`.
///
/// The tool layer never reads environment variables; all env access happens
/// in the CLI effect processor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentMessageSpec {
    /// Recipient: a bare team-member name or `"*"` for broadcast.
    pub to: String,
    /// 5–10 word preview supplied with plain-text messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Full message text / prompt to persist as the steering instruction.
    pub content: String,
    /// Kind label (e.g. `"text"`, `"shutdown_request"`).
    pub message_kind: String,
}

/// Typed side-effects that a tool may request from the calling runtime.
///
/// Effects are returned inside [`ToolResult::effects`] and processed **after**
/// the tool result is received.  The tool itself never writes storage or spawns
/// processes; it delegates those concerns to the runtime via this mechanism.
///
/// # Backward compatibility
///
/// [`ToolResult::effects`] defaults to an empty `Vec` and is omitted from JSON
/// when empty, so older readers silently ignore it.
// `LaunchAgentTask` carries a ~504-byte `AgentLaunchSpec`; boxing it would
// add a heap allocation for every tool dispatch.  `ToolEffect` values are
// short-lived (drained immediately in `process_tool_effects`) so the size
// difference is acceptable.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ToolEffect {
    /// Request the runtime to launch a local agent task from the enclosed spec.
    ///
    /// The runtime **must** choose exactly one of:
    /// - Direct launch: convert to `AgentTaskLaunch`, call `TaskManager::start_agent_task`.
    /// - Pending queue fallback: write a [`FleetMemberRequest`] file via
    ///   `FleetStore::queue_member_request` if direct launch is unavailable or fails.
    ///
    /// These two paths are mutually exclusive for a single invocation to prevent
    /// duplicate tasks.
    LaunchAgentTask(AgentLaunchSpec),

    /// Request the runtime to persist a `send_message` call as an advisory
    /// fleet steering message with `source = SteeringSource::Agent`.
    ///
    /// The runtime:
    /// 1. Reads `WONDER_OF_U_FLEET_ID` from the environment.
    /// 2. Validates the fleet exists and is not terminal.
    /// 3. Writes a `FleetSteeringMessage` with `source = Agent`.
    /// 4. Updates the result with `status = queued_advisory` and the steering id.
    ///
    /// If `WONDER_OF_U_FLEET_ID` is absent, the storage dir is not configured,
    /// or the fleet is terminal/missing, the result is marked failed and
    /// nothing is written.  This is **advisory routing only** — no live
    /// delivery to any running subprocess is attempted.
    SendAgentMessage(AgentMessageSpec),
}

/// Represents tool result
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    /// Stores the use identifier
    pub use_id: ToolUseId,
    /// Stores the success
    pub success: bool,
    /// Stores the content
    pub content: String,
    /// Stores the metadata
    #[serde(default)]
    pub metadata: Value,
    /// Typed side-effects the runtime should process after receiving this result.
    ///
    /// Defaults to an empty `Vec` and is omitted from serialized JSON when
    /// empty, preserving backward compatibility with older readers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<ToolEffect>,
}

impl ToolResult {
    /// Handles success
    #[must_use]
    pub fn success(use_id: ToolUseId, content: impl Into<String>) -> Self {
        Self {
            use_id,
            success: true,
            content: content.into(),
            metadata: Value::Null,
            effects: Vec::new(),
        }
    }
    /// Handles failure
    #[must_use]
    pub fn failure(use_id: ToolUseId, content: impl Into<String>) -> Self {
        Self {
            use_id,
            success: false,
            content: content.into(),
            metadata: Value::Null,
            effects: Vec::new(),
        }
    }

    /// Attach structured metadata to this result.
    #[must_use]
    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Attach typed side-effects the runtime should process after this result.
    #[must_use]
    pub fn with_effects(mut self, effects: Vec<ToolEffect>) -> Self {
        self.effects = effects;
        self
    }
}
/// Represents tool progress
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolProgress {
    /// Stores the use identifier
    pub use_id: ToolUseId,
    /// Stores the message
    pub message: String,
    /// Stores the percent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<f32>,
}
/// Defines tool behavior
#[async_trait]
pub trait Tool: Send + Sync {
    /// Returns the item specification
    fn spec(&self) -> ToolSpec;

    /// Validates the tool input
    fn validate_input(&self, _input: &Value) -> Result<()> {
        Ok(())
    }

    /// Handles permission decision
    fn permission_decision(&self, context: &ToolContext, input: &Value) -> PermissionDecision {
        let spec = self.spec();
        let request = permission_request_for_spec(&spec, input);
        evaluate_permission(&context.permission_context(), &request)
    }

    /// Executes the operation
    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult>;
}

#[derive(Clone)]
struct ToolEntry {
    spec: ToolSpec,
    tool: Arc<dyn Tool>,
}
/// Stores tool registry
#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, ToolEntry>,
    aliases: BTreeMap<String, String>,
    order: Vec<String>,
}

impl ToolRegistry {
    /// Creates a new value
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles register
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<()> {
        let spec = tool.spec();
        spec.validate()?;
        let canonical = normalize_name(&spec.name)?;
        if self.tools.contains_key(&canonical) || self.aliases.contains_key(&canonical) {
            return Err(WonderError::validation(format!(
                "duplicate tool: {canonical}"
            )));
        }

        for alias in &spec.aliases {
            let alias = normalize_name(alias)?;
            if self.tools.contains_key(&alias) || self.aliases.contains_key(&alias) {
                return Err(WonderError::validation(format!(
                    "duplicate tool alias: {alias}"
                )));
            }
        }

        for alias in &spec.aliases {
            self.aliases
                .insert(normalize_name(alias)?, canonical.clone());
        }
        self.order.push(canonical.clone());
        self.tools.insert(canonical, ToolEntry { spec, tool });
        Ok(())
    }
    /// Handles resolve
    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.resolve_entry(name)
            .map(|entry| Arc::clone(&entry.tool))
    }
    /// Resolves enabled
    #[must_use]
    pub fn resolve_enabled(&self, name: &str, query: &ToolQuery) -> Option<Arc<dyn Tool>> {
        let entry = self.resolve_entry(name)?;
        query.allows(&entry.spec).then(|| Arc::clone(&entry.tool))
    }

    /// Handles all specs
    pub fn all_specs(&self) -> Vec<ToolSpec> {
        self.order
            .iter()
            .filter_map(|canonical| self.tools.get(canonical))
            .map(|entry| entry.spec.clone())
            .collect()
    }

    /// Handles enabled specs
    pub fn enabled_specs(&self, features: &FeatureSet) -> Vec<ToolSpec> {
        self.enabled_specs_for(&ToolQuery::new(features.clone()))
    }

    /// Handles enabled specs for
    pub fn enabled_specs_for(&self, query: &ToolQuery) -> Vec<ToolSpec> {
        self.all_specs()
            .into_iter()
            .filter(|spec| query.allows(spec))
            .collect()
    }

    fn resolve_entry(&self, name: &str) -> Option<&ToolEntry> {
        let normalized = normalize_lookup(name)?;
        self.tools.get(&normalized).or_else(|| {
            self.aliases
                .get(&normalized)
                .and_then(|canonical| self.tools.get(canonical))
        })
    }
}

fn normalize_name(name: &str) -> Result<String> {
    let normalized = normalize_lookup(name)
        .ok_or_else(|| WonderError::validation("tool name cannot be empty"))?;
    if normalized.chars().any(char::is_whitespace) {
        return Err(WonderError::validation(format!(
            "tool name contains whitespace: {name}"
        )));
    }
    Ok(normalized)
}

fn normalize_lookup(name: &str) -> Option<String> {
    let normalized = name.trim().to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn permission_request_for_spec(spec: &ToolSpec, input: &Value) -> PermissionRequest {
    let mut request = PermissionRequest::new(spec.name.clone())
        .with_aliases(spec.aliases.clone())
        .read_only(spec.read_only)
        .destructive(spec.destructive);

    if let Some(command) = infer_shell_command(spec, input) {
        request = request.with_shell_command(command);
    }

    let inferred_paths = infer_paths(spec, input);
    if !inferred_paths.is_empty() {
        request = request.with_paths(inferred_paths);
    }

    request
}

fn infer_shell_command(spec: &ToolSpec, input: &Value) -> Option<String> {
    if spec.kind != ToolKind::Shell {
        return None;
    }

    let object = input.as_object()?;
    ["command", "cmd", "script"]
        .into_iter()
        .find_map(|field| object.get(field).and_then(Value::as_str))
        .map(str::to_string)
}

fn infer_paths(spec: &ToolSpec, input: &Value) -> Vec<PathBuf> {
    let Some(object) = input.as_object() else {
        return Vec::new();
    };

    let mut path_fields = vec![
        "path",
        "paths",
        "file_path",
        "file_paths",
        "notebook_path",
        "notebook_paths",
        "directory",
        "directories",
        "target",
        "plan_file_path",
        "planFilePath",
    ];

    if spec.kind == ToolKind::Shell {
        path_fields.push("cwd");
    }

    path_fields
        .into_iter()
        .filter_map(|field| object.get(field))
        .flat_map(value_to_paths)
        .collect()
}

fn value_to_paths(value: &Value) -> Vec<PathBuf> {
    match value {
        Value::String(path) => vec![PathBuf::from(path)],
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(PathBuf::from)
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ReadTool;
    struct WebTool;
    struct BashTool;

    #[async_trait]
    impl Tool for ReadTool {
        fn spec(&self) -> ToolSpec {
            let mut spec = ToolSpec::new("file_read", "read a file", ToolKind::FileRead)
                .with_input_schema(
                    ToolSchema::object()
                        .property("path", ToolSchema::string("absolute file path"))
                        .required("path"),
                );
            spec.read_only = true;
            spec
        }

        async fn execute(
            &self,
            _context: ToolContext,
            use_id: ToolUseId,
            _input: Value,
        ) -> Result<ToolResult> {
            Ok(ToolResult::success(use_id, "ok"))
        }
    }

    #[async_trait]
    impl Tool for WebTool {
        fn spec(&self) -> ToolSpec {
            let mut spec = ToolSpec::new("web_fetch", "fetch a page", ToolKind::Web);
            spec.required_features.insert(FeatureFlag::WebTools);
            spec
        }

        async fn execute(
            &self,
            _context: ToolContext,
            use_id: ToolUseId,
            _input: Value,
        ) -> Result<ToolResult> {
            Ok(ToolResult::success(use_id, "ok"))
        }
    }

    #[async_trait]
    impl Tool for BashTool {
        fn spec(&self) -> ToolSpec {
            let mut spec = ToolSpec::new("bash", "run a shell command", ToolKind::Shell)
                .with_input_schema(
                    ToolSchema::object()
                        .property("command", ToolSchema::string("shell command to run"))
                        .required("command"),
                );
            spec.destructive = true;
            spec
        }

        async fn execute(
            &self,
            _context: ToolContext,
            use_id: ToolUseId,
            _input: Value,
        ) -> Result<ToolResult> {
            Ok(ToolResult::success(use_id, "ok"))
        }
    }

    fn tool_context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd: PathBuf::from("/workspace"),
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            provider: None,
            model: None,
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: None,
            progress_tx: None,
            interaction_rx: None,
            fork_context: None,
            file_checkpointer: None,
            network_policy: None,
        }
    }

    #[test]
    fn registry_resolves_tool_aliases() {
        let mut registry = ToolRegistry::new();
        let tool = Arc::new(ReadTool);
        registry.register(tool).expect("register");

        let mut alias_registry = ToolRegistry::new();
        let mut spec = ToolSpec::new("file_read", "read a file", ToolKind::FileRead);
        spec.aliases.push("read".into());

        struct AliasTool(ToolSpec);
        #[async_trait]
        impl Tool for AliasTool {
            fn spec(&self) -> ToolSpec {
                self.0.clone()
            }

            async fn execute(
                &self,
                _context: ToolContext,
                use_id: ToolUseId,
                _input: Value,
            ) -> Result<ToolResult> {
                Ok(ToolResult::success(use_id, "ok"))
            }
        }

        alias_registry
            .register(Arc::new(AliasTool(spec)))
            .expect("register alias");

        assert!(alias_registry.resolve("file_read").is_some());
        assert!(alias_registry.resolve("read").is_some());
        assert!(alias_registry.resolve("READ").is_some());
    }

    #[test]
    fn default_permission_allows_read_only_tools() {
        let tool = ReadTool;
        let context = tool_context();

        assert!(
            tool.permission_decision(&context, &json!({ "path": "src/lib.rs" }))
                .is_allowed()
        );
    }

    #[test]
    fn file_tools_require_review_for_absolute_paths_outside_scope() {
        let tool = ReadTool;
        let context = tool_context();

        let decision = tool.permission_decision(&context, &json!({ "path": "/secret.txt" }));

        assert!(matches!(decision, PermissionDecision::Ask { .. }));
    }

    #[test]
    fn allow_rule_grants_access_to_extra_path_prefix() {
        let tool = ReadTool;
        let mut context = tool_context();
        context.permission_rules.push(
            PermissionRule::new(
                "file_read",
                crate::PermissionRuleBehavior::Allow,
                crate::PermissionRuleSource::CliArg,
            )
            .for_path_prefix("../shared"),
        );

        let decision =
            tool.permission_decision(&context, &json!({ "path": "../shared/readme.md" }));

        assert!(matches!(decision, PermissionDecision::Allow { .. }));
    }

    #[test]
    fn shell_tools_apply_safety_checks_from_input() {
        let tool = BashTool;
        let context = tool_context();

        let decision = tool.permission_decision(&context, &json!({ "command": "echo ${cmd@P}" }));

        assert!(matches!(decision, PermissionDecision::Deny { .. }));
    }

    #[test]
    fn infer_paths_captures_source_compatible_notebook_and_plan_fields() {
        let notebook = ToolSpec::new("notebook_edit", "edit notebook", ToolKind::FileWrite);
        let plan = ToolSpec::new("exit_plan_mode", "exit plan mode", ToolKind::Planning);

        assert_eq!(
            infer_paths(&notebook, &json!({ "notebook_path": "notes.ipynb" })),
            vec![PathBuf::from("notes.ipynb")]
        );
        assert_eq!(
            infer_paths(&plan, &json!({ "planFilePath": "docs/plan.md" })),
            vec![PathBuf::from("docs/plan.md")]
        );
    }

    #[test]
    fn permission_request_carries_tool_aliases_for_rule_matching() {
        let mut spec = ToolSpec::new("mcp_resource_read", "read mcp resource", ToolKind::Mcp);
        spec.aliases.push("ReadMcpResourceTool".into());

        let request = permission_request_for_spec(&spec, &json!({ "resource_name": "demo" }));

        assert!(request.matches_tool_name("mcp_resource_read"));
        assert!(request.matches_tool_name("ReadMcpResourceTool"));
    }

    #[test]
    fn registry_preserves_order_and_filters_disabled_tools() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(ReadTool)).expect("read");
        registry.register(Arc::new(WebTool)).expect("web");

        let disabled_query = ToolQuery::new(FeatureSet::empty());
        let enabled_query = ToolQuery::new(FeatureSet::first_release());

        assert_eq!(
            registry
                .all_specs()
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>(),
            vec!["file_read", "web_fetch"]
        );
        assert!(
            registry
                .resolve_enabled("web_fetch", &disabled_query)
                .is_none()
        );
        assert!(
            registry
                .resolve_enabled("web_fetch", &enabled_query)
                .is_some()
        );
        assert_eq!(
            registry
                .enabled_specs_for(&disabled_query)
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>(),
            vec!["file_read"]
        );
    }

    #[test]
    fn schema_builder_tracks_properties_and_required_fields() {
        let schema = ToolSchema::object()
            .property("query", ToolSchema::string("search term"))
            .property("limit", ToolSchema::integer("maximum result count"))
            .required("query");

        assert_eq!(
            schema.into_value(),
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "search term"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "maximum result count"
                    }
                },
                "required": ["query"],
                "additionalProperties": false,
            })
        );
    }

    #[test]
    fn tool_spec_validation_rejects_duplicate_aliases() {
        let mut spec = ToolSpec::new("demo_tool", "demo tool", ToolKind::Skill);
        spec.aliases = vec!["run".into(), "run".into()];

        let error = spec.validate().expect_err("duplicate alias");
        assert!(error.to_string().contains("duplicate tool alias"));
    }

    // ── ToolResult.effects back-compat ────────────────────────────────────────

    /// JSON from older binaries (before this field existed) must deserialize
    /// with an empty effects Vec — the `#[serde(default)]` annotation handles
    /// this, but an explicit test makes the contract visible.
    #[test]
    fn tool_result_effects_field_defaults_to_empty_on_legacy_json() {
        let legacy_json = serde_json::json!({
            "use_id": "0191e4a2-c54e-7000-8000-000000000001",
            "success": true,
            "content": "agent task queued for fleet dispatch: abc-123",
            "metadata": { "request_id": "abc-123", "status": "pending_dispatch" }
        });

        let result: ToolResult =
            serde_json::from_value(legacy_json).expect("deserialize legacy ToolResult");

        assert!(
            result.effects.is_empty(),
            "expected empty effects from legacy JSON, got {:?}",
            result.effects
        );
    }

    /// A result with no effects must round-trip without the field in the JSON
    /// output (the `skip_serializing_if` annotation keeps payloads compact).
    #[test]
    fn tool_result_with_no_effects_omits_field_from_serialized_json() {
        let id = ToolUseId::new();
        let result = ToolResult::success(id, "ok");
        let json = serde_json::to_value(&result).expect("serialize");
        assert!(
            !json.as_object().unwrap().contains_key("effects"),
            "effects field should be absent when empty, got: {json}"
        );
    }

    /// Ensure a `ToolEffect::LaunchAgentTask` round-trips through serde
    /// with the expected discriminant shape `{type: "launch_agent_task", data: ...}`.
    #[test]
    fn tool_effect_launch_agent_task_serde_round_trip() {
        use crate::FleetMemberRequest;

        let req = FleetMemberRequest::new("test prompt");
        let spec = AgentLaunchSpec {
            request: req.clone(),
            reserved_task_id: None,
        };
        let effect = ToolEffect::LaunchAgentTask(spec);
        let json = serde_json::to_value(&effect).expect("serialize effect");

        assert_eq!(json["type"], "launch_agent_task");
        assert!(json["data"].is_object());

        let roundtripped: ToolEffect = serde_json::from_value(json).expect("deserialize effect");
        assert_eq!(
            roundtripped,
            ToolEffect::LaunchAgentTask(AgentLaunchSpec {
                request: req,
                reserved_task_id: None,
            })
        );
    }

    /// `ToolEffect::SendAgentMessage` round-trips with the expected discriminant
    /// shape `{type: "send_agent_message", data: {...}}`.
    #[test]
    fn tool_effect_send_agent_message_serde_round_trip() {
        let spec = AgentMessageSpec {
            to: "reviewer".into(),
            summary: Some("review auth".into()),
            content: "please review the auth module".into(),
            message_kind: "text".into(),
        };
        let effect = ToolEffect::SendAgentMessage(spec.clone());
        let json = serde_json::to_value(&effect).expect("serialize effect");

        assert_eq!(json["type"], "send_agent_message");
        assert!(json["data"].is_object());
        assert_eq!(json["data"]["to"], "reviewer");
        assert_eq!(json["data"]["message_kind"], "text");

        let roundtripped: ToolEffect = serde_json::from_value(json).expect("deserialize effect");
        assert_eq!(roundtripped, ToolEffect::SendAgentMessage(spec));
    }

    /// `AgentMessageSpec` with no summary omits the `summary` field from JSON.
    #[test]
    fn agent_message_spec_absent_summary_omitted_from_json() {
        let spec = AgentMessageSpec {
            to: "*".into(),
            summary: None,
            content: "broadcast update".into(),
            message_kind: "text".into(),
        };
        let json = serde_json::to_string(&spec).expect("serialize");
        assert!(
            !json.contains("summary"),
            "absent summary should be omitted: {json}"
        );
    }

    /// Legacy `ToolResult` JSON (no `effects`) deserializes with empty effects,
    /// and a result carrying `SendAgentMessage` round-trips correctly.
    #[test]
    fn tool_result_with_send_agent_message_effect_round_trips() {
        let id = ToolUseId::new();
        let spec = AgentMessageSpec {
            to: "team-lead".into(),
            summary: Some("status update".into()),
            content: "milestone reached".into(),
            message_kind: "text".into(),
        };
        let result = ToolResult::success(id, "queued")
            .with_effects(vec![ToolEffect::SendAgentMessage(spec.clone())]);

        let json = serde_json::to_value(&result).expect("serialize");
        let decoded: ToolResult = serde_json::from_value(json).expect("deserialize");

        assert_eq!(decoded.effects.len(), 1);
        assert_eq!(decoded.effects[0], ToolEffect::SendAgentMessage(spec));
    }
}
