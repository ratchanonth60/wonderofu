//! Append-only session storage primitives.
//!
//! # Local-first design
//!
//! All storage in this crate is **local-only**.  There is no telemetry
//! pipeline (no Datadog, no first-party event sink), no GrowthBook remote
//! feature-flag evaluation, and no cloud upload/download backend.  Feature
//! gates are resolved exclusively from the static [`wonder_of_u_core::FeatureSet`]
//! compiled into the binary.  The [`SyncStatusReport`] reflects this honestly:
//! its `analytics` and `experiments` surfaces are permanently `unsupported`.
#![warn(missing_docs)]

/// Provides memdir support
pub mod memdir;

/// Storage schema migration framework
pub mod migrations;
pub use migrations::{Migration, MigrationRunner, StorageVersionFile, default_migration_runner};

/// TypeScript-upstream transcript importer (explicit one-shot migration path).
///
/// The normal [`TranscriptStore`] load path is never touched; all conversion
/// happens through the explicit [`ts_import::import_ts_file`] /
/// [`ts_import::inspect_ts_file`] entry points.
pub mod ts_import;
pub use ts_import::{
    PasteRefWarning, SkipCategory, SkipReason, TsCapturedMetadata, TsImportReport, TsImportWarning,
    TsLeafSummary, TsSkipReport, TsWarningCategory, import_ts_file, inspect_ts_file,
};

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::ErrorKind,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use wonder_of_u_core::{
    AGENT_TASK_RESULT_SCHEMA_VERSION, AgentTaskResult, AppState, CostState, FLEET_SCHEMA_VERSION,
    FleetId, FleetMemberRequest, FleetRunState, MESSAGE_SCHEMA_VERSION, MessageEnvelope, MessageId,
    MessagePayload, Result, SessionId, TaskId, TaskState, TaskStatus, WonderError,
};

/// Schema version for storage
pub const STORAGE_SCHEMA_VERSION: u16 = 1;
/// Default paste max bytes value
pub const DEFAULT_PASTE_MAX_BYTES: usize = 1024 * 1024;

fn default_storage_schema_version() -> u16 {
    STORAGE_SCHEMA_VERSION
}

fn ensure_supported_schema(kind: &str, version: u16) -> Result<()> {
    let supported = match kind {
        "message" => MESSAGE_SCHEMA_VERSION,
        "fleet run" => FLEET_SCHEMA_VERSION,
        "agent task result" => AGENT_TASK_RESULT_SCHEMA_VERSION,
        _ => STORAGE_SCHEMA_VERSION,
    };

    if version == supported {
        return Ok(());
    }

    Err(WonderError::validation(format!(
        "unsupported {kind} schema version: {version}"
    )))
}

fn transcript_parse_warning(
    line: usize,
    error: impl std::fmt::Display,
    trailing: bool,
) -> TranscriptWarning {
    let message = if trailing {
        format!("ignored corrupt trailing transcript line: {error}")
    } else {
        format!("ignored corrupt transcript line: {error}")
    };

    TranscriptWarning { line, message }
}

fn ensure_supported_transcript_schema(value: &Value) -> Result<()> {
    let Some(schema_version) = value.get("schema_version") else {
        return Ok(());
    };
    let Ok(schema_version) = serde_json::from_value::<u16>(schema_version.clone()) else {
        return Ok(());
    };

    ensure_supported_schema("message", schema_version)
}

fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let next_extension = match path.extension().and_then(OsStr::to_str) {
        Some(extension) => format!("{extension}.next"),
        None => "next".into(),
    };
    let pending_path = path.with_extension(next_extension);

    let file = File::create(&pending_path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    fs::rename(pending_path, path)?;
    Ok(())
}

fn write_text_atomically(path: &Path, content: &str) -> Result<()> {
    let next_extension = match path.extension().and_then(OsStr::to_str) {
        Some(extension) => format!("{extension}.next"),
        None => "next".into(),
    };
    let pending_path = path.with_extension(next_extension);

    let mut file = File::create(&pending_path)?;
    file.write_all(content.as_bytes())?;
    file.flush()?;
    file.sync_all()?;
    fs::rename(pending_path, path)?;
    Ok(())
}

fn validate_paste_hash(sha256: &str) -> Result<()> {
    if sha256.len() == 64 && sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(());
    }

    Err(WonderError::validation(format!(
        "invalid paste hash: {sha256}"
    )))
}
/// Represents storage paths
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoragePaths {
    base_dir: PathBuf,
}

impl StoragePaths {
    /// Creates a new value
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }
    /// Returns the base directory
    #[must_use]
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }
    /// Handles sessions dir
    #[must_use]
    pub fn sessions_dir(&self) -> PathBuf {
        self.base_dir.join("sessions")
    }
    /// Handles transcript path
    #[must_use]
    pub fn transcript_path(&self, session_id: SessionId) -> PathBuf {
        self.sessions_dir().join(format!("{session_id}.jsonl"))
    }
    /// Handles metadata dir
    #[must_use]
    pub fn metadata_dir(&self) -> PathBuf {
        self.sessions_dir().join(".metadata")
    }
    /// Handles metadata path
    #[must_use]
    pub fn metadata_path(&self, session_id: SessionId) -> PathBuf {
        self.metadata_dir().join(format!("{session_id}.json"))
    }
    /// Handles snapshot dir
    #[must_use]
    pub fn snapshot_dir(&self) -> PathBuf {
        self.sessions_dir().join(".snapshots")
    }
    /// Handles snapshot path
    #[must_use]
    pub fn snapshot_path(&self, session_id: SessionId) -> PathBuf {
        self.snapshot_dir().join(format!("{session_id}.json"))
    }
    /// Returns the session memory index dir
    #[must_use]
    pub fn session_memory_index_dir(&self) -> PathBuf {
        self.sessions_dir().join(".memory-index")
    }
    /// Returns the session memory index path
    #[must_use]
    pub fn session_memory_index_path(&self, session_id: SessionId) -> PathBuf {
        self.session_memory_index_dir()
            .join(format!("{session_id}.json"))
    }
    /// Handles cost path
    #[must_use]
    pub fn cost_path(&self, session_id: SessionId) -> PathBuf {
        self.sessions_dir().join(format!("{session_id}.costs"))
    }
    /// Handles pastes dir
    #[must_use]
    pub fn pastes_dir(&self) -> PathBuf {
        self.base_dir.join("pastes")
    }
    /// Handles plugins dir
    #[must_use]
    pub fn plugins_dir(&self) -> PathBuf {
        self.base_dir.join("plugins")
    }
    /// Handles skills dir
    #[must_use]
    pub fn skills_dir(&self) -> PathBuf {
        self.base_dir.join("skills")
    }
    /// Handles config dir
    #[must_use]
    pub fn config_dir(&self) -> PathBuf {
        self.base_dir.join("config")
    }
    /// Handles settings path
    #[must_use]
    pub fn settings_path(&self) -> PathBuf {
        self.config_dir().join("settings.json")
    }
    /// Handles user memory path
    #[must_use]
    pub fn user_memory_path(&self) -> PathBuf {
        self.config_dir().join("CLAUDE.md")
    }
    /// Handles credentials path
    #[must_use]
    pub fn credentials_path(&self) -> PathBuf {
        self.config_dir().join("credentials.json")
    }
    /// Handles plugins config dir
    #[must_use]
    pub fn plugins_config_dir(&self) -> PathBuf {
        self.config_dir().join("plugins")
    }
    /// Handles plugin settings path
    #[must_use]
    pub fn plugin_settings_path(&self) -> PathBuf {
        self.plugins_config_dir().join("settings.json")
    }
    /// Handles mcp config dir
    #[must_use]
    pub fn mcp_config_dir(&self) -> PathBuf {
        self.config_dir().join("mcp")
    }
    /// Handles mcp servers path
    #[must_use]
    pub fn mcp_servers_path(&self) -> PathBuf {
        self.mcp_config_dir().join("servers.json")
    }
    /// Handles tasks dir
    #[must_use]
    pub fn tasks_dir(&self) -> PathBuf {
        self.base_dir.join("tasks")
    }
    /// Returns the task metadata dir
    #[must_use]
    pub fn task_metadata_dir(&self) -> PathBuf {
        self.tasks_dir().join("metadata")
    }
    /// Returns the task logs dir
    #[must_use]
    pub fn task_logs_dir(&self) -> PathBuf {
        self.tasks_dir().join("logs")
    }
    /// Returns the task exit dir
    #[must_use]
    pub fn task_exit_dir(&self) -> PathBuf {
        self.tasks_dir().join("exit")
    }
    /// Returns the task heartbeat dir
    #[must_use]
    pub fn task_heartbeat_dir(&self) -> PathBuf {
        self.tasks_dir().join("heartbeat")
    }
    /// Returns the task state path
    #[must_use]
    pub fn task_state_path(&self, task_id: TaskId) -> PathBuf {
        self.task_metadata_dir().join(format!("{task_id}.json"))
    }
    /// Returns the task log path
    #[must_use]
    pub fn task_log_path(&self, task_id: TaskId) -> PathBuf {
        self.task_logs_dir().join(format!("{task_id}.log"))
    }
    /// Returns the task exit path
    #[must_use]
    pub fn task_exit_path(&self, task_id: TaskId) -> PathBuf {
        self.task_exit_dir().join(format!("{task_id}.exit"))
    }
    /// Returns the task heartbeat path
    #[must_use]
    pub fn task_heartbeat_path(&self, task_id: TaskId) -> PathBuf {
        self.task_heartbeat_dir()
            .join(format!("{task_id}.heartbeat"))
    }

    /// Returns the directory where agent task result sidecars are stored.
    #[must_use]
    pub fn task_results_dir(&self) -> PathBuf {
        self.tasks_dir().join("results")
    }

    /// Returns the path for an agent task result sidecar.
    #[must_use]
    pub fn task_result_path(&self, task_id: TaskId) -> PathBuf {
        self.task_results_dir().join(format!("{task_id}.json"))
    }
    /// Handles paste path
    #[must_use]
    pub fn paste_path(&self, sha256: &str) -> PathBuf {
        self.pastes_dir().join(sha256)
    }

    // ── Fleet paths ───────────────────────────────────────────────────────────

    /// Returns the root directory for fleet data.
    #[must_use]
    pub fn fleet_dir(&self) -> PathBuf {
        self.base_dir.join("fleet")
    }

    /// Returns the directory where fleet run state files are stored.
    #[must_use]
    pub fn fleet_runs_dir(&self) -> PathBuf {
        self.fleet_dir().join("runs")
    }

    /// Returns the directory where pending member request files are stored.
    #[must_use]
    pub fn fleet_pending_dir(&self) -> PathBuf {
        self.fleet_dir().join("pending")
    }

    /// Returns the path for a fleet run state file.
    #[must_use]
    pub fn fleet_run_path(&self, fleet_id: wonder_of_u_core::FleetId) -> PathBuf {
        self.fleet_runs_dir().join(format!("{fleet_id}.json"))
    }

    /// Returns the path for a pending fleet member request file.
    #[must_use]
    pub fn fleet_pending_path(&self, request_id: &str) -> PathBuf {
        self.fleet_pending_path_for(None, request_id)
    }

    /// Returns the path for a pending fleet member request file.
    ///
    /// Fleet-scoped requests include the fleet id in the filename so separate
    /// fleet plans can reuse human-readable member ids like `build` or `test`
    /// without clobbering each other.
    #[must_use]
    pub fn fleet_pending_path_for(
        &self,
        fleet_id: Option<wonder_of_u_core::FleetId>,
        request_id: &str,
    ) -> PathBuf {
        let file_name = match fleet_id {
            Some(fleet_id) => format!("{fleet_id}-{request_id}.json"),
            None => format!("{request_id}.json"),
        };
        self.fleet_pending_dir().join(file_name)
    }
}
/// Stores transcript store
#[derive(Clone, Debug)]
pub struct TranscriptStore {
    paths: StoragePaths,
}

