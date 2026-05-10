use std::{cmp::Reverse, collections::BTreeMap, path::PathBuf};

use async_trait::async_trait;
use clap::Parser;
use time::{OffsetDateTime, format_description::parse};
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec,
    FeatureFlag, MessageEnvelope, MessagePayload, Result, SessionId, WonderError,
};
use wonder_of_u_storage::TranscriptStore;

use super::{parse_command_args, parse_session_id, prompt::truncate_chars};

/// Prints a conversation summary for a persisted session.
pub struct SummaryCommand {
    storage_dir: Option<PathBuf>,
}

impl SummaryCommand {
    /// Creates a new value.
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Builds the command spec.
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "summary",
            "Print a summary of a conversation session",
            CommandKind::Local,
        );
        spec.required_features =
            std::collections::BTreeSet::from([FeatureFlag::SessionPersistence]);
        spec
    }
}

#[derive(Debug, Parser)]
struct SummaryArgs {
    #[arg(long)]
    session: Option<String>,
    #[arg(long, default_value = "markdown")]
    format: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SummaryFormat {
    Markdown,
    Text,
}

#[derive(Debug, Eq, PartialEq)]
struct ConversationSummary {
    session_id: SessionId,
    started: String,
    user_messages: usize,
    assistant_messages: usize,
    tool_uses: Vec<(String, usize)>,
    first_user_message: String,
    last_assistant_message: String,
}

impl ConversationSummary {
    fn total_messages(&self) -> usize {
        self.user_messages + self.assistant_messages
    }
}

#[async_trait]
impl Command for SummaryCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<SummaryArgs>("summary", &invocation)?;
        let store = require_store(&self.storage_dir)?;
        let session_id = resolve_session_id(&store, args.session.as_deref())?;
        let restored = store.restore_session(session_id)?;
        let summary = summarize_session(
            session_id,
            restored.metadata.created_at,
            &restored.transcript.messages,
        )?;
        let rendered = match SummaryFormat::from_flag(&args.format) {
            SummaryFormat::Markdown => render_markdown_summary(&summary),
            SummaryFormat::Text => render_text_summary(&summary),
        };
        Ok(CommandOutput::Text(rendered))
    }
}

impl SummaryFormat {
    fn from_flag(value: &str) -> Self {
        if value.eq_ignore_ascii_case("markdown") {
            Self::Markdown
        } else {
            Self::Text
        }
    }
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

fn resolve_session_id(store: &TranscriptStore, session_id: Option<&str>) -> Result<SessionId> {
    match session_id {
        Some(session_id) => parse_session_id(session_id),
        None => store
            .list_metadata()?
            .into_iter()
            .next()
            .map(|metadata| metadata.session_id)
            .ok_or_else(|| WonderError::not_found("persisted session", "latest")),
    }
}

fn summarize_session(
    session_id: SessionId,
    started: OffsetDateTime,
    messages: &[MessageEnvelope],
) -> Result<ConversationSummary> {
    let format = parse("[year]-[month]-[day] [hour]:[minute]")
        .map_err(|error| WonderError::internal(format!("invalid summary time format: {error}")))?;
    let started = started.format(&format).map_err(|error| {
        WonderError::internal(format!("failed to format summary time: {error}"))
    })?;
    let mut user_messages = 0;
    let mut assistant_messages = 0;
    let mut tool_uses = BTreeMap::new();
    let mut first_user_message = None;
    let mut last_assistant_message = None;

    for message in messages {
        match &message.payload {
            MessagePayload::UserText { content } => {
                user_messages += 1;
                first_user_message.get_or_insert_with(|| excerpt(content));
            }
            MessagePayload::UserAttachment { .. } | MessagePayload::UserPasteReference { .. } => {
                user_messages += 1;
            }
            MessagePayload::AssistantText { content } => {
                assistant_messages += 1;
                last_assistant_message = Some(excerpt(content));
            }
            MessagePayload::AssistantThinking { .. }
            | MessagePayload::ToolResult { .. }
            | MessagePayload::BashOutput { .. }
            | MessagePayload::Progress { .. }
            | MessagePayload::Command { .. }
            | MessagePayload::HookResult { .. }
            | MessagePayload::HookProgress { .. }
            | MessagePayload::CompactBoundary { .. }
            | MessagePayload::Task { .. }
            | MessagePayload::Permission { .. }
            | MessagePayload::PlanApproval { .. }
            | MessagePayload::ProviderError { .. } => {
                assistant_messages += 1;
            }
            MessagePayload::AssistantToolUse { tool, .. } => {
                assistant_messages += 1;
                *tool_uses.entry(tool.clone()).or_insert(0) += 1;
            }
            MessagePayload::System { .. } => {}
        }
    }

    let mut tool_uses = tool_uses.into_iter().collect::<Vec<_>>();
    tool_uses.sort_by(|left, right| {
        Reverse(left.1)
            .cmp(&Reverse(right.1))
            .then_with(|| left.0.cmp(&right.0))
    });

    Ok(ConversationSummary {
        session_id,
        started,
        user_messages,
        assistant_messages,
        tool_uses,
        first_user_message: first_user_message.unwrap_or_else(|| "n/a".into()),
        last_assistant_message: last_assistant_message.unwrap_or_else(|| "n/a".into()),
    })
}

fn excerpt(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        "n/a".into()
    } else {
        truncate_chars(&normalized, 200)
    }
}

