//! Local-first mailbox storage for team and agent inbox messages.
//!
//! # Concurrency model
//!
//! Messages are appended to `inbox.jsonl` via `O_APPEND`.  On POSIX systems
//! writes smaller than `PIPE_BUF` (≥ 4096 bytes; our JSON lines are well
//! below that) are atomic, so concurrent appenders produce a valid JSONL file
//! without corruption or interleaving.
//!
//! The read index (`inbox.read.json`) is rewritten atomically via
//! `rename(2)` on every [`MailboxStore::mark_read`] call.  Concurrent
//! mark-read operations on separate message IDs may race and one writer can
//! overwrite the other's change; for the local-first single-agent use-case
//! this is acceptable.  If stricter multi-writer mark-read consistency is
//! needed later, a per-entry append log can replace the index file.
//!
//! # Layout
//!
//! ```text
//! mailboxes/
//!   teams/{name}/inbox.jsonl      ← append-only message log
//!   teams/{name}/inbox.read.json  ← read-message ID index
//!   agents/{name}/inbox.jsonl
//!   agents/{name}/inbox.read.json
//! ```

use std::{
    fs::{self, OpenOptions},
    io::{BufWriter, ErrorKind, Write},
    path::PathBuf,
};

use wonder_of_u_core::{
    MailboxKind, MailboxMessage, MailboxMessageId, MailboxReadIndex, Result,
    mailbox::sanitize_mailbox_name,
};

use super::{StoragePaths, ensure_supported_schema, write_json_atomically};

// ── MailboxStore ──────────────────────────────────────────────────────────────

/// Local-first mailbox store for team and agent inbox messages.
///
/// Each `(kind, name)` pair addresses one inbox; names are validated by
/// [`sanitize_mailbox_name`] before any I/O occurs.  The store is `Clone`
/// and cheap to share across threads; each method opens and closes its own
/// file handles.
///
/// # Examples
///
/// ```no_run
/// use wonder_of_u_core::{MailboxKind, MailboxMessage};
/// use wonder_of_u_storage::MailboxStore;
///
/// let store = MailboxStore::new("/var/wonder/storage");
/// let msg = MailboxMessage::new("orchestrator", "backend-team", "Deploy ready", "Please deploy.");
/// store.append(MailboxKind::Team, "backend-team", &msg).unwrap();
/// let msgs = store.list(MailboxKind::Team, "backend-team").unwrap();
/// assert_eq!(msgs.len(), 1);
/// ```
#[derive(Clone, Debug)]
pub struct MailboxStore {
    paths: StoragePaths,
}

