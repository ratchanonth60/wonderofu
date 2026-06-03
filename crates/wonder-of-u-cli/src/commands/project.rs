use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use clap::{Parser, Subcommand};
use serde_json::json;
use walkdir::WalkDir;
use wonder_of_u_core::{
    AppState, Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec,
    FeatureFlag, MessageEnvelope, MessagePayload, PermissionRuleSource, Result, SessionId,
    TaskStatus, WonderError,
};
use wonder_of_u_storage::TranscriptStore;

use super::{detect_git_branch, git_command_output, is_hidden_path, parse_command_args};

/// Represents add dir command
pub struct AddDirCommand;

impl AddDirCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "add-dir",
            "Add an additional working directory to the current session",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::Permissions]);
        spec
    }
}

/// Represents init command
pub struct InitCommand;

impl InitCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "init",
            "Initialize project memory in CLAUDE.md for the current workspace",
            CommandKind::Local,
        )
    }
}

/// Represents context command
pub struct ContextCommand {
    storage_dir: Option<PathBuf>,
}

impl ContextCommand {
    /// Creates a new value
    pub fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "context",
            "Show current context usage and session composition",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::Tools]);
        spec
    }
}

#[async_trait]
impl Command for ContextCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let state = load_snapshot_state(self.storage_dir.as_deref(), &context)?
            .unwrap_or_else(|| context_state_fallback(&context));
        Ok(CommandOutput::Text(render_context_summary(&state)))
    }
}

/// Represents memory command
pub struct MemoryCommand {
    storage_dir: Option<PathBuf>,
}

impl MemoryCommand {
    /// Creates a new value
    pub fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "memory",
            "Show or open project and user memory files",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::Tools]);
        spec
    }
}

/// Slash command `/remember <text>` — ask the model to save a memory.
///
/// Enqueues a prompt instructing the model to write the given text as a memory
/// entry in its auto-memory directory.  The model uses its Write tool to
/// create an appropriately-typed file and update MEMORY.md.
pub struct RememberCommand;

impl RememberCommand {
    /// Creates a new value
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "remember",
            "Ask the AI to save something to its memory system",
            CommandKind::Local,
        );
        spec.argument_hint = Some("<text to remember>".into());
        spec
    }
}

#[async_trait]
impl Command for RememberCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let text = invocation.args.trim().to_string();
        if text.is_empty() {
            return Ok(CommandOutput::Text(
                "usage: /remember <text to save to memory>".into(),
            ));
        }
        let prompt = format!(
            "Please save the following to your memory system now. Choose the most \
             appropriate memory type (user/feedback/project/reference), write it to a \
             suitably-named file in your memory directory, and update MEMORY.md with a \
             one-line index entry.\n\nText to remember:\n\n{text}"
        );
        Ok(CommandOutput::EnqueuePrompt(prompt))
    }
}

/// Represents copy command
pub struct CopyCommand {
    storage_dir: Option<PathBuf>,
}

impl CopyCommand {
    /// Creates a new value
    pub fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "copy",
            "Copy a recent assistant response to the clipboard with file fallback",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::Tools]);
        spec
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Subcommand)]
enum MemorySubcommand {
    Show,
    Open(MemoryOpenArgs),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Parser)]
struct MemoryOpenArgs {
    #[arg(default_value = "project")]
    target: MemoryTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub(crate) enum MemoryTarget {
    Project,
    User,
}

#[derive(Debug, Parser)]
struct MemoryArgs {
    #[command(subcommand)]
    command: Option<MemorySubcommand>,
}

#[derive(Debug, Parser)]
struct CopyArgs {
    #[arg()]
    index: Option<usize>,
}

#[derive(Debug, Parser)]
struct InitArgs {
    #[arg(long, default_value_t = false)]
    open: bool,
    #[arg(long, default_value_t = false)]
    force: bool,
}

#[async_trait]
impl Command for InitCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<InitArgs>("init", &invocation)?;
        let path = resolve_memory_path(context.cwd.as_path(), None, MemoryTarget::Project);
        let existed = path.exists();
        if !existed || args.force {
            write_init_template(&path, &context.cwd)?;
        } else {
            ensure_memory_file(&path)?;
        }

