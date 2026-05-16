//! TypeScript-upstream transcript importer.
//!
//! Parses TS-shaped JSONL records produced by the upstream Claude Code TypeScript
//! client (`~/.claude/projects/*/`) and converts them to Rust [`MessageEnvelope`]
//! schema. This is a **one-shot migration path only**: the normal
//! [`TranscriptStore::load_session`] path does not convert TS records into Rust
//! envelopes and still rejects explicit Rust schema-version mismatches.
//!
//! # Supported conversions
//!
//! | TS `type` | TS `message.content` | Rust `MessagePayload` |
//! |-----------|----------------------|----------------------|
//! | `"user"`  | string               | `UserText`           |
//! | `"user"`  | `[{type:"text",...}]`| `UserText` (joined)  |
//! | `"assistant"` | `[{type:"text",...}]` | `AssistantText` |
//! | `"assistant"` | `[{type:"thinking",...}]` | `AssistantThinking` |
//! | `"assistant"` | `[{type:"tool_use",...}]` | `AssistantToolUse` |
//! | `"user"` | `[{type:"tool_result",...}]` | `ToolResult` |
//!
//! Everything else is skipped and reported in [`TsImportReport::skipped`].
//!
//! # Paste references
//!
//! The TS client embeds unresolvable references such as `[Pasted text #1 +5 lines]`
//! inside user messages.  This importer preserves the placeholder text as-is and
//! records a [`PasteRefWarning`] in [`TsImportReport::paste_warnings`]
//! so callers can surface the information without silent data loss.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    str::FromStr,
};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use wonder_of_u_core::Result;
use wonder_of_u_core::{
    MessageEnvelope, MessageId, MessagePayload, SessionId, ToolUseId, WonderError,
};

use crate::{SessionMetadata, TranscriptStore};

// ─── paste-reference detection ────────────────────────────────────────────────

/// Matches TS paste-reference placeholders such as `[Pasted text #1 +5 lines]`,
/// `[Pasted text #3]`, `[Image #2]`, `[...Truncated text #4]`.
const PASTE_REF_PATTERN: &str = "[Pasted text #";
const IMAGE_REF_PATTERN: &str = "[Image #";
const TRUNCATED_TEXT_PATTERN: &str = "[...Truncated text #";

fn contains_paste_ref(text: &str) -> bool {
    text.contains(PASTE_REF_PATTERN)
        || text.contains(IMAGE_REF_PATTERN)
        || text.contains(TRUNCATED_TEXT_PATTERN)
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

    /// TS summary metadata leaf UUID.
    #[serde(rename = "leafUuid")]
    leaf_uuid: Option<String>,

    /// TS summary metadata text.
    summary: Option<String>,

    /// TS custom-title metadata value.
    #[serde(rename = "customTitle")]
    custom_title: Option<String>,
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

    /// Number of messages that can be converted from the source file.
    pub convertible_count: usize,

    /// Records that could not be converted, with per-record explanations.
    pub skipped: Vec<TsSkipReport>,

    /// Per-record warnings for partially imported content.
    pub warnings: Vec<TsImportWarning>,

    /// User messages that contain TS paste-reference placeholders
    /// (`[Pasted text #N]`, `[Image #N]`) which cannot be resolved without the
    /// originating history file.  The placeholder text is preserved verbatim
    /// in the imported envelope.
    pub paste_warnings: Vec<PasteRefWarning>,

    /// `true` when [`inspect_ts_file`] was called (nothing was written).
    pub dry_run: bool,

    /// Metadata captured from TS-only records.
    pub captured_metadata: TsCapturedMetadata,
}

impl TsImportReport {
    /// Returns a single-line human-readable summary.
    #[must_use]
    pub fn summary(&self) -> String {
        let mode = if self.dry_run { "dry-run" } else { "imported" };
        let converted = if self.dry_run {
            self.convertible_count
        } else {
            self.imported_count
        };
        let skip_counts = format_count_summary(
            self.skip_counts()
                .into_iter()
                .map(|(category, count)| (category.label(), count)),
        );
        let warning_counts = format_count_summary(
            self.warning_counts()
                .into_iter()
                .map(|(category, count)| (category.label(), count)),
        );
        format!(
            "{mode}: {converted} converted, {} skipped{skip_counts}, {} warnings{warning_counts}, {} paste-ref warnings, {} metadata captures  (session {})",
            self.skipped.len(),
            self.warnings.len(),
            self.paste_warnings.len(),
            self.captured_metadata.captured_count(),
            self.session_id,
        )
    }

