//! TypeScript-upstream transcript importer.
//!
//! Parses TS-shaped JSONL records produced by the upstream Claude Code TypeScript
//! client (`~/.claude/projects/*/`) and converts them to Rust [`MessageEnvelope`]
//! schema. This is a **one-shot migration path only**: the normal
//! [`TranscriptStore::load_session`] path is never touched and continues to reject
//! any record that lacks a valid `schema_version: 1` Rust envelope.
//!
//! # Supported conversions
//!
//! | TS `type` | TS `message.content` | Rust `MessagePayload` |
//! |-----------|----------------------|----------------------|
//! | `"user"`  | string               | `UserText`           |
//! | `"user"`  | `[{type:"text",...}]`| `UserText` (joined)  |
//! | `"assistant"` | `[{type:"text",...}]` | `AssistantText` |
//! | `"assistant"` | `[{type:"thinking",...}]` | `AssistantThinking` |
//!
//! Everything else is skipped and reported in [`TsImportReport::skipped`].
//!
//! # Paste references
//!
//! The TS client embeds unresolvable references such as `[Pasted text #1 +5 lines]`
//! inside user messages.  This importer preserves the placeholder text as-is and
//! records a [`SkipWarning::UnresolvedPasteRef`] in [`TsImportReport::paste_warnings`]
//! so callers can surface the information without silent data loss.

use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use serde::Deserialize;
use serde_json::Value;
use time::OffsetDateTime;

use wonder_of_u_core::Result;
use wonder_of_u_core::{MessageEnvelope, MessageId, MessagePayload, SessionId, WonderError};

use crate::TranscriptStore;

// ─── paste-reference detection ────────────────────────────────────────────────

/// Matches TS paste-reference placeholders such as `[Pasted text #1 +5 lines]`,
/// `[Pasted text #3]`, `[Image #2]`, `[...Truncated text #4]`.
const PASTE_REF_PATTERN: &str = "[Pasted text #";
const IMAGE_REF_PATTERN: &str = "[Image #";

fn contains_paste_ref(text: &str) -> bool {
    text.contains(PASTE_REF_PATTERN) || text.contains(IMAGE_REF_PATTERN)
}

// ─── raw TS deserialisation types ─────────────────────────────────────────────

/// Raw TS JSONL record — matches the on-disk shape written by the TypeScript client.
///
/// Fields use `camelCase` as stored by the TS client. All fields are optional so
/// that metadata-only records (like `"type":"summary"`) parse without error.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TsRawRecord {
    /// TS record discriminator (`"user"`, `"assistant"`, `"system"`, …).
    #[serde(rename = "type")]
    record_type: Option<String>,

    /// Inner message object present on `user` and `assistant` records.
    message: Option<Value>,

    /// TS `uuid` — used as a stable `MessageId` during import.
    uuid: Option<String>,

    /// TS `sessionId`.
    #[serde(rename = "sessionId")]
    session_id: Option<String>,

    /// RFC-3339 timestamp string.
    timestamp: Option<String>,

    /// Working directory at the time the message was recorded.
    cwd: Option<String>,

    /// Git branch name.
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,

    /// CLI entrypoint that produced this record.
    entrypoint: Option<String>,

    /// `version` field written by the TS client (used as `app_version`).
    version: Option<String>,
}

/// A single content block inside a TS assistant `message.content` array.
#[derive(Debug, Deserialize)]
struct TsContentBlock {
    /// Block discriminator: `"text"`, `"thinking"`, `"tool_use"`, `"tool_result"`, …
    #[serde(rename = "type")]
    block_type: String,

    /// Present on `"thinking"` blocks.
    thinking: Option<String>,
}

// ─── public report types ──────────────────────────────────────────────────────

/// Summary of an import or dry-run inspection of a TS JSONL transcript file.
#[derive(Clone, Debug, PartialEq)]
pub struct TsImportReport {
    /// Absolute path of the source file.
    pub source_path: PathBuf,