        let should_open = args.open || (context.interactive && !existed);
        let lines = init_output(&context, &path, existed, args.force, should_open)?;
        Ok(CommandOutput::Text(lines))
    }
}

#[async_trait]
impl Command for MemoryCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        if context.interactive && invocation.args.trim().is_empty() {
            return Ok(CommandOutput::Text(render_memory_picker_output(
                &context,
                self.storage_dir.as_deref(),
            )));
        }
        let normalized = normalize_memory_invocation(&invocation);
        let args = parse_command_args::<MemoryArgs>("memory", &normalized)?;
        match args.command.unwrap_or(MemorySubcommand::Show) {
            MemorySubcommand::Show => Ok(CommandOutput::Text(render_memory_status(
                &context,
                self.storage_dir.as_deref(),
            ))),
            MemorySubcommand::Open(args) => {
                memory_open_output(&context, self.storage_dir.as_deref(), args.target)
                    .map(CommandOutput::Text)
            }
        }
    }
}

#[async_trait]
impl Command for CopyCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<CopyArgs>("copy", &invocation)?;
        let requested_index = args.index.unwrap_or(1);
        if requested_index == 0 {
            return Err(WonderError::validation(
                "`/copy 0` is invalid; use `/copy` or `/copy 1` for the latest response",
            ));
        }

        let source = load_copy_source(self.storage_dir.as_deref(), &context)?;
        let responses = collect_recent_assistant_texts(&source.messages);
        let selected = responses
            .get(requested_index.saturating_sub(1))
            .ok_or_else(|| {
                WonderError::not_found("assistant response", format!("index {requested_index}"))
            })?
            .clone();

        let file_path = write_copy_text_to_temp_file(&selected.text, "response.md")?;
        let clipboard = try_copy_to_clipboard(&selected.text);
        let mut lines = vec!["copy=ready".into()];
        lines.push(format!("source_session_id={}", source.session_id));
        lines.push(format!("assistant_responses={}", responses.len()));
        lines.push(format!("selected_response={requested_index}"));
        lines.push(format!(
            "clipboard={}",
            clipboard.as_deref().unwrap_or("unavailable")
        ));
        lines.push(format!("file={}", file_path.display()));
        lines.push(format!("chars={}", selected.text.chars().count()));
        lines.push(format!("lines={}", selected.text.lines().count().max(1)));
        lines.push(format!(
            "preview={}",
            sanitize_single_line(selected.text.lines().next().unwrap_or_default())
        ));
        lines.push(if clipboard.is_some() {
            "status=copied assistant response".into()
        } else {
            "status=wrote assistant response to fallback file".into()
        });
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

#[derive(Debug, Parser)]
struct AddDirArgs {
    #[arg()]
    path: PathBuf,
}

#[async_trait]
impl Command for AddDirCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<AddDirArgs>("add-dir", &invocation)?;
        let path = if args.path.is_absolute() {
            args.path
        } else {
            context.cwd.join(args.path)
        };
        let canonical = path.canonicalize().map_err(|error| {
            WonderError::validation(format!(
                "working directory `{}` is not accessible: {error}",
                path.display()
            ))
        })?;
        if !canonical.is_dir() {
            return Err(WonderError::validation(format!(
                "`{}` is not a directory",
                canonical.display()
            )));
        }
        if canonical == context.cwd
            || context
                .additional_working_directories
                .iter()
                .any(|directory| directory.path == canonical)
        {
            return Ok(CommandOutput::Text(format!(
                concat!(
                    "add_dir={}\n",
                    "add_dir_source={}\n",
                    "status=working directory already available"
                ),
                canonical.display(),
                PermissionRuleSource::SessionRuntime.label(),
            )));
        }

        Ok(CommandOutput::Text(format!(
            concat!(
                "add_dir={}\n",
                "add_dir_source={}\n",
                "status=added working directory"
            ),
            canonical.display(),
            PermissionRuleSource::SessionRuntime.label(),
        )))
    }
}

/// Represents files command
pub struct FilesCommand;

impl FilesCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "files",
            "List workspace files from the current working directory",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::Tools]);
        spec
    }
}

