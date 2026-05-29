//! Persistent history for the tips system.
//!
//! [`TipsHistory`] tracks which tips have been shown and in which session, so
//! that the selection logic in `wonder-of-u-core::tips` can apply per-tip
//! cooldown rules.  The history is persisted as a single atomic JSON file under
//! the storage base directory.

use std::{collections::HashMap, fs, io::ErrorKind, path::PathBuf};

use serde::{Deserialize, Serialize};
use wonder_of_u_core::{Result, WonderError};

use crate::write_json_atomically;

/// Schema version for [`TipsHistory`].  Increment when fields are removed or
/// semantics change incompatibly.
pub const TIPS_HISTORY_SCHEMA_VERSION: u16 = 1;

fn default_tips_history_schema_version() -> u16 {
    TIPS_HISTORY_SCHEMA_VERSION
}

/// Persistent record of which tips have been shown and when (expressed as a
/// session counter, not a wall-clock timestamp).
///
/// The `entries` map keys are tip IDs; values are the value of `current_session`
/// at the time the tip was last shown.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TipsHistory {
    /// Schema version — must equal [`TIPS_HISTORY_SCHEMA_VERSION`] when loading.
    #[serde(default = "default_tips_history_schema_version")]
    pub schema_version: u16,
    /// Monotonically increasing session counter, incremented by
    /// [`TipsStore::start_session`] at the beginning of each session.
    pub current_session: u64,
    /// Maps tip ID → session number at which the tip was last shown.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub entries: HashMap<String, u64>,
}

impl Default for TipsHistory {
    fn default() -> Self {
        Self {
            schema_version: TIPS_HISTORY_SCHEMA_VERSION,
            current_session: 0,
            entries: HashMap::new(),
        }
    }
}

impl TipsHistory {
    /// Returns the number of sessions elapsed since `tip_id` was last shown.
    ///
    /// Returns [`u64::MAX`] when the tip has never been shown.
    #[must_use]
    pub fn sessions_since_shown(&self, tip_id: &str) -> u64 {
        match self.entries.get(tip_id) {
            None => u64::MAX,
            Some(&shown_at) => self.current_session.saturating_sub(shown_at),
        }
    }

    /// Records that `tip_id` was shown in the current session.
    ///
    /// Idempotent within the same session: calling this twice with the same
    /// `tip_id` and an unchanged `current_session` has no effect.
    pub fn record_shown(&mut self, tip_id: &str) {
        // Only update when we would change the value to avoid unnecessary writes.
        let entry = self.entries.entry(tip_id.to_owned()).or_insert(u64::MAX);
        if *entry != self.current_session {
            *entry = self.current_session;
        }
    }
}

/// Persistent storage for [`TipsHistory`].
///
/// The history file lives at `{base_dir}/config/tips-history.json` and is
/// written atomically via a `.next` temporary file.
#[derive(Clone, Debug)]
pub struct TipsStore {
    base_dir: PathBuf,
}

impl TipsStore {
    /// Creates a new [`TipsStore`] rooted at `base_dir`.
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Returns the absolute path of the history file.
    #[must_use]
    pub fn history_path(&self) -> PathBuf {
        self.base_dir.join("config").join("tips-history.json")
    }

    /// Reads the history file, returning a default (empty) [`TipsHistory`] when
    /// the file does not exist.
    ///
    /// Returns an error when the file exists but cannot be parsed, or when it
    /// carries an unsupported `schema_version`.
    pub fn read_or_default(&self) -> Result<TipsHistory> {
        let path = self.history_path();
        match fs::read_to_string(&path) {
            Ok(contents) => {
                let history: TipsHistory = serde_json::from_str(&contents)?;
                if history.schema_version != TIPS_HISTORY_SCHEMA_VERSION {
                    return Err(WonderError::validation(format!(
                        "unsupported tips history schema version: {}",
                        history.schema_version
                    )));
                }
                Ok(history)
            }
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(TipsHistory::default()),
            Err(err) => Err(err.into()),
        }
    }

