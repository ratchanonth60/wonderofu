//! Pre-modification file checkpoints used by `/rewind` to restore edited files.
//!
//! Layout (under the storage base dir):
//!
//! ```text
//! sessions/.checkpoints/{session_id}/index.jsonl   — append-only entry log
//! sessions/.checkpoints/{session_id}/blobs/{id}    — original file contents
//! ```
//!
//! Each entry records the on-disk state of a file (including "absent") just
//! before a tool mutated it, tagged with the persisted transcript message
//! count at capture time.  Rewinding to message index `n` restores, for every
//! touched path, the earliest checkpoint captured at or after `n`.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
use wonder_of_u_core::{FileCheckpointer, Result, SessionId, WonderError};

use crate::{
    STORAGE_SCHEMA_VERSION, StoragePaths, TranscriptStore, default_storage_schema_version,
};

/// One captured pre-modification file state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileCheckpointEntry {
    /// Stores the schema version.
    #[serde(default = "default_storage_schema_version")]
    pub schema_version: u16,
    /// Unique id; also the blob file name when `existed` is true.
    pub checkpoint_id: Uuid,
    /// Session the checkpoint belongs to.
    pub session_id: SessionId,
    /// Persisted transcript message count when the checkpoint was captured.
    pub transcript_message_index: usize,
    /// Absolute path of the checkpointed file.
    pub path: PathBuf,
    /// Whether the file existed at capture time (`false` → restore deletes it).
    pub existed: bool,
    /// Capture timestamp.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// A file restored by [`FileCheckpointStore::restore_after`].
#[derive(Clone, Debug, PartialEq)]
pub struct RestoredFile {
    /// Absolute path that was restored.
    pub path: PathBuf,
    /// `true` when the file was deleted because it did not exist at capture time.
    pub deleted: bool,
}

/// Stores and restores pre-modification file checkpoints for a session.
#[derive(Clone, Debug)]
pub struct FileCheckpointStore {
    paths: StoragePaths,
}

impl FileCheckpointStore {
    /// Creates a store rooted at the storage base directory.
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
        }
    }

    fn session_dir(&self, session_id: SessionId) -> PathBuf {
        self.paths
            .sessions_dir()
            .join(".checkpoints")
            .join(session_id.to_string())
    }

    fn index_path(&self, session_id: SessionId) -> PathBuf {
        self.session_dir(session_id).join("index.jsonl")
    }

    fn blob_path(&self, session_id: SessionId, checkpoint_id: Uuid) -> PathBuf {
        self.session_dir(session_id)
            .join("blobs")
            .join(checkpoint_id.to_string())
    }

    /// Records the current on-disk state of `path` before a mutation.
    ///
    /// Repeated mutations of the same file within the same transcript position
    /// are deduplicated: only the first capture (the state the user would want
    /// back) is kept.
    pub fn record(
        &self,
        session_id: SessionId,
        transcript_message_index: usize,
        path: &Path,
    ) -> Result<()> {
        let path = normalize_path(path);
        if self.entries(session_id)?.iter().any(|entry| {
            entry.transcript_message_index == transcript_message_index && entry.path == path
        }) {
            return Ok(());
        }

        let checkpoint_id = Uuid::new_v4();
        let existed = path.exists();
        fs::create_dir_all(self.session_dir(session_id).join("blobs"))?;
        if existed {
            let bytes = fs::read(&path)?;
            let blob_path = self.blob_path(session_id, checkpoint_id);
            let tmp_path = blob_path.with_extension("next");
            fs::write(&tmp_path, &bytes)?;
            fs::rename(&tmp_path, &blob_path)?;
        }

        let entry = FileCheckpointEntry {
            schema_version: STORAGE_SCHEMA_VERSION,
            checkpoint_id,
            session_id,
            transcript_message_index,
            path,
            existed,
            created_at: OffsetDateTime::now_utc(),
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.index_path(session_id))?;
        serde_json::to_writer(&mut file, &entry)?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        Ok(())
    }

    /// Loads all checkpoint entries for a session in capture order.
    ///
    /// A corrupt trailing line (interrupted write) is tolerated and skipped,
    /// matching the transcript loader's recovery behaviour.
    pub fn entries(&self, session_id: SessionId) -> Result<Vec<FileCheckpointEntry>> {
        let index_path = self.index_path(session_id);
        if !index_path.exists() {
            return Ok(Vec::new());
        }
        let contents = fs::read_to_string(&index_path)?;
        let lines: Vec<&str> = contents.lines().collect();
        let mut entries = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<FileCheckpointEntry>(line) {
                Ok(entry) => {
                    if entry.schema_version != STORAGE_SCHEMA_VERSION {
                        return Err(WonderError::validation(format!(
                            "unsupported file checkpoint schema version {}",
                            entry.schema_version
                        )));
                    }
                    entries.push(entry);
                }
                Err(_) if index + 1 == lines.len() => break,
                Err(error) => return Err(WonderError::Json(error)),
            }
        }
        Ok(entries)
    }

    /// Restores every file touched at or after `cutoff_index` to its earliest
    /// checkpointed state, then drops the consumed checkpoints.
    ///
    /// Returns the restored files. Paths whose blob is missing are skipped.
    pub fn restore_after(
        &self,
        session_id: SessionId,
        cutoff_index: usize,
    ) -> Result<Vec<RestoredFile>> {
        let entries = self.entries(session_id)?;
        let (consumed, kept): (Vec<_>, Vec<_>) = entries
            .into_iter()
            .partition(|entry| entry.transcript_message_index >= cutoff_index);

        // Earliest entry per path = the file's state when the rewound-away
        // portion of the conversation began.
        let mut earliest: BTreeMap<PathBuf, &FileCheckpointEntry> = BTreeMap::new();
        for entry in &consumed {
            earliest.entry(entry.path.clone()).or_insert(entry);
        }

        let mut restored = Vec::new();
        for (path, entry) in earliest {
            if entry.existed {
                let blob_path = self.blob_path(session_id, entry.checkpoint_id);
                let Ok(bytes) = fs::read(&blob_path) else {
                    continue;
                };
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, bytes)?;
                restored.push(RestoredFile {
                    path,
                    deleted: false,
                });
            } else {
                match fs::remove_file(&path) {
                    Ok(()) => restored.push(RestoredFile {
                        path,
                        deleted: true,
                    }),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }

        // Rewrite the index without the consumed entries and drop their blobs.
        let index_path = self.index_path(session_id);
        if index_path.exists() {
            let tmp_path = index_path.with_extension("next");
            {
                let mut file = fs::File::create(&tmp_path)?;
                for entry in &kept {
                    serde_json::to_writer(&mut file, entry)?;
                    file.write_all(b"\n")?;
                }
                file.sync_data()?;
            }
            fs::rename(&tmp_path, &index_path)?;
        }
        for entry in &consumed {
            if entry.existed {
                let _ = fs::remove_file(self.blob_path(session_id, entry.checkpoint_id));
            }
        }

        Ok(restored)
    }
}