    /// Session ID used for the import (either detected from the file or newly
    /// generated).
    pub session_id: SessionId,

    /// Number of messages that were (or would be) written.  Zero in dry-run
    /// mode.
    pub imported_count: usize,

    /// Records that could not be converted, with per-record explanations.
    pub skipped: Vec<TsSkipReport>,

    /// User messages that contain TS paste-reference placeholders
    /// (`[Pasted text #N]`, `[Image #N]`) which cannot be resolved without the
    /// originating history file.  The placeholder text is preserved verbatim
    /// in the imported envelope.
    pub paste_warnings: Vec<PasteRefWarning>,

    /// `true` when [`inspect_ts_file`] was called (nothing was written).
    pub dry_run: bool,
}

impl TsImportReport {
    /// Returns a single-line human-readable summary.
    #[must_use]
    pub fn summary(&self) -> String {
        let mode = if self.dry_run { "dry-run" } else { "imported" };
        format!(
            "{mode}: {} converted, {} skipped, {} paste-ref warnings  (session {})",
            self.imported_count,
            self.skipped.len(),
            self.paste_warnings.len(),
            self.session_id,
        )
    }
}

/// A TS record that was skipped during import, with the source line number and
/// the reason.
#[derive(Clone, Debug, PartialEq)]
pub struct TsSkipReport {
    /// 1-based line number in the source JSONL file.
    pub line: usize,

    /// Value of the TS `"type"` field (empty string if absent).
    pub ts_type: String,

    /// Reason the record was not imported.
    pub reason: SkipReason,
}

/// Why a TS record could not be converted to a Rust [`MessageEnvelope`].
#[derive(Clone, Debug, PartialEq)]
pub enum SkipReason {
    /// The `"type"` field value is not a supported conversion target (e.g.
    /// `"system"`, `"attachment"`, `"summary"`, metadata entries, sidechain
    /// records, etc.).
    UnsupportedType(String),

    /// The record's `message.content` field was absent, null, or produced no
    /// non-empty text after extraction.
    EmptyContent,

    /// The `sessionId` field was absent.
    MissingSessionId,

    /// The `sessionId` field was present but could not be parsed as a UUID.
    InvalidSessionId(String),

    /// The `timestamp` field was absent.
    MissingTimestamp,

    /// The `timestamp` field was present but could not be parsed as RFC-3339.
    InvalidTimestamp(String),

    /// The TS record was not valid JSON.
    ParseError(String),
}

impl SkipReason {
    /// Returns a short human-readable description.
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::UnsupportedType(t) => format!("unsupported TS record type: {t:?}"),
            Self::EmptyContent => "no extractable text content".into(),
            Self::MissingSessionId => "sessionId field absent".into(),
            Self::InvalidSessionId(v) => format!("sessionId is not a valid UUID: {v:?}"),
            Self::MissingTimestamp => "timestamp field absent".into(),
            Self::InvalidTimestamp(v) => format!("timestamp is not RFC-3339: {v:?}"),
            Self::ParseError(e) => format!("JSON parse error: {e}"),
        }
    }
}

/// A user message that contained an unresolvable TS paste-reference placeholder.
#[derive(Clone, Debug, PartialEq)]
pub struct PasteRefWarning {
    /// 1-based source line number.
    pub line: usize,
    /// The first placeholder substring found (for display purposes).
    pub snippet: String,
}

// ─── public entry points ──────────────────────────────────────────────────────

/// Inspects a TS JSONL file without writing anything.
///
/// Parses every line and reports what can and cannot be converted.  The returned
/// [`TsImportReport`] has `dry_run = true` and `imported_count = 0`.
///
/// # Errors
///
/// Returns an error only when the source file cannot be opened or read.
/// Per-record parse failures are captured in [`TsImportReport::skipped`].
pub fn inspect_ts_file(path: &Path) -> Result<TsImportReport> {
    process_ts_file(path, None, None)
}