    /// Counts skipped records by broad category.
    #[must_use]
    pub fn skip_counts(&self) -> BTreeMap<SkipCategory, usize> {
        let mut counts = BTreeMap::new();
        for skip in &self.skipped {
            *counts.entry(skip.reason.category()).or_default() += 1;
        }
        counts
    }

    /// Counts partial-import warnings by category.
    #[must_use]
    pub fn warning_counts(&self) -> BTreeMap<TsWarningCategory, usize> {
        let mut counts = BTreeMap::new();
        for warning in &self.warnings {
            *counts.entry(warning.category).or_default() += 1;
        }
        counts
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

    /// The record had content blocks, but none could be mapped safely.
    UnsupportedContent {
        /// The source role (`"user"` or `"assistant"`).
        role: String,
        /// Content block types that could not be mapped.
        block_types: Vec<String>,
    },

    /// A tool result referenced a tool-use ID whose tool name could not be
    /// resolved from earlier assistant blocks.
    UnresolvedToolResult(Vec<String>),
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
            Self::UnsupportedContent { role, block_types } => format!(
                "{role} record contains only unsupported content blocks: {}",
                block_types.join(", ")
            ),
            Self::UnresolvedToolResult(ids) => format!(
                "tool_result block(s) reference unresolved tool_use_id value(s): {}",
                ids.join(", ")
            ),
        }
    }

    /// Groups skip reasons into stable reporting categories.
    #[must_use]
    pub fn category(&self) -> SkipCategory {
        match self {
            Self::UnsupportedType(ts_type) if is_metadata_type(ts_type) => SkipCategory::Metadata,
            Self::UnsupportedType(_) => SkipCategory::UnsupportedRecord,
            Self::EmptyContent | Self::UnsupportedContent { .. } => {
                SkipCategory::UnsupportedContent
            }
            Self::MissingSessionId | Self::MissingTimestamp => SkipCategory::MissingField,
            Self::InvalidSessionId(_) | Self::InvalidTimestamp(_) | Self::ParseError(_) => {
                SkipCategory::InvalidRecord
            }
            Self::UnresolvedToolResult(_) => SkipCategory::UnresolvedReference,
        }
    }
}

/// Broad categories for skipped TS records.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SkipCategory {
    /// TS metadata-only records that are not imported as transcript messages.
    Metadata,
    /// Unsupported top-level transcript record types.
    UnsupportedRecord,
    /// Content existed but could not be represented safely.
    UnsupportedContent,
    /// Required fields were absent.
    MissingField,
    /// JSON or scalar fields were malformed.
    InvalidRecord,
    /// A cross-record linkage could not be resolved safely.
    UnresolvedReference,
}

impl SkipCategory {
    fn label(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::UnsupportedRecord => "unsupported_record",
            Self::UnsupportedContent => "unsupported_content",
            Self::MissingField => "missing_field",
            Self::InvalidRecord => "invalid_record",
            Self::UnresolvedReference => "unresolved_reference",
        }
    }
}

/// Warning emitted when a TS record is only partially imported.
#[derive(Clone, Debug, PartialEq)]
pub struct TsImportWarning {
    /// 1-based source line number.
    pub line: usize,
    /// Broad warning category.
    pub category: TsWarningCategory,
    /// Human-readable warning text.
    pub message: String,
}

/// Broad categories for partial-import warnings.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TsWarningCategory {
    /// Some blocks from an otherwise imported record were dropped or degraded.
    PartialImport,
    /// Metadata was captured but could not be written back into Rust storage.
    Metadata,
}

impl TsWarningCategory {
    fn label(self) -> &'static str {
        match self {
            Self::PartialImport => "partial_import",
            Self::Metadata => "metadata",
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

/// TS metadata captured during import or inspection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TsCapturedMetadata {
    /// Last seen custom title for the session, if present.
    pub custom_title: Option<String>,
    /// Captured per-leaf summaries.
    pub summaries: Vec<TsLeafSummary>,
}

impl TsCapturedMetadata {
    /// Returns the number of captured metadata records represented in this report.
    #[must_use]
    pub fn captured_count(&self) -> usize {
        usize::from(self.custom_title.is_some()) + self.summaries.len()
    }
}

/// TS summary metadata retained in the import report.
#[derive(Clone, Debug, PartialEq)]
pub struct TsLeafSummary {
    /// 1-based source line number.
    pub line: usize,
    /// TS `leafUuid`.
    pub leaf_uuid: String,
    /// Summary text.
    pub summary: String,
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
    let mut warnings = Vec::new();
    let mut converted: Vec<MessageEnvelope> = Vec::new();
    let mut detected_session_id: Option<SessionId> = None;
    let mut captured_metadata = TsCapturedMetadata::default();
    let mut tool_names = BTreeMap::new();
    let mut observed = ObservedSessionContext::default();

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