fn render_tools_line(tool_uses: &[(String, usize)]) -> String {
    if tool_uses.is_empty() {
        "none".into()
    } else {
        tool_uses
            .iter()
            .map(|(tool, count)| format!("{tool} ({count}x)"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn render_markdown_summary(summary: &ConversationSummary) -> String {
    format!(
        concat!(
            "# Session Summary\n\n",
            "**Session**: {}\n",
            "**Started**: {}\n",
            "**Messages**: {} ({} user, {} assistant)\n",
            "**Tools used**: {}\n\n",
            "## Conversation\n\n",
            "**First user**: {}\n",
            "**Last assistant**: {}"
        ),
        summary.session_id,
        summary.started,
        summary.total_messages(),
        summary.user_messages,
        summary.assistant_messages,
        render_tools_line(&summary.tool_uses),
        summary.first_user_message,
        summary.last_assistant_message,
    )
}

fn render_text_summary(summary: &ConversationSummary) -> String {
    format!(
        concat!(
            "Session Summary\n",
            "Session: {}\n",
            "Started: {}\n",
            "Messages: {} ({} user, {} assistant)\n",
            "Tools used: {}\n\n",
            "Conversation\n",
            "First user: {}\n",
            "Last assistant: {}"
        ),
        summary.session_id,
        summary.started,
        summary.total_messages(),
        summary.user_messages,
        summary.assistant_messages,
        render_tools_line(&summary.tool_uses),
        summary.first_user_message,
        summary.last_assistant_message,
    )
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use serde_json::json;
    use time::OffsetDateTime;
    use wonder_of_u_core::{AppState, CommandContext, FeatureSet, PermissionMode, ToolUseId};
    use wonder_of_u_storage::SessionMetadata;
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn command_context(cwd: &std::path::Path, session_id: SessionId) -> CommandContext {
        CommandContext {
            session_id,
            cwd: cwd.to_path_buf(),
            features: FeatureSet::first_release(),
            authenticated: false,
            interactive: false,
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

    fn persist_transcript(
        dir: &std::path::Path,
        session_id: SessionId,
        created_at: OffsetDateTime,
        messages: Vec<MessageEnvelope>,
    ) {
        let store = TranscriptStore::new(dir);
        let mut state = AppState::new(dir.to_path_buf());
        state.session.id = session_id;
        state.session.created_at = created_at;
        state.session.updated_at = created_at;

        for message in messages {
            store.append_message(&message).expect("append message");
            state.push_message(message).expect("push message");
        }

        let metadata = SessionMetadata::from_state_with_transcript(&state, state.messages.len());
        store.write_metadata(&metadata).expect("write metadata");
    }

    #[test]
    fn summary_command_renders_counts_for_two_message_transcript() {
        let dir = unique_test_dir("summary-command-counts");
        let created_at = OffsetDateTime::parse(
            "2026-05-10T14:00:00Z",
            &time::format_description::well_known::Rfc3339,
        )
        .expect("parse timestamp");
        let session_id = SessionId::new();

        let mut user = MessageEnvelope::new(
            session_id,
            MessagePayload::UserText {
                content: "Hello from the user.".into(),
            },
        );
        user.timestamp = created_at;

        let mut assistant = MessageEnvelope::new(
            session_id,
            MessagePayload::AssistantText {
                content: "Hello from the assistant.".into(),
            },
        );
        assistant.timestamp = created_at + time::Duration::minutes(1);

        persist_transcript(&dir, session_id, created_at, vec![user, assistant]);

        let output = block_on(SummaryCommand::new(Some(dir.clone())).execute(
            command_context(&dir, session_id),
            CommandInvocation {
                name: "summary".into(),
                args: format!("--session {session_id}"),
                raw: format!("/summary --session {session_id}"),
            },
        ))
        .expect("summary succeeds");

        let CommandOutput::Text(text) = output else {
            panic!("summary must render text output");
        };
        assert!(text.contains("# Session Summary"));
        assert!(text.contains(&format!("**Session**: {session_id}")));
        assert!(text.contains("**Started**: 2026-05-10 14:00"));
        assert!(text.contains("**Messages**: 2 (1 user, 1 assistant)"));
        assert!(text.contains("**Tools used**: none"));
        assert!(text.contains("**First user**: Hello from the user."));
        assert!(text.contains("**Last assistant**: Hello from the assistant."));
    }

    #[test]
    fn summary_command_defaults_unknown_format_to_text() {
        let dir = unique_test_dir("summary-command-text-default");
        let created_at = OffsetDateTime::parse(
            "2026-05-10T14:00:00Z",
            &time::format_description::well_known::Rfc3339,
        )
        .expect("parse timestamp");
        let session_id = SessionId::new();

        let mut user = MessageEnvelope::new(
            session_id,
            MessagePayload::UserText {
                content: "First line".into(),
            },
        );
        user.timestamp = created_at;

        let mut assistant = MessageEnvelope::new(
            session_id,
            MessagePayload::AssistantText {
                content: "Second line".into(),
            },
        );
        assistant.timestamp = created_at + time::Duration::minutes(1);

        persist_transcript(&dir, session_id, created_at, vec![user, assistant]);

        let output = block_on(SummaryCommand::new(Some(dir.clone())).execute(
            command_context(&dir, session_id),
            CommandInvocation {
                name: "summary".into(),
                args: "--format html".into(),
                raw: "/summary --format html".into(),
            },
        ))
        .expect("summary succeeds");

        let CommandOutput::Text(text) = output else {
            panic!("summary must render text output");
        };
        assert!(text.starts_with("Session Summary"));
        assert!(!text.contains("# Session Summary"));
        assert!(text.contains("Session:"));
    }

    #[test]
    fn summary_command_counts_tool_use_frequencies() {
        let dir = unique_test_dir("summary-command-tools");
        let created_at = OffsetDateTime::parse(
            "2026-05-10T14:00:00Z",
            &time::format_description::well_known::Rfc3339,
        )
        .expect("parse timestamp");
        let session_id = SessionId::new();

        let mut user = MessageEnvelope::new(
            session_id,
            MessagePayload::UserText {
                content: "Check the repo".into(),
            },
        );
        user.timestamp = created_at;

        let mut bash_first = MessageEnvelope::new(
            session_id,
            MessagePayload::AssistantToolUse {
                tool: "bash".into(),
                use_id: ToolUseId::new(),
                input: json!({ "command": "pwd" }),
            },
        );
        bash_first.timestamp = created_at + time::Duration::minutes(1);

        let mut file_read = MessageEnvelope::new(
            session_id,
            MessagePayload::AssistantToolUse {
                tool: "file_read".into(),
                use_id: ToolUseId::new(),
                input: json!({ "path": "Cargo.toml" }),
            },
        );
        file_read.timestamp = created_at + time::Duration::minutes(2);

        let mut bash_second = MessageEnvelope::new(
            session_id,
            MessagePayload::AssistantToolUse {
                tool: "bash".into(),
                use_id: ToolUseId::new(),
                input: json!({ "command": "git status" }),
            },
        );
        bash_second.timestamp = created_at + time::Duration::minutes(3);

        let mut assistant = MessageEnvelope::new(
            session_id,
            MessagePayload::AssistantText {
                content: "Done.".into(),
            },
        );
        assistant.timestamp = created_at + time::Duration::minutes(4);

        persist_transcript(
            &dir,
            session_id,
            created_at,
            vec![user, bash_first, file_read, bash_second, assistant],
        );

        let output = block_on(SummaryCommand::new(Some(dir.clone())).execute(
            command_context(&dir, session_id),
            CommandInvocation {
                name: "summary".into(),
                args: String::new(),
                raw: "/summary".into(),
            },
        ))
        .expect("summary succeeds");

        let CommandOutput::Text(text) = output else {
            panic!("summary must render text output");
        };
        assert!(text.contains("**Tools used**: bash (2x), file_read (1x)"));
        assert!(text.contains("**Messages**: 5 (1 user, 4 assistant)"));
    }
}
