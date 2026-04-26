//! Append-only session storage primitives.

use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use wonder_of_u_core::{AppState, MessageEnvelope, Result, SessionId, WonderError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoragePaths {
    base_dir: PathBuf,
}

impl StoragePaths {
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    #[must_use]
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    #[must_use]
    pub fn sessions_dir(&self) -> PathBuf {
        self.base_dir.join("sessions")
    }

    #[must_use]
    pub fn transcript_path(&self, session_id: SessionId) -> PathBuf {
        self.sessions_dir().join(format!("{session_id}.jsonl"))
    }

    #[must_use]
    pub fn metadata_dir(&self) -> PathBuf {
        self.sessions_dir().join(".metadata")
    }

    #[must_use]
    pub fn metadata_path(&self, session_id: SessionId) -> PathBuf {
        self.metadata_dir().join(format!("{session_id}.json"))
    }
}

#[derive(Clone, Debug)]
pub struct TranscriptStore {
    paths: StoragePaths,
}

impl TranscriptStore {
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
        }
    }

    #[must_use]
    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.paths.sessions_dir())?;
        fs::create_dir_all(self.paths.metadata_dir())?;
        Ok(())
    }

    pub fn append_message(&self, message: &MessageEnvelope) -> Result<()> {
        self.ensure_layout()?;
        let path = self.paths.transcript_path(message.session_id);
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, message)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        writer.get_ref().sync_data()?;
        Ok(())
    }

    pub fn load_session(&self, session_id: SessionId) -> Result<LoadedTranscript> {
        let path = self.paths.transcript_path(session_id);
        if !path.exists() {
            return Err(WonderError::not_found(
                "session transcript",
                session_id.to_string(),
            ));
        }

        let contents = fs::read_to_string(path)?;
        let lines: Vec<&str> = contents.lines().collect();
        let mut messages = Vec::new();
        let mut warnings = Vec::new();

        for (index, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<MessageEnvelope>(line) {
                Ok(message) => messages.push(message),
                Err(error) if index + 1 == lines.len() => {
                    warnings.push(TranscriptWarning {
                        line: index + 1,
                        message: format!("ignored corrupt trailing transcript line: {error}"),
                    });
                    break;
                }
                Err(error) => return Err(WonderError::Json(error)),
            }
        }

        Ok(LoadedTranscript { messages, warnings })
    }

    pub fn write_metadata(&self, metadata: &SessionMetadata) -> Result<()> {
        self.ensure_layout()?;
        let path = self.paths.metadata_path(metadata.session_id);
        let pending_path = path.with_extension("json.next");

        {
            let file = File::create(&pending_path)?;
            let mut writer = BufWriter::new(file);
            serde_json::to_writer_pretty(&mut writer, metadata)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
            writer.get_ref().sync_all()?;
        }

        fs::rename(pending_path, path)?;
        Ok(())
    }

    pub fn read_metadata(&self, session_id: SessionId) -> Result<SessionMetadata> {
        let path = self.paths.metadata_path(session_id);
        if !path.exists() {
            return Err(WonderError::not_found(
                "session metadata",
                session_id.to_string(),
            ));
        }
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LoadedTranscript {
    pub messages: Vec<MessageEnvelope>,
    pub warnings: Vec<TranscriptWarning>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptWarning {
    pub line: usize,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub session_id: SessionId,
    pub title: String,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub message_count: usize,
}

impl SessionMetadata {
    #[must_use]
    pub fn from_app_state(state: &AppState) -> Self {
        Self {
            session_id: state.session.id,
            title: state.session.title.clone(),
            cwd: state.session.cwd.clone(),
            git_branch: state.session.git_branch.clone(),
            created_at: state.session.created_at,
            updated_at: state.session.updated_at,
            message_count: state.messages.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs::OpenOptions, io::Write};

    use wonder_of_u_core::{AppState, MessageEnvelope};
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    #[test]
    fn appends_and_loads_jsonl_messages() {
        let dir = unique_test_dir("storage-jsonl");
        let store = TranscriptStore::new(dir);
        let session_id = SessionId::new();
        let first = MessageEnvelope::user_text(session_id, "hello");
        let second = MessageEnvelope::system(session_id, "ready");

        store.append_message(&first).expect("append first");
        store.append_message(&second).expect("append second");
        let loaded = store.load_session(session_id).expect("load session");

        assert_eq!(loaded.messages, vec![first, second]);
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn load_tolerates_corrupt_trailing_line() {
        let dir = unique_test_dir("storage-corrupt-tail");
        let store = TranscriptStore::new(&dir);
        let session_id = SessionId::new();
        let message = MessageEnvelope::user_text(session_id, "hello");
        store.append_message(&message).expect("append");

        let mut file = OpenOptions::new()
            .append(true)
            .open(store.paths().transcript_path(session_id))
            .expect("open transcript");
        file.write_all(b"{not-json")
            .expect("write corrupt trailing line");

        let loaded = store.load_session(session_id).expect("load session");

        assert_eq!(loaded.messages, vec![message]);
        assert_eq!(loaded.warnings.len(), 1);
    }

    #[test]
    fn writes_and_reads_session_metadata() {
        let dir = unique_test_dir("storage-metadata");
        let store = TranscriptStore::new(dir);
        let mut state = AppState::new(PathBuf::from("/workspace"));
        state
            .push_message(MessageEnvelope::system(state.session.id, "ready"))
            .expect("push");
        let metadata = SessionMetadata::from_app_state(&state);

        store.write_metadata(&metadata).expect("write metadata");
        let loaded = store
            .read_metadata(state.session.id)
            .expect("read metadata");

        assert_eq!(loaded, metadata);
    }
}