#[derive(Debug, Parser)]
struct FilesArgs {
    #[arg()]
    path: Option<PathBuf>,
    #[arg(long, default_value_t = 100)]
    limit: usize,
    #[arg(long)]
    hidden: bool,
}

#[async_trait]
impl Command for FilesCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<FilesArgs>("files", &invocation)?;
        let root = args
            .path
            .map_or_else(|| context.cwd.clone(), |path| context.cwd.join(path));
        let limit = args.limit.max(1);
        let mut entries = Vec::new();
        for entry in WalkDir::new(&root)
            .into_iter()
            .filter_entry(|entry| args.hidden || !is_hidden_path(entry.path()))
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
        {
            let display = entry
                .path()
                .strip_prefix(&context.cwd)
                .unwrap_or(entry.path())
                .display()
                .to_string();
            entries.push(display);
            if entries.len() > limit {
                break;
            }
        }
        entries.sort();
        let truncated = entries.len() > limit;
        if truncated {
            entries.truncate(limit);
        }

        let mut lines = vec![format!("files={}", entries.len())];
        lines.extend(entries);
        if truncated {
            lines.push(format!("truncated=true ({limit} shown)"));
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

fn load_snapshot_state(
    storage_dir: Option<&Path>,
    context: &CommandContext,
) -> Result<Option<AppState>> {
    let Some(storage_dir) = storage_dir else {
        return Ok(None);
    };
    Ok(TranscriptStore::new(storage_dir)
        .read_snapshot_if_exists(context.session_id)?
        .map(|snapshot| snapshot.state))
}

fn context_state_fallback(context: &CommandContext) -> AppState {
    let mut state = AppState::new(context.cwd.clone());
    state.permission_mode = context.permission_mode;
    state.set_theme(context.theme.clone());
    state.set_session_color(context.session_color.clone());
    state.set_effort_level(context.effort_level.clone());
    state.set_brief_mode(context.brief_mode);
    state.additional_working_directories = context.additional_working_directories.clone();
    state
}

#[derive(Clone)]
struct CopyResponse {
    text: String,
}

struct CopySource {
    session_id: SessionId,
    messages: Vec<MessageEnvelope>,
}

fn load_copy_source(storage_dir: Option<&Path>, context: &CommandContext) -> Result<CopySource> {
    let Some(storage_dir) = storage_dir else {
        return Ok(CopySource {
            session_id: context.session_id,
            messages: Vec::new(),
        });
    };
    let store = TranscriptStore::new(storage_dir);
    if let Some(snapshot) = store.read_snapshot_if_exists(context.session_id)? {
        return Ok(CopySource {
            session_id: context.session_id,
            messages: snapshot.state.messages,
        });
    }
    if let Ok(restored) = store.restore_session(context.session_id) {
        return Ok(CopySource {
            session_id: context.session_id,
            messages: restored.state.messages,
        });
    }
    if let Some(latest) = store.list_metadata()?.into_iter().next() {
        if let Some(snapshot) = store.read_snapshot_if_exists(latest.session_id)? {
            return Ok(CopySource {
                session_id: latest.session_id,
                messages: snapshot.state.messages,
            });
        }
        if let Ok(restored) = store.restore_session(latest.session_id) {
            return Ok(CopySource {
                session_id: latest.session_id,
                messages: restored.state.messages,
            });
        }
    }
    Ok(CopySource {
        session_id: context.session_id,
        messages: Vec::new(),
    })
}

fn collect_recent_assistant_texts(messages: &[MessageEnvelope]) -> Vec<CopyResponse> {
    let mut responses = Vec::new();
    for message in messages.iter().rev() {
        if let MessagePayload::AssistantText { content } = &message.payload {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                responses.push(CopyResponse {
                    text: trimmed.to_string(),
                });
            }
        }
        if responses.len() >= 20 {
            break;
        }
    }
    responses
}

fn write_copy_text_to_temp_file(text: &str, filename: &str) -> Result<PathBuf> {
    static COPY_TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join("wonder-of-u");
    fs::create_dir_all(&dir)?;
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let counter = COPY_TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!(
        "{}-{unique}-{counter}-{filename}",
        std::process::id()
    ));
    fs::write(&path, text)?;
    Ok(path)
}

