//! Rewinds persisted session transcripts by removing recent exchanges.

use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use clap::Parser;
use time::OffsetDateTime;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec,
    FeatureFlag, MESSAGE_SCHEMA_VERSION, MessageEnvelope, MessagePayload, Result, SessionId,
    WonderError,
};
use wonder_of_u_storage::{FileCheckpointStore, SessionMemoryIndexStore, TranscriptStore};

use super::{parse_command_args, parse_session_id};

/// Rewinds a persisted session transcript.
pub struct RewindCommand {
    storage_dir: Option<PathBuf>,
}

impl RewindCommand {
    /// Creates a new value.
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Builds the shared command spec.
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "rewind",
            "Remove recent user/assistant exchanges from a persisted session",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::SessionPersistence]);
        spec.aliases.push("checkpoint".into());
        spec
    }
}

#[derive(Debug, Parser)]
struct RewindArgs {
    /// Session ID (defaults to most recent).
    #[arg(long)]
    session: Option<String>,
    /// Number of exchanges to remove.
    #[arg(long, short, default_value_t = 1)]
    n: usize,
    /// Skip the confirmation prompt.
    #[arg(long, short)]
    yes: bool,
}

#[async_trait]
impl Command for RewindCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<RewindArgs>("rewind", &invocation)?;
        let storage_dir = require_storage_dir(&self.storage_dir)?;
        Ok(CommandOutput::Text(rewind(
            storage_dir,
            args.session,
            args.n,
            args.yes,
        )?))
    }
}

/// Rewinds a session by removing the last `n` exchanges from its transcript.
pub fn rewind(
    storage_dir: &Path,
    session_id: Option<String>,
    n: usize,
    yes: bool,
) -> Result<String> {
    if n == 0 {
        return Err(WonderError::validation("rewind count must be at least 1"));
    }

    let store = TranscriptStore::new(storage_dir);
    let session_id = resolve_session_id(&store, session_id.as_deref())?;
    let transcript_path = store.paths().transcript_path(session_id);
    let messages = load_raw_transcript(&transcript_path)?;
    let Some(cutoff_idx) = rewind_cutoff_index(&messages, n) else {
        return Ok(format!("Nothing to rewind for session {session_id}."));
    };
    let removed_messages = messages.len().saturating_sub(cutoff_idx);
    let removed_exchanges = exchange_count(&messages).min(n);

    if removed_messages == 0 {
        return Ok(format!("Nothing to rewind for session {session_id}."));
    }

    if !yes && !confirm_rewind(session_id, removed_exchanges, removed_messages)? {
        return Ok(format!("Aborted rewind for session {session_id}."));
    }

    let restored_files =
        FileCheckpointStore::new(storage_dir).restore_after(session_id, cutoff_idx)?;

    write_transcript(&transcript_path, &messages[..cutoff_idx])?;
    update_metadata(&store, session_id, cutoff_idx)?;
    invalidate_snapshot(&store, session_id)?;
    rebuild_session_memory_index(storage_dir, session_id, &messages[..cutoff_idx])?;

    let restored_suffix = match restored_files.len() {
        0 => String::new(),
        1 => "; restored 1 file".to_owned(),
        count => format!("; restored {count} files"),
    };

    Ok(format!(
        "Rewound session {session_id} by {removed_exchanges} exchanges ({removed_messages} messages removed){restored_suffix}"
    ))
}

fn require_storage_dir(storage_dir: &Option<PathBuf>) -> Result<&Path> {
    storage_dir.as_deref().ok_or_else(|| {
        WonderError::validation(
            "this command requires --storage-dir so persisted session data can be loaded",
        )
    })
}

fn resolve_session_id(
    store: &TranscriptStore,
    requested_session_id: Option<&str>,
) -> Result<SessionId> {
    match requested_session_id {
        Some(session_id) => parse_session_id(session_id),
        None => store
            .list_metadata()?
            .into_iter()
            .next()
            .map(|metadata| metadata.session_id)
            .ok_or_else(|| WonderError::not_found("persisted session", "latest")),
    }
}