impl MailboxStore {
    /// Creates a new `MailboxStore` rooted at `base_dir`.
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
        }
    }

    /// Returns the underlying [`StoragePaths`] helper.
    #[must_use]
    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    /// Ensures the inbox directory for `(kind, name)` exists on disk.
    ///
    /// `name` is validated by [`sanitize_mailbox_name`] before any I/O.
    pub fn ensure_layout(&self, kind: MailboxKind, name: &str) -> Result<()> {
        let safe = sanitize_mailbox_name(name)?;
        fs::create_dir_all(self.paths.mailbox_inbox_dir(kind, safe))?;
        Ok(())
    }

    /// Appends `message` to the inbox identified by `(kind, name)`.
    ///
    /// The message is serialised as a single JSON line (`\n`-terminated) and
    /// flushed + `sync_data`'d before returning so readers see a complete
    /// record.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` fails [`sanitize_mailbox_name`] or when
    /// any I/O operation fails.
    pub fn append(&self, kind: MailboxKind, name: &str, message: &MailboxMessage) -> Result<()> {
        let safe = sanitize_mailbox_name(name)?;
        self.ensure_layout(kind, safe)?;
        let path = self.paths.mailbox_inbox_log_path(kind, safe);

        // O_APPEND makes each write atomic on POSIX for sizes < PIPE_BUF.
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, message)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        writer.get_ref().sync_data()?;
        Ok(())
    }

    /// Returns all messages in the inbox for `(kind, name)`, oldest first.
    ///
    /// Returns an empty `Vec` when the inbox has never received any messages.
    /// Corrupt or schema-mismatched JSON lines are rejected with an error.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` fails [`sanitize_mailbox_name`], when the
    /// file contains invalid JSON, or when a schema version is unsupported.
    pub fn list(&self, kind: MailboxKind, name: &str) -> Result<Vec<MailboxMessage>> {
        let safe = sanitize_mailbox_name(name)?;
        let path = self.paths.mailbox_inbox_log_path(kind, safe);

        let contents = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        let mut messages = Vec::new();
        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let msg: MailboxMessage = serde_json::from_str(trimmed)?;
            ensure_supported_schema("mailbox message", msg.schema_version)?;
            messages.push(msg);
        }
        Ok(messages)
    }

    /// Marks `message_id` as read in the inbox for `(kind, name)`.
    ///
    /// Reads the existing read index (or starts from an empty one), inserts
    /// `message_id`, and atomically replaces the index file on disk.
    ///
    /// Returns `true` when the ID was newly marked (was previously unread),
    /// `false` when it was already in the index.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` fails [`sanitize_mailbox_name`] or when
    /// any I/O operation fails.
    pub fn mark_read(
        &self,
        kind: MailboxKind,
        name: &str,
        message_id: MailboxMessageId,
    ) -> Result<bool> {
        let safe = sanitize_mailbox_name(name)?;
        self.ensure_layout(kind, safe)?;

        let mut index = self.load_read_index(kind, safe)?;
        let changed = index.mark_read(message_id);
        if changed {
            let path = self.paths.mailbox_read_index_path(kind, safe);
            write_json_atomically(&path, &index)?;
        }
        Ok(changed)
    }

    /// Returns the number of unread messages in the inbox for `(kind, name)`.
    ///
    /// A message is unread when its ID is absent from the read index.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`MailboxStore::list`].
    pub fn unread_count(&self, kind: MailboxKind, name: &str) -> Result<usize> {
        let messages = self.list(kind, name)?;
        if messages.is_empty() {
            return Ok(0);
        }
        // Validate the name before loading the index (list already does this,
        // but we need the sanitized form again here).
        let safe = sanitize_mailbox_name(name)?;
        let index = self.load_read_index(kind, safe)?;
        Ok(messages.iter().filter(|m| !index.is_read(m.id)).count())
    }

    // ── private helpers ───────────────────────────────────────────────────────

    fn load_read_index(&self, kind: MailboxKind, sanitized_name: &str) -> Result<MailboxReadIndex> {
        let path = self.paths.mailbox_read_index_path(kind, sanitized_name);
        match fs::read_to_string(&path) {
            Ok(contents) => {
                let index: MailboxReadIndex = serde_json::from_str(&contents)?;
                ensure_supported_schema("mailbox read index", index.schema_version)?;
                Ok(index)
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(MailboxReadIndex::empty()),
            Err(e) => Err(e.into()),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use wonder_of_u_core::{MailboxKind, MailboxMessage, MailboxMessageId};
    use wonder_of_u_test_support::unique_test_dir;

    use super::MailboxStore;

    fn make_store(prefix: &str) -> (MailboxStore, std::path::PathBuf) {
        let dir = unique_test_dir(prefix);
        (MailboxStore::new(&dir), dir)
    }

    // ── name sanitization (storage layer) ────────────────────────────────────

    #[test]
    fn append_rejects_traversal_name() {
        let (store, _dir) = make_store("mailbox-traversal");
        let msg = MailboxMessage::new("a", "b", "s", "b");
        let err = store
            .append(MailboxKind::Team, "../evil", &msg)
            .unwrap_err();
        // `..` starts with `.` which is not ASCII-alphanumeric.
        assert!(
            err.to_string().contains("start") || err.to_string().contains("disallowed"),
            "expected name-validation error: {err}"
        );
    }

    #[test]
    fn list_rejects_traversal_name() {
        let (store, _dir) = make_store("mailbox-list-traversal");
        let err = store.list(MailboxKind::Agent, "/etc/passwd").unwrap_err();
        assert!(
            err.to_string().contains("disallowed") || err.to_string().contains("start"),
            "{err}"
        );
    }

    #[test]
    fn mark_read_rejects_invalid_name() {
        let (store, _dir) = make_store("mailbox-mark-read-invalid");
        let id = MailboxMessageId::new();
        let err = store.mark_read(MailboxKind::Team, "", id).unwrap_err();
        assert!(err.to_string().contains("empty"), "{err}");
    }

    // ── append + list roundtrip ───────────────────────────────────────────────

    #[test]
    fn append_and_list_team_inbox() {
        let (store, _dir) = make_store("mailbox-team-append");
        let m1 = MailboxMessage::new("agent-a", "backend-team", "First", "body1");
        let m2 = MailboxMessage::new("agent-b", "backend-team", "Second", "body2");

        store
            .append(MailboxKind::Team, "backend-team", &m1)
            .unwrap();
        store
            .append(MailboxKind::Team, "backend-team", &m2)
            .unwrap();

        let msgs = store.list(MailboxKind::Team, "backend-team").unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].subject, "First");
        assert_eq!(msgs[1].subject, "Second");
    }

    #[test]
    fn append_and_list_agent_inbox() {
        let (store, _dir) = make_store("mailbox-agent-append");
        let msg = MailboxMessage::new("orchestrator", "worker-01", "Task", "Do the thing");
        store.append(MailboxKind::Agent, "worker-01", &msg).unwrap();

        let msgs = store.list(MailboxKind::Agent, "worker-01").unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, "Do the thing");
    }

    #[test]
    fn list_returns_empty_when_inbox_never_written() {
        let (store, _dir) = make_store("mailbox-empty-inbox");
        let msgs = store.list(MailboxKind::Team, "no-team-yet").unwrap();
        assert!(msgs.is_empty());
    }

    #[test]
    fn team_and_agent_inboxes_are_independent() {
        let (store, _dir) = make_store("mailbox-isolated");
        let msg = MailboxMessage::new("x", "shared-name", "subj", "body");

        store
            .append(MailboxKind::Team, "shared-name", &msg)
            .unwrap();

        let agent_msgs = store.list(MailboxKind::Agent, "shared-name").unwrap();
        let team_msgs = store.list(MailboxKind::Team, "shared-name").unwrap();
        assert!(agent_msgs.is_empty(), "agent inbox should be empty");
        assert_eq!(team_msgs.len(), 1, "team inbox should have 1 message");
    }

    // ── mark_read + unread_count ──────────────────────────────────────────────

    #[test]
    fn mark_read_changes_unread_count() {
        let (store, _dir) = make_store("mailbox-mark-read");
        let m1 = MailboxMessage::new("a", "my-team", "M1", "body");
        let m2 = MailboxMessage::new("a", "my-team", "M2", "body");
        let id1 = m1.id;

        store.append(MailboxKind::Team, "my-team", &m1).unwrap();
        store.append(MailboxKind::Team, "my-team", &m2).unwrap();

        assert_eq!(store.unread_count(MailboxKind::Team, "my-team").unwrap(), 2);

        let changed = store.mark_read(MailboxKind::Team, "my-team", id1).unwrap();
        assert!(changed, "first mark_read should report a change");

        assert_eq!(store.unread_count(MailboxKind::Team, "my-team").unwrap(), 1);
    }

    #[test]
    fn mark_read_is_idempotent() {
        let (store, _dir) = make_store("mailbox-mark-read-idempotent");
        let msg = MailboxMessage::new("a", "t", "subj", "body");
        let id = msg.id;

        store.append(MailboxKind::Team, "t", &msg).unwrap();
        assert!(store.mark_read(MailboxKind::Team, "t", id).unwrap());
        assert!(!store.mark_read(MailboxKind::Team, "t", id).unwrap());
        assert_eq!(store.unread_count(MailboxKind::Team, "t").unwrap(), 0);
    }

    #[test]
    fn mark_read_unknown_id_does_not_panic() {
        let (store, _dir) = make_store("mailbox-mark-read-unknown");
        let unknown = MailboxMessageId::new();
        // No messages appended – marking an arbitrary ID should succeed (it
        // gets added to the index and counts as "changed").
        let changed = store
            .mark_read(MailboxKind::Agent, "agent-x", unknown)
            .unwrap();
        assert!(changed);
    }

    // ── schema version ────────────────────────────────────────────────────────

    #[test]
    fn list_rejects_unsupported_schema_version() {
        use std::io::Write;

        let (store, _dir) = make_store("mailbox-schema-reject");
        store
            .ensure_layout(MailboxKind::Team, "schema-test")
            .unwrap();

        let path = store
            .paths()
            .mailbox_inbox_log_path(MailboxKind::Team, "schema-test");

        // Write a message with a future schema version.
        let bad_line = r#"{"schema_version":9999,"id":"00000000-0000-0000-0000-000000000001","from":"a","to":"b","subject":"s","body":"b","created_at":"2024-01-01T00:00:00Z"}"#;
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "{bad_line}").unwrap();

        let err = store.list(MailboxKind::Team, "schema-test").unwrap_err();
        assert!(
            err.to_string().contains("9999"),
            "error should mention bad version: {err}"
        );
    }

    #[test]
    fn read_index_schema_mismatch_returns_error() {
        use std::io::Write;

        let (store, _dir) = make_store("mailbox-read-index-schema");
        store.ensure_layout(MailboxKind::Team, "idx-test").unwrap();

        let path = store
            .paths()
            .mailbox_read_index_path(MailboxKind::Team, "idx-test");

        let bad = r#"{"schema_version":9999,"read_ids":[]}"#;
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "{bad}").unwrap();

        // unread_count triggers load_read_index via list (which is empty) +
        // explicit index load.  Use mark_read to force the index load directly.
        let id = MailboxMessageId::new();
        let err = store
            .mark_read(MailboxKind::Team, "idx-test", id)
            .unwrap_err();
        assert!(
            err.to_string().contains("9999"),
            "error should mention bad version: {err}"
        );
    }

    // ── concurrent append (basic smoke) ──────────────────────────────────────

    #[test]
    fn concurrent_appends_produce_complete_message_list() {
        use std::{sync::Arc, thread};

        let (store, _dir) = make_store("mailbox-concurrent");
        let store = Arc::new(store);
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let s = Arc::clone(&store);
                thread::spawn(move || {
                    let msg = MailboxMessage::new(
                        format!("agent-{i}"),
                        "shared-team",
                        format!("msg-{i}"),
                        "body",
                    );
                    s.append(MailboxKind::Team, "shared-team", &msg).unwrap();
                })
            })
            .collect();

        for h in handles {
            h.join().expect("thread panicked");
        }

        let msgs = store.list(MailboxKind::Team, "shared-team").unwrap();
        assert_eq!(msgs.len(), 8, "all 8 messages should be present");
    }
}
