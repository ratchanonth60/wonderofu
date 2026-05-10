use std::{collections::BTreeSet, path::PathBuf};

use async_trait::async_trait;
use clap::{Args, Parser, Subcommand};
use serde_json::json;
use wonder_of_u_agent::ProviderResolver;
use wonder_of_u_core::{
    AppState, Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec,
    FeatureFlag, InputMode, MessageEnvelope, MessagePayload, Result, SessionId, WonderError,
    session_footer_text, session_status_text,
};
use wonder_of_u_storage::{
    LoadedTranscript, RestoredSession, SessionMemoryIndexStore, SessionMetadata,
    SessionResumeSource, SessionSnapshot, TranscriptStore,
};

use super::{detect_git_branch, parse_command_args, parse_session_id};

/// Represents session command
pub struct SessionCommand {
    storage_dir: Option<PathBuf>,
}

impl SessionCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "session",
            "Create or list persisted session metadata",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::SessionPersistence]);
        spec
    }
}

#[derive(Debug, Parser)]
struct SessionArgs {
    #[command(subcommand)]
    command: Option<SessionSubcommand>,
}

#[derive(Debug, Subcommand)]
enum SessionSubcommand {
    New(SessionNewArgs),
    List(SessionListArgs),
}

#[derive(Debug, Args)]
struct SessionNewArgs {
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    cwd: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct SessionListArgs {
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

#[async_trait]
impl Command for SessionCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<SessionArgs>("session", &invocation)?;
        match args
            .command
            .unwrap_or(SessionSubcommand::List(SessionListArgs { limit: 20 }))
        {
            SessionSubcommand::New(args) => self.create_session(args),
            SessionSubcommand::List(args) => self.list_sessions(args),
        }
    }
}

impl SessionCommand {
    fn create_session(&self, args: SessionNewArgs) -> Result<CommandOutput> {
        let cwd = args.cwd.unwrap_or(std::env::current_dir()?);
        let mut state = AppState::new(cwd.clone());
        if let Some(title) = args.title {
            state.session.title = title;
        }
        state.session.git_branch = detect_git_branch(&cwd);
        let report = ProviderResolver::builtin().load_report(self.storage_dir.as_deref())?;
        state.set_provider_context(report.provider, report.model, report.auth);
        let message = MessageEnvelope::system(state.session.id, "Session created")
            .with_context(
                Some(state.session.cwd.clone()),
                state.session.git_branch.clone(),
            )
            .with_runtime(
                state.session.entrypoint.clone(),
                state.session.app_version.clone(),
            );
        state.push_message(message.clone())?;

        let persisted = if let Some(storage_dir) = &self.storage_dir {
            let store = TranscriptStore::new(storage_dir);
            store.append_message(&message)?;
            persist_session_state(&store, &state, state.messages.len(), 0)?;
            true
        } else {
            false
        };

        let mut lines = vec![
            format!("session_id={}", state.session.id),
            format!("title={}", state.session.title),
            format!("persisted={persisted}"),
        ];
        if let Some(branch) = state.session.git_branch {
            lines.push(format!("git_branch={branch}"));
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }

    fn list_sessions(&self, args: SessionListArgs) -> Result<CommandOutput> {
        let store = require_store(&self.storage_dir)?;
        let sessions = store.list_metadata()?;
        let limit = args.limit.max(1);
        let total = sessions.len();
        let mut lines = vec![format!("sessions={total}")];
        for (index, metadata) in sessions.into_iter().take(limit).enumerate() {
            let snapshot = store.read_snapshot_if_exists(metadata.session_id)?;
            let view_messages = snapshot
                .as_ref()
                .map_or(metadata.message_count, |snapshot| {
                    snapshot.state.messages.len()
                });
            let compacted_messages = snapshot.as_ref().map_or(0, |snapshot| {
                compacted_message_count(metadata.message_count, &snapshot.state.messages)
            });
            lines.push(format!("session[{index}].id={}", metadata.session_id));
            lines.push(format!("session[{index}].title={}", metadata.title));
            lines.push(format!("session[{index}].cwd={}", metadata.cwd.display()));
            lines.push(format!(
                "session[{index}].updated_at={}",
                metadata.updated_at
            ));
            lines.push(format!(
                "session[{index}].messages={}",
                metadata.message_count
            ));
            lines.push(format!(
                "session[{index}].resume_source={}",
                snapshot_source_label(snapshot.as_ref())
            ));
            lines.push(format!("session[{index}].view_messages={view_messages}"));
            lines.push(format!(
                "session[{index}].compacted_messages={compacted_messages}"
            ));
            if !metadata.tags.is_empty() {
                lines.push(format!(
                    "session[{index}].tags={}",
                    format_session_tags(&metadata.tags)
                ));
            }
        }
        if total > limit {
            lines.push(format!("truncated=true ({limit} shown)"));
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

/// Represents resume command
pub struct ResumeCommand {
    storage_dir: Option<PathBuf>,
}

impl ResumeCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "resume",
            "Resume a persisted session in the live shell when interactive, or show a summary",
            CommandKind::ResumeEntrypoint,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::SessionPersistence]);
        spec
    }
}