        match convert_record(
            raw,
            line_no,
            &ts_type,
            &mut paste_warnings,
            &mut warnings,
            &mut captured_metadata,
            &mut tool_names,
        ) {
            Ok(mut envelopes) => {
                for envelope in &envelopes {
                    observed.observe(envelope);
                }
                converted.append(&mut envelopes);
            }
            Err(skip) => skipped.push(skip),
        }
    }

    // Determine the final session ID.
    let session_id = override_session_id
        .or(detected_session_id)
        .unwrap_or_default();

    let convertible_count = converted.len();

    // Stamp every envelope with the resolved session_id, then write if not
    // dry-run.
    let mut imported_count = 0;
    if let Some(store) = store {
        for mut envelope in converted {
            envelope.session_id = session_id;
            store.append_message(&envelope)?;
            imported_count += 1;
        }

        maybe_write_import_metadata(
            store,
            session_id,
            imported_count,
            &observed,
            &captured_metadata,
            &mut warnings,
        )?;
    } else {
        // dry-run: count what would be imported
        imported_count = 0; // stays 0 per contract (nothing written)
    }

    Ok(TsImportReport {
        source_path: path.to_path_buf(),
        session_id,
        imported_count,
        convertible_count,
        skipped,
        warnings,
        paste_warnings,
        dry_run,
        captured_metadata,
    })
}