fn try_copy_to_clipboard(text: &str) -> Option<String> {
    [
        ("wl-copy", &[][..]),
        ("xclip", &["-selection", "clipboard"][..]),
        ("xsel", &["--clipboard", "--input"][..]),
        ("pbcopy", &[][..]),
        ("clip", &[][..]),
    ]
    .into_iter()
    .find_map(|(command, args)| run_clipboard_command(command, args, text).then(|| command.into()))
}

fn run_clipboard_command(command: &str, args: &[&str], text: &str) -> bool {
    let mut child = match ProcessCommand::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(text.as_bytes()).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }
    }
    child.wait().is_ok_and(|status| status.success())
}

fn render_context_summary(state: &AppState) -> String {
    let total_messages = state.messages.len();
    let mut user_messages = 0usize;
    let mut assistant_messages = 0usize;
    let mut tool_uses = 0usize;
    let mut tool_results = 0usize;
    let mut command_messages = 0usize;
    let mut compact_boundaries = 0usize;
    let mut permissions = 0usize;
    let mut task_updates = 0usize;
    for message in &state.messages {
        match &message.payload {
            MessagePayload::UserText { .. }
            | MessagePayload::UserAttachment { .. }
            | MessagePayload::UserPasteReference { .. } => user_messages += 1,
            MessagePayload::AssistantText { .. } | MessagePayload::AssistantThinking { .. } => {
                assistant_messages += 1;
            }
            MessagePayload::AssistantToolUse { .. } => tool_uses += 1,
            MessagePayload::ToolResult { .. } | MessagePayload::BashOutput { .. } => {
                tool_results += 1;
            }
            MessagePayload::Command { .. } => command_messages += 1,
            MessagePayload::CompactBoundary { .. } => compact_boundaries += 1,
            MessagePayload::Permission { .. } => permissions += 1,
            MessagePayload::Task { .. } => task_updates += 1,
            MessagePayload::System { .. }
            | MessagePayload::Progress { .. }
            | MessagePayload::HookResult { .. }
            | MessagePayload::HookProgress { .. }
            | MessagePayload::PlanApproval { .. }
            | MessagePayload::ProviderError { .. }
            | MessagePayload::TaskNotification { .. } => {}
        }
    }
    let running_tasks = state
        .background_tasks
        .values()
        .filter(|task| matches!(task.status, TaskStatus::Pending | TaskStatus::Running))
        .count();

    let mut lines = vec!["## Context Usage".into()];
    lines.push(format!("working_directory={}", state.session.cwd.display()));
    lines.push(format!(
        "model={}",
        state
            .model
            .as_ref()
            .map_or_else(|| "unconfigured".into(), Clone::clone)
    ));
    lines.push(format!(
        "provider={}",
        state
            .provider
            .as_ref()
            .map_or_else(|| "unconfigured".into(), Clone::clone)
    ));
    lines.push(format!(
        "permission_mode={}",
        permission_mode_label(state.permission_mode)
    ));
    lines.push(format!("total_tokens={}", state.costs.usage.total_tokens()));
    if let Some(cost) = state.costs.estimated_cost_usd {
        lines.push(format!("estimated_cost_usd={cost:.4}"));
    }
    lines.push(String::new());
    lines.push("### Message breakdown".into());
    lines.push(format!("messages={total_messages}"));
    lines.push(format!("user_messages={user_messages}"));
    lines.push(format!("assistant_messages={assistant_messages}"));
    lines.push(format!("tool_uses={tool_uses}"));
    lines.push(format!("tool_results={tool_results}"));
    lines.push(format!("commands={command_messages}"));
    lines.push(format!("compact_boundaries={compact_boundaries}"));
    lines.push(format!("permission_events={permissions}"));
    lines.push(format!("task_updates={task_updates}"));
    lines.push(String::new());
    lines.push("### Session extras".into());
    lines.push(format!(
        "additional_working_directories={}",
        state.additional_working_directories.len()
    ));
    for directory in &state.additional_working_directories {
        lines.push(format!(
            "directory={};source={}",
            directory.path.display(),
            directory.source.label()
        ));
    }
    lines.push(format!("queued_commands={}", state.queued_commands.len()));
    lines.push(format!("background_tasks={}", state.background_tasks.len()));
    lines.push(format!("active_tasks={running_tasks}"));
    lines.join("\n")
}

