//! Append-only session storage primitives.

use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::ErrorKind,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use wonder_of_u_core::{
    AppState, CostState, MESSAGE_SCHEMA_VERSION, MessageEnvelope, Result, SessionId, TaskId,
    TaskState, WonderError,
};

pub const STORAGE_SCHEMA_VERSION: u16 = 1;
pub const DEFAULT_PASTE_MAX_BYTES: usize = 1024 * 1024;

fn default_storage_schema_version() -> u16 {
    STORAGE_SCHEMA_VERSION
}

fn ensure_supported_schema(kind: &str, version: u16) -> Result<()> {
    let supported = match kind {
        "message" => MESSAGE_SCHEMA_VERSION,
        _ => STORAGE_SCHEMA_VERSION,
    };

    if version == supported {
        return Ok(());
    }

    Err(WonderError::validation(format!(
        "unsupported {kind} schema version: {version}"
    )))
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

fn validate_paste_hash(sha256: &str) -> Result<()> {
    if sha256.len() == 64 && sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(());
    }

    Err(WonderError::validation(format!(
        "invalid paste hash: {sha256}"
    )))
}

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

    #[must_use]
    pub fn snapshot_dir(&self) -> PathBuf {
        self.sessions_dir().join(".snapshots")
    }

    #[must_use]
    pub fn snapshot_path(&self, session_id: SessionId) -> PathBuf {
        self.snapshot_dir().join(format!("{session_id}.json"))
    }

    #[must_use]
    pub fn cost_path(&self, session_id: SessionId) -> PathBuf {
        self.sessions_dir().join(format!("{session_id}.costs"))
    }

    #[must_use]
    pub fn pastes_dir(&self) -> PathBuf {
        self.base_dir.join("pastes")
    }

    #[must_use]
    pub fn plugins_dir(&self) -> PathBuf {
        self.base_dir.join("plugins")
    }

    #[must_use]
    pub fn skills_dir(&self) -> PathBuf {
        self.base_dir.join("skills")
    }

    #[must_use]
    pub fn config_dir(&self) -> PathBuf {
        self.base_dir.join("config")
    }

    #[must_use]
    pub fn settings_path(&self) -> PathBuf {
        self.config_dir().join("settings.json")
    }

    #[must_use]
    pub fn credentials_path(&self) -> PathBuf {
        self.config_dir().join("credentials.json")
    }

    #[must_use]
    pub fn plugins_config_dir(&self) -> PathBuf {
        self.config_dir().join("plugins")
    }

    #[must_use]
    pub fn plugin_settings_path(&self) -> PathBuf {
        self.plugins_config_dir().join("settings.json")
    }

    #[must_use]
    pub fn mcp_config_dir(&self) -> PathBuf {
        self.config_dir().join("mcp")
    }

    #[must_use]
    pub fn mcp_servers_path(&self) -> PathBuf {
        self.mcp_config_dir().join("servers.json")
    }

    #[must_use]
    pub fn tasks_dir(&self) -> PathBuf {
        self.base_dir.join("tasks")
    }

    #[must_use]
    pub fn task_metadata_dir(&self) -> PathBuf {
        self.tasks_dir().join("metadata")
    }

    #[must_use]
    pub fn task_logs_dir(&self) -> PathBuf {
        self.tasks_dir().join("logs")
    }

    #[must_use]
    pub fn task_exit_dir(&self) -> PathBuf {
        self.tasks_dir().join("exit")
    }

    #[must_use]
    pub fn task_heartbeat_dir(&self) -> PathBuf {
        self.tasks_dir().join("heartbeat")
    }

    #[must_use]
    pub fn task_state_path(&self, task_id: TaskId) -> PathBuf {
        self.task_metadata_dir().join(format!("{task_id}.json"))
    }

    #[must_use]
    pub fn task_log_path(&self, task_id: TaskId) -> PathBuf {
        self.task_logs_dir().join(format!("{task_id}.log"))
    }

    #[must_use]
    pub fn task_exit_path(&self, task_id: TaskId) -> PathBuf {
        self.task_exit_dir().join(format!("{task_id}.exit"))
    }

    #[must_use]
    pub fn task_heartbeat_path(&self, task_id: TaskId) -> PathBuf {
        self.task_heartbeat_dir()
            .join(format!("{task_id}.heartbeat"))
    }

    #[must_use]
    pub fn paste_path(&self, sha256: &str) -> PathBuf {
        self.pastes_dir().join(sha256)
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
        fs::create_dir_all(self.paths.snapshot_dir())?;
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
                Ok(message) => {
                    ensure_supported_schema("message", message.schema_version)?;
                    messages.push(message);
                }
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
        write_json_atomically(&path, metadata)
    }

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

    pub fn write_snapshot(&self, snapshot: &SessionSnapshot) -> Result<()> {
        self.ensure_layout()?;
        let path = self.paths.snapshot_path(snapshot.session_id);
        write_json_atomically(&path, snapshot)
    }

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

#[derive(Clone, Debug)]
pub struct CostStore {
    paths: StoragePaths,
}

impl CostStore {
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
        Ok(())
    }

    pub fn write_costs(&self, ledger: &SessionCostLedger) -> Result<()> {
        self.ensure_layout()?;
        let path = self.paths.cost_path(ledger.session_id);
        write_json_atomically(&path, ledger)
    }

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

#[derive(Clone, Debug)]
pub struct TaskStore {
    paths: StoragePaths,
}