impl TranscriptStore {
    /// Creates a new value
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
        }
    }
    /// Handles paths
    #[must_use]
    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    /// Handles ensure layout
    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.paths.sessions_dir())?;
        fs::create_dir_all(self.paths.metadata_dir())?;
        fs::create_dir_all(self.paths.snapshot_dir())?;
        Ok(())
    }

    /// Handles append message
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

    /// Loads session
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
            let line_no = index + 1;
            if line.trim().is_empty() {
                continue;
            }

            let value = match serde_json::from_str::<Value>(line) {
                Ok(value) => value,
                Err(error) => {
                    warnings.push(transcript_parse_warning(
                        line_no,
                        error,
                        line_no == lines.len(),
                    ));
                    continue;
                }
            };

            ensure_supported_transcript_schema(&value)?;

            let message = match serde_json::from_value::<MessageEnvelope>(value) {
                Ok(message) => message,
                Err(error) => {
                    warnings.push(transcript_parse_warning(
                        line_no,
                        error,
                        line_no == lines.len(),
                    ));
                    continue;
                }
            };

            messages.push(self.expand_paste_reference(message)?);
        }

        Ok(LoadedTranscript { messages, warnings })
    }

    fn expand_paste_reference(&self, mut message: MessageEnvelope) -> Result<MessageEnvelope> {
        let MessagePayload::UserPasteReference { sha256, .. } = &message.payload else {
            return Ok(message);
        };

        let content = PasteStore::new(self.paths.base_dir().to_path_buf()).load_text(sha256)?;
        message.payload = MessagePayload::UserText { content };
        Ok(message)
    }

    /// Writes metadata
    pub fn write_metadata(&self, metadata: &SessionMetadata) -> Result<()> {
        self.ensure_layout()?;
        let path = self.paths.metadata_path(metadata.session_id);
        write_json_atomically(&path, metadata)
    }

    /// Reads metadata
    pub fn read_metadata(&self, session_id: SessionId) -> Result<SessionMetadata> {
        let path = self.paths.metadata_path(session_id);
        if !path.exists() {
            return Err(WonderError::not_found(
                "session metadata",
                session_id.to_string(),
            ));
        }

        let metadata: SessionMetadata = serde_json::from_str(&fs::read_to_string(path)?)?;
        ensure_supported_schema("session metadata", metadata.schema_version)?;
        Ok(metadata)
    }

    /// Handles list metadata
    pub fn list_metadata(&self) -> Result<Vec<SessionMetadata>> {
        let dir = self.paths.metadata_dir();
        match fs::read_dir(dir) {
            Ok(entries) => {
                let mut sessions = Vec::new();
                for entry in entries {
                    let entry = entry?;
                    if !entry.file_type()?.is_file() {
                        continue;
                    }
                    if entry.path().extension().and_then(OsStr::to_str) != Some("json") {
                        continue;
                    }

                    let metadata: SessionMetadata =
                        serde_json::from_str(&fs::read_to_string(entry.path())?)?;
                    ensure_supported_schema("session metadata", metadata.schema_version)?;
                    sessions.push(metadata);
                }

                sessions.sort_by(|left, right| {
                    right
                        .updated_at
                        .cmp(&left.updated_at)
                        .then_with(|| right.created_at.cmp(&left.created_at))
                        .then_with(|| left.session_id.cmp(&right.session_id))
                });
                Ok(sessions)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    /// Writes snapshot
    pub fn write_snapshot(&self, snapshot: &SessionSnapshot) -> Result<()> {
        self.ensure_layout()?;
        let path = self.paths.snapshot_path(snapshot.session_id);
        write_json_atomically(&path, snapshot)
    }

    /// Reads snapshot
    pub fn read_snapshot(&self, session_id: SessionId) -> Result<SessionSnapshot> {
        let path = self.paths.snapshot_path(session_id);
        if !path.exists() {
            return Err(WonderError::not_found(
                "session snapshot",
                session_id.to_string(),
            ));
        }

        let snapshot: SessionSnapshot = serde_json::from_str(&fs::read_to_string(path)?)?;
        ensure_supported_schema("session snapshot", snapshot.schema_version)?;
        if snapshot.session_id != session_id || snapshot.state.session.id != session_id {
            return Err(WonderError::validation(format!(
                "session snapshot id mismatch for {session_id}"
            )));
        }
        Ok(snapshot)
    }

    /// Reads snapshot if exists
    pub fn read_snapshot_if_exists(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionSnapshot>> {
        match self.read_snapshot(session_id) {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(WonderError::NotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Handles restore session
    pub fn restore_session(&self, session_id: SessionId) -> Result<RestoredSession> {
        let metadata = self.read_metadata(session_id)?;
        let transcript = self.load_session(session_id)?;
        if let Some(snapshot) = self.read_snapshot_if_exists(session_id)? {
            let mut state = snapshot.state;
            apply_metadata_to_state(&mut state, &metadata);
            return Ok(RestoredSession {
                metadata,
                transcript,
                state,
                resume_source: SessionResumeSource::Snapshot,
            });
        }

        let mut state = AppState::new(metadata.cwd.clone());
        apply_metadata_to_state(&mut state, &metadata);
        state.messages = transcript.messages.clone();
        Ok(RestoredSession {
            metadata,
            transcript,
            state,
            resume_source: SessionResumeSource::Transcript,
        })
    }
}
/// Enumerates sync support
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncSupport {
    /// Represents local only
    LocalOnly,
    /// Represents unsupported
    Unsupported,
    /// Represents deferred
    Deferred,
}

impl SyncSupport {
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LocalOnly => "local_only",
            Self::Unsupported => "unsupported",
            Self::Deferred => "deferred",
        }
    }
}
/// Represents settings sync status
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingsSyncStatus {
    /// Stores the schema version
    #[serde(default = "default_storage_schema_version")]
    pub schema_version: u16,
    /// Stores the status
    pub status: SyncSupport,
    /// Stores the cloud status
    pub cloud_status: SyncSupport,
    /// Stores the settings path
    pub settings_path: PathBuf,
    /// Stores the settings exists
    pub settings_exists: bool,
    /// Stores the user memory path
    pub user_memory_path: PathBuf,
    /// Stores the user memory exists
    pub user_memory_exists: bool,
    /// Stores the cloud attempted
    pub cloud_attempted: bool,
    /// Stores the reason
    pub reason: String,
}

impl SettingsSyncStatus {
    /// Handles inspect
    #[must_use]
    pub fn inspect(paths: &StoragePaths) -> Self {
        let settings_path = paths.settings_path();
        let user_memory_path = paths.user_memory_path();
        Self {
            schema_version: STORAGE_SCHEMA_VERSION,
            status: SyncSupport::LocalOnly,
            cloud_status: SyncSupport::Unsupported,
            settings_exists: settings_path.exists(),
            user_memory_exists: user_memory_path.exists(),
            settings_path,
            user_memory_path,
            cloud_attempted: false,
            reason: "local settings and user memory stay on disk; cloud upload/download backends are unavailable in this Rust port".into(),
        }
    }
}
/// Represents remote surface status
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteSurfaceStatus {
    /// Stores the schema version
    #[serde(default = "default_storage_schema_version")]
    pub schema_version: u16,
    /// Stores the service
    pub service: String,
    /// Stores the status
    pub status: SyncSupport,
    /// Stores the cloud attempted
    pub cloud_attempted: bool,
    /// Stores the reason
    pub reason: String,
}

impl RemoteSurfaceStatus {
    /// Handles unsupported
    #[must_use]
    pub fn unsupported(service: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            schema_version: STORAGE_SCHEMA_VERSION,
            service: service.into(),
            status: SyncSupport::Unsupported,
            cloud_attempted: false,
            reason: reason.into(),
        }
    }
    /// Handles deferred
    #[must_use]
    pub fn deferred(service: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            schema_version: STORAGE_SCHEMA_VERSION,
            service: service.into(),
            status: SyncSupport::Deferred,
            cloud_attempted: false,
            reason: reason.into(),
        }
    }
}
/// Represents sync status report
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncStatusReport {
    /// Stores the settings sync
    pub settings_sync: SettingsSyncStatus,
    /// Stores the remote managed settings
    pub remote_managed_settings: RemoteSurfaceStatus,
    /// Stores the team memory sync
    pub team_memory_sync: RemoteSurfaceStatus,
    /// Analytics event sink status.
    ///
    /// Always `unsupported`: no Datadog or first-party event pipeline exists in
    /// this Rust port; usage data stays local.
    pub analytics: RemoteSurfaceStatus,
    /// Remote experiment / feature-flag evaluation status.
    ///
    /// Always `unsupported`: GrowthBook remote evaluation is intentionally
    /// absent.  Feature gates are resolved from the static `FeatureSet` only.
    pub experiments: RemoteSurfaceStatus,
}

impl SyncStatusReport {
    /// Handles inspect
    #[must_use]
    pub fn inspect(paths: &StoragePaths) -> Self {
        Self {
            settings_sync: SettingsSyncStatus::inspect(paths),
            remote_managed_settings: RemoteSurfaceStatus::deferred(
                "remote_managed_settings",
                "enterprise-managed remote settings are deferred until a real policy backend exists",
            ),
            team_memory_sync: RemoteSurfaceStatus::unsupported(
                "team_memory_sync",
                "repo-scoped cloud memory sync is unsupported; session memory remains local-only",
            ),
            analytics: RemoteSurfaceStatus::unsupported(
                "analytics",
                "no Datadog/first-party event sink; usage analytics are intentionally omitted in this Rust port",
            ),
            experiments: RemoteSurfaceStatus::unsupported(
                "experiments",
                "no GrowthBook remote feature evaluation; feature gates resolved from static FeatureSet only",
            ),
        }
    }
}
/// Enumerates session memory source
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMemorySource {
    /// Represents user
    User,
    /// Represents assistant
    Assistant,
    /// Represents tool
    Tool,
    /// Represents system
    System,
}

impl SessionMemorySource {
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::System => "system",
        }
    }
}
/// Represents session memory entry
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionMemoryEntry {
    /// Stores the key
    pub key: String,
    /// Stores the source
    pub source: SessionMemorySource,
    /// Stores the summary
    pub summary: String,
    /// Stores the occurrences
    pub occurrences: usize,
    /// Stores the first message identifier
    pub first_message_id: MessageId,
    /// Stores the last message identifier
    pub last_message_id: MessageId,
    /// Stores the first seen at
    #[serde(with = "time::serde::rfc3339")]
    pub first_seen_at: OffsetDateTime,
    /// Stores the last seen at
    #[serde(with = "time::serde::rfc3339")]
    pub last_seen_at: OffsetDateTime,
}
/// Represents session memory index
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionMemoryIndex {
    /// Stores the schema version
    #[serde(default = "default_storage_schema_version")]
    pub schema_version: u16,
    /// Stores the session identifier
    pub session_id: SessionId,
    /// Stores the transcript message count
    pub transcript_message_count: usize,
    /// Stores the indexed message count
    pub indexed_message_count: usize,
    /// Stores the indexed at
    #[serde(with = "time::serde::rfc3339")]
    pub indexed_at: OffsetDateTime,
    /// Stores the entries
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<SessionMemoryEntry>,
}

impl SessionMemoryIndex {
    /// Handles extract
    #[must_use]
    pub fn extract(session_id: SessionId, messages: &[MessageEnvelope]) -> Self {
        let mut entries = BTreeMap::<String, SessionMemoryEntry>::new();
        let mut indexed_message_count = 0;

        for message in messages {
            let Some((source, text)) = session_memory_text(&message.payload) else {
                continue;
            };
            let normalized = normalize_memory_text(&text);
            if normalized.is_empty() {
                continue;
            }

            indexed_message_count += 1;
            let key = format!(
                "{}:{:x}",
                source.label(),
                Sha256::digest(normalized.as_bytes())
            );
            let summary = truncate_memory_summary(&normalized, 160);

            if let Some(entry) = entries.get_mut(&key) {
                entry.occurrences += 1;
                entry.last_message_id = message.id;
                entry.last_seen_at = message.timestamp;
                continue;
            }

            entries.insert(
                key.clone(),
                SessionMemoryEntry {
                    key,
                    source,
                    summary,
                    occurrences: 1,
                    first_message_id: message.id,
                    last_message_id: message.id,
                    first_seen_at: message.timestamp,
                    last_seen_at: message.timestamp,
                },
            );
        }

        let mut entries = entries.into_values().collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            left.first_seen_at
                .cmp(&right.first_seen_at)
                .then_with(|| left.key.cmp(&right.key))
        });

        Self {
            schema_version: STORAGE_SCHEMA_VERSION,
            session_id,
            transcript_message_count: messages.len(),
            indexed_message_count,
            indexed_at: OffsetDateTime::now_utc(),
            entries,
        }
    }
}
/// Stores session memory index store
#[derive(Clone, Debug)]
pub struct SessionMemoryIndexStore {
    paths: StoragePaths,
}