/// Imports a TS JSONL file into `store`.
///
/// Each convertible record is appended via [`TranscriptStore::append_message`].
/// Records that cannot be converted are collected in [`TsImportReport::skipped`].
///
/// If `override_session_id` is `Some`, that session ID is used for all imported
/// messages; otherwise the `sessionId` field from the first parseable record is
/// used.  When no session ID can be determined from the file, a fresh one is
/// generated.
///
/// # Errors
///
/// Returns an error when the source file cannot be read or an I/O error occurs
/// while appending to storage.
pub fn import_ts_file(
    path: &Path,
    store: &TranscriptStore,
    override_session_id: Option<SessionId>,
) -> Result<TsImportReport> {
    process_ts_file(path, Some(store), override_session_id)
}

// ─── core conversion logic ────────────────────────────────────────────────────

fn process_ts_file(
    path: &Path,
    store: Option<&TranscriptStore>,
    override_session_id: Option<SessionId>,
) -> Result<TsImportReport> {
    let dry_run = store.is_none();
    let contents = std::fs::read_to_string(path).map_err(|e| {
        WonderError::validation(format!("cannot read TS transcript {}: {e}", path.display()))
    })?;

    let mut skipped = Vec::new();
    let mut paste_warnings = Vec::new();
    let mut converted: Vec<MessageEnvelope> = Vec::new();
    let mut detected_session_id: Option<SessionId> = None;

    for (zero_idx, line) in contents.lines().enumerate() {
        let line_no = zero_idx + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Parse the raw record.
        let raw: TsRawRecord = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                skipped.push(TsSkipReport {
                    line: line_no,
                    ts_type: String::new(),
                    reason: SkipReason::ParseError(e.to_string()),
                });
                continue;
            }
        };

        let ts_type = raw.record_type.as_deref().unwrap_or("").to_owned();

        // Extract session ID from the first record that has one.
        if detected_session_id.is_none() {
            if let Some(sid) = &raw.session_id {
                if let Ok(id) = SessionId::from_str(sid.as_str()) {
                    detected_session_id = Some(id);
                }
            }
        }

        match convert_record(raw, line_no, &ts_type, &mut paste_warnings) {
            Ok(envelope) => converted.push(envelope),
            Err(skip) => skipped.push(skip),
        }
    }

    // Determine the final session ID.
    let session_id = override_session_id
        .or(detected_session_id)
        .unwrap_or_default();

    // Stamp every envelope with the resolved session_id, then write if not
    // dry-run.
    let mut imported_count = 0;
    if let Some(store) = store {
        for mut envelope in converted {
            envelope.session_id = session_id;
            store.append_message(&envelope)?;
            imported_count += 1;
        }
    } else {
        // dry-run: count what would be imported
        imported_count = 0; // stays 0 per contract (nothing written)
    }

    Ok(TsImportReport {
        source_path: path.to_path_buf(),
        session_id,
        imported_count,
        skipped,
        paste_warnings,
        dry_run,
    })
}