#[derive(Debug, Parser)]
struct ResumeArgs {
    #[arg()]
    session_id: String,
}

#[async_trait]
impl Command for ResumeCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<ResumeArgs>("resume", &invocation)?;
        let restored = load_restored_session(&self.storage_dir, &args.session_id)?;
        let mut lines = resume_summary_lines(&restored);
        lines.push(
            format!(
                "note=run `wonder-of-u resume {}` or `wonder-of-u tui --session-id {}` in a terminal to continue in the live shell; non-interactive mode prints this persisted summary",
                args.session_id, args.session_id
            ),
        );
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

/// Represents tag command
pub struct TagCommand {
    storage_dir: Option<PathBuf>,
}

impl TagCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "tag",
            "Toggle a searchable tag on the current session",
            CommandKind::Local,
        );
        spec.interactive_only = true;
        spec
    }

    fn current_session_tags(&self, context: &CommandContext) -> Result<Vec<String>> {
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return Ok(context.session_tags.clone());
        };
        let store = TranscriptStore::new(storage_dir);
        if let Some(snapshot) = store.read_snapshot_if_exists(context.session_id)? {
            return Ok(snapshot.state.session.tags);
        }
        Ok(store.read_metadata(context.session_id)?.tags)
    }

    fn persist_session_tags(&self, session_id: SessionId, tags: &[String]) -> Result<()> {
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return Ok(());
        };
        let store = TranscriptStore::new(storage_dir);
        let snapshot = store.read_snapshot(session_id)?;
        let mut state = snapshot.state;
        state.set_session_tags(tags.to_vec());
        persist_session_state(
            &store,
            &state,
            snapshot.transcript_message_count,
            snapshot.transcript_warning_count,
        )
    }
}

#[async_trait]
impl Command for TagCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let raw = invocation.args.trim();
        if raw.is_empty() || is_help_arg(raw) {
            return Ok(CommandOutput::Text(tag_help_text()));
        }

        let tag = normalize_tag_name(raw)?;
        let current_tags = self.current_session_tags(&context)?;
        let current_tag = current_tags.first().cloned();

        if current_tag.as_deref() == Some(tag.as_str()) && context.interactive {
            return Ok(CommandOutput::Text(format!(
                "tag_remove_confirmation={tag}\nstatus=confirm tag removal"
            )));
        }

        let next_tags = if current_tag.as_deref() == Some(tag.as_str()) {
            Vec::new()
        } else {
            vec![tag.clone()]
        };
        self.persist_session_tags(context.session_id, &next_tags)?;

        let status = if next_tags.is_empty() {
            format!("removed tag #{tag}")
        } else {
            format!("tagged session with #{tag}")
        };

        Ok(CommandOutput::Text(format!(
            "session_tags={}\nstatus={status}",
            next_tags.join(",")
        )))
    }
}

/// Represents rename command
pub struct RenameCommand {
    storage_dir: Option<PathBuf>,
}

impl RenameCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "rename",
            "Rename a persisted session and refresh resumable state",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::SessionPersistence]);
        spec
    }
}

#[derive(Debug, Parser)]
struct RenameArgs {
    #[arg()]
    session_id: String,
    #[arg()]
    title: String,
}