fn normalize_memory_invocation(invocation: &CommandInvocation) -> CommandInvocation {
    let trimmed = invocation.args.trim();
    if matches!(trimmed, "project" | "user" | "--project" | "--user") {
        let target = trimmed.trim_start_matches('-');
        return CommandInvocation {
            name: invocation.name.clone(),
            args: format!("open {target}"),
            raw: invocation.raw.clone(),
        };
    }
    invocation.clone()
}

fn render_memory_status(context: &CommandContext, storage_dir: Option<&Path>) -> String {
    let project = resolve_memory_path(context.cwd.as_path(), storage_dir, MemoryTarget::Project);
    let user = resolve_memory_path(context.cwd.as_path(), storage_dir, MemoryTarget::User);
    [
        "memory=ready".into(),
        "note=run `/memory open project` or `/memory open user` to edit".into(),
        format!("project_memory={}", project.display()),
        format!("project_exists={}", project.exists()),
        format!("user_memory={}", user.display()),
        format!("user_exists={}", user.exists()),
    ]
    .join("\n")
}

pub(crate) fn memory_open_output(
    context: &CommandContext,
    storage_dir: Option<&Path>,
    target: MemoryTarget,
) -> Result<String> {
    let path = resolve_memory_path(context.cwd.as_path(), storage_dir, target);
    ensure_memory_file(&path)?;
    if context.interactive {
        let mut lines = vec![
            "memory=ready".into(),
            format!("memory_target={}", memory_target_label(target)),
            format!("memory_path={}", path.display()),
            format!("memory_exists={}", path.exists()),
        ];
        if editor_command().is_some() {
            lines.push("open_external=true".into());
            lines.push("note=opening memory file in external editor".into());
        } else {
            lines.push("note=set VISUAL or EDITOR to enable `/memory open`".into());
        }
        return Ok(lines.join("\n"));
    }
    let Some((editor, args)) = editor_command() else {
        return Ok(format!(
            concat!(
                "memory=ready\n",
                "memory_target={}\n",
                "memory_path={}\n",
                "note=set VISUAL or EDITOR to enable `/memory open`"
            ),
            memory_target_label(target),
            path.display(),
        ));
    };
    let status = ProcessCommand::new(&editor)
        .args(args)
        .arg(&path)
        .current_dir(&context.cwd)
        .status()?;
    Ok(format!(
        concat!(
            "memory=ready\n",
            "memory_target={}\n",
            "memory_path={}\n",
            "editor={}\n",
            "editor_success={}"
        ),
        memory_target_label(target),
        path.display(),
        editor,
        status.success(),
    ))
}

fn render_memory_picker_output(context: &CommandContext, storage_dir: Option<&Path>) -> String {
    let options = [MemoryTarget::Project, MemoryTarget::User]
        .into_iter()
        .map(|target| {
            let path = resolve_memory_path(context.cwd.as_path(), storage_dir, target);
            format!(
                "memory_option={}",
                json!({
                    "target": memory_target_label(target),
                    "label": match target {
                        MemoryTarget::Project => "Project memory",
                        MemoryTarget::User => "User memory",
                    },
                    "description": path.display().to_string(),
                    "path": path.display().to_string(),
                    "selected": matches!(target, MemoryTarget::Project),
                })
            )
        })
        .collect::<Vec<_>>();
    let mut lines = vec!["memory_picker=true".into()];
    lines.extend(options);
    lines.join("\n")
}

