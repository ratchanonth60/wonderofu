//! Core types for the local-first team/agent inbox mailbox.
//!
//! # Layout
//!
//! Messages are stored under the storage base directory:
//!
//! ```text
//! mailboxes/
//!   teams/{team_name}/inbox.jsonl      ← append-only message log
//!   teams/{team_name}/inbox.read.json  ← read-message index
//!   agents/{agent_name}/inbox.jsonl
//!   agents/{agent_name}/inbox.read.json
//! ```
//!
//! # Name rules
//!
//! Team and agent names must pass [`sanitize_mailbox_name`]:
//!
//! - Non-empty and ≤ [`MAX_MAILBOX_NAME_LEN`] bytes
//! - First character is ASCII alphanumeric
//! - Every subsequent character is ASCII alphanumeric, `-`, or `_`
//!
//! This rejects `.`, `..`, `/`, `\`, null bytes, control characters, and any
//! sequence that could escape the mailbox directory via path traversal.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{MailboxMessageId, Result, ToolUseId, WonderError};

// ── Schema versions ───────────────────────────────────────────────────────────

/// Schema version for [`MailboxMessage`] records.
pub const MAILBOX_MESSAGE_SCHEMA_VERSION: u16 = 1;

/// Schema version for [`MailboxReadIndex`] files.
pub const MAILBOX_READ_INDEX_SCHEMA_VERSION: u16 = 1;

fn default_mailbox_message_schema_version() -> u16 {
    MAILBOX_MESSAGE_SCHEMA_VERSION
}

fn default_mailbox_read_index_schema_version() -> u16 {
    MAILBOX_READ_INDEX_SCHEMA_VERSION
}

// ── MailboxKind ───────────────────────────────────────────────────────────────

/// Selects whether the mailbox belongs to a team or an individual agent.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailboxKind {
    /// A shared team inbox; path segment is the team name.
    Team,
    /// A per-agent inbox; path segment is the agent name.
    Agent,
}

impl MailboxKind {
    /// Returns the sub-directory name for this kind inside `mailboxes/`.
    #[must_use]
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Team => "teams",
            Self::Agent => "agents",
        }
    }
}

// ── MailboxMessage ────────────────────────────────────────────────────────────

/// A single message stored in a team or agent inbox.
///
/// Messages are appended as newline-delimited JSON to `inbox.jsonl` and are
/// never deleted; read state is tracked separately in `inbox.read.json`.
///
/// # Schema stability
///
/// All optional fields use `#[serde(default, skip_serializing_if)]` so future
/// readers that do not know a new field silently ignore it.
///
/// # Examples
///
/// ```
/// use wonder_of_u_core::mailbox::MailboxMessage;
///
/// let msg = MailboxMessage::new("orchestrator", "backend-team", "Deploy ready", "Please deploy v1.2.");
/// assert_eq!(msg.schema_version, wonder_of_u_core::mailbox::MAILBOX_MESSAGE_SCHEMA_VERSION);
/// assert!(!msg.subject.is_empty());
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MailboxMessage {
    /// Storage schema guard – always [`MAILBOX_MESSAGE_SCHEMA_VERSION`].
    #[serde(default = "default_mailbox_message_schema_version")]
    pub schema_version: u16,

    /// Unique identifier for this message (used for mark-read).
    pub id: MailboxMessageId,

    /// Sender identity (agent name, team name, or free text).
    pub from: String,

    /// Addressee: the team or agent name this message was sent to.
    pub to: String,

    /// Short subject line (≤ 200 chars recommended).
    pub subject: String,

    /// Full message body (plain text).
    pub body: String,

    /// UTC timestamp when the message was created.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,

    /// The `tool_use_id` of the `SendMessage` tool call that created this
    /// message, linking the mailbox record back to the tool invocation in the
    /// conversation transcript.
    ///
    /// Optional – absent for messages created outside a tool-call context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<ToolUseId>,

    /// Arbitrary caller-supplied tags for filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl MailboxMessage {
    /// Creates a new [`MailboxMessage`] stamped with the current UTC time.
    ///
    /// # Examples
    ///
    /// ```
    /// use wonder_of_u_core::mailbox::MailboxMessage;
    ///
    /// let msg = MailboxMessage::new("agent-a", "team-x", "Hello", "body text");
    /// assert_eq!(msg.from, "agent-a");
    /// assert_eq!(msg.to, "team-x");
    /// ```
    #[must_use]
    pub fn new(
        from: impl Into<String>,
        to: impl Into<String>,
        subject: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: MAILBOX_MESSAGE_SCHEMA_VERSION,
            id: MailboxMessageId::new(),
            from: from.into(),
            to: to.into(),
            subject: subject.into(),
            body: body.into(),
            created_at: OffsetDateTime::now_utc(),
            tool_use_id: None,
            tags: Vec::new(),
        }
    }

    /// Attaches the originating `tool_use_id` to this message, linking the
    /// mailbox record back to the tool invocation in the conversation
    /// transcript.
    ///
    /// # Examples
    ///
    /// ```
    /// use wonder_of_u_core::{ToolUseId, mailbox::MailboxMessage};
    ///
    /// let id = ToolUseId::new();
    /// let msg = MailboxMessage::new("a", "b", "s", "body").with_tool_use_id(id);
    /// assert_eq!(msg.tool_use_id, Some(id));
    /// ```
    #[must_use]
    pub fn with_tool_use_id(mut self, id: ToolUseId) -> Self {
        self.tool_use_id = Some(id);
        self
    }
}