/// Attempts to convert one raw TS record into zero or more [`MessageEnvelope`]s.
///
/// Returns `Ok(envelopes)` on success or `Err(TsSkipReport)` when the record is
/// unsupported or malformed.
fn convert_record(
    raw: TsRawRecord,
    line_no: usize,
    ts_type: &str,
    paste_warnings: &mut Vec<PasteRefWarning>,
    warnings: &mut Vec<TsImportWarning>,
    captured_metadata: &mut TsCapturedMetadata,
    tool_names: &mut BTreeMap<String, String>,
) -> std::result::Result<Vec<MessageEnvelope>, TsSkipReport> {
    let skip = |reason: SkipReason| TsSkipReport {
        line: line_no,
        ts_type: ts_type.to_owned(),
        reason,
    };

    match ts_type {
        "summary" => {
            if let (Some(leaf_uuid), Some(summary)) = (raw.leaf_uuid, raw.summary) {
                if !summary.trim().is_empty() {
                    captured_metadata.summaries.push(TsLeafSummary {
                        line: line_no,
                        leaf_uuid,
                        summary,
                    });
                }
            }
            return Ok(Vec::new());
        }
        "custom-title" => {
            if let Some(custom_title) = raw.custom_title {
                let trimmed = custom_title.trim();
                if !trimmed.is_empty() {
                    captured_metadata.custom_title = Some(trimmed.to_owned());
                }
            }
            return Ok(Vec::new());
        }
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

    let payloads = match ts_type {
        "user" => {
            extract_user_payloads(&raw.message, line_no, paste_warnings, warnings, tool_names)
                .map_err(&skip)?
        }
        "assistant" => {
            extract_assistant_payloads(&raw.message, line_no, warnings, tool_names).map_err(skip)?
        }
        _ => unreachable!("already gated above"),
    };

    let mut envelopes = Vec::with_capacity(payloads.len());
    for (index, payload) in payloads.into_iter().enumerate() {
        let mut envelope = MessageEnvelope::new(session_id, payload);
        envelope.id = if index == 0 {
            id
        } else {
            derived_message_id(raw.uuid.as_deref(), index)
        };
        envelope.timestamp = timestamp;
        envelope.cwd = raw.cwd.clone().map(PathBuf::from);
        envelope.git_branch = raw.git_branch.clone();
        envelope.entrypoint = raw.entrypoint.clone();
        envelope.app_version = raw.version.clone();
        envelopes.push(envelope);
    }

    Ok(envelopes)
}

/// Extracts one or more user-facing payloads from a TS user record.
fn extract_user_payloads(
    message: &Option<Value>,
    line_no: usize,
    paste_warnings: &mut Vec<PasteRefWarning>,
    warnings: &mut Vec<TsImportWarning>,
    tool_names: &BTreeMap<String, String>,
) -> std::result::Result<Vec<MessagePayload>, SkipReason> {
    match message {
        Some(Value::Object(obj)) => match obj.get("content") {
            Some(Value::String(content)) => {
                let payload = user_text_payload(content.clone(), line_no, paste_warnings)?;
                Ok(vec![payload])
            }
            Some(Value::Array(blocks)) => {
                extract_user_block_payloads(blocks, line_no, paste_warnings, warnings, tool_names)
            }
            _ => Err(SkipReason::EmptyContent),
        },
        Some(Value::String(content)) => {
            let payload = user_text_payload(content.clone(), line_no, paste_warnings)?;
            Ok(vec![payload])
        }
        _ => Err(SkipReason::EmptyContent),
    }
}

/// Extracts one or more assistant-side payloads from a TS assistant record.
fn extract_assistant_payloads(
    message: &Option<Value>,
    line_no: usize,
    warnings: &mut Vec<TsImportWarning>,
    tool_names: &mut BTreeMap<String, String>,
) -> std::result::Result<Vec<MessagePayload>, SkipReason> {
    let blocks = match message {
        Some(Value::Object(obj)) => match obj.get("content") {
            Some(Value::Array(arr)) => arr.as_slice(),
            Some(Value::String(s)) => {
                // Rare: content is a plain string (early TS versions)
                let text = s.trim().to_owned();
                if text.is_empty() {
                    return Err(SkipReason::EmptyContent);
                }
                return Ok(vec![MessagePayload::AssistantText { content: text }]);
            }
            _ => return Err(SkipReason::EmptyContent),
        },
        _ => return Err(SkipReason::EmptyContent),
    };

    let mut payloads = Vec::new();
    let mut pending_text = Vec::new();
    let mut unsupported = Vec::new();

    for block in blocks {
        let Some(obj) = block.as_object() else {
            unsupported.push("<non-object>".to_owned());
            continue;
        };

        match obj.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = obj.get("text").and_then(Value::as_str) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        pending_text.push(trimmed.to_owned());
                    }
                }
            }
            Some("thinking") => {
                flush_assistant_text(&mut pending_text, &mut payloads);
                if let Some(thinking) = obj.get("thinking").and_then(Value::as_str) {
                    let trimmed = thinking.trim();
                    if !trimmed.is_empty() {
                        payloads.push(MessagePayload::AssistantThinking {
                            content: trimmed.to_owned(),
                            collapsed: false,
                        });
                    }
                }
            }
            Some("tool_use") => {
                flush_assistant_text(&mut pending_text, &mut payloads);
                let Some(raw_use_id) = obj.get("id").and_then(Value::as_str) else {
                    unsupported.push("tool_use(missing id)".to_owned());
                    continue;
                };
                let Some(tool_name) = obj.get("name").and_then(Value::as_str) else {
                    unsupported.push("tool_use(missing name)".to_owned());
                    continue;
                };
                tool_names.insert(raw_use_id.to_owned(), tool_name.to_owned());
                payloads.push(MessagePayload::AssistantToolUse {
                    tool: tool_name.to_owned(),
                    use_id: derived_tool_use_id(raw_use_id),
                    input: obj.get("input").cloned().unwrap_or(Value::Null),
                });
            }
            Some(other) => unsupported.push(other.to_owned()),
            None => unsupported.push("<missing type>".to_owned()),
        }
    }

    flush_assistant_text(&mut pending_text, &mut payloads);
    record_partial_import_warning(
        warnings,
        line_no,
        "assistant",
        unsupported.as_slice(),
        payloads.is_empty(),
    );

    if payloads.is_empty() {
        return if unsupported.is_empty() {
            Err(SkipReason::EmptyContent)
        } else {
            Err(SkipReason::UnsupportedContent {
                role: "assistant".to_owned(),
                block_types: unsupported,
            })
        };
    }

    Ok(payloads)
}

