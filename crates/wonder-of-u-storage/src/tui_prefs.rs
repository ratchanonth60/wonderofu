//! Persistent TUI preferences (sidebar visibility, sidebar mode, etc.)
//!
//! Layout: `{base_dir}/config/preferences.json`

use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use wonder_of_u_core::Result;

use crate::write_json_atomically;

/// Schema version for the TUI preferences file.
pub const TUI_PREFS_SCHEMA_VERSION: u16 = 1;

fn default_schema_version() -> u16 {
    TUI_PREFS_SCHEMA_VERSION
}

fn default_sidebar_mode() -> String {
    "push".into()
}

/// Persistent TUI preferences stored at `{base_dir}/config/preferences.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TuiPrefs {
    /// Schema version for forward-compatible deserialization.
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    /// Sidebar display mode: `"push"` or `"overlay"`.
    #[serde(default = "default_sidebar_mode")]
    pub sidebar_mode: String,
    /// Whether the sidebar panel starts visible.
    #[serde(default)]
    pub sidebar_visible: bool,
}

impl Default for TuiPrefs {
    fn default() -> Self {
        Self {
            schema_version: TUI_PREFS_SCHEMA_VERSION,
            sidebar_mode: "push".into(),
            sidebar_visible: true,
        }
    }
}

/// Store for persistent TUI preferences, backed by `{base_dir}/config/preferences.json`.
#[derive(Clone, Debug)]
pub struct TuiPrefsStore {
    base_dir: PathBuf,
}

impl TuiPrefsStore {
    /// Creates a new store rooted at `base_dir`.
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    fn path(&self) -> PathBuf {
        self.base_dir.join("config").join("preferences.json")
    }

    /// Reads preferences from disk, returning defaults when the file does not exist.
    pub fn read_or_default(&self) -> Result<TuiPrefs> {
        let path = self.path();
        match fs::read_to_string(&path) {
            Ok(contents) => {
                let prefs: TuiPrefs = serde_json::from_str(&contents)?;
                if prefs.schema_version != TUI_PREFS_SCHEMA_VERSION {
                    return Err(wonder_of_u_core::WonderError::validation(format!(
                        "unsupported tui prefs schema version: {}",
                        prefs.schema_version
                    )));
                }
                Ok(prefs)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(TuiPrefs::default()),
            Err(err) => Err(err.into()),
        }
    }

    /// Atomically writes preferences to disk.
    pub fn write(&self, prefs: &TuiPrefs) -> Result<()> {
        let path = self.path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_json_atomically(&path, prefs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = TuiPrefsStore::new(dir.path());
        let prefs = TuiPrefs::default();
        store.write(&prefs).expect("write");
        let loaded = store.read_or_default().expect("read");
        assert_eq!(loaded.sidebar_mode, "push");
        assert!(loaded.sidebar_visible);
    }

    #[test]
    fn missing_file_returns_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = TuiPrefsStore::new(dir.path());
        let loaded = store.read_or_default().expect("read");
        assert_eq!(loaded.sidebar_mode, TuiPrefs::default().sidebar_mode);
        assert_eq!(loaded.sidebar_visible, TuiPrefs::default().sidebar_visible);
    }
}