// ── MailboxReadIndex ──────────────────────────────────────────────────────────

/// Tracks which message IDs in an inbox have been marked as read.
///
/// Stored atomically at `inbox.read.json` alongside the `inbox.jsonl` log.
/// The index is a simple set of IDs; messages absent from the set are unread.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MailboxReadIndex {
    /// Storage schema guard – always [`MAILBOX_READ_INDEX_SCHEMA_VERSION`].
    #[serde(default = "default_mailbox_read_index_schema_version")]
    pub schema_version: u16,

    /// Ordered set of message IDs that have been marked read.
    #[serde(default)]
    pub read_ids: BTreeSet<MailboxMessageId>,
}

impl MailboxReadIndex {
    /// Creates an empty read index.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema_version: MAILBOX_READ_INDEX_SCHEMA_VERSION,
            read_ids: BTreeSet::new(),
        }
    }

    /// Returns `true` when `id` has been marked read.
    #[must_use]
    pub fn is_read(&self, id: MailboxMessageId) -> bool {
        self.read_ids.contains(&id)
    }

    /// Marks `id` as read; returns `true` if the set changed (i.e. the ID
    /// was not already present).
    pub fn mark_read(&mut self, id: MailboxMessageId) -> bool {
        self.read_ids.insert(id)
    }
}

// ── Sanitization ──────────────────────────────────────────────────────────────

/// Maximum byte length for a team or agent mailbox name.
pub const MAX_MAILBOX_NAME_LEN: usize = 120;

