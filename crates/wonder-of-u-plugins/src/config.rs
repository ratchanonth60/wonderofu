use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use wonder_of_u_core::{Result, WonderError};
use wonder_of_u_storage::StoragePaths;

use crate::normalize_plugin_id;

/// Schema version for plugin config
pub const PLUGIN_CONFIG_SCHEMA_VERSION: u16 = 1;

fn default_schema_version() -> u16 {
    PLUGIN_CONFIG_SCHEMA_VERSION
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
/// Enumerates plugin trust decision
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginTrustDecision {
    /// Represents trusted
    Trusted,
    /// Represents untrusted
    #[default]
    Untrusted,
    /// Represents blocked
    Blocked,
}

impl PluginTrustDecision {
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Untrusted => "untrusted",
            Self::Blocked => "blocked",
        }
    }
}
/// Represents plugin config
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PluginConfig {
    /// Stores the schema version
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    /// Stores the additional plugin dirs
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_plugin_dirs: Vec<PathBuf>,
    /// Stores the additional skill dirs
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_skill_dirs: Vec<PathBuf>,
    /// Stores the trust
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub trust: BTreeMap<String, PluginTrustDecision>,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            schema_version: PLUGIN_CONFIG_SCHEMA_VERSION,
            additional_plugin_dirs: Vec::new(),
            additional_skill_dirs: Vec::new(),
            trust: BTreeMap::new(),
        }
    }
}

impl PluginConfig {
    /// Validates the value
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != PLUGIN_CONFIG_SCHEMA_VERSION {
            return Err(WonderError::validation(format!(
                "unsupported plugin config schema version: {}",
                self.schema_version
            )));
        }

        for path in self
            .additional_plugin_dirs
            .iter()
            .chain(&self.additional_skill_dirs)
        {
            if path.as_os_str().is_empty() {
                return Err(WonderError::validation(
                    "plugin config directories cannot be empty",
                ));
            }
        }

        for plugin_id in self.trust.keys() {
            let normalized = normalize_plugin_id(plugin_id)?;
            if normalized.is_empty() {
                return Err(WonderError::validation(
                    "plugin trust entries must use non-empty plugin ids",
                ));
            }
        }

        Ok(())
    }
    /// Handles trust for
    #[must_use]
    pub fn trust_for(&self, plugin_id: &str) -> Option<PluginTrustDecision> {
        let normalized = normalize_plugin_id(plugin_id).ok()?;
        self.trust.get(&normalized).copied()
    }

    /// Handles set trust
    pub fn set_trust(&mut self, plugin_id: &str, decision: PluginTrustDecision) -> Result<()> {
        let normalized = normalize_plugin_id(plugin_id)?;
        self.trust.insert(normalized, decision);
        Ok(())
    }
}
/// Stores plugin config store
#[derive(Clone, Debug)]
pub struct PluginConfigStore {
    paths: StoragePaths,
}

impl PluginConfigStore {
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

    /// Handles read
    pub fn read(&self) -> Result<PluginConfig> {
        let path = self.paths.plugin_settings_path();
        if !path.exists() {
            return Ok(PluginConfig::default());
        }

        let config: PluginConfig = serde_json::from_str(&fs::read_to_string(path)?)?;
        config.validate()?;
        Ok(config)
    }

    /// Handles write
    pub fn write(&self, config: &PluginConfig) -> Result<()> {
        config.validate()?;
        fs::create_dir_all(self.paths.plugins_config_dir())?;
        write_json_atomically(&self.paths.plugin_settings_path(), config)
    }

    /// Handles set trust
    pub fn set_trust(
        &self,
        plugin_id: &str,
        decision: PluginTrustDecision,
    ) -> Result<PluginConfig> {
        let mut config = self.read()?;
        config.set_trust(plugin_id, decision)?;
        self.write(&config)?;
        Ok(config)
    }

    /// Add a plugin directory to the additional search paths. No-ops if already present.
    pub fn add_plugin_dir(&self, dir: &Path) -> Result<PluginConfig> {
        let mut config = self.read()?;
        if !config.additional_plugin_dirs.contains(&dir.to_path_buf()) {
            config.additional_plugin_dirs.push(dir.to_path_buf());
        }
        self.write(&config)?;
        Ok(config)
    }

    /// Remove a plugin directory that was previously added via [`add_plugin_dir`].
    /// Returns `true` if a directory was removed. Uses the plugin id as a basename
    /// match when the argument does not look like a path.
    pub fn remove_plugin_dir_by_id(&self, plugin_id_or_path: &str) -> Result<bool> {
        let mut config = self.read()?;
        let before = config.additional_plugin_dirs.len();
        config.additional_plugin_dirs.retain(|dir| {
            // Match by exact path or by directory basename containing the plugin id
            let path_match = dir.to_string_lossy() == plugin_id_or_path;
            let id_match = dir
                .file_name()
                .map(|n| {
                    let name = n.to_string_lossy().to_lowercase();
                    let normalized = plugin_id_or_path.to_lowercase().replace(' ', "-");
                    name == normalized
                })
                .unwrap_or(false);
            !path_match && !id_match
        });
        let removed = config.additional_plugin_dirs.len() < before;
        if removed {
            self.write(&config)?;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    #[test]
    fn config_store_round_trips_dirs_and_trust() {
        let dir = unique_test_dir("plugin-config");
        let store = PluginConfigStore::new(&dir);
        let mut config = PluginConfig {
            additional_plugin_dirs: vec![dir.join("seed/plugins")],
            additional_skill_dirs: vec![dir.join("seed/skills")],
            ..PluginConfig::default()
        };
        config
            .set_trust("Demo Plugin", PluginTrustDecision::Trusted)
            .expect("trust entry");

        store.write(&config).expect("write plugin config");

        let loaded = store.read().expect("read plugin config");
        assert_eq!(loaded.additional_plugin_dirs, config.additional_plugin_dirs);
        assert_eq!(loaded.additional_skill_dirs, config.additional_skill_dirs);
        assert_eq!(
            loaded.trust_for("demo-plugin"),
            Some(PluginTrustDecision::Trusted)
        );
    }
}