    /// Atomically writes `history` to the history file, creating parent
    /// directories as needed.
    pub fn write(&self, history: &TipsHistory) -> Result<()> {
        let path = self.history_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_json_atomically(&path, history)
    }

    /// Increments `current_session` and persists the updated history.
    ///
    /// Call this once at application startup, before any tip is selected or
    /// shown.
    pub fn start_session(&self) -> Result<TipsHistory> {
        let mut history = self.read_or_default()?;
        history.current_session = history.current_session.saturating_add(1);
        self.write(&history)?;
        Ok(history)
    }

    /// Records `tip_id` as shown in the current session and persists the
    /// updated history.
    pub fn record_shown(&self, tip_id: &str) -> Result<TipsHistory> {
        let mut history = self.read_or_default()?;
        history.record_shown(tip_id);
        self.write(&history)?;
        Ok(history)
    }
}

#[cfg(test)]
mod tests {
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn make_store(prefix: &str) -> TipsStore {
        TipsStore::new(unique_test_dir(prefix))
    }

    #[test]
    fn read_or_default_returns_empty_when_no_file() {
        let store = make_store("tips-default");
        let history = store.read_or_default().expect("should succeed");
        assert_eq!(history.current_session, 0);
        assert!(history.entries.is_empty());
    }

    #[test]
    fn start_session_increments_counter() {
        let store = make_store("tips-start-session");
        let h1 = store.start_session().expect("first session");
        assert_eq!(h1.current_session, 1);
        let h2 = store.start_session().expect("second session");
        assert_eq!(h2.current_session, 2);
    }

    #[test]
    fn record_shown_persists_tip() {
        let store = make_store("tips-record-shown");
        let _ = store.start_session().expect("session");
        store.record_shown("shift-enter-multiline").expect("record");
        let history = store.read_or_default().expect("read back");
        assert_eq!(
            history.entries.get("shift-enter-multiline").copied(),
            Some(1)
        );
    }

    #[test]
    fn sessions_since_shown_returns_max_for_never_shown() {
        let history = TipsHistory::default();
        assert_eq!(history.sessions_since_shown("anything"), u64::MAX);
    }

    #[test]
    fn sessions_since_shown_returns_correct_delta() {
        let mut history = TipsHistory {
            current_session: 10,
            ..TipsHistory::default()
        };
        history.record_shown("tip-a");
        // Shown at session 10; still session 10 -> delta = 0.
        assert_eq!(history.sessions_since_shown("tip-a"), 0);

        history.current_session = 15;
        assert_eq!(history.sessions_since_shown("tip-a"), 5);
    }

    #[test]
    fn write_and_read_roundtrip() {
        let store = make_store("tips-roundtrip");
        let mut history = TipsHistory {
            current_session: 7,
            ..TipsHistory::default()
        };
        history.entries.insert("shift-tab".into(), 3);

        store.write(&history).expect("write");
        let read_back = store.read_or_default().expect("read back");

        assert_eq!(read_back.current_session, 7);
        assert_eq!(read_back.entries.get("shift-tab").copied(), Some(3));
    }

    #[test]
    fn schema_mismatch_returns_error() {
        use std::io::Write;

        let store = make_store("tips-schema-mismatch");
        let path = store.history_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        let bad = serde_json::json!({
            "schema_version": 9999_u32,
            "current_session": 1_u64,
        });
        let mut f = std::fs::File::create(&path).expect("create");
        f.write_all(serde_json::to_string(&bad).unwrap().as_bytes())
            .expect("write");

        let err = store.read_or_default().expect_err("should fail");
        assert!(
            err.to_string().contains("9999"),
            "error should mention bad version: {err}"
        );
    }

    #[test]
    fn record_shown_is_idempotent_within_session() {
        let mut history = TipsHistory {
            current_session: 3,
            ..TipsHistory::default()
        };
        history.record_shown("tip-x");
        history.record_shown("tip-x"); // second call in same session
        assert_eq!(history.entries.get("tip-x").copied(), Some(3));
    }
}