/// [`FileCheckpointer`] implementation bound to one persisted session.
///
/// Captures the persisted transcript message count at checkpoint time so
/// rewind can correlate file states with conversation positions.
#[derive(Clone, Debug)]
pub struct SessionFileCheckpointer {
    base_dir: PathBuf,
    session_id: SessionId,
}

impl SessionFileCheckpointer {
    /// Creates a checkpointer for `session_id` rooted at the storage dir.
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>, session_id: SessionId) -> Self {
        Self {
            base_dir: base_dir.into(),
            session_id,
        }
    }
}

impl FileCheckpointer for SessionFileCheckpointer {
    fn checkpoint_file(&self, path: &Path) -> Result<()> {
        let message_index = TranscriptStore::new(&self.base_dir)
            .read_metadata(self.session_id)
            .map(|metadata| metadata.message_count)
            .unwrap_or(0);
        FileCheckpointStore::new(&self.base_dir).record(self.session_id, message_index, path)
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use wonder_of_u_core::AppState;
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn session() -> SessionId {
        SessionId::new()
    }

    #[test]
    fn record_and_restore_roundtrip_restores_original_content() {
        let dir = unique_test_dir("checkpoint-roundtrip");
        let store = FileCheckpointStore::new(&dir);
        let session_id = session();
        let target = dir.join("note.txt");
        fs::write(&target, "original").expect("seed file");

        store
            .record(session_id, 2, &target)
            .expect("record checkpoint");
        fs::write(&target, "modified").expect("modify file");

        let restored = store
            .restore_after(session_id, 2)
            .expect("restore checkpoints");

        assert_eq!(restored.len(), 1);
        assert!(!restored[0].deleted);
        assert_eq!(
            fs::read_to_string(&target).expect("read restored"),
            "original"
        );
        assert!(
            store.entries(session_id).expect("entries").is_empty(),
            "consumed checkpoints must be dropped"
        );
    }

    #[test]
    fn restore_deletes_files_created_after_cutoff() {
        let dir = unique_test_dir("checkpoint-delete-created");
        let store = FileCheckpointStore::new(&dir);
        let session_id = session();
        let target = dir.join("created.txt");

        store
            .record(session_id, 4, &target)
            .expect("record absent state");
        fs::write(&target, "new file").expect("create file");

        let restored = store
            .restore_after(session_id, 0)
            .expect("restore checkpoints");

        assert_eq!(restored.len(), 1);
        assert!(restored[0].deleted);
        assert!(!target.exists(), "created file must be removed");
    }

    #[test]
    fn restore_uses_earliest_checkpoint_after_cutoff() {
        let dir = unique_test_dir("checkpoint-earliest");
        let store = FileCheckpointStore::new(&dir);
        let session_id = session();
        let target = dir.join("multi.txt");
        fs::write(&target, "v1").expect("seed file");

        store.record(session_id, 2, &target).expect("record v1");
        fs::write(&target, "v2").expect("write v2");
        store.record(session_id, 4, &target).expect("record v2");
        fs::write(&target, "v3").expect("write v3");

        let restored = store
            .restore_after(session_id, 2)
            .expect("restore checkpoints");

        assert_eq!(restored.len(), 1);
        assert_eq!(fs::read_to_string(&target).expect("read"), "v1");
    }

    #[test]
    fn restore_keeps_checkpoints_before_cutoff() {
        let dir = unique_test_dir("checkpoint-keep-earlier");
        let store = FileCheckpointStore::new(&dir);
        let session_id = session();
        let early = dir.join("early.txt");
        let late = dir.join("late.txt");
        fs::write(&early, "early original").expect("seed early");
        fs::write(&late, "late original").expect("seed late");

        store.record(session_id, 1, &early).expect("record early");
        fs::write(&early, "early modified").expect("modify early");
        store.record(session_id, 5, &late).expect("record late");
        fs::write(&late, "late modified").expect("modify late");

        let restored = store
            .restore_after(session_id, 3)
            .expect("restore checkpoints");

        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].path, late.canonicalize().expect("canonical"));
        assert_eq!(
            fs::read_to_string(&early).expect("read early"),
            "early modified",
            "checkpoints before the cutoff must stay untouched"
        );
        assert_eq!(
            fs::read_to_string(&late).expect("read late"),
            "late original"
        );
        assert_eq!(
            store.entries(session_id).expect("entries").len(),
            1,
            "entry before the cutoff must be kept"
        );
    }

    #[test]
    fn record_deduplicates_same_path_at_same_message_index() {
        let dir = unique_test_dir("checkpoint-dedup");
        let store = FileCheckpointStore::new(&dir);
        let session_id = session();
        let target = dir.join("dedup.txt");
        fs::write(&target, "original").expect("seed file");

        store.record(session_id, 3, &target).expect("first record");
        fs::write(&target, "changed").expect("modify");
        store.record(session_id, 3, &target).expect("second record");

        let entries = store.entries(session_id).expect("entries");
        assert_eq!(entries.len(), 1, "duplicate capture must be skipped");
    }

    #[test]
    fn session_checkpointer_uses_persisted_message_count() {
        let dir = unique_test_dir("checkpoint-session-checkpointer");
        let mut state = AppState::new(std::path::PathBuf::from("/workspace"));
        let session_id = state.session.id;
        state.session.title = "checkpoint session".into();
        TranscriptStore::new(&dir)
            .write_metadata(&crate::SessionMetadata::from_state_with_transcript(
                &state, 7,
            ))
            .expect("write metadata");
        let target = dir.join("tracked.txt");
        fs::write(&target, "original").expect("seed file");

        SessionFileCheckpointer::new(&dir, session_id)
            .checkpoint_file(&target)
            .expect("checkpoint file");

        let entries = FileCheckpointStore::new(&dir)
            .entries(session_id)
            .expect("entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].transcript_message_index, 7);
    }

    #[test]
    fn entries_tolerate_corrupt_trailing_line() {
        let dir = unique_test_dir("checkpoint-corrupt-tail");
        let store = FileCheckpointStore::new(&dir);
        let session_id = session();
        let target = dir.join("file.txt");
        fs::write(&target, "data").expect("seed file");
        store.record(session_id, 1, &target).expect("record");

        let index_path = dir
            .join("sessions")
            .join(".checkpoints")
            .join(session_id.to_string())
            .join("index.jsonl");
        let mut contents = fs::read_to_string(&index_path).expect("read index");
        contents.push_str("{\"truncated");
        fs::write(&index_path, contents).expect("append corrupt line");

        assert_eq!(store.entries(session_id).expect("entries").len(), 1);
    }
}
