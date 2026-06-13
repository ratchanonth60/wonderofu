//! Cross-session thread management store.
//!
//! Threads group related conversation sessions, enabling search, resumption,
//! and cross-session context. Inspired by codex-rs `thread-store` crate.
//!
//! # Layout
//!
//! ```text
//! {base_dir}/threads/
//!   index.json           ← Thread index for fast listing/search
//!   .queue/              ← Pending thread creation/update queue
//!   {thread_id}.json     ← Per-thread metadata
//! ```

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    io::{BufWriter, Write},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use wonder_of_u_core::{Result, SessionId, WonderError};

use crate::StoragePaths;

/// Schema version for thread metadata.
pub const THREAD_SCHEMA_VERSION: u16 = 1;

/// Lifecycle status of a thread.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStatus {
    /// Thread is actively being used.
    #[default]
    Active,
    /// Thread is archived (read-only).
    Archived,
    /// Thread has been deleted (soft-delete, recoverable).
    Deleted,
}

impl std::fmt::Display for ThreadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Archived => write!(f, "archived"),
            Self::Deleted => write!(f, "deleted"),
        }
    }
}

/// A cross-session thread grouping related conversations.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    /// Schema version for forward compatibility.
    #[serde(default = "default_thread_schema_version")]
    pub schema_version: u16,
    /// Unique thread identifier.
    pub id: String,
    /// Human-readable title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// When the thread was created.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When the thread was last updated.
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// Session IDs belonging to this thread.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_ids: Vec<SessionId>,
    /// User-defined tags for filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Current lifecycle status.
    #[serde(default)]
    pub status: ThreadStatus,
    /// AI-generated summary of the thread's conversations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Git branch this thread is associated with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    /// Working directory where conversations happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
}

fn default_thread_schema_version() -> u16 {
    THREAD_SCHEMA_VERSION
}

impl Thread {
    /// Create a new thread.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            schema_version: THREAD_SCHEMA_VERSION,
            id: id.into(),
            title: None,
            created_at: now,
            updated_at: now,
            session_ids: Vec::new(),
            tags: Vec::new(),
            status: ThreadStatus::Active,
            summary: None,
            git_branch: None,
            cwd: None,
        }
    }
}

/// Represents objects related to indexed thread
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct ThreadIndexEntry {
    id: String,
    title: Option<String>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    session_count: usize,
    status: ThreadStatus,
    tags: Vec<String>,
    git_branch: Option<String>,
}

/// Index of all threads for fast listing/search.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct ThreadIndex {
    #[serde(default)]
    threads: BTreeMap<String, ThreadIndexEntry>,
}

/// Persistent store for cross-session threads.
#[derive(Clone, Debug)]
pub struct ThreadStore {
    paths: StoragePaths,
}