#[async_trait]
impl Command for RenameCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<RenameArgs>("rename", &invocation)?;
        let session_id = parse_session_id(&args.session_id)?;
        let store = require_store(&self.storage_dir)?;
        let restored = store.restore_session(session_id)?;
        let mut state = restored.state;
        state.session.title = args.title;
        let message = MessageEnvelope::system(
            session_id,
            format!("Session renamed to {}", state.session.title),
        )
        .with_context(
            Some(state.session.cwd.clone()),
            state.session.git_branch.clone(),
        )
        .with_runtime(
            state.session.entrypoint.clone(),
            state.session.app_version.clone(),
        );
        store.append_message(&message)?;
        state.push_message(message)?;
        persist_session_state(
            &store,
            &state,
            restored.transcript.messages.len() + 1,
            restored.transcript.warnings.len(),
        )?;

        Ok(CommandOutput::Text(format!(
            concat!(
                "session_id={}\n",
                "title={}\n",
                "message_count={}\n",
                "resume_source={}"
            ),
            state.session.id,
            state.session.title,
            restored.transcript.messages.len() + 1,
            SessionResumeSource::Snapshot.label(),
        )))
    }
}

/// Represents export command
pub struct ExportCommand {
    storage_dir: Option<PathBuf>,
}

impl ExportCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "export",
            "Export a persisted session transcript and resume view",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::SessionPersistence]);
        spec
    }
}

#[derive(Debug, Parser)]
struct ExportArgs {
    #[arg()]
    session_id: String,
    #[arg(long, default_value = "text")]
    format: String,
}

#[async_trait]
impl Command for ExportCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<ExportArgs>("export", &invocation)?;
        let restored = load_restored_session(&self.storage_dir, &args.session_id)?;
        match args.format.as_str() {
            "json" => Ok(CommandOutput::Text(serde_json::to_string_pretty(&json!({
                "metadata": restored.metadata,
                "transcript": restored.transcript,
                "resume": render_resume_export(&restored),
            }))?)),
            "text" => Ok(CommandOutput::Text(render_text_export(&restored))),
            other => Err(WonderError::validation(format!(
                "unsupported export format: {other}"
            ))),
        }
    }
}

/// Represents clear command
pub struct ClearCommand {
    storage_dir: Option<PathBuf>,
}

impl ClearCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "clear",
            "Persist a cleared resume view for the latest or specified session",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::SessionPersistence]);
        spec
    }
}

#[derive(Debug, Parser)]
struct ClearArgs {
    #[arg()]
    session_id: Option<String>,
}

#[async_trait]
impl Command for ClearCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<ClearArgs>("clear", &invocation)?;
        self.persist_view_state(args.session_id.as_deref(), 0, ViewAction::Clear)
    }
}

/// Represents compact command
pub struct CompactCommand {
    storage_dir: Option<PathBuf>,
}

impl CompactCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "compact",
            "Persist a compacted resume view for the latest or specified session",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::SessionPersistence]);
        spec
    }
}

#[derive(Debug, Parser)]
struct CompactArgs {
    #[arg()]
    session_id: Option<String>,
    #[arg(long, default_value_t = 8)]
    keep_last: usize,
}

#[async_trait]
impl Command for CompactCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<CompactArgs>("compact", &invocation)?;
        self.persist_view_state(
            args.session_id.as_deref(),
            args.keep_last,
            ViewAction::Compact,
        )
    }
}

impl ClearCommand {
    fn persist_view_state(
        &self,
        requested_session_id: Option<&str>,
        keep_last: usize,
        action: ViewAction,
    ) -> Result<CommandOutput> {
        persist_view_state(&self.storage_dir, requested_session_id, keep_last, action)
    }
}