fn init_output(
    context: &CommandContext,
    path: &Path,
    existed: bool,
    forced: bool,
    should_open: bool,
) -> Result<String> {
    if context.interactive {
        let mut lines = vec![
            "init=ready".into(),
            format!("path={}", path.display()),
            format!("created={}", !existed),
            format!("overwritten={}", existed && forced),
        ];
        if should_open && editor_command().is_some() {
            lines.push("open_external=true".into());
            lines.push(format!("external_path={}", path.display()));
            lines.push("status=initialized project memory".into());
            lines.push("note=opening CLAUDE.md in external editor".into());
        } else {
            lines.push("status=initialized project memory".into());
        }
        return Ok(lines.join("\n"));
    }

    if should_open {
        let Some((editor, args)) = editor_command() else {
            return Ok(format!(
                concat!(
                    "init=ready\n",
                    "path={}\n",
                    "created={}\n",
                    "overwritten={}\n",
                    "status=initialized project memory\n",
                    "note=set VISUAL or EDITOR to open CLAUDE.md automatically"
                ),
                path.display(),
                !existed,
                existed && forced,
            ));
        };
        let status = ProcessCommand::new(&editor)
            .args(args)
            .arg(path)
            .current_dir(&context.cwd)
            .status()?;
        return Ok(format!(
            concat!(
                "init=ready\n",
                "path={}\n",
                "created={}\n",
                "overwritten={}\n",
                "editor={}\n",
                "editor_success={}\n",
                "status=initialized project memory"
            ),
            path.display(),
            !existed,
            existed && forced,
            editor,
            status.success(),
        ));
    }

    Ok(format!(
        concat!(
            "init=ready\n",
            "path={}\n",
            "created={}\n",
            "overwritten={}\n",
            "status=initialized project memory"
        ),
        path.display(),
        !existed,
        existed && forced,
    ))
}

fn resolve_memory_path(cwd: &Path, storage_dir: Option<&Path>, target: MemoryTarget) -> PathBuf {
    match target {
        MemoryTarget::Project => cwd.join("CLAUDE.md"),
        MemoryTarget::User => storage_dir
            .map(TranscriptStore::new)
            .map(|store| store.paths().config_dir())
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".wonder-of-u")
                    .join("config")
            })
            .join("CLAUDE.md"),
    }
}

fn ensure_memory_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new().create(true).append(true).open(path)?;
    Ok(())
}

fn write_init_template(path: &Path, cwd: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, render_init_template(cwd))?;
    Ok(())
}

fn render_init_template(cwd: &Path) -> String {
    format!(
        concat!(
            "# CLAUDE.md\n\n",
            "Project-specific instructions for agents working in `{}`.\n\n",
            "## Fill this in\n",
            "- Build and test commands for this repo\n",
            "- Code style and architecture rules\n",
            "- Important workflows, release checks, and review expectations\n",
            "- Domain constraints or files that need extra care\n"
        ),
        cwd.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("this workspace")
    )
}

fn memory_target_label(target: MemoryTarget) -> &'static str {
    match target {
        MemoryTarget::Project => "project",
        MemoryTarget::User => "user",
    }
}

fn editor_command() -> Option<(String, Vec<String>)> {
    let raw = std::env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })?;
    let mut tokens = shell_words::split(&raw).ok()?;
    let command = tokens.first()?.clone();
    Some((command, tokens.drain(1..).collect()))
}

fn permission_mode_label(mode: wonder_of_u_core::PermissionMode) -> &'static str {
    match mode {
        wonder_of_u_core::PermissionMode::Default => "default",
        wonder_of_u_core::PermissionMode::AcceptEdits => "accept-edits",
        wonder_of_u_core::PermissionMode::BypassPermissions => "bypass-permissions",
        wonder_of_u_core::PermissionMode::DontAsk => "dont-ask",
        wonder_of_u_core::PermissionMode::Plan => "plan",
    }
}

fn sanitize_single_line(value: &str) -> String {
    value.lines().collect::<Vec<_>>().join("\\n")
}

/// Represents branch command
pub struct BranchCommand;

impl BranchCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "branch",
            "Show the current git branch for the working directory",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::Tools]);
        spec
    }
}

#[async_trait]
impl Command for BranchCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let mut lines = vec![format!(
            "branch={}",
            detect_git_branch(&context.cwd).unwrap_or_else(|| "unavailable".into())
        )];
        if let Some(root) = git_command_output(&context.cwd, &["rev-parse", "--show-toplevel"]) {
            lines.push(format!("git_root={root}"));
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

/// Represents diff command
pub struct DiffCommand;

impl DiffCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "diff",
            "Show a git diff summary for the working directory",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::Tools]);
        spec
    }
}

#[derive(Debug, Parser)]
struct DiffArgs {
    #[arg(long)]
    staged: bool,
    #[arg(long)]
    name_only: bool,
}