impl TaskStore {
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
        fs::create_dir_all(self.paths.task_metadata_dir())?;
        fs::create_dir_all(self.paths.task_logs_dir())?;
        fs::create_dir_all(self.paths.task_exit_dir())?;
        fs::create_dir_all(self.paths.task_heartbeat_dir())?;
        Ok(())
    }

    pub fn write_task(&self, task: &TaskState) -> Result<()> {
        self.ensure_layout()?;
        write_json_atomically(&self.paths.task_state_path(task.id), task)
    }

    pub fn read_task(&self, task_id: TaskId) -> Result<TaskState> {
        let path = self.paths.task_state_path(task_id);
        if !path.exists() {
            return Err(WonderError::not_found("task", task_id.to_string()));
        }
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

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

    pub fn read_log(&self, task_id: TaskId) -> Result<String> {
        let path = self.paths.task_log_path(task_id);
        if !path.exists() {
            return Err(WonderError::not_found("task log", task_id.to_string()));
        }
        Ok(fs::read_to_string(path)?)
    }

    pub fn read_log_tail(&self, task_id: TaskId, line_limit: usize) -> Result<Vec<String>> {
        let content = self.read_log(task_id)?;
        let line_limit = line_limit.max(1);
        let mut lines = content.lines().map(ToString::to_string).collect::<Vec<_>>();
        if lines.len() > line_limit {
            lines.drain(0..lines.len() - line_limit);
        }
        Ok(lines)
    }

    pub fn write_exit_code(&self, task_id: TaskId, exit_code: i32) -> Result<()> {
        self.ensure_layout()?;
        let mut file = File::create(self.paths.task_exit_path(task_id))?;
        writeln!(file, "{exit_code}")?;
        file.sync_all()?;
        Ok(())
    }

    pub fn write_heartbeat_at(&self, task_id: TaskId, heartbeat_at: OffsetDateTime) -> Result<()> {
        self.ensure_layout()?;
        let mut file = File::create(self.paths.task_heartbeat_path(task_id))?;
        writeln!(
            file,
            "{}",
            heartbeat_at
                .format(&Rfc3339)
                .map_err(|error| WonderError::validation(format!(
                    "invalid heartbeat timestamp: {error}"
                )))?
        )?;
        file.sync_all()?;
        Ok(())
    }

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

#[derive(Clone, Debug)]
pub struct PasteStore {
    paths: StoragePaths,
    max_bytes: usize,
}

impl PasteStore {
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self::with_max_bytes(base_dir, DEFAULT_PASTE_MAX_BYTES)
    }

    #[must_use]
    pub fn with_max_bytes(base_dir: impl Into<PathBuf>, max_bytes: usize) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
            max_bytes,
        }
    }

    #[must_use]
    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    #[must_use]
    pub const fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.paths.pastes_dir())?;
        Ok(())
    }

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

    pub fn load(&self, sha256: &str) -> Result<Vec<u8>> {
        validate_paste_hash(sha256)?;
        let path = self.paths.paste_path(sha256);
        if !path.exists() {
            return Err(WonderError::not_found("paste", sha256));
        }
        Ok(fs::read(path)?)
    }

    pub fn load_text(&self, sha256: &str) -> Result<String> {
        String::from_utf8(self.load(sha256)?)
            .map_err(|error| WonderError::validation(format!("paste is not valid UTF-8: {error}")))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoadedTranscript {
    pub messages: Vec<MessageEnvelope>,
    pub warnings: Vec<TranscriptWarning>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionResumeSource {
    Snapshot,
    Transcript,
}

impl SessionResumeSource {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Transcript => "transcript",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    #[serde(default = "default_storage_schema_version")]
    pub schema_version: u16,
    pub session_id: SessionId,
    pub state: AppState,
    #[serde(default)]
    pub transcript_message_count: usize,
    #[serde(default)]
    pub transcript_warning_count: usize,
}

impl SessionSnapshot {
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

#[derive(Clone, Debug, PartialEq)]
pub struct RestoredSession {
    pub metadata: SessionMetadata,
    pub transcript: LoadedTranscript,
    pub state: AppState,
    pub resume_source: SessionResumeSource,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TranscriptWarning {
    pub line: usize,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionMetadata {
    #[serde(default = "default_storage_schema_version")]
    pub schema_version: u16,
    pub session_id: SessionId,
    pub title: String,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub message_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub auth: wonder_of_u_core::AuthState,
    #[serde(default)]
    pub costs: CostState,
}

impl SessionMetadata {
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

    #[must_use]
    pub fn from_state_with_transcript(state: &AppState, transcript_message_count: usize) -> Self {
        let mut metadata = Self::from_app_state(state);
        metadata.message_count = transcript_message_count;
        metadata
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionCostLedger {
    #[serde(default = "default_storage_schema_version")]
    pub schema_version: u16,
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub costs: CostState,
}

impl SessionCostLedger {
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoredPaste {
    pub sha256: String,
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
        TaskId, TaskKind, TaskState, TaskStatus, TokenUsage, ToolUseId,
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
    #[ignore = "paste references are not implemented in transcript messages yet"]
    fn transcript_paste_reference_expands_on_reload() {
        // TODO: When transcript messages can reference `StoredPaste` entries, persist a
        // paste-backed message here and assert reload expands it back to the original content.
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
        second.updated_at = second.updated_at + time::Duration::seconds(30);

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

        let listed = store.list_tasks().expect("list tasks");
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|task| task.kind == TaskKind::LocalShell));
        assert!(listed.iter().any(|task| task.kind == TaskKind::LocalAgent));
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