impl CompactCommand {
    fn persist_view_state(
        &self,
        requested_session_id: Option<&str>,
        keep_last: usize,
        action: ViewAction,
    ) -> Result<CommandOutput> {
        persist_view_state(&self.storage_dir, requested_session_id, keep_last, action)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ViewAction {
    Clear,
    Compact,
}

impl ViewAction {
    const fn label(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::Compact => "compact",
        }
    }
}

fn persist_view_state(
    storage_dir: &Option<PathBuf>,
    requested_session_id: Option<&str>,
    keep_last: usize,
    action: ViewAction,
) -> Result<CommandOutput> {
    let (store, session_id) = resolve_session_target(storage_dir, requested_session_id)?;
    let restored = store.restore_session(session_id)?;
    let compacted_messages = restored.transcript.messages.len().saturating_sub(keep_last);
    let mut state = restored.state;
    state.messages =
        compacted_view_messages(&restored.transcript, session_id, keep_last, action, &state);
    state.input_mode = InputMode::Prompt;
    state.session.updated_at = time::OffsetDateTime::now_utc();
    persist_session_state(
        &store,
        &state,
        restored.transcript.messages.len(),
        restored.transcript.warnings.len(),
    )?;

    let mut lines = vec![
        format!("session_id={session_id}"),
        format!("view_action={}", action.label()),
        format!("resume_source={}", SessionResumeSource::Snapshot.label()),
        format!("transcript_messages={}", restored.transcript.messages.len()),
        format!("view_messages={}", state.messages.len()),
        format!("compacted_messages={compacted_messages}"),
        format!("keep_last={keep_last}"),
        "transcript_unchanged=true".into(),
        format!("status={}", session_status_text(&state)),
        format!("footer={}", session_footer_text(&state)),
    ];
    if let Some(summary) = state.messages.first().and_then(boundary_summary) {
        lines.push(format!("boundary_summary={summary}"));
    }
    lines.push(
        "note=this updates the persisted resume view; live TUI sessions reload the new snapshot after the slash command completes"
            .into(),
    );
    Ok(CommandOutput::Text(lines.join("\n")))
}

fn require_store(storage_dir: &Option<PathBuf>) -> Result<TranscriptStore> {
    storage_dir
        .as_ref()
        .map(TranscriptStore::new)
        .ok_or_else(|| {
            WonderError::validation(
                "this command requires --storage-dir so persisted session data can be loaded",
            )
        })
}

fn load_restored_session(
    storage_dir: &Option<PathBuf>,
    session_id: &str,
) -> Result<RestoredSession> {
    let store = require_store(storage_dir)?;
    store.restore_session(SessionId::parse(session_id)?)
}

fn resolve_session_target(
    storage_dir: &Option<PathBuf>,
    requested_session_id: Option<&str>,
) -> Result<(TranscriptStore, SessionId)> {
    let store = require_store(storage_dir)?;
    let session_id = match requested_session_id {
        Some(session_id) => parse_session_id(session_id)?,
        None => store
            .list_metadata()?
            .into_iter()
            .next()
            .map(|metadata| metadata.session_id)
            .ok_or_else(|| WonderError::not_found("persisted session", "latest"))?,
    };
    Ok((store, session_id))
}

fn persist_session_state(
    store: &TranscriptStore,
    state: &AppState,
    transcript_message_count: usize,
    transcript_warning_count: usize,
) -> Result<()> {
    store.write_metadata(&SessionMetadata::from_state_with_transcript(
        state,
        transcript_message_count,
    ))?;
    store.write_snapshot(&SessionSnapshot::from_app_state(
        state,
        transcript_message_count,
        transcript_warning_count,
    ))?;
    rebuild_session_memory_index(store, state, transcript_message_count)
}

fn rebuild_session_memory_index(
    store: &TranscriptStore,
    state: &AppState,
    transcript_message_count: usize,
) -> Result<()> {
    let index_store = SessionMemoryIndexStore::new(store.paths().base_dir());
    if state.messages.len() == transcript_message_count {
        index_store.rebuild_from_messages(state.session.id, &state.messages)?;
    } else {
        index_store.rebuild_from_transcript(store, state.session.id)?;
    }
    Ok(())
}

fn resume_summary_lines(restored: &RestoredSession) -> Vec<String> {
    let compacted_messages =
        compacted_message_count(restored.metadata.message_count, &restored.state.messages);
    let mut lines = vec![
        format!("session_id={}", restored.metadata.session_id),
        format!("title={}", restored.metadata.title),
        format!("cwd={}", restored.metadata.cwd.display()),
        format!("transcript_messages={}", restored.transcript.messages.len()),
        format!("view_messages={}", restored.state.messages.len()),
        format!("compacted_messages={compacted_messages}"),
        format!("warnings={}", restored.transcript.warnings.len()),
        format!(
            "provider_selection={}",
            provider_selection(&restored.metadata)
        ),
        format!(
            "provider_readiness={}",
            restored.state.provider_readiness().label()
        ),
        format!("auth_status={}", restored.state.auth.status_label()),
        format!("resume_state={}", restored.resume_source.label()),
        format!(
            "input_mode={}",
            wonder_of_u_core::input_mode_label(restored.state.input_mode)
        ),
        format!("queued_commands={}", restored.state.queued_commands.len()),
        format!("background_tasks={}", restored.state.background_tasks.len()),
        format!("status={}", session_status_text(&restored.state)),
        format!("footer={}", session_footer_text(&restored.state)),
    ];
    if let Some(branch) = &restored.metadata.git_branch {
        lines.push(format!("git_branch={branch}"));
    }
    if !restored.metadata.tags.is_empty() {
        lines.push(format!(
            "tags={}",
            format_session_tags(&restored.metadata.tags)
        ));
    }
    if let Some(last) = restored.transcript.messages.last() {
        lines.push(format!("last_transcript_message={}", message_summary(last)));
    }
    if let Some(last) = restored.state.messages.last() {
        lines.push(format!("last_view_message={}", message_summary(last)));
    }
    lines
}

fn snapshot_source_label(snapshot: Option<&SessionSnapshot>) -> &'static str {
    if snapshot.is_some() {
        SessionResumeSource::Snapshot.label()
    } else {
        SessionResumeSource::Transcript.label()
    }
}