impl ThreadStore {
    /// Create a new thread store.
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
        }
    }

    /// Returns the threads directory.
    #[must_use]
    pub fn threads_dir(&self) -> PathBuf {
        self.paths.base_dir().join("threads")
    }

    fn index_path(&self) -> PathBuf {
        self.threads_dir().join("index.json")
    }

    fn thread_path(&self, thread_id: &str) -> PathBuf {
        self.threads_dir().join(format!("{thread_id}.json"))
    }

    fn load_index(&self) -> Result<ThreadIndex> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(ThreadIndex::default());
        }
        let data = fs::read_to_string(&path)?;
        serde_json::from_str(&data).map_err(Into::into)
    }

    fn save_index(&self, index: &ThreadIndex) -> Result<()> {
        fs::create_dir_all(self.threads_dir())?;
        let pending = self.threads_dir().join("index.json.next");
        let file = fs::File::create(&pending)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, index)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        fs::rename(&pending, self.index_path())?;
        Ok(())
    }

    /// Create a new thread and persist it.
    pub fn create_thread(&self, thread: &Thread) -> Result<()> {
        if thread.id.is_empty() {
            return Err(WonderError::validation("thread id must not be empty"));
        }

        fs::create_dir_all(self.threads_dir())?;
        let path = self.thread_path(&thread.id);

        let data = serde_json::to_string_pretty(thread)?;
        let pending = path.with_extension("json.next");
        fs::write(&pending, data)?;
        fs::rename(&pending, &path)?;

        let mut index = self.load_index()?;
        index.threads.insert(
            thread.id.clone(),
            ThreadIndexEntry {
                id: thread.id.clone(),
                title: thread.title.clone(),
                created_at: thread.created_at,
                updated_at: thread.updated_at,
                session_count: thread.session_ids.len(),
                status: thread.status,
                tags: thread.tags.clone(),
                git_branch: thread.git_branch.clone(),
            },
        );
        self.save_index(&index)
    }

    /// Load a thread by ID.
    pub fn load_thread(&self, thread_id: &str) -> Result<Option<Thread>> {
        let path = self.thread_path(thread_id);
        if !path.exists() {
            return Ok(None);
        }
        let data = fs::read_to_string(&path)?;
        serde_json::from_str(&data).map_err(Into::into).map(Some)
    }

    /// Update an existing thread.
    pub fn update_thread(&self, thread: &Thread) -> Result<()> {
        let mut existing = self
            .load_thread(&thread.id)?
            .ok_or_else(|| WonderError::not_found("thread", thread.id.clone()))?;

        existing.title = thread.title.clone().or(existing.title);
        existing.tags = thread.tags.clone();
        existing.status = thread.status;
        existing.summary = thread.summary.clone().or(existing.summary);
        existing.git_branch = thread.git_branch.clone().or(existing.git_branch);
        existing.cwd = thread.cwd.clone().or(existing.cwd);
        existing.session_ids = thread.session_ids.clone();
        existing.updated_at = OffsetDateTime::now_utc();

        self.create_thread(&existing)
    }

    /// Add a session to a thread.
    pub fn add_session(&self, thread_id: &str, session_id: SessionId) -> Result<()> {
        let mut thread = self
            .load_thread(thread_id)?
            .unwrap_or_else(|| Thread::new(thread_id));

        if !thread.session_ids.contains(&session_id) {
            thread.session_ids.push(session_id);
            thread.updated_at = OffsetDateTime::now_utc();
            self.create_thread(&thread)?;
        }
        Ok(())
    }

    /// List all active threads, sorted by update time (newest first).
    pub fn list_active_threads(&self) -> Result<Vec<Thread>> {
        let index = self.load_index()?;
        let mut threads = Vec::new();
        for entry in index.threads.values() {
            if entry.status == ThreadStatus::Active {
                if let Some(thread) = self.load_thread(&entry.id)? {
                    threads.push(thread);
                }
            }
        }
        threads.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        Ok(threads)
    }

    /// List threads matching a title or tag search query.
    pub fn search_threads(&self, query: &str) -> Result<Vec<Thread>> {
        let index = self.load_index()?;
        let query = query.to_lowercase();
        let mut results = Vec::new();
        for entry in index.threads.values() {
            let title_match = entry
                .title
                .as_ref()
                .is_some_and(|t| t.to_lowercase().contains(&query));
            let tag_match = entry.tags.iter().any(|t| t.to_lowercase().contains(&query));
            if title_match || tag_match {
                if let Some(thread) = self.load_thread(&entry.id)? {
                    results.push(thread);
                }
            }
        }
        results.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        Ok(results)
    }

    /// Archive a thread (soft-delete).
    pub fn archive_thread(&self, thread_id: &str) -> Result<()> {
        let mut thread = self
            .load_thread(thread_id)?
            .ok_or_else(|| WonderError::not_found("thread", thread_id.to_string()))?;
        thread.status = ThreadStatus::Archived;
        thread.updated_at = OffsetDateTime::now_utc();
        self.create_thread(&thread)
    }

    /// Permanently delete a thread.
    pub fn delete_thread(&self, thread_id: &str) -> Result<()> {
        let path = self.thread_path(thread_id);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        let mut index = self.load_index()?;
        index.threads.remove(thread_id);
        self.save_index(&index)
    }

    /// Rebuild the index from individual thread files.
    pub fn rebuild_index(&self) -> Result<()> {
        let dir = self.threads_dir();
        if !dir.exists() {
            return Ok(());
        }
        let mut index = ThreadIndex::default();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension() != Some(OsStr::new("json")) {
                continue;
            }
            if path.file_name().and_then(OsStr::to_str) == Some("index.json") {
                continue;
            }
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(thread) = serde_json::from_str::<Thread>(&data) {
                    index.threads.insert(
                        thread.id.clone(),
                        ThreadIndexEntry {
                            id: thread.id.clone(),
                            title: thread.title.clone(),
                            created_at: thread.created_at,
                            updated_at: thread.updated_at,
                            session_count: thread.session_ids.len(),
                            status: thread.status,
                            tags: thread.tags.clone(),
                            git_branch: thread.git_branch.clone(),
                        },
                    );
                }
            }
        }
        self.save_index(&index)
    }
}

#[cfg(test)]
mod tests {
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    #[test]
    fn create_and_load_thread() {
        let dir = unique_test_dir("thread-store-create");
        let store = ThreadStore::new(&dir);
        let thread = Thread::new("test-1");
        store.create_thread(&thread).expect("create");
        let loaded = store.load_thread("test-1").expect("load").expect("exists");
        assert_eq!(loaded.id, "test-1");
        assert_eq!(loaded.status, ThreadStatus::Active);
    }

    #[test]
    fn add_session_to_thread() {
        let dir = unique_test_dir("thread-store-session");
        let store = ThreadStore::new(&dir);
        let thread = Thread::new("test-2");
        store.create_thread(&thread).expect("create");
        let sid = SessionId::new();
        store.add_session("test-2", sid).expect("add");
        let loaded = store.load_thread("test-2").expect("load").expect("exists");
        assert!(loaded.session_ids.contains(&sid));
    }

    #[test]
    fn list_active_threads() {
        let dir = unique_test_dir("thread-store-list");
        let store = ThreadStore::new(&dir);
        store.create_thread(&Thread::new("a")).expect("create");
        store.create_thread(&Thread::new("b")).expect("create");
        let active = store.list_active_threads().expect("list");
        assert_eq!(active.len(), 2);
    }

    #[test]
    fn search_threads_by_tag() {
        let dir = unique_test_dir("thread-store-search");
        let store = ThreadStore::new(&dir);
        let mut thread = Thread::new("searchable");
        thread.tags = vec!["rust".into(), "cli".into()];
        store.create_thread(&thread).expect("create");
        let results = store.search_threads("rust").expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "searchable");
    }

    #[test]
    fn archive_and_delete_thread() {
        let dir = unique_test_dir("thread-store-archive");
        let store = ThreadStore::new(&dir);
        store.create_thread(&Thread::new("temp")).expect("create");
        store.archive_thread("temp").expect("archive");
        let archived = store.load_thread("temp").expect("load").expect("exists");
        assert_eq!(archived.status, ThreadStatus::Archived);
        store.delete_thread("temp").expect("delete");
        assert!(store.load_thread("temp").expect("load").is_none());
    }
}