fn extract_user_block_payloads(
    blocks: &[Value],
    line_no: usize,
    paste_warnings: &mut Vec<PasteRefWarning>,
    warnings: &mut Vec<TsImportWarning>,
    tool_names: &BTreeMap<String, String>,
) -> std::result::Result<Vec<MessagePayload>, SkipReason> {
    let mut payloads = Vec::new();
    let mut pending_text = Vec::new();
    let mut unsupported = Vec::new();
    let mut unresolved_tool_results = Vec::new();

    for block in blocks {
        let Some(obj) = block.as_object() else {
            unsupported.push("<non-object>".to_owned());
            continue;
        };

        match obj.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = obj.get("text").and_then(Value::as_str) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        pending_text.push(trimmed.to_owned());
                    }
                }
            }
            Some("image") => pending_text.push(render_image_placeholder(obj)),
            Some("tool_result") => {
                flush_user_text(&mut pending_text, &mut payloads, line_no, paste_warnings)?;
                let Some(raw_use_id) = obj.get("tool_use_id").and_then(Value::as_str) else {
                    unsupported.push("tool_result(missing tool_use_id)".to_owned());
                    continue;
                };
                let Some(tool_name) = tool_names.get(raw_use_id) else {
                    unresolved_tool_results.push(raw_use_id.to_owned());
                    continue;
                };
                payloads.push(MessagePayload::ToolResult {
                    tool: tool_name.clone(),
                    use_id: derived_tool_use_id(raw_use_id),
                    success: !obj
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    content: render_tool_result_content(
                        obj.get("content"),
                        line_no,
                        warnings,
                        raw_use_id,
                    ),
                });
            }
            Some(other) => unsupported.push(other.to_owned()),
            None => unsupported.push("<missing type>".to_owned()),
        }
    }

    flush_user_text(&mut pending_text, &mut payloads, line_no, paste_warnings)?;
    record_partial_import_warning(
        warnings,
        line_no,
        "user",
        unsupported.as_slice(),
        payloads.is_empty(),
    );

    if !unresolved_tool_results.is_empty() && !payloads.is_empty() {
        warnings.push(TsImportWarning {
            line: line_no,
            category: TsWarningCategory::PartialImport,
            message: format!(
                "user record dropped tool_result block(s) with unresolved tool_use_id value(s): {}",
                unresolved_tool_results.join(", ")
            ),
        });
    }

    if payloads.is_empty() {
        if !unresolved_tool_results.is_empty() {
            return Err(SkipReason::UnresolvedToolResult(unresolved_tool_results));
        }
        return if unsupported.is_empty() {
            Err(SkipReason::EmptyContent)
        } else {
            Err(SkipReason::UnsupportedContent {
                role: "user".to_owned(),
                block_types: unsupported,
            })
        };
    }

    Ok(payloads)
}

fn user_text_payload(
    content: String,
    line_no: usize,
    paste_warnings: &mut Vec<PasteRefWarning>,
) -> std::result::Result<MessagePayload, SkipReason> {
    let content = content.trim().to_owned();
    if content.is_empty() {
        return Err(SkipReason::EmptyContent);
    }
    record_paste_warning(&content, line_no, paste_warnings);
    Ok(MessagePayload::UserText { content })
}

fn flush_user_text(
    pending_text: &mut Vec<String>,
    payloads: &mut Vec<MessagePayload>,
    line_no: usize,
    paste_warnings: &mut Vec<PasteRefWarning>,
) -> std::result::Result<(), SkipReason> {
    let joined = pending_text.join("\n").trim().to_owned();
    pending_text.clear();
    if joined.is_empty() {
        return Ok(());
    }
    payloads.push(user_text_payload(joined, line_no, paste_warnings)?);
    Ok(())
}

fn flush_assistant_text(pending_text: &mut Vec<String>, payloads: &mut Vec<MessagePayload>) {
    let joined = pending_text.join("\n").trim().to_owned();
    pending_text.clear();
    if joined.is_empty() {
        return;
    }
    payloads.push(MessagePayload::AssistantText { content: joined });
}