fn provider_selection(metadata: &SessionMetadata) -> String {
    match (&metadata.provider, &metadata.model) {
        (Some(provider), Some(model)) => format!("{provider}:{model}"),
        _ => "unconfigured".into(),
    }
}

fn is_help_arg(value: &str) -> bool {
    matches!(value, "-h" | "--help" | "help" | "info")
}

fn normalize_tag_name(value: &str) -> Result<String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(WonderError::validation("tag name cannot be empty"));
    }
    Ok(normalized.into())
}

fn tag_help_text() -> String {
    concat!(
        "Usage: /tag <tag-name>\n\n",
        "Toggle a searchable tag on the current session.\n",
        "Run the same command again to remove the tag.\n",
        "Tags are displayed after the branch name in /resume and can be searched later.\n\n",
        "Examples:\n",
        "  /tag bugfix\n",
        "  /tag feature-auth\n",
        "  /tag wip"
    )
    .into()
}

fn format_session_tags(tags: &[String]) -> String {
    tags.iter()
        .map(|tag| format!("#{tag}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn render_text_export(restored: &RestoredSession) -> String {
    let mut lines = resume_summary_lines(restored);
    lines.push(String::new());
    for message in &restored.transcript.messages {
        lines.push(format!(
            "[{}] {}",
            message.timestamp,
            message_summary(message)
        ));
    }
    if !restored.transcript.warnings.is_empty() {
        lines.push(String::new());
        for warning in &restored.transcript.warnings {
            lines.push(format!(
                "warning[line {}]={}",
                warning.line, warning.message
            ));
        }
    }
    lines.join("\n")
}

fn render_resume_export(restored: &RestoredSession) -> serde_json::Value {
    json!({
        "source": restored.resume_source,
        "status_line": session_status_text(&restored.state),
        "footer_line": session_footer_text(&restored.state),
        "compacted_messages": compacted_message_count(
            restored.metadata.message_count,
            &restored.state.messages,
        ),
        "state": &restored.state,
    })
}

fn compacted_view_messages(
    transcript: &LoadedTranscript,
    session_id: SessionId,
    keep_last: usize,
    action: ViewAction,
    state: &AppState,
) -> Vec<MessageEnvelope> {
    if transcript.messages.is_empty() {
        return Vec::new();
    }

    let keep_last = keep_last.min(transcript.messages.len());
    let split_index = transcript.messages.len().saturating_sub(keep_last);
    let summarized = &transcript.messages[..split_index];
    let mut visible = Vec::new();

    if !summarized.is_empty() {
        visible.push(
            MessageEnvelope::new(
                session_id,
                MessagePayload::CompactBoundary {
                    summary: summarized_messages_summary(summarized, action),
                },
            )
            .with_context(
                Some(state.session.cwd.clone()),
                state.session.git_branch.clone(),
            )
            .with_runtime(
                state.session.entrypoint.clone(),
                state.session.app_version.clone(),
            ),
        );
    }

    visible.extend(transcript.messages[split_index..].iter().cloned());
    visible
}

fn summarized_messages_summary(messages: &[MessageEnvelope], action: ViewAction) -> String {
    let count = messages.len();
    let first_timestamp = messages
        .first()
        .map(|message| message.timestamp.to_string())
        .unwrap_or_else(|| "n/a".into());
    let last_timestamp = messages
        .last()
        .map(|message| message.timestamp.to_string())
        .unwrap_or_else(|| "n/a".into());
    let counts = payload_distribution(messages);
    let last_message = messages
        .last()
        .map(message_summary)
        .unwrap_or_else(|| "none".into());

    match action {
        ViewAction::Clear => format!(
            "Cleared the visible transcript and summarized {count} messages ({first_timestamp} -> {last_timestamp}; {counts}). Last summarized message: {last_message}"
        ),
        ViewAction::Compact => format!(
            "Compacted {count} earlier messages ({first_timestamp} -> {last_timestamp}; {counts}). Last summarized message: {last_message}"
        ),
    }
}

fn payload_distribution(messages: &[MessageEnvelope]) -> String {
    let mut user = 0usize;
    let mut assistant = 0usize;
    let mut system = 0usize;
    let mut tool = 0usize;
    let mut progress = 0usize;

    for message in messages {
        match &message.payload {
            MessagePayload::UserText { .. }
            | MessagePayload::UserAttachment { .. }
            | MessagePayload::UserPasteReference { .. } => user += 1,
            MessagePayload::AssistantText { .. } | MessagePayload::AssistantThinking { .. } => {
                assistant += 1;
            }
            MessagePayload::System { .. } | MessagePayload::CompactBoundary { .. } => system += 1,
            MessagePayload::AssistantToolUse { .. }
            | MessagePayload::ToolResult { .. }
            | MessagePayload::BashOutput { .. }
            | MessagePayload::HookResult { .. } => tool += 1,
            MessagePayload::Progress { .. }
            | MessagePayload::HookProgress { .. }
            | MessagePayload::Task { .. }
            | MessagePayload::Permission { .. }
            | MessagePayload::PlanApproval { .. }
            | MessagePayload::ProviderError { .. }
            | MessagePayload::Command { .. } => progress += 1,
        }
    }

    [
        ("user", user),
        ("assistant", assistant),
        ("system", system),
        ("tool", tool),
        ("progress", progress),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .map(|(label, count)| format!("{label}={count}"))
    .collect::<Vec<_>>()
    .join(",")
}

fn boundary_summary(message: &MessageEnvelope) -> Option<&str> {
    match &message.payload {
        MessagePayload::CompactBoundary { summary } => Some(summary.as_str()),
        _ => None,
    }
}

fn compacted_message_count(
    transcript_message_count: usize,
    visible_messages: &[MessageEnvelope],
) -> usize {
    let boundary_count = visible_messages
        .iter()
        .filter(|message| matches!(message.payload, MessagePayload::CompactBoundary { .. }))
        .count();
    transcript_message_count.saturating_sub(visible_messages.len().saturating_sub(boundary_count))
}

fn message_summary(message: &MessageEnvelope) -> String {
    match &message.payload {
        MessagePayload::UserText { content }
        | MessagePayload::AssistantText { content }
        | MessagePayload::System { content } => content.clone(),
        MessagePayload::UserAttachment { label, uri } => format!("attachment {label} ({uri})"),
        MessagePayload::UserPasteReference { sha256, bytes } => {
            format!("paste {sha256} ({bytes} bytes)")
        }
        MessagePayload::AssistantThinking { content, collapsed } => {
            if *collapsed {
                format!("thinking [collapsed] {content}")
            } else {
                format!("thinking {content}")
            }
        }
        MessagePayload::AssistantToolUse { tool, input, .. } => {
            format!("tool {tool} <= {input}")
        }
        MessagePayload::ToolResult {
            tool,
            success,
            content,
            ..
        } => format!(
            "tool {tool} {} {content}",
            if *success { "ok" } else { "error" }
        ),
        MessagePayload::BashOutput {
            stdout,
            stderr,
            exit_code,
        } => {
            if !stdout.is_empty() {
                format!("bash stdout {}", stdout.lines().next().unwrap_or_default())
            } else if !stderr.is_empty() {
                format!("bash stderr {}", stderr.lines().next().unwrap_or_default())
            } else if let Some(code) = exit_code {
                format!("bash exit code {code}")
            } else {
                "bash output".into()
            }
        }
        MessagePayload::Progress { label, detail } => match detail {
            Some(detail) => format!("progress {label}: {detail}"),
            None => format!("progress {label}"),
        },
        MessagePayload::Command { input, output } => match output {
            Some(output) => format!("command {input} => {output}"),
            None => format!("command {input}"),
        },
        MessagePayload::HookResult {
            hook,
            success,
            output,
        } => format!(
            "hook {hook} {} {output}",
            if *success { "ok" } else { "error" }
        ),
        MessagePayload::HookProgress {
            event,
            tool_name,
            hook_count,
            success,
        } => format!(
            "hook progress {event} {hook_count} for {tool_name} ({})",
            if *success { "ok" } else { "error" }
        ),
        MessagePayload::CompactBoundary { summary } => format!("summary {summary}"),
        MessagePayload::Task {
            status, message, ..
        } => format!("task {status:?}: {message}"),
        MessagePayload::Permission {
            tool,
            decision,
            reason,
        } => format!("permission {tool} {decision}: {reason}"),
        MessagePayload::PlanApproval { summary, approved } => format!(
            "plan {}: {summary}",
            if *approved { "approved" } else { "rejected" }
        ),
        MessagePayload::ProviderError { kind, message } => {
            format!("error[{kind}]: {message}")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use futures::executor::block_on;
    use wonder_of_u_core::{FeatureSet, PermissionMode};
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn command_context(cwd: &Path, session_id: SessionId) -> CommandContext {
        CommandContext {
            session_id,
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
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        }
    }

    #[test]
    fn tag_command_adds_session_tag_and_persists_metadata() {
        let dir = unique_test_dir("session-tag-add");
        let state = AppState::new(dir.clone());
        let store = TranscriptStore::new(&dir);
        persist_session_state(&store, &state, 0, 0).expect("persist initial session");

        let output = block_on(TagCommand::new(Some(dir.clone())).execute(
            command_context(&dir, state.session.id),
            CommandInvocation {
                name: "tag".into(),
                args: "bugfix".into(),
                raw: "/tag bugfix".into(),
            },
        ))
        .expect("tag succeeds");

        assert_eq!(
            output,
            CommandOutput::Text("session_tags=bugfix\nstatus=tagged session with #bugfix".into())
        );

        let metadata = store
            .read_metadata(state.session.id)
            .expect("read tagged metadata");
        let snapshot = store
            .read_snapshot(state.session.id)
            .expect("read tagged snapshot");
        assert_eq!(metadata.tags, vec!["bugfix"]);
        assert_eq!(snapshot.state.session.tags, vec!["bugfix"]);

        let summary = resume_summary_lines(&RestoredSession {
            metadata,
            transcript: LoadedTranscript {
                messages: Vec::new(),
                warnings: Vec::new(),
            },
            state: snapshot.state,
            resume_source: SessionResumeSource::Snapshot,
        });
        assert!(summary.iter().any(|line| line == "tags=#bugfix"));
    }

    #[test]
    fn tag_command_prompts_before_removing_current_tag_interactively() {
        let dir = unique_test_dir("session-tag-confirm");
        let mut state = AppState::new(dir.clone());
        state.set_session_tags(vec!["bugfix".into()]);
        let store = TranscriptStore::new(&dir);
        persist_session_state(&store, &state, 0, 0).expect("persist tagged session");

        let output = block_on(TagCommand::new(Some(dir.clone())).execute(
            command_context(&dir, state.session.id),
            CommandInvocation {
                name: "tag".into(),
                args: "bugfix".into(),
                raw: "/tag bugfix".into(),
            },
        ))
        .expect("tag prompt succeeds");

        assert_eq!(
            output,
            CommandOutput::Text(
                "tag_remove_confirmation=bugfix\nstatus=confirm tag removal".into()
            )
        );

        let metadata = store
            .read_metadata(state.session.id)
            .expect("read tagged metadata");
        assert_eq!(metadata.tags, vec!["bugfix"]);
    }
}