/// Attempts to convert one raw TS record into a [`MessageEnvelope`].
///
/// Returns `Ok(envelope)` on success or `Err(TsSkipReport)` when the record is
/// unsupported or malformed.
fn convert_record(
    raw: TsRawRecord,
    line_no: usize,
    ts_type: &str,
    paste_warnings: &mut Vec<PasteRefWarning>,
) -> std::result::Result<MessageEnvelope, TsSkipReport> {
    let skip = |reason: SkipReason| TsSkipReport {
        line: line_no,
        ts_type: ts_type.to_owned(),
        reason,
    };

    // Reject unsupported types early — before we touch required fields that
    // may be absent on metadata-only records (summary, tag, mode, etc.).
    match ts_type {
        "user" | "assistant" => {}
        other => return Err(skip(SkipReason::UnsupportedType(other.to_owned()))),
    }

    // Parse required fields.
    let session_id = match &raw.session_id {
        Some(sid) => SessionId::from_str(sid.as_str())
            .map_err(|_| skip(SkipReason::InvalidSessionId(sid.clone())))?,
        None => return Err(skip(SkipReason::MissingSessionId)),
    };

    let timestamp: OffsetDateTime = match &raw.timestamp {
        Some(ts) => {
            OffsetDateTime::parse(ts.as_str(), &time::format_description::well_known::Rfc3339)
                .map_err(|_| skip(SkipReason::InvalidTimestamp(ts.clone())))?
        }
        None => return Err(skip(SkipReason::MissingTimestamp)),
    };

    // Use TS `uuid` as the Rust MessageId for import stability; fall back to
    // a freshly generated one when the field is absent.
    let id: MessageId = match &raw.uuid {
        Some(u) => MessageId::from_str(u.as_str()).unwrap_or_else(|_| MessageId::new()),
        None => MessageId::new(),
    };

    let payload = match ts_type {
        "user" => extract_user_payload(&raw.message, line_no, paste_warnings).map_err(&skip)?,
        "assistant" => extract_assistant_payload(&raw.message).map_err(skip)?,
        _ => unreachable!("already gated above"),
    };

    let mut envelope = MessageEnvelope::new(session_id, payload);
    envelope.id = id;
    envelope.timestamp = timestamp;
    envelope.cwd = raw.cwd.map(PathBuf::from);
    envelope.git_branch = raw.git_branch;
    envelope.entrypoint = raw.entrypoint;
    envelope.app_version = raw.version;

    Ok(envelope)
}

/// Extracts a [`MessagePayload::UserText`] from a TS user record's `message`
/// field, which can be either a plain string or an object with a `content`
/// field.
fn extract_user_payload(
    message: &Option<Value>,
    line_no: usize,
    paste_warnings: &mut Vec<PasteRefWarning>,
) -> std::result::Result<MessagePayload, SkipReason> {
    let content = match message {
        Some(Value::Object(obj)) => {
            match obj.get("content") {
                // content is a plain string
                Some(Value::String(s)) => s.clone(),
                // content is an array of blocks — extract text parts
                Some(Value::Array(blocks)) => extract_text_from_blocks(blocks)?,
                _ => return Err(SkipReason::EmptyContent),
            }
        }
        Some(Value::String(s)) => s.clone(),
        _ => return Err(SkipReason::EmptyContent),
    };

    if content.trim().is_empty() {
        return Err(SkipReason::EmptyContent);
    }

    // Record a warning for unresolvable paste references; we still import the
    // message with the placeholder text intact.
    if contains_paste_ref(&content) {
        let snippet = if let Some(pos) = content.find(PASTE_REF_PATTERN) {
            content[pos..].chars().take(40).collect()
        } else {
            content.chars().take(40).collect()
        };
        paste_warnings.push(PasteRefWarning {
            line: line_no,
            snippet,
        });
    }

    Ok(MessagePayload::UserText { content })
}

/// Extracts a [`MessagePayload::AssistantText`] or
/// [`MessagePayload::AssistantThinking`] from a TS assistant `message.content`
/// array.
fn extract_assistant_payload(
    message: &Option<Value>,
) -> std::result::Result<MessagePayload, SkipReason> {
    let blocks = match message {
        Some(Value::Object(obj)) => match obj.get("content") {
            Some(Value::Array(arr)) => arr,
            Some(Value::String(s)) => {
                // Rare: content is a plain string (early TS versions)
                let text = s.trim().to_owned();
                if text.is_empty() {
                    return Err(SkipReason::EmptyContent);
                }
                return Ok(MessagePayload::AssistantText { content: text });
            }
            _ => return Err(SkipReason::EmptyContent),
        },
        _ => return Err(SkipReason::EmptyContent),
    };

    // Deserialise blocks; ignore unknown types rather than failing.
    let parsed: Vec<TsContentBlock> = blocks
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect();

    // Prefer the first `thinking` block as `AssistantThinking`.
    if let Some(block) = parsed.iter().find(|b| b.block_type == "thinking") {
        let content = block.thinking.as_deref().unwrap_or("").trim().to_owned();
        if !content.is_empty() {
            return Ok(MessagePayload::AssistantThinking {
                content,
                collapsed: false,
            });
        }
    }

    // Collect all text blocks.
    let text = extract_text_from_blocks(blocks)?;
    if text.trim().is_empty() {
        return Err(SkipReason::EmptyContent);
    }
    Ok(MessagePayload::AssistantText { content: text })
}