fn render_tool_result_content(
    content: Option<&Value>,
    line_no: usize,
    warnings: &mut Vec<TsImportWarning>,
    raw_use_id: &str,
) -> String {
    match content {
        Some(Value::String(text)) => text.trim().to_owned(),
        Some(Value::Array(blocks)) => {
            let mut parts = Vec::new();
            let mut degraded = Vec::new();

            for block in blocks {
                let Some(obj) = block.as_object() else {
                    degraded.push("<non-object>".to_owned());
                    continue;
                };

                match obj.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = obj.get("text").and_then(Value::as_str) {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                parts.push(trimmed.to_owned());
                            }
                        }
                    }
                    Some("image") => parts.push(render_image_placeholder(obj)),
                    Some(other) => {
                        degraded.push(other.to_owned());
                        parts.push(format!("[tool_result {other} block]"));
                    }
                    None => {
                        degraded.push("<missing type>".to_owned());
                        parts.push("[tool_result block]".to_owned());
                    }
                }
            }

            if !degraded.is_empty() {
                warnings.push(TsImportWarning {
                    line: line_no,
                    category: TsWarningCategory::PartialImport,
                    message: format!(
                        "tool_result for tool_use_id {raw_use_id} used placeholder text for block type(s): {}",
                        degraded.join(", ")
                    ),
                });
            }

            parts.join("\n").trim().to_owned()
        }
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn render_image_placeholder(obj: &serde_json::Map<String, Value>) -> String {
    let media_type = obj
        .get("source")
        .and_then(Value::as_object)
        .and_then(|source| source.get("media_type"))
        .and_then(Value::as_str);
    let filename = obj.get("filename").and_then(Value::as_str);
    match (filename, media_type) {
        (Some(filename), Some(media_type)) => format!("[Image: {filename} ({media_type})]"),
        (Some(filename), None) => format!("[Image: {filename}]"),
        (None, Some(media_type)) => format!("[Image: {media_type}]"),
        (None, None) => "[Image]".to_owned(),
    }
}

fn record_paste_warning(content: &str, line_no: usize, paste_warnings: &mut Vec<PasteRefWarning>) {
    let Some(start) = [PASTE_REF_PATTERN, IMAGE_REF_PATTERN, TRUNCATED_TEXT_PATTERN]
        .iter()
        .filter_map(|pattern| content.find(pattern))
        .min()
    else {
        return;
    };

    if !contains_paste_ref(content) {
        return;
    }

    paste_warnings.push(PasteRefWarning {
        line: line_no,
        snippet: content[start..].chars().take(40).collect(),
    });
}

fn record_partial_import_warning(
    warnings: &mut Vec<TsImportWarning>,
    line_no: usize,
    role: &str,
    unsupported: &[String],
    fully_dropped: bool,
) {
    if unsupported.is_empty() || fully_dropped {
        return;
    }

    warnings.push(TsImportWarning {
        line: line_no,
        category: TsWarningCategory::PartialImport,
        message: format!(
            "{role} record dropped unsupported block type(s): {}",
            unsupported.join(", ")
        ),
    });
}

#[derive(Default)]
struct ObservedSessionContext {
    cwd: Option<PathBuf>,
    git_branch: Option<String>,
    entrypoint: Option<String>,
    app_version: Option<String>,
    created_at: Option<OffsetDateTime>,
    updated_at: Option<OffsetDateTime>,
}

impl ObservedSessionContext {
    fn observe(&mut self, envelope: &MessageEnvelope) {
        if self.cwd.is_none() {
            self.cwd = envelope.cwd.clone();
        }
        if envelope.git_branch.is_some() {
            self.git_branch = envelope.git_branch.clone();
        }
        if envelope.entrypoint.is_some() {
            self.entrypoint = envelope.entrypoint.clone();
        }
        if envelope.app_version.is_some() {
            self.app_version = envelope.app_version.clone();
        }
        self.created_at = Some(self.created_at.map_or(envelope.timestamp, |current| {
            current.min(envelope.timestamp)
        }));
        self.updated_at = Some(self.updated_at.map_or(envelope.timestamp, |current| {
            current.max(envelope.timestamp)
        }));
    }
}

fn maybe_write_import_metadata(
    store: &TranscriptStore,
    session_id: SessionId,
    imported_count: usize,
    observed: &ObservedSessionContext,
    captured_metadata: &TsCapturedMetadata,
    warnings: &mut Vec<TsImportWarning>,
) -> Result<()> {
    let Some(custom_title) = &captured_metadata.custom_title else {
        return Ok(());
    };
    if imported_count == 0 {
        warnings.push(TsImportWarning {
            line: 0,
            category: TsWarningCategory::Metadata,
            message: "captured custom-title metadata but skipped metadata write because no transcript messages were imported".to_owned(),
        });
        return Ok(());
    }
    let Some(created_at) = observed.created_at else {
        warnings.push(TsImportWarning {
            line: 0,
            category: TsWarningCategory::Metadata,
            message: "captured custom-title metadata but skipped metadata write because no message timestamps were observed".to_owned(),
        });
        return Ok(());
    };

    store.write_metadata(&SessionMetadata {
        schema_version: crate::STORAGE_SCHEMA_VERSION,
        session_id,
        title: custom_title.clone(),
        cwd: observed.cwd.clone().unwrap_or_default(),
        git_branch: observed.git_branch.clone(),
        entrypoint: observed.entrypoint.clone(),
        app_version: observed.app_version.clone(),
        created_at,
        updated_at: observed.updated_at.unwrap_or(created_at),
        message_count: imported_count,
        tags: Vec::new(),
        provider: None,
        model: None,
        auth: Default::default(),
        costs: Default::default(),
    })
}