fn load_raw_transcript(path: &Path) -> Result<Vec<MessageEnvelope>> {
    if !path.exists() {
        return Err(WonderError::not_found(
            "session transcript",
            path.display().to_string(),
        ));
    }

    let contents = fs::read_to_string(path)?;
    let lines: Vec<&str> = contents.lines().collect();
    let mut messages = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<MessageEnvelope>(line) {
            Ok(message) => {
                if message.schema_version != MESSAGE_SCHEMA_VERSION {
                    return Err(WonderError::validation(format!(
                        "unsupported message schema version {}",
                        message.schema_version
                    )));
                }
                messages.push(message);
            }
            Err(_) if index + 1 == lines.len() => break,
            Err(error) => return Err(WonderError::Json(error)),
        }
    }

    Ok(messages)
}

fn rewind_cutoff_index(messages: &[MessageEnvelope], n: usize) -> Option<usize> {
    let user_indices = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| is_user_message(&message.payload).then_some(index))
        .collect::<Vec<_>>();

    if user_indices.is_empty() {
        return None;
    }

    if n >= user_indices.len() {
        return Some(0);
    }

    Some(user_indices[user_indices.len() - n])
}

fn exchange_count(messages: &[MessageEnvelope]) -> usize {
    messages
        .iter()
        .filter(|message| is_user_message(&message.payload))
        .count()
}

fn is_user_message(payload: &MessagePayload) -> bool {
    matches!(
        payload,
        MessagePayload::UserText { .. }
            | MessagePayload::UserAttachment { .. }
            | MessagePayload::UserPasteReference { .. }
    )
}

fn confirm_rewind(session_id: SessionId, exchanges: usize, messages: usize) -> Result<bool> {
    let prompt = format!(
        "Remove last {exchanges} exchanges ({messages} messages) from session {session_id}? [y/N] "
    );
    let mut stdout = io::stdout().lock();
    stdout.write_all(prompt.as_bytes())?;
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(matches!(input.trim().chars().next(), Some('y' | 'Y')))
}

fn write_transcript(path: &Path, messages: &[MessageEnvelope]) -> Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    for message in messages {
        serde_json::to_writer(&mut writer, message)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    writer.get_ref().sync_data()?;
    Ok(())
}

fn update_metadata(
    store: &TranscriptStore,
    session_id: SessionId,
    message_count: usize,
) -> Result<()> {
    let mut metadata = store.read_metadata(session_id)?;
    metadata.message_count = message_count;
    metadata.updated_at = OffsetDateTime::now_utc();
    store.write_metadata(&metadata)
}