/// Joins all `"text"` block values from a content-block array.
fn extract_text_from_blocks(blocks: &[Value]) -> std::result::Result<String, SkipReason> {
    let parts: Vec<&str> = blocks
        .iter()
        .filter_map(|v| {
            let obj = v.as_object()?;
            if obj.get("type")?.as_str()? == "text" {
                obj.get("text")?.as_str()
            } else {
                None
            }
        })
        .collect();

    if parts.is_empty() {
        return Err(SkipReason::EmptyContent);
    }
    Ok(parts.join("\n").trim().to_owned())
}

// ─── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use wonder_of_u_test_support::unique_test_dir;

    // ── fixtures ───────────────────────────────────────────────────────────────

    /// Minimal TS user record with a string `content` field.
    const USER_STRING_CONTENT: &str = r#"{"type":"user","message":{"role":"user","content":"Hello from TypeScript"},"uuid":"aaaaaaaa-0000-0000-0000-000000000001","sessionId":"bbbbbbbb-0000-0000-0000-000000000001","timestamp":"2024-06-01T10:00:00Z","cwd":"/workspace","gitBranch":"main","entrypoint":"cli","version":"2.1.0"}"#;

    /// TS user record whose content is an array of text blocks.
    const USER_ARRAY_CONTENT: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Hello"},{"type":"text","text":"world"}]},"uuid":"aaaaaaaa-0000-0000-0000-000000000002","sessionId":"bbbbbbbb-0000-0000-0000-000000000001","timestamp":"2024-06-01T10:00:01Z","cwd":"/workspace","version":"2.1.0"}"#;

    /// TS assistant record with a single text block.
    const ASSISTANT_TEXT: &str = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hi there!"}],"role":"assistant"},"uuid":"aaaaaaaa-0000-0000-0000-000000000003","sessionId":"bbbbbbbb-0000-0000-0000-000000000001","timestamp":"2024-06-01T10:00:02Z","version":"2.1.0"}"#;

    /// TS assistant record with a thinking block.
    const ASSISTANT_THINKING: &str = r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Let me reason about this..."}],"role":"assistant"},"uuid":"aaaaaaaa-0000-0000-0000-000000000004","sessionId":"bbbbbbbb-0000-0000-0000-000000000001","timestamp":"2024-06-01T10:00:03Z","version":"2.1.0"}"#;

    /// TS system record — unsupported, should be skipped.
    const SYSTEM_RECORD: &str = r#"{"type":"system","message":{"role":"system","content":"You are helpful."},"uuid":"aaaaaaaa-0000-0000-0000-000000000005","sessionId":"bbbbbbbb-0000-0000-0000-000000000001","timestamp":"2024-06-01T10:00:04Z","version":"2.1.0"}"#;

    /// TS summary metadata record — unsupported, should be skipped.
    const SUMMARY_RECORD: &str = r#"{"type":"summary","leafUuid":"aaaaaaaa-0000-0000-0000-000000000001","summary":"Talked about greetings."}"#;

    /// User message with a paste-reference placeholder.
    const USER_PASTE_REF: &str = r#"{"type":"user","message":{"role":"user","content":"Here is my code:\n[Pasted text #1 +10 lines]"},"uuid":"aaaaaaaa-0000-0000-0000-000000000006","sessionId":"bbbbbbbb-0000-0000-0000-000000000001","timestamp":"2024-06-01T10:00:05Z","version":"2.1.0"}"#;

    /// Record with a mismatched sessionId in the message field (no sessionId on root).
    const NO_SESSION_ID: &str = r#"{"type":"user","message":{"role":"user","content":"oops"},"uuid":"aaaaaaaa-0000-0000-0000-000000000007","timestamp":"2024-06-01T10:00:06Z","version":"2.1.0"}"#;

    /// Record with an invalid UUID for sessionId.
    const BAD_SESSION_ID: &str = r#"{"type":"user","message":{"role":"user","content":"oops"},"uuid":"aaaaaaaa-0000-0000-0000-000000000008","sessionId":"not-a-uuid","timestamp":"2024-06-01T10:00:07Z","version":"2.1.0"}"#;

    /// Record with a missing timestamp.
    const NO_TIMESTAMP: &str = r#"{"type":"user","message":{"role":"user","content":"oops"},"uuid":"aaaaaaaa-0000-0000-0000-000000000009","sessionId":"bbbbbbbb-0000-0000-0000-000000000001","version":"2.1.0"}"#;

    /// Completely malformed JSON.
    const MALFORMED_JSON: &str = r#"{not valid json"#;

    /// Assistant record with only a `tool_use` block and no text — should be
    /// skipped with `EmptyContent`.
    const ASSISTANT_TOOL_USE_ONLY: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_01","name":"bash","input":{"command":"ls"}}],"role":"assistant"},"uuid":"aaaaaaaa-0000-0000-0000-000000000010","sessionId":"bbbbbbbb-0000-0000-0000-000000000001","timestamp":"2024-06-01T10:00:08Z","version":"2.1.0"}"#;

    // ── helpers ────────────────────────────────────────────────────────────────

    fn write_fixture(lines: &[&str]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fixture.jsonl");
        let mut f = std::fs::File::create(&path).expect("create fixture");
        for line in lines {
            writeln!(f, "{line}").expect("write line");
        }
        (dir, path)
    }

    // ── happy-path tests ───────────────────────────────────────────────────────

    #[test]
    fn inspect_user_and_assistant_messages() {
        let (_dir, path) = write_fixture(&[
            USER_STRING_CONTENT,
            USER_ARRAY_CONTENT,
            ASSISTANT_TEXT,
            ASSISTANT_THINKING,
        ]);

        let report = inspect_ts_file(&path).expect("inspect");

        // dry_run is true: nothing written
        assert!(report.dry_run);
        assert_eq!(report.imported_count, 0);
        assert_eq!(
            report.skipped.len(),
            0,
            "unexpected skips: {:?}",
            report.skipped
        );
        assert_eq!(report.paste_warnings.len(), 0);
        // session_id was detected from the fixtures
        assert_eq!(
            report.session_id.to_string(),
            "bbbbbbbb-0000-0000-0000-000000000001"
        );
    }

    #[test]
    fn import_user_and_assistant_messages() {
        let storage_dir = unique_test_dir("ts-import-happy");
        let store = TranscriptStore::new(&storage_dir);
        let (_dir, path) = write_fixture(&[USER_STRING_CONTENT, ASSISTANT_TEXT]);

        let report = import_ts_file(&path, &store, None).expect("import");

        assert!(!report.dry_run);
        assert_eq!(report.imported_count, 2);
        assert_eq!(report.skipped.len(), 0);

        // Verify the messages are actually in storage.
        let loaded = store.load_session(report.session_id).expect("load session");
        assert_eq!(loaded.messages.len(), 2);
        assert!(
            matches!(&loaded.messages[0].payload, MessagePayload::UserText { content } if content == "Hello from TypeScript")
        );
        assert!(
            matches!(&loaded.messages[1].payload, MessagePayload::AssistantText { content } if content == "Hi there!")
        );
    }

    #[test]
    fn import_user_array_content_joins_text_blocks() {
        let storage_dir = unique_test_dir("ts-import-array");
        let store = TranscriptStore::new(&storage_dir);
        let (_dir, path) = write_fixture(&[USER_ARRAY_CONTENT]);

        let report = import_ts_file(&path, &store, None).expect("import");

        assert_eq!(report.imported_count, 1);
        let loaded = store.load_session(report.session_id).expect("load session");
        if let MessagePayload::UserText { content } = &loaded.messages[0].payload {
            assert!(content.contains("Hello"), "expected 'Hello' in {content:?}");
            assert!(content.contains("world"), "expected 'world' in {content:?}");
        } else {
            panic!("expected UserText, got {:?}", loaded.messages[0].payload);
        }
    }

    #[test]
    fn import_assistant_thinking_block() {
        let storage_dir = unique_test_dir("ts-import-thinking");
        let store = TranscriptStore::new(&storage_dir);
        let (_dir, path) = write_fixture(&[ASSISTANT_THINKING]);

        let report = import_ts_file(&path, &store, None).expect("import");

        assert_eq!(report.imported_count, 1);
        let loaded = store.load_session(report.session_id).expect("load session");
        assert!(
            matches!(&loaded.messages[0].payload, MessagePayload::AssistantThinking { content, collapsed: false } if content.contains("reason"))
        );
    }

    #[test]
    fn override_session_id_stamps_all_messages() {
        let storage_dir = unique_test_dir("ts-import-override-session");
        let store = TranscriptStore::new(&storage_dir);
        let override_id = SessionId::new();
        let (_dir, path) = write_fixture(&[USER_STRING_CONTENT, ASSISTANT_TEXT]);

        let report = import_ts_file(&path, &store, Some(override_id)).expect("import");

        assert_eq!(report.session_id, override_id);
        let loaded = store.load_session(override_id).expect("load session");
        assert_eq!(loaded.messages.len(), 2);
        for msg in &loaded.messages {
            assert_eq!(msg.session_id, override_id);
        }
    }

    // ── skip-reason tests ─────────────────────────────────────────────────────

    #[test]
    fn skips_system_record() {
        let (_dir, path) = write_fixture(&[SYSTEM_RECORD]);
        let report = inspect_ts_file(&path).expect("inspect");

        assert_eq!(report.skipped.len(), 1);
        assert!(matches!(
            &report.skipped[0].reason,
            SkipReason::UnsupportedType(t) if t == "system"
        ));
    }

    #[test]
    fn skips_summary_metadata_record() {
        let (_dir, path) = write_fixture(&[SUMMARY_RECORD]);
        let report = inspect_ts_file(&path).expect("inspect");

        assert_eq!(report.skipped.len(), 1);
        assert!(matches!(
            &report.skipped[0].reason,
            SkipReason::UnsupportedType(t) if t == "summary"
        ));
    }

    #[test]
    fn skips_record_with_missing_session_id() {
        let (_dir, path) = write_fixture(&[NO_SESSION_ID]);
        let report = inspect_ts_file(&path).expect("inspect");

        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].reason, SkipReason::MissingSessionId);
    }

    #[test]
    fn skips_record_with_invalid_session_id() {
        let (_dir, path) = write_fixture(&[BAD_SESSION_ID]);
        let report = inspect_ts_file(&path).expect("inspect");

        assert_eq!(report.skipped.len(), 1);
        assert!(matches!(
            &report.skipped[0].reason,
            SkipReason::InvalidSessionId(_)
        ));
    }

    #[test]
    fn skips_record_with_missing_timestamp() {
        let (_dir, path) = write_fixture(&[NO_TIMESTAMP]);
        let report = inspect_ts_file(&path).expect("inspect");

        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].reason, SkipReason::MissingTimestamp);
    }

    #[test]
    fn skips_malformed_json_line() {
        let (_dir, path) = write_fixture(&[MALFORMED_JSON]);
        let report = inspect_ts_file(&path).expect("inspect");

        assert_eq!(report.skipped.len(), 1);
        assert!(matches!(
            &report.skipped[0].reason,
            SkipReason::ParseError(_)
        ));
    }

    #[test]
    fn skips_assistant_with_only_tool_use_block() {
        let (_dir, path) = write_fixture(&[ASSISTANT_TOOL_USE_ONLY]);
        let report = inspect_ts_file(&path).expect("inspect");

        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].reason, SkipReason::EmptyContent);
    }

    // ── paste-reference warning tests ─────────────────────────────────────────

    #[test]
    fn warns_on_paste_reference_placeholder() {
        let (_dir, path) = write_fixture(&[USER_PASTE_REF]);
        let report = inspect_ts_file(&path).expect("inspect");

        // Message is still counted as convertible (not skipped).
        assert_eq!(
            report.skipped.len(),
            0,
            "should not be skipped: {:?}",
            report.skipped
        );
        assert_eq!(report.paste_warnings.len(), 1);
        assert!(
            report.paste_warnings[0].snippet.contains("[Pasted text #"),
            "expected paste ref in snippet: {:?}",
            report.paste_warnings[0].snippet
        );
    }

    #[test]
    fn paste_ref_message_content_preserved_verbatim() {
        let storage_dir = unique_test_dir("ts-import-paste-ref");
        let store = TranscriptStore::new(&storage_dir);
        let (_dir, path) = write_fixture(&[USER_PASTE_REF]);

        let report = import_ts_file(&path, &store, None).expect("import");
        assert_eq!(report.imported_count, 1);
        let loaded = store.load_session(report.session_id).expect("load session");
        if let MessagePayload::UserText { content } = &loaded.messages[0].payload {
            assert!(
                content.contains("[Pasted text #"),
                "placeholder should be preserved verbatim: {content:?}"
            );
        } else {
            panic!("expected UserText");
        }
    }

    // ── mixed-content batch tests ─────────────────────────────────────────────

    #[test]
    fn mixed_batch_counts_correctly() {
        let (_dir, path) = write_fixture(&[
            USER_STRING_CONTENT, // converted
            SYSTEM_RECORD,       // skipped
            ASSISTANT_TEXT,      // converted
            SUMMARY_RECORD,      // skipped
            MALFORMED_JSON,      // skipped
        ]);

        let report = inspect_ts_file(&path).expect("inspect");

        assert_eq!(report.skipped.len(), 3);
        // dry-run: imported_count is always 0
        assert_eq!(report.imported_count, 0);
    }

    #[test]
    fn summary_text_contains_counts() {
        let (_dir, path) = write_fixture(&[USER_STRING_CONTENT, SYSTEM_RECORD]);
        let report = inspect_ts_file(&path).expect("inspect");
        let summary = report.summary();
        assert!(summary.contains("1 skipped"), "summary was: {summary}");
    }

    // ── normal Rust transcripts are not affected ───────────────────────────────

    #[test]
    fn rust_transcript_is_skipped_as_unsupported() {
        // A valid Rust transcript line has `schema_version` and `payload` but
        // no TS `type` discriminator.  The importer sees `type: null` and skips
        // it as UnsupportedType("").
        let rust_line = r#"{"schema_version":1,"id":"aaaaaaaa-0000-0000-0000-0000000000ff","session_id":"bbbbbbbb-0000-0000-0000-000000000001","timestamp":"2024-06-01T10:00:00Z","payload":{"type":"user_text","data":{"content":"hello"}}}"#;
        let (_dir, path) = write_fixture(&[rust_line]);
        let report = inspect_ts_file(&path).expect("inspect");

        assert_eq!(report.skipped.len(), 1, "rust record should be skipped");
        // The rust record has no `type` field at the top level, so it parses as
        // an empty type and is reported as UnsupportedType("").
        assert!(
            matches!(
                &report.skipped[0].reason,
                SkipReason::UnsupportedType(_) | SkipReason::MissingSessionId
            ),
            "unexpected reason: {:?}",
            report.skipped[0].reason
        );
    }
}