impl SessionMemoryIndexStore {
    /// Creates a new value
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
        }
    }
    /// Handles paths
    #[must_use]
    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    /// Handles ensure layout
    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.paths.session_memory_index_dir())?;
        Ok(())
    }

    /// Handles write
    pub fn write(&self, index: &SessionMemoryIndex) -> Result<()> {
        self.ensure_layout()?;
        write_json_atomically(
            &self.paths.session_memory_index_path(index.session_id),
            index,
        )
    }

    /// Handles read
    pub fn read(&self, session_id: SessionId) -> Result<SessionMemoryIndex> {
        let path = self.paths.session_memory_index_path(session_id);
        if !path.exists() {
            return Err(WonderError::not_found(
                "session memory index",
                session_id.to_string(),
            ));
        }

        let index: SessionMemoryIndex = serde_json::from_str(&fs::read_to_string(path)?)?;
        ensure_supported_schema("session memory index", index.schema_version)?;
        if index.session_id != session_id {
            return Err(WonderError::validation(format!(
                "session memory index id mismatch for {session_id}"
            )));
        }
        Ok(index)
    }

    /// Reads if exists
    pub fn read_if_exists(&self, session_id: SessionId) -> Result<Option<SessionMemoryIndex>> {
        match self.read(session_id) {
            Ok(index) => Ok(Some(index)),
            Err(WonderError::NotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Handles list
    pub fn list(&self) -> Result<Vec<SessionMemoryIndex>> {
        let dir = self.paths.session_memory_index_dir();
        match fs::read_dir(dir) {
            Ok(entries) => {
                let mut indexes = Vec::new();
                for entry in entries {
                    let entry = entry?;
                    if !entry.file_type()?.is_file() {
                        continue;
                    }
                    if entry.path().extension().and_then(OsStr::to_str) != Some("json") {
                        continue;
                    }

                    let index: SessionMemoryIndex =
                        serde_json::from_str(&fs::read_to_string(entry.path())?)?;
                    ensure_supported_schema("session memory index", index.schema_version)?;
                    indexes.push(index);
                }

                indexes.sort_by(|left, right| {
                    right
                        .indexed_at
                        .cmp(&left.indexed_at)
                        .then_with(|| left.session_id.cmp(&right.session_id))
                });
                Ok(indexes)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    /// Handles rebuild from messages
    pub fn rebuild_from_messages(
        &self,
        session_id: SessionId,
        messages: &[MessageEnvelope],
    ) -> Result<SessionMemoryIndex> {
        let index = SessionMemoryIndex::extract(session_id, messages);
        self.write(&index)?;
        Ok(index)
    }

    /// Handles rebuild from transcript
    pub fn rebuild_from_transcript(
        &self,
        store: &TranscriptStore,
        session_id: SessionId,
    ) -> Result<SessionMemoryIndex> {
        let transcript = store.load_session(session_id)?;
        self.rebuild_from_messages(session_id, &transcript.messages)
    }
}

fn session_memory_text(payload: &MessagePayload) -> Option<(SessionMemorySource, String)> {
    match payload {
        MessagePayload::UserText { content } => Some((SessionMemorySource::User, content.clone())),
        MessagePayload::UserAttachment { label, uri } => {
            Some((SessionMemorySource::User, format!("{label} {uri}")))
        }
        MessagePayload::UserPasteReference { sha256, bytes } => Some((
            SessionMemorySource::User,
            format!("pasted {bytes} bytes ({sha256})"),
        )),
        MessagePayload::AssistantText { content }
        | MessagePayload::AssistantThinking { content, .. } => {
            Some((SessionMemorySource::Assistant, content.clone()))
        }
        MessagePayload::AssistantToolUse { tool, input, .. } => {
            Some((SessionMemorySource::Tool, format!("tool {tool} {}", input)))
        }
        MessagePayload::ToolResult { tool, content, .. } => {
            Some((SessionMemorySource::Tool, format!("{tool} {content}")))
        }
        MessagePayload::BashOutput {
            stdout,
            stderr,
            exit_code,
        } => {
            let combined = [stdout.trim(), stderr.trim()]
                .into_iter()
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let detail = if combined.is_empty() {
                exit_code
                    .map(|code| format!("bash exited with {code}"))
                    .unwrap_or_default()
            } else if let Some(code) = exit_code {
                format!("{combined} (exit {code})")
            } else {
                combined
            };
            Some((SessionMemorySource::Tool, detail))
        }
        MessagePayload::System { content } => Some((SessionMemorySource::System, content.clone())),
        MessagePayload::Progress { label, detail } => Some((
            SessionMemorySource::System,
            detail
                .as_ref()
                .map_or_else(|| label.clone(), |detail| format!("{label} {detail}")),
        )),
        MessagePayload::Command { input, output } => Some((
            SessionMemorySource::System,
            output
                .as_ref()
                .map_or_else(|| input.clone(), |output| format!("{input} {output}")),
        )),
        MessagePayload::HookResult { hook, output, .. } => {
            Some((SessionMemorySource::Tool, format!("{hook} {output}")))
        }
        MessagePayload::HookProgress {
            event,
            tool_name,
            hook_count,
            success,
        } => Some((
            SessionMemorySource::System,
            format!(
                "{event} {hook_count} for {tool_name} {}",
                if *success { "ok" } else { "error" }
            ),
        )),
        MessagePayload::CompactBoundary { summary } => {
            Some((SessionMemorySource::System, summary.clone()))
        }
        MessagePayload::Task { message, .. } => {
            Some((SessionMemorySource::System, message.clone()))
        }
        MessagePayload::Permission {
            tool,
            decision,
            reason,
        } => Some((
            SessionMemorySource::System,
            format!("{tool} {decision} {reason}"),
        )),
        MessagePayload::PlanApproval { summary, approved } => Some((
            SessionMemorySource::System,
            format!(
                "plan {} {summary}",
                if *approved { "approved" } else { "rejected" }
            ),
        )),
        // Provider errors are not searchable session memory; skip them.
        MessagePayload::ProviderError { .. } => None,
    }
}

fn normalize_memory_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_memory_summary(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }

    let keep = max_chars.saturating_sub(1);
    let truncated = text.chars().take(keep).collect::<String>();
    format!("{truncated}…")
}
/// Stores cost store
#[derive(Clone, Debug)]
pub struct CostStore {
    paths: StoragePaths,
}

impl CostStore {
    /// Creates a new value
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
        }
    }
    /// Handles paths
    #[must_use]
    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    /// Handles ensure layout
    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.paths.sessions_dir())?;
        Ok(())
    }

    /// Writes costs
    pub fn write_costs(&self, ledger: &SessionCostLedger) -> Result<()> {
        self.ensure_layout()?;
        let path = self.paths.cost_path(ledger.session_id);
        write_json_atomically(&path, ledger)
    }

    /// Reads costs
    pub fn read_costs(&self, session_id: SessionId) -> Result<SessionCostLedger> {
        let path = self.paths.cost_path(session_id);
        if !path.exists() {
            return Err(WonderError::not_found(
                "session costs",
                session_id.to_string(),
            ));
        }

        let ledger: SessionCostLedger = serde_json::from_str(&fs::read_to_string(path)?)?;
        ensure_supported_schema("session costs", ledger.schema_version)?;
        Ok(ledger)
    }
}
/// Stores task store
#[derive(Clone, Debug)]
pub struct TaskStore {
    paths: StoragePaths,
}

