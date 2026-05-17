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
        )
        .with_argument_hint("[conversation id or search term]");
        spec.required_features = BTreeSet::from([FeatureFlag::SessionPersistence]);
        // Upstream alias: /continue resolves to /resume.
        spec.aliases = vec!["continue".into()];
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
        // Upstream aliases: /reset and /new both resolve to /clear.
        spec.aliases = vec!["reset".into(), "new".into()];
        // immediate=true: /clear must not drain queued prompts after executing.
        spec.immediate = true;
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
            "Clear conversation history but keep a summary in context. Optional: /compact --instructions <hint for summarization>",
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
    /// Optional hint passed to the summarization step, e.g.
    /// `--instructions "focus on tool calls only"`.
    #[arg(long)]
    instructions: Option<String>,
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
            args.instructions.as_deref(),
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
        persist_view_state(
            &self.storage_dir,
            requested_session_id,
            keep_last,
            action,
            None,
        )
    }
}

impl CompactCommand {
    fn persist_view_state(
        &self,
        requested_session_id: Option<&str>,
        keep_last: usize,
        action: ViewAction,
        custom_instructions: Option<&str>,
    ) -> Result<CommandOutput> {
        persist_view_state(
            &self.storage_dir,
            requested_session_id,
            keep_last,
            action,
            custom_instructions,
        )
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
    custom_instructions: Option<&str>,
) -> Result<CommandOutput> {
    let (store, session_id) = resolve_session_target(storage_dir, requested_session_id)?;
    let restored = store.restore_session(session_id)?;
    let compacted_messages = restored.transcript.messages.len().saturating_sub(keep_last);
    let mut state = restored.state;
    state.messages = compacted_view_messages(
        &restored.transcript,
        session_id,
        keep_last,
        action,
        &state,
        custom_instructions,
    );
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
    if let Some(instructions) = custom_instructions {
        lines.push(format!("custom_instructions={instructions}"));
    }
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
    custom_instructions: Option<&str>,
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
                    summary: summarized_messages_summary(summarized, action, custom_instructions),
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

fn summarized_messages_summary(
    messages: &[MessageEnvelope],
    action: ViewAction,
    custom_instructions: Option<&str>,
) -> String {
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
        ViewAction::Clear => {
            let mut s = format!(
                "Cleared the visible transcript and summarized {count} messages ({first_timestamp} -> {last_timestamp}; {counts}). Last summarized message: {last_message}"
            );
            if let Some(instructions) = custom_instructions {
                s.push_str(&format!("\nSummarization instructions: {instructions}"));
            }
            s
        }
        ViewAction::Compact => {
            let mut s = format!(
                "Compacted {count} earlier messages ({first_timestamp} -> {last_timestamp}; {counts}). Last summarized message: {last_message}"
            );
            if let Some(instructions) = custom_instructions {
                s.push_str(&format!("\nSummarization instructions: {instructions}"));
            }
            s
        }
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{CompactCommand, ResumeCommand, ViewAction, summarized_messages_summary};
    use wonder_of_u_core::CommandSpec;

    // ── CompactCommand spec ──────────────────────────────────────────────────

    #[test]
    fn compact_spec_name_is_compact() {
        let spec: CommandSpec = CompactCommand::command_spec();
        assert_eq!(spec.name, "compact");
    }

    #[test]
    fn compact_spec_description_mentions_instructions() {
        let spec = CompactCommand::command_spec();
        assert!(
            spec.description.contains("instructions") || spec.description.contains("hint"),
            "compact description should mention custom instructions; got: {}",
            spec.description
        );
    }

    #[test]
    fn compact_spec_description_mentions_compact_or_history() {
        let spec = CompactCommand::command_spec();
        let desc = spec.description.to_lowercase();
        assert!(
            desc.contains("compact") || desc.contains("history"),
            "compact description should mention compact/history; got: {}",
            spec.description
        );
    }

    // ── summarized_messages_summary ─────────────────────────────────────────

    #[test]
    fn summarized_messages_summary_without_instructions() {
        let summary = summarized_messages_summary(&[], ViewAction::Compact, None);
        assert!(
            summary.contains("Compacted"),
            "summary without instructions must use Compact wording; got: {summary}"
        );
        assert!(
            !summary.contains("Summarization instructions"),
            "summary without instructions must not include instructions line; got: {summary}"
        );
    }

    #[test]
    fn summarized_messages_summary_with_instructions_includes_hint() {
        let summary =
            summarized_messages_summary(&[], ViewAction::Compact, Some("focus on tool calls only"));
        assert!(
            summary.contains("Summarization instructions: focus on tool calls only"),
            "summary with instructions must include them; got: {summary}"
        );
    }

    #[test]
    fn summarized_messages_summary_clear_action_without_instructions() {
        let summary = summarized_messages_summary(&[], ViewAction::Clear, None);
        assert!(
            summary.contains("Cleared"),
            "clear summary must use Cleared wording; got: {summary}"
        );
        assert!(
            !summary.contains("Summarization instructions"),
            "clear without instructions must not include instructions line; got: {summary}"
        );
    }

    #[test]
    fn summarized_messages_summary_clear_action_with_instructions() {
        let summary =
            summarized_messages_summary(&[], ViewAction::Clear, Some("remove tool results"));
        assert!(
            summary.contains("Summarization instructions: remove tool results"),
            "clear with instructions must include them; got: {summary}"
        );
    }

    #[test]
    fn resume_command_spec_carries_argument_hint() {
        let spec = ResumeCommand::command_spec();
        assert_eq!(
            spec.argument_hint.as_deref(),
            Some("[conversation id or search term]"),
            "/resume spec should carry the argument hint"
        );
    }
}