fn derived_message_id(raw_uuid: Option<&str>, block_index: usize) -> MessageId {
    match raw_uuid {
        Some(raw_uuid) => MessageId::from(stable_uuid(
            "ts-import-message",
            &format!("{raw_uuid}:{block_index}"),
        )),
        None => MessageId::new(),
    }
}

fn derived_tool_use_id(raw_use_id: &str) -> ToolUseId {
    ToolUseId::from(stable_uuid("ts-import-tool-use", raw_use_id))
}

fn stable_uuid(namespace: &str, value: &str) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(namespace.as_bytes());
    hasher.update([0]);
    hasher.update(value.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn is_metadata_type(ts_type: &str) -> bool {
    matches!(
        ts_type,
        "summary"
            | "custom-title"
            | "tag"
            | "agent-name"
            | "agent-color"
            | "agent-setting"
            | "mode"
            | "worktree-state"
            | "pr-link"
            | "ai-title"
            | "task-summary"
            | "last-prompt"
            | "content-replacement"
            | "file-history-snapshot"
            | "attribution-snapshot"
    )
}

fn format_count_summary<I>(counts: I) -> String
where
    I: IntoIterator<Item = (&'static str, usize)>,
{
    let counts: Vec<String> = counts
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(label, count)| format!("{label}={count}"))
        .collect();
    if counts.is_empty() {
        String::new()
    } else {
        format!(" [{}]", counts.join(", "))
    }
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

    /// TS summary metadata record — captured into the report.
    const SUMMARY_RECORD: &str = r#"{"type":"summary","leafUuid":"aaaaaaaa-0000-0000-0000-000000000001","summary":"Talked about greetings."}"#;

    /// TS custom-title metadata record — captured and written to Rust metadata on import.
    const CUSTOM_TITLE_RECORD: &str = r#"{"type":"custom-title","sessionId":"bbbbbbbb-0000-0000-0000-000000000001","customTitle":"Imported transcript title"}"#;

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

    /// Assistant record with only a `tool_use` block.
    const ASSISTANT_TOOL_USE_ONLY: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_01","name":"bash","input":{"command":"ls"}}],"role":"assistant"},"uuid":"aaaaaaaa-0000-0000-0000-000000000010","sessionId":"bbbbbbbb-0000-0000-0000-000000000001","timestamp":"2024-06-01T10:00:08Z","version":"2.1.0"}"#;

    /// User record with a tool result linked to `ASSISTANT_TOOL_USE_ONLY`.
    const USER_TOOL_RESULT: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu_01","content":[{"type":"text","text":"file_a\nfile_b"}]}]},"uuid":"aaaaaaaa-0000-0000-0000-000000000011","sessionId":"bbbbbbbb-0000-0000-0000-000000000001","timestamp":"2024-06-01T10:00:09Z","version":"2.1.0"}"#;

    /// User record containing an inline image block.
    const USER_IMAGE_BLOCK: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"See screenshot"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"Zm9v"}}]},"uuid":"aaaaaaaa-0000-0000-0000-000000000012","sessionId":"bbbbbbbb-0000-0000-0000-000000000001","timestamp":"2024-06-01T10:00:10Z","version":"2.1.0"}"#;

    /// Assistant record with a dropped unsupported block and preserved text.
    const ASSISTANT_MIXED_BLOCKS: &str = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hi"},{"type":"redacted_thinking"},{"type":"tool_use","id":"tu_02","name":"bash","input":{"command":"pwd"}}],"role":"assistant"},"uuid":"aaaaaaaa-0000-0000-0000-000000000013","sessionId":"bbbbbbbb-0000-0000-0000-000000000001","timestamp":"2024-06-01T10:00:11Z","version":"2.1.0"}"#;

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
        assert_eq!(report.convertible_count, 4);
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
    fn captures_summary_metadata_record() {
        let (_dir, path) = write_fixture(&[SUMMARY_RECORD]);
        let report = inspect_ts_file(&path).expect("inspect");

        assert!(
            report.skipped.is_empty(),
            "unexpected skips: {:?}",
            report.skipped
        );
        assert_eq!(report.captured_metadata.summaries.len(), 1);
        assert_eq!(
            report.captured_metadata.summaries[0].summary,
            "Talked about greetings."
        );
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
    fn imports_assistant_tool_use_and_user_tool_result_blocks() {
        let storage_dir = unique_test_dir("ts-import-tool-blocks");
        let store = TranscriptStore::new(&storage_dir);
        let (_dir, path) = write_fixture(&[ASSISTANT_TOOL_USE_ONLY, USER_TOOL_RESULT]);

        let report = import_ts_file(&path, &store, None).expect("import");

        assert_eq!(report.imported_count, 2);
        let loaded = store.load_session(report.session_id).expect("load session");
        assert_eq!(loaded.messages.len(), 2);

        let use_id = match &loaded.messages[0].payload {
            MessagePayload::AssistantToolUse {
                tool,
                use_id,
                input,
            } => {
                assert_eq!(tool, "bash");
                assert_eq!(input["command"], "ls");
                *use_id
            }
            other => panic!("expected AssistantToolUse, got {other:?}"),
        };

        match &loaded.messages[1].payload {
            MessagePayload::ToolResult {
                tool,
                use_id: result_use_id,
                success,
                content,
            } => {
                assert_eq!(tool, "bash");
                assert_eq!(*result_use_id, use_id);
                assert!(*success);
                assert!(content.contains("file_a"));
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn imports_custom_title_into_session_metadata() {
        let storage_dir = unique_test_dir("ts-import-custom-title");
        let store = TranscriptStore::new(&storage_dir);
        let (_dir, path) = write_fixture(&[CUSTOM_TITLE_RECORD, USER_STRING_CONTENT]);

        let report = import_ts_file(&path, &store, None).expect("import");

        assert_eq!(
            report.captured_metadata.custom_title.as_deref(),
            Some("Imported transcript title")
        );
        let metadata = store
            .read_metadata(report.session_id)
            .expect("read metadata");
        assert_eq!(metadata.title, "Imported transcript title");
        assert_eq!(metadata.message_count, 1);
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

    #[test]
    fn converts_user_image_block_into_explicit_text_placeholder() {
        let storage_dir = unique_test_dir("ts-import-image-placeholder");
        let store = TranscriptStore::new(&storage_dir);
        let (_dir, path) = write_fixture(&[USER_IMAGE_BLOCK]);

        let report = import_ts_file(&path, &store, None).expect("import");

        assert_eq!(report.imported_count, 1);
        let loaded = store.load_session(report.session_id).expect("load session");
        match &loaded.messages[0].payload {
            MessagePayload::UserText { content } => {
                assert!(content.contains("See screenshot"));
                assert!(content.contains("[Image: image/png]"));
            }
            other => panic!("expected UserText, got {other:?}"),
        }
    }

    // ── mixed-content batch tests ─────────────────────────────────────────────

    #[test]
    fn mixed_batch_counts_correctly() {
        let (_dir, path) = write_fixture(&[
            CUSTOM_TITLE_RECORD,    // metadata capture
            USER_STRING_CONTENT,    // converted
            SYSTEM_RECORD,          // skipped
            ASSISTANT_MIXED_BLOCKS, // converted + warning
            SUMMARY_RECORD,         // metadata capture
            MALFORMED_JSON,         // skipped
        ]);

        let report = inspect_ts_file(&path).expect("inspect");

        assert_eq!(report.convertible_count, 3);
        assert_eq!(report.skipped.len(), 2);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.captured_metadata.captured_count(), 2);
        assert_eq!(
            report.skip_counts().get(&SkipCategory::UnsupportedRecord),
            Some(&1)
        );
        assert_eq!(
            report.skip_counts().get(&SkipCategory::InvalidRecord),
            Some(&1)
        );
        assert_eq!(
            report
                .warning_counts()
                .get(&TsWarningCategory::PartialImport),
            Some(&1)
        );
        // dry-run: imported_count is always 0
        assert_eq!(report.imported_count, 0);
    }

    #[test]
    fn summary_text_contains_counts() {
        let (_dir, path) =
            write_fixture(&[USER_STRING_CONTENT, SYSTEM_RECORD, ASSISTANT_MIXED_BLOCKS]);
        let report = inspect_ts_file(&path).expect("inspect");
        let summary = report.summary();
        assert!(summary.contains("1 skipped"), "summary was: {summary}");
        assert!(
            summary.contains("unsupported_record=1"),
            "summary was: {summary}"
        );
        assert!(
            summary.contains("partial_import=1"),
            "summary was: {summary}"
        );
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