#[async_trait]
impl Command for DiffCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<DiffArgs>("diff", &invocation)?;
        let mut command = std::process::Command::new("git");
        command
            .current_dir(&context.cwd)
            .arg("--no-pager")
            .arg("diff");
        if args.staged {
            command.arg("--cached");
        }
        if args.name_only {
            command.arg("--name-only");
        } else {
            command.args(["--stat", "--find-renames"]);
        }

        let output = match command.output() {
            Ok(output) if output.status.success() => output,
            _ => {
                return Ok(CommandOutput::Text(
                    "diff=unavailable\nnote=current working directory is not a readable git worktree".into(),
                ));
            }
        };
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() {
            return Ok(CommandOutput::Text("diff=clean".into()));
        }
        Ok(CommandOutput::Text(stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use wonder_of_u_core::{
        AdditionalWorkingDirectory, FeatureSet, MessagePayload, PermissionMode, SessionId,
    };
    use wonder_of_u_storage::{SessionMetadata, SessionSnapshot};
    use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

    fn command_context(cwd: &std::path::Path) -> CommandContext {
        CommandContext {
            session_id: SessionId::new(),
            cwd: cwd.to_path_buf(),
            features: FeatureSet::first_release(),
            authenticated: false,
            interactive: true,
            permission_mode: PermissionMode::Default,
            theme: None,
            session_color: None,
            effort_level: None,
            brief_mode: false,
            fast_mode: false,
            optimize_token_mode: false,
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        }
    }

    #[test]
    fn add_dir_returns_canonical_hint_for_relative_paths() {
        let workspace = unique_test_dir("project-add-dir-relative");
        let nested = workspace.join("nested");
        std::fs::create_dir_all(&nested).expect("create nested");

        let output = block_on(AddDirCommand::new().execute(
            command_context(&workspace),
            CommandInvocation {
                name: "add-dir".into(),
                args: "nested".into(),
                raw: "/add-dir nested".into(),
            },
        ))
        .expect("run add-dir");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains(&format!("add_dir={}", nested.display())));
        assert!(text.contains("add_dir_source=session_runtime"));
        assert!(text.contains("status=added working directory"));
    }

    #[test]
    fn add_dir_reports_duplicate_directories() {
        let workspace = unique_test_dir("project-add-dir-duplicate");
        let nested = workspace.join("nested");
        std::fs::create_dir_all(&nested).expect("create nested");
        let mut context = command_context(&workspace);
        context.additional_working_directories = vec![AdditionalWorkingDirectory::new(
            nested.clone(),
            PermissionRuleSource::SessionRuntime,
        )];

        let output = block_on(AddDirCommand::new().execute(
            context,
            CommandInvocation {
                name: "add-dir".into(),
                args: nested.to_string_lossy().into_owned(),
                raw: format!("/add-dir {}", nested.display()),
            },
        ))
        .expect("run add-dir");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("status=working directory already available"));
    }

    #[test]
    fn memory_open_outputs_external_editor_hint() {
        let workspace = unique_test_dir("project-memory-open");
        let storage = workspace.join(".wonder");
        let _editor = EnvVarGuard::set("EDITOR", "vi");

        let output = block_on(MemoryCommand::new(Some(storage.clone())).execute(
            command_context(&workspace),
            CommandInvocation {
                name: "memory".into(),
                args: "open user".into(),
                raw: "/memory open user".into(),
            },
        ))
        .expect("run memory open");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("open_external=true"));
        assert!(text.contains("memory_target=user"));
        assert!(text.contains(&format!(
            "memory_path={}",
            storage.join("config/CLAUDE.md").display()
        )));
    }

    #[test]
    fn memory_bare_command_opens_picker_in_interactive_mode() {
        let workspace = unique_test_dir("project-memory-picker");
        let storage = workspace.join(".wonder");

        let output = block_on(MemoryCommand::new(Some(storage.clone())).execute(
            command_context(&workspace),
            CommandInvocation {
                name: "memory".into(),
                args: String::new(),
                raw: "/memory".into(),
            },
        ))
        .expect("run memory");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("memory_picker=true"));
        assert!(text.contains("\"target\":\"project\""));
        assert!(text.contains("\"target\":\"user\""));
    }

    #[test]
    fn init_command_creates_project_memory_file() {
        let workspace = unique_test_dir("project-init");
        let claude_md = workspace.join("CLAUDE.md");

        let output = block_on(InitCommand::new().execute(
            command_context(&workspace),
            CommandInvocation {
                name: "init".into(),
                args: String::new(),
                raw: "/init".into(),
            },
        ))
        .expect("run init");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("init=ready"));
        assert!(text.contains("created=true"));
        assert_eq!(
            std::fs::read_to_string(&claude_md).expect("read CLAUDE.md"),
            render_init_template(&workspace)
        );
    }

    #[test]
    fn init_command_requests_editor_in_interactive_mode_for_new_file() {
        let workspace = unique_test_dir("project-init-editor");
        let _editor = EnvVarGuard::set("EDITOR", "true");
        let mut context = command_context(&workspace);
        context.interactive = true;

        let output = block_on(InitCommand::new().execute(
            context,
            CommandInvocation {
                name: "init".into(),
                args: String::new(),
                raw: "/init".into(),
            },
        ))
        .expect("run init");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("open_external=true"));
        assert!(text.contains(&format!(
            "external_path={}",
            workspace.join("CLAUDE.md").display()
        )));
    }

    #[test]
    fn copy_command_uses_current_session_snapshot() {
        let storage = unique_test_dir("project-copy-current");
        let workspace = storage.join("workspace");
        let mut state = AppState::new(workspace.clone());
        state
            .push_message(MessageEnvelope::new(
                state.session.id,
                MessagePayload::AssistantText {
                    content: "older answer".into(),
                },
            ))
            .expect("push older answer");
        state
            .push_message(MessageEnvelope::new(
                state.session.id,
                MessagePayload::AssistantText {
                    content: "latest answer".into(),
                },
            ))
            .expect("push latest answer");
        let store = TranscriptStore::new(storage.clone());
        store
            .write_snapshot(&SessionSnapshot::from_app_state(
                &state,
                state.messages.len(),
                0,
            ))
            .expect("write snapshot");
        store
            .write_metadata(&SessionMetadata::from_app_state(&state))
            .expect("write metadata");

        let output = block_on(CopyCommand::new(Some(storage.clone())).execute(
            command_context(&workspace),
            CommandInvocation {
                name: "copy".into(),
                args: "2".into(),
                raw: "/copy 2".into(),
            },
        ))
        .expect("run copy");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains(&format!("source_session_id={}", state.session.id)));
        assert!(text.contains("selected_response=2"));
        let file = text
            .lines()
            .find_map(|line| line.strip_prefix("file="))
            .expect("file path");
        assert_eq!(
            std::fs::read_to_string(file).expect("read copied file"),
            "older answer"
        );
    }

    #[test]
    fn copy_command_falls_back_to_latest_persisted_session() {
        let storage = unique_test_dir("project-copy-fallback");
        let workspace = storage.join("workspace");
        let mut state = AppState::new(workspace.clone());
        state
            .push_message(MessageEnvelope::new(
                state.session.id,
                MessagePayload::AssistantText {
                    content: "persisted answer".into(),
                },
            ))
            .expect("push answer");
        let store = TranscriptStore::new(storage.clone());
        store
            .write_snapshot(&SessionSnapshot::from_app_state(
                &state,
                state.messages.len(),
                0,
            ))
            .expect("write snapshot");
        store
            .write_metadata(&SessionMetadata::from_app_state(&state))
            .expect("write metadata");

        let mut unrelated = command_context(&workspace);
        unrelated.session_id = SessionId::new();

        let output = block_on(CopyCommand::new(Some(storage.clone())).execute(
            unrelated,
            CommandInvocation {
                name: "copy".into(),
                args: String::new(),
                raw: "/copy".into(),
            },
        ))
        .expect("run copy");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains(&format!("source_session_id={}", state.session.id)));
        let file = text
            .lines()
            .find_map(|line| line.strip_prefix("file="))
            .expect("file path");
        assert_eq!(
            std::fs::read_to_string(file).expect("read copied file"),
            "persisted answer"
        );
    }
}