/// Validates and returns `name` unchanged when it is safe to use as a mailbox
/// path segment; otherwise returns a [`WonderError::Validation`] error.
///
/// # Rules
///
/// - Non-empty
/// - ≤ [`MAX_MAILBOX_NAME_LEN`] bytes
/// - First character is ASCII alphanumeric (`[a-zA-Z0-9]`)
/// - Every subsequent character is ASCII alphanumeric, `-`, or `_`
///
/// Dots (`.`), slashes (`/`, `\`), null bytes, and control characters are all
/// rejected, preventing any form of path traversal.
///
/// # Examples
///
/// ```
/// use wonder_of_u_core::mailbox::sanitize_mailbox_name;
///
/// assert!(sanitize_mailbox_name("my-team").is_ok());
/// assert!(sanitize_mailbox_name("agent_01").is_ok());
/// assert!(sanitize_mailbox_name("../evil").is_err());
/// assert!(sanitize_mailbox_name("").is_err());
/// assert!(sanitize_mailbox_name("bad/name").is_err());
/// ```
pub fn sanitize_mailbox_name(name: &str) -> Result<&str> {
    if name.is_empty() {
        return Err(WonderError::validation("mailbox name must not be empty"));
    }

    if name.len() > MAX_MAILBOX_NAME_LEN {
        return Err(WonderError::validation(format!(
            "mailbox name too long ({} bytes; max {MAX_MAILBOX_NAME_LEN})",
            name.len()
        )));
    }

    let mut chars = name.chars();
    // Unwrap is safe: non-empty check above guarantees at least one char.
    let first = chars.next().expect("non-empty string has a first char");
    if !first.is_ascii_alphanumeric() {
        return Err(WonderError::validation(format!(
            "mailbox name must start with ASCII alphanumeric, got {first:?}"
        )));
    }

    for ch in chars {
        if !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_' {
            return Err(WonderError::validation(format!(
                "mailbox name contains disallowed character {ch:?}; \
                 only ASCII alphanumeric, '-', and '_' are permitted"
            )));
        }
    }

    Ok(name)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── sanitize_mailbox_name ─────────────────────────────────────────────────

    #[test]
    fn sanitize_accepts_valid_names() {
        assert!(sanitize_mailbox_name("team").is_ok());
        assert!(sanitize_mailbox_name("my-team").is_ok());
        assert!(sanitize_mailbox_name("agent_01").is_ok());
        assert!(sanitize_mailbox_name("BackendTeam").is_ok());
        assert!(sanitize_mailbox_name("a").is_ok());
        assert!(sanitize_mailbox_name(&"x".repeat(MAX_MAILBOX_NAME_LEN)).is_ok());
    }

    #[test]
    fn sanitize_rejects_empty() {
        let err = sanitize_mailbox_name("").unwrap_err();
        assert!(err.to_string().contains("empty"), "{err}");
    }

    #[test]
    fn sanitize_rejects_name_too_long() {
        let long = "a".repeat(MAX_MAILBOX_NAME_LEN + 1);
        let err = sanitize_mailbox_name(&long).unwrap_err();
        assert!(err.to_string().contains("too long"), "{err}");
    }

    #[test]
    fn sanitize_rejects_dot_traversal() {
        for bad in [".", "..", "../evil", "a..b", "a.b"] {
            assert!(
                sanitize_mailbox_name(bad).is_err(),
                "expected rejection for {bad:?}"
            );
        }
    }

    #[test]
    fn sanitize_rejects_slash() {
        assert!(sanitize_mailbox_name("a/b").is_err());
        assert!(sanitize_mailbox_name("a\\b").is_err());
    }

    #[test]
    fn sanitize_rejects_control_characters() {
        assert!(sanitize_mailbox_name("a\x00b").is_err()); // NUL
        assert!(sanitize_mailbox_name("a\nb").is_err()); // newline
        assert!(sanitize_mailbox_name("a\tb").is_err()); // tab
        assert!(sanitize_mailbox_name("\x1bname").is_err()); // ESC at start
    }

    #[test]
    fn sanitize_rejects_non_alphanumeric_start() {
        assert!(sanitize_mailbox_name("-team").is_err());
        assert!(sanitize_mailbox_name("_team").is_err());
        assert!(sanitize_mailbox_name(" team").is_err());
    }

    #[test]
    fn sanitize_rejects_unicode_non_ascii() {
        assert!(sanitize_mailbox_name("tëam").is_err());
        assert!(sanitize_mailbox_name("チーム").is_err());
    }

    // ── MailboxReadIndex ──────────────────────────────────────────────────────

    #[test]
    fn read_index_starts_empty() {
        let index = MailboxReadIndex::empty();
        let id = MailboxMessageId::new();
        assert!(!index.is_read(id));
    }

    #[test]
    fn read_index_mark_read_idempotent() {
        let mut index = MailboxReadIndex::empty();
        let id = MailboxMessageId::new();
        assert!(index.mark_read(id)); // first call: changed
        assert!(!index.mark_read(id)); // second call: already present
        assert!(index.is_read(id));
    }

    // ── MailboxMessage ────────────────────────────────────────────────────────

    #[test]
    fn mailbox_message_new_sets_correct_fields() {
        let msg = MailboxMessage::new("sender", "receiver", "subj", "body text");
        assert_eq!(msg.schema_version, MAILBOX_MESSAGE_SCHEMA_VERSION);
        assert_eq!(msg.from, "sender");
        assert_eq!(msg.to, "receiver");
        assert_eq!(msg.subject, "subj");
        assert_eq!(msg.body, "body text");
        assert!(msg.tags.is_empty());
        assert!(msg.tool_use_id.is_none(), "tool_use_id defaults to None");
    }

    #[test]
    fn mailbox_message_with_tool_use_id_round_trips() {
        use crate::ToolUseId;

        let id = ToolUseId::new();
        let msg = MailboxMessage::new("a", "b", "s", "body").with_tool_use_id(id);
        assert_eq!(msg.tool_use_id, Some(id));

        // Serialise → deserialise and confirm the field survives.
        let json = serde_json::to_string(&msg).expect("serialize");
        let back: MailboxMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            back.tool_use_id,
            Some(id),
            "tool_use_id survives round-trip"
        );
    }

    #[test]
    fn mailbox_message_without_tool_use_id_omitted_from_json() {
        let msg = MailboxMessage::new("a", "b", "s", "body");
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(
            !json.contains("tool_use_id"),
            "tool_use_id must be absent from JSON when None; json={json}"
        );
    }
}