impl TaskStore {
    /// Creates a new value
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
        }
    }
    /// Handles paths
    #[must_use]
    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    /// Handles ensure layout
    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.paths.task_metadata_dir())?;
        fs::create_dir_all(self.paths.task_logs_dir())?;
        fs::create_dir_all(self.paths.task_exit_dir())?;
        fs::create_dir_all(self.paths.task_heartbeat_dir())?;
        Ok(())
    }

    /// Writes task
    pub fn write_task(&self, task: &TaskState) -> Result<()> {
        self.ensure_layout()?;
        write_json_atomically(&self.paths.task_state_path(task.id), task)
    }

    /// Reads task
    pub fn read_task(&self, task_id: TaskId) -> Result<TaskState> {
        let path = self.paths.task_state_path(task_id);
        if !path.exists() {
            return Err(WonderError::not_found("task", task_id.to_string()));
        }
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    /// Handles list tasks
    pub fn list_tasks(&self) -> Result<Vec<TaskState>> {
        let dir = self.paths.task_metadata_dir();
        match fs::read_dir(dir) {
            Ok(entries) => {
                let mut tasks = Vec::new();
                for entry in entries {
                    let entry = entry?;
                    if !entry.file_type()?.is_file() {
                        continue;
                    }
                    if entry.path().extension().and_then(OsStr::to_str) != Some("json") {
                        continue;
                    }
                    tasks.push(serde_json::from_str::<TaskState>(&fs::read_to_string(
                        entry.path(),
                    )?)?);
                }

                tasks.sort_by(|left, right| {
                    right
                        .started_at
                        .cmp(&left.started_at)
                        .then_with(|| left.id.cmp(&right.id))
                });
                Ok(tasks)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    /// Handles append log
    pub fn append_log(&self, task_id: TaskId, content: impl AsRef<str>) -> Result<()> {
        self.ensure_layout()?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.paths.task_log_path(task_id))?;
        let mut writer = BufWriter::new(file);
        writer.write_all(content.as_ref().as_bytes())?;
        writer.flush()?;
        writer.get_ref().sync_data()?;
        Ok(())
    }

    /// Reads log
    pub fn read_log(&self, task_id: TaskId) -> Result<String> {
        let path = self.paths.task_log_path(task_id);
        if !path.exists() {
            return Err(WonderError::not_found("task log", task_id.to_string()));
        }
        Ok(fs::read_to_string(path)?)
    }

    /// Reads log tail
    pub fn read_log_tail(&self, task_id: TaskId, line_limit: usize) -> Result<Vec<String>> {
        let content = self.read_log(task_id)?;
        let line_limit = line_limit.max(1);
        let mut lines = content.lines().map(ToString::to_string).collect::<Vec<_>>();
        if lines.len() > line_limit {
            lines.drain(0..lines.len() - line_limit);
        }
        Ok(lines)
    }

    /// Writes exit code
    pub fn write_exit_code(&self, task_id: TaskId, exit_code: i32) -> Result<()> {
        self.ensure_layout()?;
        let mut file = File::create(self.paths.task_exit_path(task_id))?;
        writeln!(file, "{exit_code}")?;
        file.sync_all()?;
        Ok(())
    }

    /// Writes heartbeat at
    pub fn write_heartbeat_at(&self, task_id: TaskId, heartbeat_at: OffsetDateTime) -> Result<()> {
        self.ensure_layout()?;
        let content = format!(
            "{}\n",
            heartbeat_at
                .format(&Rfc3339)
                .map_err(|error| WonderError::validation(format!(
                    "invalid heartbeat timestamp: {error}"
                )))?
        );
        write_text_atomically(&self.paths.task_heartbeat_path(task_id), &content)
    }

    /// Reads exit code
    pub fn read_exit_code(&self, task_id: TaskId) -> Result<Option<i32>> {
        let path = self.paths.task_exit_path(task_id);
        match fs::read_to_string(&path) {
            Ok(content) => {
                let exit_code = content.trim().parse::<i32>().map_err(|error| {
                    WonderError::validation(format!(
                        "invalid task exit code for {task_id}: {error}"
                    ))
                })?;
                Ok(Some(exit_code))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Reads heartbeat at
    pub fn read_heartbeat_at(&self, task_id: TaskId) -> Result<Option<OffsetDateTime>> {
        let path = self.paths.task_heartbeat_path(task_id);
        match fs::read_to_string(&path) {
            Ok(content) => OffsetDateTime::parse(content.trim(), &Rfc3339)
                .map(Some)
                .map_err(|error| {
                    WonderError::validation(format!(
                        "invalid task heartbeat for {task_id}: {error}"
                    ))
                }),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

// ── AgentTaskResultStore ──────────────────────────────────────────────────────

/// Persistent store for agent task result sidecars.
///
/// Layout under the storage base directory:
///
/// ```text
/// tasks/
///   results/{task_id}.json   ← AgentTaskResult sidecars
/// ```
///
/// Sidecars are written by the `prompt` command when it detects that it is
/// running as a fleet agent subprocess (via `WONDER_OF_U_TASK_ID`).
#[derive(Clone, Debug)]
pub struct AgentTaskResultStore {
    paths: StoragePaths,
}

impl AgentTaskResultStore {
    /// Creates a new `AgentTaskResultStore` rooted at `base_dir`.
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
        }
    }

    /// Returns the underlying path helper.
    #[must_use]
    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    /// Ensures `tasks/results/` directory exists.
    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.paths.task_results_dir())?;
        Ok(())
    }

    /// Atomically writes an [`AgentTaskResult`] sidecar.
    pub fn write_result(&self, result: &AgentTaskResult) -> Result<()> {
        self.ensure_layout()?;
        write_json_atomically(&self.paths.task_result_path(result.task_id), result)
    }

    /// Reads an [`AgentTaskResult`] sidecar by task id.
    ///
    /// Returns `Err(not_found)` when no sidecar exists for `task_id`.
    pub fn read_result(&self, task_id: TaskId) -> Result<AgentTaskResult> {
        let path = self.paths.task_result_path(task_id);
        if !path.exists() {
            return Err(WonderError::not_found(
                "agent task result",
                task_id.to_string(),
            ));
        }
        let result: AgentTaskResult = serde_json::from_str(&fs::read_to_string(path)?)?;
        ensure_supported_schema("agent task result", result.schema_version)?;
        Ok(result)
    }

    /// Lists all available agent task result sidecars, sorted by `finished_at`
    /// descending.
    ///
    /// Returns an empty list when the results directory does not exist.
    pub fn list_results(&self) -> Result<Vec<AgentTaskResult>> {
        let dir = self.paths.task_results_dir();
        match fs::read_dir(dir) {
            Ok(entries) => {
                let mut results = Vec::new();
                for entry in entries {
                    let entry = entry?;
                    if !entry.file_type()?.is_file() {
                        continue;
                    }
                    if entry.path().extension().and_then(OsStr::to_str) != Some("json") {
                        continue;
                    }
                    let result: AgentTaskResult =
                        serde_json::from_str(&fs::read_to_string(entry.path())?)?;
                    ensure_supported_schema("agent task result", result.schema_version)?;
                    results.push(result);
                }
                results.sort_by(|a, b| {
                    b.finished_at
                        .cmp(&a.finished_at)
                        .then_with(|| a.task_id.cmp(&b.task_id))
                });
                Ok(results)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }
}

// ── FleetInspector ────────────────────────────────────────────────────────────

/// Classification of a single fleet member task's current state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemberObservationClass {
    /// Task record missing or status is `Pending`.
    Pending,
    /// Task is actively running.
    Running,
    /// Task completed successfully.
    Completed,
    /// Task failed, was killed, or was cancelled.
    Failed,
}

impl MemberObservationClass {
    /// Returns `true` when this class represents a terminal state.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }

    /// Human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// Observation for a single fleet member task.
#[derive(Clone, Debug)]
pub struct FleetMemberObservation {
    /// The member's task id.
    pub task_id: TaskId,
    /// Derived classification from the task status.
    pub class: MemberObservationClass,
    /// Live task state, if readable.
    pub task: Option<TaskState>,
    /// Result sidecar, if available.
    pub result: Option<AgentTaskResult>,
}

/// Snapshot observation of a fleet run and all of its member tasks.
#[derive(Clone, Debug)]
pub struct FleetObservation {
    /// The observed fleet id.
    pub fleet_id: FleetId,
    /// The fleet run state.
    pub fleet: wonder_of_u_core::FleetRunState,
    /// Per-member observations (in member_task_ids order).
    pub members: Vec<FleetMemberObservation>,
}

impl FleetObservation {
    /// Returns `true` when every member is in a terminal state.
    #[must_use]
    pub fn all_terminal(&self) -> bool {
        self.members.iter().all(|m| m.class.is_terminal())
    }

    /// Counts members by class.
    #[must_use]
    pub fn count_by_class(&self, class: MemberObservationClass) -> usize {
        self.members.iter().filter(|m| m.class == class).count()
    }
}

/// Read-only inspector that correlates fleet runs, task states, and result
/// sidecars into a single [`FleetObservation`].
///
/// Concurrency-safe: each call reads files independently; no shared state is
/// held between calls.
#[derive(Clone, Debug)]
pub struct FleetInspector {
    fleet_store: FleetStore,
    task_store: TaskStore,
    result_store: AgentTaskResultStore,
}

impl FleetInspector {
    /// Creates a new `FleetInspector` rooted at `base_dir`.
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        let base_dir = base_dir.into();
        Self {
            fleet_store: FleetStore::new(&base_dir),
            task_store: TaskStore::new(&base_dir),
            result_store: AgentTaskResultStore::new(&base_dir),
        }
    }

    /// Observes a fleet run: loads its [`FleetRunState`], then for each member
    /// task id loads the [`TaskState`] and any available [`AgentTaskResult`]
    /// sidecar.
    ///
    /// Individual task / result read failures are silenced — missing records
    /// are represented as `None` in the observation so callers can still
    /// inspect the members that are readable.
    pub fn observe(&self, fleet_id: FleetId) -> Result<FleetObservation> {
        let fleet = self.fleet_store.read_run(fleet_id)?;
        let mut members = Vec::with_capacity(fleet.member_task_ids.len());
        for &task_id in &fleet.member_task_ids {
            let task = self.task_store.read_task(task_id).ok();
            let result = self.result_store.read_result(task_id).ok();
            let class = classify_member(task.as_ref());
            members.push(FleetMemberObservation {
                task_id,
                class,
                task,
                result,
            });
        }
        Ok(FleetObservation {
            fleet_id,
            fleet,
            members,
        })
    }
}

/// Derives a [`MemberObservationClass`] from an optional [`TaskState`].
fn classify_member(task: Option<&TaskState>) -> MemberObservationClass {
    match task {
        None => MemberObservationClass::Pending,
        Some(t) => match t.status {
            TaskStatus::Pending => MemberObservationClass::Pending,
            TaskStatus::Running => MemberObservationClass::Running,
            TaskStatus::Completed => MemberObservationClass::Completed,
            TaskStatus::Failed | TaskStatus::Killed | TaskStatus::Cancelled => {
                MemberObservationClass::Failed
            }
        },
    }
}

/// Persistent store for fleet runs and pending member requests.
///
/// Layout under the storage base directory:
///
/// ```text
/// fleet/
///   runs/{fleet_id}.json        ← FleetRunState records
///   pending/{request_id}.json   ← FleetMemberRequest records awaiting dispatch
/// ```
#[derive(Clone, Debug)]
pub struct FleetStore {
    paths: StoragePaths,
}

impl FleetStore {
    /// Creates a new `FleetStore` rooted at `base_dir`.
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
        }
    }

    /// Returns the underlying path helper.
    #[must_use]
    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    /// Creates `fleet/runs/` and `fleet/pending/` if they do not exist.
    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.paths.fleet_runs_dir())?;
        fs::create_dir_all(self.paths.fleet_pending_dir())?;
        Ok(())
    }

    // ── Run CRUD ─────────────────────────────────────────────────────────────

    /// Atomically writes a [`FleetRunState`] to disk.
    pub fn write_run(&self, run: &FleetRunState) -> Result<()> {
        self.ensure_layout()?;
        write_json_atomically(&self.paths.fleet_run_path(run.id), run)
    }

    /// Reads a [`FleetRunState`] by id.
    pub fn read_run(&self, fleet_id: FleetId) -> Result<FleetRunState> {
        let path = self.paths.fleet_run_path(fleet_id);
        if !path.exists() {
            return Err(WonderError::not_found("fleet run", fleet_id.to_string()));
        }
        let run: FleetRunState = serde_json::from_str(&fs::read_to_string(path)?)?;
        ensure_supported_schema("fleet run", run.schema_version)?;
        Ok(run)
    }

    /// Lists all stored fleet runs, sorted by `started_at` descending.
    ///
    /// Returns an empty list when the runs directory does not exist.
    pub fn list_runs(&self) -> Result<Vec<FleetRunState>> {
        let dir = self.paths.fleet_runs_dir();
        match fs::read_dir(dir) {
            Ok(entries) => {
                let mut runs = Vec::new();
                for entry in entries {
                    let entry = entry?;
                    if !entry.file_type()?.is_file() {
                        continue;
                    }
                    if entry.path().extension().and_then(OsStr::to_str) != Some("json") {
                        continue;
                    }
                    let run: FleetRunState =
                        serde_json::from_str(&fs::read_to_string(entry.path())?)?;
                    ensure_supported_schema("fleet run", run.schema_version)?;
                    runs.push(run);
                }
                runs.sort_by(|a, b| {
                    b.started_at
                        .cmp(&a.started_at)
                        .then_with(|| a.id.cmp(&b.id))
                });
                Ok(runs)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    // ── Pending requests ──────────────────────────────────────────────────────

    /// Atomically writes a [`FleetMemberRequest`] to `fleet/pending/`.
    pub fn queue_member_request(&self, request: &FleetMemberRequest) -> Result<()> {
        self.ensure_layout()?;
        write_json_atomically(
            &self
                .paths
                .fleet_pending_path_for(request.fleet_id, &request.id),
            request,
        )
    }

    /// Reads a single pending request by its id.
    pub fn read_pending_request(&self, request_id: &str) -> Result<FleetMemberRequest> {
        let path = self.paths.fleet_pending_path(request_id);
        if !path.exists() {
            return Err(WonderError::not_found("fleet pending request", request_id));
        }
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    /// Reads a pending request using the request's fleet-scoped path.
    pub fn read_pending_request_for(
        &self,
        request: &FleetMemberRequest,
    ) -> Result<FleetMemberRequest> {
        let path = self
            .paths
            .fleet_pending_path_for(request.fleet_id, &request.id);
        if !path.exists() {
            return Err(WonderError::not_found(
                "fleet pending request",
                request.id.clone(),
            ));
        }
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    /// Lists all pending requests, sorted by `queued_at` ascending (oldest
    /// first so that `fleet dispatch` processes them in queue order).
    ///
    /// Returns an empty list when the pending directory does not exist.
    pub fn list_pending_requests(&self) -> Result<Vec<FleetMemberRequest>> {
        let dir = self.paths.fleet_pending_dir();
        match fs::read_dir(dir) {
            Ok(entries) => {
                let mut requests = Vec::new();
                for entry in entries {
                    let entry = entry?;
                    if !entry.file_type()?.is_file() {
                        continue;
                    }
                    if entry.path().extension().and_then(OsStr::to_str) != Some("json") {
                        continue;
                    }
                    let request: FleetMemberRequest =
                        serde_json::from_str(&fs::read_to_string(entry.path())?)?;
                    requests.push(request);
                }
                requests
                    .sort_by(|a, b| a.queued_at.cmp(&b.queued_at).then_with(|| a.id.cmp(&b.id)));
                Ok(requests)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    /// Deletes a pending request file after successful dispatch.
    ///
    /// Idempotent: if the file has already been removed this returns `Ok(())`.
    pub fn delete_pending_request(&self, request_id: &str) -> Result<()> {
        let path = self.paths.fleet_pending_path(request_id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Deletes a pending request file using its fleet-scoped path.
    ///
    /// Idempotent: if the file has already been removed this returns `Ok(())`.
    pub fn delete_pending_request_for(&self, request: &FleetMemberRequest) -> Result<()> {
        let path = self
            .paths
            .fleet_pending_path_for(request.fleet_id, &request.id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(test)]
mod fleet_store_tests {
    use wonder_of_u_core::{FleetRunState, PermissionMode};
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn make_store(prefix: &str) -> FleetStore {
        FleetStore::new(unique_test_dir(prefix))
    }

    #[test]
    fn write_and_read_run() {
        let store = make_store("fleet-write-read");
        let run = FleetRunState::new("test fleet", PermissionMode::Default, None);
        let id = run.id;
        store.write_run(&run).expect("write");
        let loaded = store.read_run(id).expect("read");
        assert_eq!(loaded.id, id);
        assert_eq!(loaded.description, "test fleet");
    }

    #[test]
    fn list_runs_empty_when_dir_absent() {
        let store = make_store("fleet-list-absent");
        // Do not call ensure_layout so dirs never exist.
        let runs = store.list_runs().expect("list");
        assert!(runs.is_empty());
    }

    #[test]
    fn list_runs_sorted_by_started_at_desc() {
        let store = make_store("fleet-list-sorted");
        for desc in ["alpha", "beta", "gamma"] {
            let run = FleetRunState::new(desc, PermissionMode::Default, None);
            store.write_run(&run).expect("write");
        }
        let runs = store.list_runs().expect("list");
        assert_eq!(runs.len(), 3);
        // Most recently started should come first.
        for window in runs.windows(2) {
            assert!(window[0].started_at >= window[1].started_at);
        }
    }

    #[test]
    fn queue_and_list_pending_requests() {
        let store = make_store("fleet-pending-queue");
        let req = FleetMemberRequest::new("do the thing");
        let id = req.id.clone();
        store.queue_member_request(&req).expect("queue");
        let pending = store.list_pending_requests().expect("list");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);
        assert_eq!(pending[0].prompt, "do the thing");
    }

    #[test]
    fn fleet_pending_requests_are_namespaced_by_fleet_id() {
        let store = make_store("fleet-pending-namespaced");
        let fleet_a = FleetId::new();
        let fleet_b = FleetId::new();

        let mut req_a = FleetMemberRequest::new("build a");
        req_a.id = "build".into();
        req_a.fleet_id = Some(fleet_a);
        let mut req_b = FleetMemberRequest::new("build b");
        req_b.id = "build".into();
        req_b.fleet_id = Some(fleet_b);

        store.queue_member_request(&req_a).expect("queue a");
        store.queue_member_request(&req_b).expect("queue b");

        let pending = store.list_pending_requests().expect("list");
        assert_eq!(pending.len(), 2);
        assert_eq!(
            store
                .read_pending_request_for(&req_a)
                .expect("read a")
                .prompt,
            "build a"
        );
        assert_eq!(
            store
                .read_pending_request_for(&req_b)
                .expect("read b")
                .prompt,
            "build b"
        );

        store
            .delete_pending_request_for(&req_a)
            .expect("delete only a");
        assert!(store.read_pending_request_for(&req_a).is_err());
        assert!(store.read_pending_request_for(&req_b).is_ok());
    }

    #[test]
    fn list_pending_empty_when_dir_absent() {
        let store = make_store("fleet-pending-absent");
        let pending = store.list_pending_requests().expect("list");
        assert!(pending.is_empty());
    }

    #[test]
    fn delete_pending_request_removes_file() {
        let store = make_store("fleet-pending-delete");
        let req = FleetMemberRequest::new("ephemeral");
        let id = req.id.clone();
        store.queue_member_request(&req).expect("queue");
        assert_eq!(store.list_pending_requests().expect("list").len(), 1);
        store.delete_pending_request(&id).expect("delete");
        assert!(store.list_pending_requests().expect("list").is_empty());
    }

    #[test]
    fn delete_nonexistent_pending_request_is_idempotent() {
        let store = make_store("fleet-pending-delete-idempotent");
        store.ensure_layout().expect("layout");
        // Should not error.
        store
            .delete_pending_request("00000000-0000-0000-0000-000000000000")
            .expect("idempotent delete");
    }

    #[test]
    fn read_missing_run_returns_not_found() {
        let store = make_store("fleet-read-missing");
        store.ensure_layout().expect("layout");
        let fleet_id = FleetId::new();
        let err = store.read_run(fleet_id).expect_err("missing");
        assert!(err.to_string().contains("not found"));
    }
}

#[cfg(test)]
mod agent_task_result_store_tests {
    use time::OffsetDateTime;
    use wonder_of_u_core::{AGENT_TASK_RESULT_SCHEMA_VERSION, AgentTaskResult, TaskId, TaskStatus};
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn make_store(prefix: &str) -> AgentTaskResultStore {
        AgentTaskResultStore::new(unique_test_dir(prefix))
    }

    #[test]
    fn write_and_read_result() {
        let store = make_store("result-write-read");
        let task_id = TaskId::new();
        let result = AgentTaskResult {
            schema_version: AGENT_TASK_RESULT_SCHEMA_VERSION,
            task_id,
            fleet_id: None,
            fleet_request_id: Some("req-1".into()),
            session_id: None,
            status: TaskStatus::Completed,
            output_excerpt: "summary of work".into(),
            output_text: Some("full output text here".into()),
            provider: Some("anthropic".into()),
            model: None,
            finished_at: OffsetDateTime::now_utc(),
        };
        store.write_result(&result).expect("write");
        let loaded = store.read_result(task_id).expect("read");
        assert_eq!(loaded.task_id, task_id);
        assert_eq!(loaded.status, TaskStatus::Completed);
        assert_eq!(loaded.fleet_request_id.as_deref(), Some("req-1"));
        assert_eq!(loaded.output_text.as_deref(), Some("full output text here"));
    }

    #[test]
    fn read_missing_result_returns_not_found() {
        let store = make_store("result-missing");
        store.ensure_layout().expect("layout");
        let err = store.read_result(TaskId::new()).expect_err("missing");
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn list_results_empty_when_dir_absent() {
        let store = make_store("result-list-absent");
        let results = store.list_results().expect("list");
        assert!(results.is_empty());
    }

    #[test]
    fn list_results_sorted_by_finished_at_desc() {
        let store = make_store("result-list-sorted");
        for i in 0u32..3 {
            let task_id = TaskId::new();
            // Vary finished_at by sleeping a small amount — or use fixed offsets.
            let finished_at = OffsetDateTime::from_unix_timestamp(1_700_000_000 + i64::from(i))
                .expect("timestamp");
            let r = AgentTaskResult {
                schema_version: AGENT_TASK_RESULT_SCHEMA_VERSION,
                task_id,
                fleet_id: None,
                fleet_request_id: None,
                session_id: None,
                status: TaskStatus::Completed,
                output_excerpt: format!("result {i}"),
                output_text: None,
                provider: None,
                model: None,
                finished_at,
            };
            store.write_result(&r).expect("write");
        }
        let results = store.list_results().expect("list");
        assert_eq!(results.len(), 3);
        // Most recent first.
        for window in results.windows(2) {
            assert!(window[0].finished_at >= window[1].finished_at);
        }
    }

    #[test]
    fn list_results_rejects_unsupported_schema() {
        let store = make_store("result-list-schema");
        store.ensure_layout().expect("layout");
        let task_id = TaskId::new();
        let path = store.paths().task_result_path(task_id);
        let result = AgentTaskResult {
            schema_version: AGENT_TASK_RESULT_SCHEMA_VERSION,
            task_id,
            fleet_id: None,
            fleet_request_id: None,
            session_id: None,
            status: TaskStatus::Completed,
            output_excerpt: String::new(),
            output_text: None,
            provider: None,
            model: None,
            finished_at: OffsetDateTime::now_utc(),
        };
        let mut value = serde_json::to_value(&result).expect("result json");
        value["schema_version"] = serde_json::json!(AGENT_TASK_RESULT_SCHEMA_VERSION + 1);
        fs::write(path, value.to_string()).expect("write unsupported schema");

        let err = store.list_results().expect_err("unsupported schema");
        assert!(err.to_string().contains("unsupported"), "got: {err}");
    }

    #[test]
    fn failed_result_round_trips() {
        let store = make_store("result-failed");
        let task_id = TaskId::new();
        let result = AgentTaskResult {
            schema_version: AGENT_TASK_RESULT_SCHEMA_VERSION,
            task_id,
            fleet_id: None,
            fleet_request_id: None,
            session_id: None,
            status: TaskStatus::Failed,
            output_excerpt: String::new(),
            output_text: None,
            provider: None,
            model: None,
            finished_at: OffsetDateTime::now_utc(),
        };
        store.write_result(&result).expect("write");
        let loaded = store.read_result(task_id).expect("read");
        assert_eq!(loaded.status, TaskStatus::Failed);
    }
}

#[cfg(test)]
mod fleet_inspector_tests {
    use time::OffsetDateTime;
    use wonder_of_u_core::{
        AGENT_TASK_RESULT_SCHEMA_VERSION, AgentRuntime, AgentTaskResult, AgentTaskState, FleetId,
        FleetRunState, PermissionMode, TaskId, TaskKind, TaskState, TaskStatus,
    };
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn setup(prefix: &str) -> (PathBuf, FleetInspector) {
        let dir = unique_test_dir(prefix);
        let inspector = FleetInspector::new(&dir);
        (dir, inspector)
    }

    fn write_minimal_fleet(dir: &Path, fleet_id: FleetId, task_ids: &[TaskId]) {
        let fleet_store = FleetStore::new(dir);
        let mut run = FleetRunState::new("test fleet", PermissionMode::Default, None);
        // Override the auto-generated id with the one we were given.
        run.id = fleet_id;
        run.member_task_ids = task_ids.to_vec();
        fleet_store.write_run(&run).expect("write fleet");
    }

    fn write_task_with_status(dir: &Path, task_id: TaskId, status: TaskStatus) {
        let task_store = TaskStore::new(dir);
        let task = TaskState {
            id: task_id,
            kind: TaskKind::LocalAgent,
            description: "test agent".into(),
            status,
            fleet_id: None,
            fleet_request_id: None,
            parent_id: None,
            cwd: None,
            command: None,
            status_message: None,
            pid: None,
            process_identity: None,
            last_heartbeat_at: None,
            exit_code: None,
            agent: Some(AgentTaskState {
                name: "test".into(),
                prompt: None,
                provider: None,
                model: None,
                runtime: AgentRuntime::PromptSubprocess,
            }),
            remote: None,
            output_log: None,
            worktree_branch: None,
            started_at: OffsetDateTime::now_utc(),
            finished_at: if status.is_terminal() {
                Some(OffsetDateTime::now_utc())
            } else {
                None
            },
        };
        task_store.write_task(&task).expect("write task");
    }

    fn write_result(dir: &Path, task_id: TaskId, status: TaskStatus) {
        let result_store = AgentTaskResultStore::new(dir);
        let result = AgentTaskResult {
            schema_version: AGENT_TASK_RESULT_SCHEMA_VERSION,
            task_id,
            fleet_id: None,
            fleet_request_id: None,
            session_id: None,
            status,
            output_excerpt: "done".into(),
            output_text: None,
            provider: None,
            model: None,
            finished_at: OffsetDateTime::now_utc(),
        };
        result_store.write_result(&result).expect("write result");
    }

    #[test]
    fn observe_pending_task_classified_pending() {
        let (dir, inspector) = setup("inspector-pending");
        let fleet_id = FleetId::new();
        let task_id = TaskId::new();
        write_minimal_fleet(&dir, fleet_id, &[task_id]);
        write_task_with_status(&dir, task_id, TaskStatus::Pending);

        let obs = inspector.observe(fleet_id).expect("observe");
        assert_eq!(obs.members.len(), 1);
        assert_eq!(obs.members[0].class, MemberObservationClass::Pending);
    }

    #[test]
    fn observe_running_task_classified_running() {
        let (dir, inspector) = setup("inspector-running");
        let fleet_id = FleetId::new();
        let task_id = TaskId::new();
        write_minimal_fleet(&dir, fleet_id, &[task_id]);
        write_task_with_status(&dir, task_id, TaskStatus::Running);

        let obs = inspector.observe(fleet_id).expect("observe");
        assert_eq!(obs.members[0].class, MemberObservationClass::Running);
        assert!(!obs.all_terminal());
    }

    #[test]
    fn observe_completed_task_with_result_sidecar() {
        let (dir, inspector) = setup("inspector-completed");
        let fleet_id = FleetId::new();
        let task_id = TaskId::new();
        write_minimal_fleet(&dir, fleet_id, &[task_id]);
        write_task_with_status(&dir, task_id, TaskStatus::Completed);
        write_result(&dir, task_id, TaskStatus::Completed);

        let obs = inspector.observe(fleet_id).expect("observe");
        assert_eq!(obs.members[0].class, MemberObservationClass::Completed);
        assert!(obs.members[0].result.is_some());
        assert!(obs.all_terminal());
    }

    #[test]
    fn observe_failed_task_classified_failed() {
        let (dir, inspector) = setup("inspector-failed");
        let fleet_id = FleetId::new();
        let task_id = TaskId::new();
        write_minimal_fleet(&dir, fleet_id, &[task_id]);
        write_task_with_status(&dir, task_id, TaskStatus::Failed);

        let obs = inspector.observe(fleet_id).expect("observe");
        assert_eq!(obs.members[0].class, MemberObservationClass::Failed);
        assert!(obs.all_terminal());
    }

    #[test]
    fn observe_missing_task_classified_pending() {
        // Task id listed in fleet but no task file written yet.
        let (dir, inspector) = setup("inspector-no-task");
        let fleet_id = FleetId::new();
        let task_id = TaskId::new();
        write_minimal_fleet(&dir, fleet_id, &[task_id]);
        // Do NOT write a task file.

        let obs = inspector.observe(fleet_id).expect("observe");
        assert_eq!(obs.members[0].class, MemberObservationClass::Pending);
        assert!(obs.members[0].task.is_none());
        assert!(obs.members[0].result.is_none());
    }

    #[test]
    fn observe_count_by_class_aggregates_correctly() {
        let (dir, inspector) = setup("inspector-counts");
        let fleet_id = FleetId::new();
        let t1 = TaskId::new();
        let t2 = TaskId::new();
        let t3 = TaskId::new();
        write_minimal_fleet(&dir, fleet_id, &[t1, t2, t3]);
        write_task_with_status(&dir, t1, TaskStatus::Completed);
        write_task_with_status(&dir, t2, TaskStatus::Failed);
        write_task_with_status(&dir, t3, TaskStatus::Running);

        let obs = inspector.observe(fleet_id).expect("observe");
        assert_eq!(obs.count_by_class(MemberObservationClass::Completed), 1);
        assert_eq!(obs.count_by_class(MemberObservationClass::Failed), 1);
        assert_eq!(obs.count_by_class(MemberObservationClass::Running), 1);
        assert!(!obs.all_terminal());
    }
}
/// Stores paste store
#[derive(Clone, Debug)]
pub struct PasteStore {
    paths: StoragePaths,
    max_bytes: usize,
}

impl PasteStore {
    /// Creates a new value
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self::with_max_bytes(base_dir, DEFAULT_PASTE_MAX_BYTES)
    }
    /// Handles with max bytes
    #[must_use]
    pub fn with_max_bytes(base_dir: impl Into<PathBuf>, max_bytes: usize) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
            max_bytes,
        }
    }
    /// Handles paths
    #[must_use]
    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }
    /// Constant fn
    #[must_use]
    pub const fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Handles ensure layout
    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.paths.pastes_dir())?;
        Ok(())
    }

    /// Handles store
    pub fn store(&self, content: impl AsRef<[u8]>) -> Result<StoredPaste> {
        let content = content.as_ref();
        if content.len() > self.max_bytes {
            return Err(WonderError::validation(format!(
                "paste exceeds {} byte limit",
                self.max_bytes
            )));
        }

        self.ensure_layout()?;
        let sha256 = format!("{:x}", Sha256::digest(content));
        let path = self.paths.paste_path(&sha256);
        if !path.exists() {
            let mut file = File::create(&path)?;
            file.write_all(content)?;
            file.sync_all()?;
        }

        Ok(StoredPaste {
            sha256,
            bytes: content.len(),
        })
    }

    /// Handles load
    pub fn load(&self, sha256: &str) -> Result<Vec<u8>> {
        validate_paste_hash(sha256)?;
        let path = self.paths.paste_path(sha256);
        if !path.exists() {
            return Err(WonderError::not_found("paste", sha256));
        }
        Ok(fs::read(path)?)
    }

    /// Loads text
    pub fn load_text(&self, sha256: &str) -> Result<String> {
        String::from_utf8(self.load(sha256)?)
            .map_err(|error| WonderError::validation(format!("paste is not valid UTF-8: {error}")))
    }
}
/// Represents loaded transcript
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoadedTranscript {
    /// Stores the messages
    pub messages: Vec<MessageEnvelope>,
    /// Stores the warnings
    pub warnings: Vec<TranscriptWarning>,
}
/// Enumerates session resume source
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionResumeSource {
    /// Represents snapshot
    Snapshot,
    /// Represents transcript
    Transcript,
}

impl SessionResumeSource {
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Transcript => "transcript",
        }
    }
}
/// Represents session snapshot
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    /// Stores the schema version
    #[serde(default = "default_storage_schema_version")]
    pub schema_version: u16,
    /// Stores the session identifier
    pub session_id: SessionId,
    /// Stores the state
    pub state: AppState,
    /// Stores the transcript message count
    #[serde(default)]
    pub transcript_message_count: usize,
    /// Stores the transcript warning count
    #[serde(default)]
    pub transcript_warning_count: usize,
}

impl SessionSnapshot {
    /// Handles from app state
    #[must_use]
    pub fn from_app_state(
        state: &AppState,
        transcript_message_count: usize,
        transcript_warning_count: usize,
    ) -> Self {
        Self {
            schema_version: STORAGE_SCHEMA_VERSION,
            session_id: state.session.id,
            state: state.clone(),
            transcript_message_count,
            transcript_warning_count,
        }
    }
}
/// Represents restored session
#[derive(Clone, Debug, PartialEq)]
pub struct RestoredSession {
    /// Stores the metadata
    pub metadata: SessionMetadata,
    /// Stores the transcript
    pub transcript: LoadedTranscript,
    /// Stores the state
    pub state: AppState,
    /// Stores the resume source
    pub resume_source: SessionResumeSource,
}
/// Represents transcript warning
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TranscriptWarning {
    /// Stores the line
    pub line: usize,
    /// Stores the message
    pub message: String,
}
/// Represents session metadata
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionMetadata {
    /// Stores the schema version
    #[serde(default = "default_storage_schema_version")]
    pub schema_version: u16,
    /// Stores the session identifier
    pub session_id: SessionId,
    /// Stores the title
    pub title: String,
    /// Stores the cwd
    pub cwd: PathBuf,
    /// Stores the git branch
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    /// Stores the entrypoint
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    /// Stores the app version
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    /// Stores the created at
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Stores the updated at
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// Stores the message count
    pub message_count: usize,
    /// Stores the tags
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Stores the provider
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Stores the model
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Stores the auth
    #[serde(default)]
    pub auth: wonder_of_u_core::AuthState,
    /// Stores the costs
    #[serde(default)]
    pub costs: CostState,
}

impl SessionMetadata {
    /// Handles from app state
    #[must_use]
    pub fn from_app_state(state: &AppState) -> Self {
        Self {
            schema_version: STORAGE_SCHEMA_VERSION,
            session_id: state.session.id,
            title: state.session.title.clone(),
            cwd: state.session.cwd.clone(),
            git_branch: state.session.git_branch.clone(),
            entrypoint: state.session.entrypoint.clone(),
            app_version: state.session.app_version.clone(),
            created_at: state.session.created_at,
            updated_at: state.session.updated_at,
            message_count: state.messages.len(),
            tags: state.session.tags.clone(),
            provider: state.provider.clone(),
            model: state.model.clone(),
            auth: state.auth.clone(),
            costs: state.costs.clone(),
        }
    }
    /// Handles from state with transcript
    #[must_use]
    pub fn from_state_with_transcript(state: &AppState, transcript_message_count: usize) -> Self {
        let mut metadata = Self::from_app_state(state);
        metadata.message_count = transcript_message_count;
        metadata
    }
}
/// Represents session cost ledger
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionCostLedger {
    /// Stores the schema version
    #[serde(default = "default_storage_schema_version")]
    pub schema_version: u16,
    /// Stores the session identifier
    pub session_id: SessionId,
    /// Stores the provider
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Stores the model
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Stores the costs
    pub costs: CostState,
}

impl SessionCostLedger {
    /// Handles from app state
    #[must_use]
    pub fn from_app_state(state: &AppState) -> Self {
        Self {
            schema_version: STORAGE_SCHEMA_VERSION,
            session_id: state.session.id,
            provider: state.provider.clone(),
            model: state.model.clone(),
            costs: state.costs.clone(),
        }
    }
}
/// Represents stored paste
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoredPaste {
    /// Stores the sha256
    pub sha256: String,
    /// Stores the bytes
    pub bytes: usize,
}

fn apply_metadata_to_state(state: &mut AppState, metadata: &SessionMetadata) {
    state.session.id = metadata.session_id;
    state.session.title = metadata.title.clone();
    state.session.cwd = metadata.cwd.clone();
    state.session.git_branch = metadata.git_branch.clone();
    state.session.entrypoint = metadata.entrypoint.clone();
    state.session.app_version = metadata.app_version.clone();
    state.session.tags = metadata.tags.clone();
    state.session.created_at = metadata.created_at;
    state.session.updated_at = metadata.updated_at;
    state.provider = metadata.provider.clone();
    state.model = metadata.model.clone();
    state.auth = metadata.auth.clone();
    state.costs = metadata.costs.clone();
}

#[cfg(test)]
mod tests {
    use std::{fs, fs::OpenOptions, io::Write};

    use serde_json::json;
    use tempfile::TempDir;
    use time::format_description::well_known::Rfc3339;
    use wonder_of_u_core::{
        AgentTaskState, AppState, InputMode, MessageEnvelope, MessagePayload, QueuePlacement,
        RemoteTaskMetadata, RemoteTaskState, RemoteTaskType, TaskId, TaskKind, TaskState,
        TaskStatus, TokenUsage, ToolUseId,
    };
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn temp_dir() -> TempDir {
        tempfile::tempdir().expect("create temp dir")
    }

    fn fixed_timestamp() -> OffsetDateTime {
        OffsetDateTime::parse("2024-01-02T03:04:05Z", &Rfc3339).expect("parse fixed timestamp")
    }

    fn transcript_message(session_id: SessionId, payload: MessagePayload) -> MessageEnvelope {
        let mut message = MessageEnvelope::new(session_id, payload)
            .with_context(Some(PathBuf::from("/workspace")), Some("main".into()))
            .with_runtime(Some("doctor".into()), Some("0.1.0".into()));
        message.timestamp = fixed_timestamp();
        message
    }

    #[test]
    fn transcript_roundtrip_all_message_variants() {
        let dir = temp_dir();
        let store = TranscriptStore::new(dir.path());
        let session_id = SessionId::new();
        let messages = vec![
            transcript_message(
                session_id,
                MessagePayload::UserText {
                    content: "hello".into(),
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::UserAttachment {
                    label: "spec".into(),
                    uri: "file:///workspace/spec.md".into(),
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::AssistantText {
                    content: "hi there".into(),
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::AssistantThinking {
                    content: "reasoning".into(),
                    collapsed: true,
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::AssistantToolUse {
                    tool: "bash".into(),
                    use_id: ToolUseId::new(),
                    input: json!({
                        "command": "cargo test -p wonder-of-u-storage",
                        "cwd": "/workspace",
                    }),
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::ToolResult {
                    tool: "bash".into(),
                    use_id: ToolUseId::new(),
                    success: true,
                    content: "ok".into(),
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::BashOutput {
                    stdout: "done".into(),
                    stderr: String::new(),
                    exit_code: Some(0),
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::System {
                    content: "ready".into(),
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::Progress {
                    label: "loading".into(),
                    detail: Some("50%".into()),
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::Command {
                    input: "/status".into(),
                    output: Some("healthy".into()),
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::HookResult {
                    hook: "preflight".into(),
                    success: true,
                    output: "passed".into(),
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::CompactBoundary {
                    summary: "Compacted 3 messages".into(),
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::Task {
                    task_id: TaskId::new(),
                    status: TaskStatus::Running,
                    message: "running tests".into(),
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::Permission {
                    tool: "bash".into(),
                    decision: "approved".into(),
                    reason: "workspace-scoped".into(),
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::PlanApproval {
                    summary: "Ship transcript recovery tests".into(),
                    approved: true,
                },
            ),
        ];

        for message in &messages {
            store
                .append_message(message)
                .expect("append transcript message");
        }
        let loaded = store.load_session(session_id).expect("load session");

        assert_eq!(loaded.messages, messages);
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn transcript_recovers_corrupt_trailing_line() {
        let dir = temp_dir();
        let store = TranscriptStore::new(dir.path());
        let session_id = SessionId::new();
        let messages = vec![
            transcript_message(
                session_id,
                MessagePayload::UserText {
                    content: "hello".into(),
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::AssistantText {
                    content: "world".into(),
                },
            ),
            transcript_message(
                session_id,
                MessagePayload::System {
                    content: "ready".into(),
                },
            ),
        ];
        for message in &messages {
            store.append_message(message).expect("append");
        }

        let mut file = OpenOptions::new()
            .append(true)
            .open(store.paths().transcript_path(session_id))
            .expect("open transcript");
        file.write_all(br#"{"schema_version":1,"payload":"partial""#)
            .expect("write corrupt trailing line");

        let loaded = store.load_session(session_id).expect("load session");

        assert_eq!(loaded.messages, messages);
        assert_eq!(loaded.warnings.len(), 1);
        assert_eq!(loaded.warnings[0].line, 4);
        assert!(
            loaded.warnings[0]
                .message
                .contains("ignored corrupt trailing transcript line")
        );
    }

    #[test]
    fn transcript_recovers_corrupt_middle_line_and_preserves_valid_tail() {
        let dir = temp_dir();
        let store = TranscriptStore::new(dir.path());
        let session_id = SessionId::new();
        let head = transcript_message(
            session_id,
            MessagePayload::UserText {
                content: "before".into(),
            },
        );
        let tail = transcript_message(
            session_id,
            MessagePayload::AssistantText {
                content: "after".into(),
            },
        );

        store.append_message(&head).expect("append head");
        store.ensure_layout().expect("ensure layout");

        let path = store.paths().transcript_path(session_id);
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open transcript");
        serde_json::to_writer(
            &mut file,
            &serde_json::json!({
                "id": "00000000-0000-0000-0000-000000000001",
                "session_id": session_id,
                "timestamp": "2024-01-02T03:04:05Z",
                "payload": { "type": "user_text", "content": "ignored" }
            }),
        )
        .expect("write malformed middle line");
        file.write_all(b"\n").expect("newline after malformed line");
        serde_json::to_writer(&mut file, &tail).expect("write tail");
        file.write_all(b"\n").expect("newline after tail");

        let loaded = store.load_session(session_id).expect("load session");

        assert_eq!(loaded.messages, vec![head, tail]);
        assert_eq!(loaded.warnings.len(), 1);
        assert_eq!(loaded.warnings[0].line, 2);
        assert!(
            loaded.warnings[0]
                .message
                .contains("ignored corrupt transcript line")
        );
    }

    #[test]
    fn load_rejects_unsupported_message_schema_version() {
        let dir = unique_test_dir("storage-message-schema");
        let store = TranscriptStore::new(&dir);
        let session_id = SessionId::new();
        let mut message = MessageEnvelope::user_text(session_id, "hello");
        message.schema_version = MESSAGE_SCHEMA_VERSION + 1;
        store.append_message(&message).expect("append");

        let error = store
            .load_session(session_id)
            .expect_err("future schema should fail");

        assert!(
            error
                .to_string()
                .contains("unsupported message schema version")
        );
    }

    #[test]
    fn writes_and_reads_session_metadata() {
        let dir = unique_test_dir("storage-metadata");
        let store = TranscriptStore::new(dir);
        let mut state = AppState::new(PathBuf::from("/workspace"));
        state.session.entrypoint = Some("doctor".into());
        state.session.app_version = Some("0.1.0".into());
        state.session.tags.push("pinned".into());
        state.provider = Some("openai".into());
        state.model = Some("gpt-5".into());
        state.record_cost_usage(
            TokenUsage {
                input_tokens: 100,
                output_tokens: 25,
                cache_creation_tokens: 10,
                cache_read_tokens: 5,
            },
            Some(0.25),
        );
        state
            .push_message(MessageEnvelope::system(state.session.id, "ready"))
            .expect("push");
        let metadata = SessionMetadata::from_app_state(&state);

        store.write_metadata(&metadata).expect("write metadata");
        let loaded = store
            .read_metadata(state.session.id)
            .expect("read metadata");

        assert_eq!(loaded, metadata);
        assert_eq!(loaded.costs.usage.total_tokens(), 140);
        assert_eq!(loaded.auth.status_label(), "not_required");
    }

    #[test]
    fn transcript_paste_reference_expands_on_reload() {
        let dir = unique_test_dir("storage-paste-reference-reload");
        let transcript = TranscriptStore::new(&dir);
        let paste_store = PasteStore::new(&dir);
        let session_id = SessionId::new();
        let paste = paste_store
            .store("large pasted content")
            .expect("store paste");
        let message = MessageEnvelope::user_paste_reference(session_id, &paste.sha256, paste.bytes);

        transcript
            .append_message(&message)
            .expect("append reference");
        let loaded = transcript.load_session(session_id).expect("load session");

        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(
            loaded.messages[0].payload,
            MessagePayload::UserText {
                content: "large pasted content".into()
            }
        );
    }

    #[test]
    fn session_memory_index_rebuilds_and_deduplicates_messages() {
        let dir = temp_dir();
        let transcript = TranscriptStore::new(dir.path());
        let index_store = SessionMemoryIndexStore::new(dir.path());
        let session_id = SessionId::new();
        let user = transcript_message(
            session_id,
            MessagePayload::UserText {
                content: "Remember the release checklist".into(),
            },
        );
        let assistant = transcript_message(
            session_id,
            MessagePayload::AssistantText {
                content: "Remember the release checklist".into(),
            },
        );
        let repeated = transcript_message(
            session_id,
            MessagePayload::UserText {
                content: "Remember   the release checklist".into(),
            },
        );

        transcript.append_message(&user).expect("append user");
        transcript
            .append_message(&assistant)
            .expect("append assistant");
        transcript
            .append_message(&repeated)
            .expect("append repeated user");

        let index = index_store
            .rebuild_from_transcript(&transcript, session_id)
            .expect("rebuild index");

        assert_eq!(index.transcript_message_count, 3);
        assert_eq!(index.indexed_message_count, 3);
        assert_eq!(index.entries.len(), 2);
        let user_entry = index
            .entries
            .iter()
            .find(|entry| entry.source == SessionMemorySource::User)
            .expect("user entry");
        assert_eq!(user_entry.occurrences, 2);
        assert_eq!(user_entry.first_message_id, user.id);
        assert_eq!(user_entry.last_message_id, repeated.id);
        assert_eq!(
            index_store.read(session_id).expect("read persisted index"),
            index
        );
    }

    #[test]
    fn sync_status_report_marks_settings_sync_local_only_and_remote_surfaces_unavailable() {
        let dir = temp_dir();
        let paths = StoragePaths::new(dir.path());
        fs::create_dir_all(paths.config_dir()).expect("create config dir");
        fs::write(paths.settings_path(), "{}\n").expect("write settings");
        fs::write(paths.user_memory_path(), "# user memory\n").expect("write user memory");

        let report = SyncStatusReport::inspect(&paths);

        assert_eq!(report.settings_sync.status, SyncSupport::LocalOnly);
        assert_eq!(report.settings_sync.cloud_status, SyncSupport::Unsupported);
        assert!(report.settings_sync.settings_exists);
        assert!(report.settings_sync.user_memory_exists);
        assert!(!report.settings_sync.cloud_attempted);
        assert_eq!(report.remote_managed_settings.status, SyncSupport::Deferred);
        assert_eq!(report.team_memory_sync.status, SyncSupport::Unsupported);
        assert!(!report.remote_managed_settings.cloud_attempted);
        assert!(!report.team_memory_sync.cloud_attempted);
    }

    #[test]
    fn sync_status_inspection_has_no_cloud_side_effects() {
        let dir = temp_dir();
        let paths = StoragePaths::new(dir.path());

        let report = SyncStatusReport::inspect(&paths);

        assert_eq!(report.settings_sync.status, SyncSupport::LocalOnly);
        let entries = fs::read_dir(dir.path())
            .expect("read temp dir")
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("collect entries");
        assert!(entries.is_empty());
    }

    /// Analytics and experiments surfaces must always be `Unsupported` with
    /// explicit reasons documenting the intentional absences (no Datadog event
    /// sink, no GrowthBook remote evaluation).
    #[test]
    fn sync_status_report_analytics_and_experiments_are_unsupported() {
        let dir = temp_dir();
        let paths = StoragePaths::new(dir.path());

        let report = SyncStatusReport::inspect(&paths);

        assert_eq!(report.analytics.status, SyncSupport::Unsupported);
        assert_eq!(report.analytics.service, "analytics");
        assert!(!report.analytics.cloud_attempted);
        assert!(
            report.analytics.reason.contains("Datadog"),
            "analytics reason should mention Datadog: {}",
            report.analytics.reason
        );

        assert_eq!(report.experiments.status, SyncSupport::Unsupported);
        assert_eq!(report.experiments.service, "experiments");
        assert!(!report.experiments.cloud_attempted);
        assert!(
            report.experiments.reason.contains("GrowthBook"),
            "experiments reason should mention GrowthBook: {}",
            report.experiments.reason
        );
        assert!(
            report.experiments.reason.contains("FeatureSet"),
            "experiments reason should mention FeatureSet: {}",
            report.experiments.reason
        );
    }

    /// Documents the recovery boundary: malformed transcript lines are skipped,
    /// but only lines with a parseable Rust schema version reach the explicit
    /// schema-version guard.
    #[test]
    fn transcript_line_missing_schema_version_is_skipped_before_version_check() {
        let dir = temp_dir();
        let store = TranscriptStore::new(dir.path());
        let session_id = SessionId::new();

        // A valid-looking transcript line that simply omits `schema_version`.
        // This simulates a TS/upstream transcript line that lacks the field.
        let ts_shaped_line = serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "session_id": session_id,
            "timestamp": "2024-01-02T03:04:05Z",
            "payload": { "type": "user_text", "content": "hello" }
        });

        // A valid Rust transcript line that proves recovery continues past the
        // malformed middle entry.
        let sentinel = transcript_message(
            session_id,
            MessagePayload::UserText {
                content: "sentinel".into(),
            },
        );

        // Write directly to the transcript file, bypassing append_message so
        // we can produce a line the encoder would never emit.
        store.ensure_layout().expect("ensure layout");
        let path = store.paths().transcript_path(session_id);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("open transcript");
        serde_json::to_writer(&mut file, &ts_shaped_line).expect("write ts-shaped line");
        file.write_all(b"\n").expect("newline after bad line");
        serde_json::to_writer(&mut file, &sentinel).expect("write sentinel line");
        file.write_all(b"\n").expect("newline after sentinel");

        let loaded = store.load_session(session_id).expect("load session");

        assert_eq!(loaded.messages, vec![sentinel]);
        assert_eq!(loaded.warnings.len(), 1);
        assert_eq!(loaded.warnings[0].line, 1);
        assert!(
            loaded.warnings[0]
                .message
                .contains("ignored corrupt transcript line")
        );
    }

    #[test]
    fn session_metadata_atomic_write() {
        let dir = temp_dir();
        let store = TranscriptStore::new(dir.path());
        let mut state = AppState::new(PathBuf::from("/workspace"));
        state.session.title = "first title".into();
        let mut metadata = SessionMetadata::from_app_state(&state);

        store.write_metadata(&metadata).expect("write metadata");

        let pending_path = store
            .paths()
            .metadata_path(metadata.session_id)
            .with_extension("json.next");
        fs::write(&pending_path, "{stale temp file").expect("write stale metadata temp file");

        metadata.title = "updated title".into();
        metadata.message_count = 7;
        store
            .write_metadata(&metadata)
            .expect("rewrite metadata over stale temp file");

        let raw = fs::read_to_string(store.paths().metadata_path(metadata.session_id))
            .expect("read final metadata");
        let decoded: SessionMetadata =
            serde_json::from_str(&raw).expect("deserialize final metadata");

        assert_eq!(decoded, metadata);
        assert_eq!(
            store
                .read_metadata(metadata.session_id)
                .expect("read metadata"),
            metadata
        );
        assert!(!pending_path.exists());
    }

    #[test]
    fn writes_and_restores_session_snapshot() {
        let dir = unique_test_dir("storage-snapshot");
        let store = TranscriptStore::new(&dir);
        let mut state = AppState::new(PathBuf::from("/workspace"));
        state.input_mode = InputMode::Bash;
        state.queue_command("/status", QueuePlacement::Later);
        state
            .push_message(MessageEnvelope::system(state.session.id, "ready"))
            .expect("push message");
        let snapshot = SessionSnapshot::from_app_state(&state, state.messages.len(), 0);

        store
            .write_metadata(&SessionMetadata::from_app_state(&state))
            .expect("write metadata");
        store
            .append_message(&state.messages[0])
            .expect("append transcript");
        store.write_snapshot(&snapshot).expect("write snapshot");

        let loaded = store
            .read_snapshot(state.session.id)
            .expect("read snapshot");

        assert_eq!(loaded, snapshot);
        assert_eq!(loaded.transcript_message_count, 1);
    }

    #[test]
    fn restore_session_prefers_snapshot_view_over_full_transcript() {
        let dir = unique_test_dir("storage-restore-session");
        let store = TranscriptStore::new(&dir);
        let mut state = AppState::new(PathBuf::from("/workspace"));
        let session_id = state.session.id;
        let first = MessageEnvelope::user_text(session_id, "hello");
        let second = MessageEnvelope::system(session_id, "ready");
        store.append_message(&first).expect("append first");
        store.append_message(&second).expect("append second");

        state.input_mode = InputMode::TaskNotification;
        state.messages = vec![MessageEnvelope::new(
            session_id,
            MessagePayload::CompactBoundary {
                summary: "Compacted 1 message".into(),
            },
        )];
        store
            .write_metadata(&SessionMetadata::from_state_with_transcript(&state, 2))
            .expect("write metadata");
        store
            .write_snapshot(&SessionSnapshot::from_app_state(&state, 2, 0))
            .expect("write snapshot");

        let restored = store.restore_session(session_id).expect("restore session");

        assert_eq!(restored.resume_source, SessionResumeSource::Snapshot);
        assert_eq!(restored.transcript.messages, vec![first, second]);
        assert_eq!(restored.state.messages.len(), 1);
        assert_eq!(restored.state.input_mode, InputMode::TaskNotification);
        assert_eq!(restored.metadata.message_count, 2);
    }

    #[test]
    fn list_metadata_returns_newest_sessions_first() {
        let dir = unique_test_dir("storage-list-metadata");
        let store = TranscriptStore::new(dir);
        let mut first =
            SessionMetadata::from_app_state(&AppState::new(PathBuf::from("/workspace")));
        let mut second =
            SessionMetadata::from_app_state(&AppState::new(PathBuf::from("/workspace/second")));
        second.updated_at += time::Duration::seconds(30);

        store.write_metadata(&first).expect("write first");
        store.write_metadata(&second).expect("write second");

        let listed = store.list_metadata().expect("list metadata");

        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].session_id, second.session_id);
        assert_eq!(listed[1].session_id, first.session_id);
        first.updated_at = listed[1].updated_at;
        assert_eq!(listed[1], first);
    }

    #[test]
    fn writes_and_reads_session_costs() {
        let dir = unique_test_dir("storage-costs");
        let store = CostStore::new(dir);
        let mut state = AppState::new(PathBuf::from("/workspace"));
        state.provider = Some("anthropic".into());
        state.model = Some("claude-3".into());
        state.record_cost_usage(
            TokenUsage {
                input_tokens: 64,
                output_tokens: 16,
                cache_creation_tokens: 0,
                cache_read_tokens: 4,
            },
            Some(0.19),
        );
        let ledger = SessionCostLedger::from_app_state(&state);

        store.write_costs(&ledger).expect("write costs");
        let loaded = store.read_costs(state.session.id).expect("read costs");

        assert_eq!(loaded, ledger);
    }

    #[test]
    fn stores_and_loads_pastes_by_digest() {
        let dir = unique_test_dir("storage-pastes");
        let store = PasteStore::new(dir);

        let first = store.store("large pasted content").expect("store paste");
        let second = store.store("large pasted content").expect("dedupe paste");
        let loaded = store.load_text(&first.sha256).expect("load paste");

        assert_eq!(first, second);
        assert_eq!(loaded, "large pasted content");
        assert!(store.paths().paste_path(&first.sha256).exists());
    }

    #[test]
    fn paste_store_enforces_size_limit() {
        let dir = unique_test_dir("storage-paste-limit");
        let store = PasteStore::with_max_bytes(dir, 8);

        let error = store
            .store("this paste is too large")
            .expect_err("oversized paste should fail");

        assert!(error.to_string().contains("paste exceeds 8 byte limit"));
    }

    #[test]
    fn writes_and_lists_task_metadata_and_logs() {
        let dir = unique_test_dir("storage-tasks");
        let store = TaskStore::new(&dir);
        let mut shell = TaskState::pending_shell("run tests", "cargo test", "/workspace");
        shell.output_log = Some(store.paths().task_log_path(shell.id));
        store.write_task(&shell).expect("write shell task");
        store
            .append_log(shell.id, "first line\nsecond line\n")
            .expect("append shell log");
        store.write_exit_code(shell.id, 0).expect("write exit code");
        let heartbeat_at = OffsetDateTime::now_utc();
        store
            .write_heartbeat_at(shell.id, heartbeat_at)
            .expect("write heartbeat");

        let mut agent = TaskState::pending_agent(
            "planner",
            AgentTaskState::metadata_only(
                "planner",
                "Summarize blockers",
                Some("openai".into()),
                Some("gpt-4.1".into()),
            ),
        );
        agent.output_log = Some(store.paths().task_log_path(agent.id));
        store.write_task(&agent).expect("write agent task");

        let mut remote = TaskState::recorded_remote(
            "cloud review",
            RemoteTaskState::deferred(
                RemoteTaskType::Ultrareview,
                Some(RemoteTaskMetadata::PullRequest {
                    owner: "wonder".into(),
                    repo: "of-u".into(),
                    pr_number: 9,
                }),
            ),
        );
        remote.output_log = Some(store.paths().task_log_path(remote.id));
        store.write_task(&remote).expect("write remote task");

        let listed = store.list_tasks().expect("list tasks");
        assert_eq!(listed.len(), 3);
        assert!(listed.iter().any(|task| task.kind == TaskKind::LocalShell));
        assert!(listed.iter().any(|task| task.kind == TaskKind::LocalAgent));
        assert!(listed.iter().any(|task| task.kind == TaskKind::RemoteAgent));
        let restored_remote = listed
            .iter()
            .find(|task| task.kind == TaskKind::RemoteAgent)
            .expect("remote task");
        assert_eq!(
            restored_remote
                .remote
                .as_ref()
                .expect("remote backend")
                .task_type,
            RemoteTaskType::Ultrareview
        );
        assert_eq!(store.read_exit_code(shell.id).expect("exit code"), Some(0));
        assert_eq!(
            store.read_heartbeat_at(shell.id).expect("heartbeat"),
            Some(heartbeat_at)
        );
        assert_eq!(
            store.read_log_tail(shell.id, 1).expect("tail"),
            vec!["second line".to_string()]
        );
    }

    #[test]
    fn storage_paths_expose_task_paths() {
        let paths = StoragePaths::new("/workspace/.wonder");
        let task_id = TaskId::new();

        assert_eq!(paths.tasks_dir(), PathBuf::from("/workspace/.wonder/tasks"));
        assert_eq!(
            paths.task_metadata_dir(),
            PathBuf::from("/workspace/.wonder/tasks/metadata")
        );
        assert_eq!(
            paths.task_logs_dir(),
            PathBuf::from("/workspace/.wonder/tasks/logs")
        );
        assert_eq!(
            paths.task_state_path(task_id),
            PathBuf::from(format!("/workspace/.wonder/tasks/metadata/{task_id}.json"))
        );
        assert_eq!(
            paths.task_log_path(task_id),
            PathBuf::from(format!("/workspace/.wonder/tasks/logs/{task_id}.log"))
        );
        assert_eq!(
            paths.task_heartbeat_dir(),
            PathBuf::from("/workspace/.wonder/tasks/heartbeat")
        );
        assert_eq!(
            paths.task_heartbeat_path(task_id),
            PathBuf::from(format!(
                "/workspace/.wonder/tasks/heartbeat/{task_id}.heartbeat"
            ))
        );
    }

    #[test]
    fn storage_paths_expose_config_paths() {
        let paths = StoragePaths::new("/workspace/.wonder");

        assert_eq!(
            paths.plugins_dir(),
            PathBuf::from("/workspace/.wonder/plugins")
        );
        assert_eq!(
            paths.skills_dir(),
            PathBuf::from("/workspace/.wonder/skills")
        );
        assert_eq!(
            paths.config_dir(),
            PathBuf::from("/workspace/.wonder/config")
        );
        assert_eq!(
            paths.settings_path(),
            PathBuf::from("/workspace/.wonder/config/settings.json")
        );
        assert_eq!(
            paths.credentials_path(),
            PathBuf::from("/workspace/.wonder/config/credentials.json")
        );
        assert_eq!(
            paths.plugin_settings_path(),
            PathBuf::from("/workspace/.wonder/config/plugins/settings.json")
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Memdir / MEMORY.md truncation utilities
// Mirrors `claude-leak/memdir/memdir.ts` → `truncateEntrypointContent`.
// ─────────────────────────────────────────────────────────────────────────────

/// Maximum lines for the MEMORY.md entrypoint file.
pub const MAX_ENTRYPOINT_LINES: usize = 200;
/// Maximum bytes for the MEMORY.md entrypoint file (~25 KB).
pub const MAX_ENTRYPOINT_BYTES: usize = 25_000;
/// Name of the entrypoint memory file.
pub const ENTRYPOINT_NAME: &str = "MEMORY.md";

/// Result of truncating MEMORY.md content.
#[derive(Clone, Debug)]
pub struct EntrypointTruncation {
    /// The possibly-truncated content, with a warning appended if cut.
    pub content: String,
    /// Stores the line count
    pub line_count: usize,
    /// Stores the byte count
    pub byte_count: usize,
    /// Stores the was line truncated
    pub was_line_truncated: bool,
    /// Stores the was byte truncated
    pub was_byte_truncated: bool,
}

/// Truncate MEMORY.md content to `MAX_ENTRYPOINT_LINES` and `MAX_ENTRYPOINT_BYTES`,
/// appending a warning that names which cap fired.
///
/// Line-truncates first (natural boundary), then byte-truncates at the last
/// newline before the cap to avoid cutting mid-line.
#[must_use]
pub fn truncate_entrypoint_content(raw: &str) -> EntrypointTruncation {
    let trimmed = raw.trim();
    let content_lines: Vec<&str> = trimmed.split('\n').collect();
    let line_count = content_lines.len();
    let byte_count = trimmed.len();

    let was_line_truncated = line_count > MAX_ENTRYPOINT_LINES;
    let was_byte_truncated = byte_count > MAX_ENTRYPOINT_BYTES;

    if !was_line_truncated && !was_byte_truncated {
        return EntrypointTruncation {
            content: trimmed.to_string(),
            line_count,
            byte_count,
            was_line_truncated: false,
            was_byte_truncated: false,
        };
    }

    let mut truncated = if was_line_truncated {
        content_lines[..MAX_ENTRYPOINT_LINES].join("\n")
    } else {
        trimmed.to_string()
    };

    if truncated.len() > MAX_ENTRYPOINT_BYTES {
        let cut_at = truncated[..MAX_ENTRYPOINT_BYTES]
            .rfind('\n')
            .map_or(MAX_ENTRYPOINT_BYTES, |p| p);
        truncated.truncate(cut_at);
    }

    let reason = match (was_byte_truncated, was_line_truncated) {
        (true, false) => format!(
            "{} bytes (limit: {} bytes) — index entries are too long",
            byte_count, MAX_ENTRYPOINT_BYTES
        ),
        (false, true) => format!("{} lines (limit: {})", line_count, MAX_ENTRYPOINT_LINES),
        _ => format!("{} lines and {} bytes", line_count, byte_count),
    };

    truncated.push_str(&format!(
        "\n\n> WARNING: {ENTRYPOINT_NAME} is {reason}. Only part of it was loaded. \
         Keep index entries to one line under ~200 chars; move detail into topic files."
    ));

    EntrypointTruncation {
        content: truncated,
        line_count,
        byte_count,
        was_line_truncated,
        was_byte_truncated,
    }
}

#[cfg(test)]
mod memdir_tests {
    use super::*;

    #[test]
    fn no_truncation_when_under_limits() {
        let content = "# Memory\n\nsome notes here\n";
        let result = truncate_entrypoint_content(content);
        assert!(!result.was_line_truncated);
        assert!(!result.was_byte_truncated);
        assert!(result.content.starts_with("# Memory"));
        assert!(!result.content.contains("WARNING"));
    }

    #[test]
    fn line_truncation_fires_at_limit() {
        let lines: Vec<String> = (0..=MAX_ENTRYPOINT_LINES)
            .map(|i| format!("line {i}"))
            .collect();
        let raw = lines.join("\n");
        let result = truncate_entrypoint_content(&raw);
        assert!(result.was_line_truncated);
        assert!(result.content.contains("WARNING"));
        assert!(result.content.contains("lines"));
    }

    #[test]
    fn byte_truncation_fires_at_limit() {
        let long_line = "x".repeat(MAX_ENTRYPOINT_BYTES + 100);
        let result = truncate_entrypoint_content(&long_line);
        assert!(result.was_byte_truncated);
        assert!(result.content.contains("WARNING"));
    }
}