fn invalidate_snapshot(store: &TranscriptStore, session_id: SessionId) -> Result<()> {
    match fs::remove_file(store.paths().snapshot_path(session_id)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn rebuild_session_memory_index(
    storage_dir: &Path,
    session_id: SessionId,
    messages: &[MessageEnvelope],
) -> Result<()> {
    SessionMemoryIndexStore::new(storage_dir).rebuild_from_messages(session_id, messages)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use wonder_of_u_core::{AppState, MessageEnvelope, MessagePayload, SessionId};
    use wonder_of_u_storage::{FileCheckpointStore, SessionSnapshot, TranscriptStore};
    use wonder_of_u_test_support::unique_test_dir;

    use super::rewind;

    #[test]
    fn rewind_removes_the_last_exchange() {
        let dir = unique_test_dir("cli-rewind-last-exchange");
        let store = TranscriptStore::new(&dir);
        let session_id = seed_session(
            &store,
            &[
                ("one", true),
                ("reply one", false),
                ("two", true),
                ("reply two", false),
                ("three", true),
                ("reply three", false),
            ],
        );

        let output = rewind(&dir, Some(session_id.to_string()), 1, true).expect("rewind session");

        assert!(output.contains("Rewound session"));
        let restored = store.load_session(session_id).expect("load transcript");
        assert_eq!(restored.messages.len(), 4);
        assert!(matches!(
            &restored.messages[2].payload,
            MessagePayload::UserText { content } if content == "two"
        ));
        assert!(matches!(
            &restored.messages[3].payload,
            MessagePayload::AssistantText { content } if content == "reply two"
        ));
        assert!(!store.paths().snapshot_path(session_id).exists());
        assert_eq!(
            store
                .read_metadata(session_id)
                .expect("read metadata")
                .message_count,
            4
        );
        assert_eq!(
            wonder_of_u_storage::SessionMemoryIndexStore::new(&dir)
                .read(session_id)
                .expect("read memory index")
                .transcript_message_count,
            4
        );
    }

    #[test]
    fn rewind_restores_files_checkpointed_after_cutoff() {
        let dir = unique_test_dir("cli-rewind-restores-files");
        let store = TranscriptStore::new(&dir);
        let session_id = seed_session(
            &store,
            &[
                ("one", true),
                ("reply one", false),
                ("two", true),
                ("reply two", false),
                ("three", true),
                ("reply three", false),
            ],
        );
        let target = dir.join("notes.txt");
        fs::write(&target, "original").expect("seed file");
        FileCheckpointStore::new(&dir)
            .record(session_id, 4, &target)
            .expect("record checkpoint");
        fs::write(&target, "modified").expect("modify file");

        let output = rewind(&dir, Some(session_id.to_string()), 1, true).expect("rewind session");

        assert!(output.contains("restored 1 file"));
        assert_eq!(
            fs::read_to_string(&target).expect("read restored file"),
            "original"
        );
        assert!(
            FileCheckpointStore::new(&dir)
                .entries(session_id)
                .expect("checkpoint entries")
                .is_empty()
        );
    }

    #[test]
    fn rewind_more_exchanges_than_exist_clears_the_transcript() {
        let dir = unique_test_dir("cli-rewind-all");
        let store = TranscriptStore::new(&dir);
        let session_id = seed_session(
            &store,
            &[
                ("one", true),
                ("reply one", false),
                ("two", true),
                ("reply two", false),
                ("three", true),
                ("reply three", false),
            ],
        );

        let output = rewind(&dir, Some(session_id.to_string()), 99, true).expect("rewind session");

        assert!(output.contains("6 messages removed"));
        let restored = store.load_session(session_id).expect("load transcript");
        assert!(restored.messages.is_empty());
        assert_eq!(
            store
                .read_metadata(session_id)
                .expect("read metadata")
                .message_count,
            0
        );
    }

    #[test]
    fn rewind_empty_session_reports_nothing_to_rewind() {
        let dir = unique_test_dir("cli-rewind-empty");
        let store = TranscriptStore::new(&dir);
        let mut state = AppState::new(PathBuf::from("/workspace"));
        let session_id = state.session.id;
        store.ensure_layout().expect("ensure layout");
        fs::write(store.paths().transcript_path(session_id), "").expect("write empty transcript");
        store
            .write_metadata(&wonder_of_u_storage::SessionMetadata::from_app_state(
                &state,
            ))
            .expect("write metadata");
        state.messages.clear();
        store
            .write_snapshot(&SessionSnapshot::from_app_state(&state, 0, 0))
            .expect("write snapshot");

        let output = rewind(&dir, Some(session_id.to_string()), 1, true).expect("rewind session");

        assert_eq!(
            output,
            format!("Nothing to rewind for session {session_id}.")
        );
        assert!(store.paths().snapshot_path(session_id).exists());
    }

    fn seed_session(store: &TranscriptStore, entries: &[(&str, bool)]) -> SessionId {
        let mut state = AppState::new(PathBuf::from("/workspace"));
        let session_id = state.session.id;
        let messages = entries
            .iter()
            .map(|(content, is_user)| message(session_id, content, *is_user))
            .collect::<Vec<_>>();
        for message in &messages {
            store.append_message(message).expect("append transcript");
        }
        state.messages = messages;
        store
            .write_metadata(
                &wonder_of_u_storage::SessionMetadata::from_state_with_transcript(
                    &state,
                    state.messages.len(),
                ),
            )
            .expect("write metadata");
        store
            .write_snapshot(&SessionSnapshot::from_app_state(
                &state,
                state.messages.len(),
                0,
            ))
            .expect("write snapshot");
        session_id
    }

    fn message(session_id: SessionId, content: &str, is_user: bool) -> MessageEnvelope {
        if is_user {
            MessageEnvelope::user_text(session_id, content)
        } else {
            MessageEnvelope::new(
                session_id,
                MessagePayload::AssistantText {
                    content: content.into(),
                },
            )
        }
    }
}
