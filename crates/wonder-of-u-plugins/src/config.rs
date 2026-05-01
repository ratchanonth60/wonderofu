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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginTrustDecision {
    Trusted,
    #[default]
    Untrusted,
    Blocked,
}

impl PluginTrustDecision {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Untrusted => "untrusted",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PluginConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_plugin_dirs: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_skill_dirs: Vec<PathBuf>,
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

    #[must_use]
    pub fn trust_for(&self, plugin_id: &str) -> Option<PluginTrustDecision> {
        let normalized = normalize_plugin_id(plugin_id).ok()?;
        self.trust.get(&normalized).copied()
    }

    pub fn set_trust(&mut self, plugin_id: &str, decision: PluginTrustDecision) -> Result<()> {
        let normalized = normalize_plugin_id(plugin_id)?;
        self.trust.insert(normalized, decision);
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct PluginConfigStore {
    paths: StoragePaths,
}

impl PluginConfigStore {
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

    pub fn read(&self) -> Result<PluginConfig> {
        let path = self.paths.plugin_settings_path();
        if !path.exists() {
            return Ok(PluginConfig::default());
        }

        let config: PluginConfig = serde_json::from_str(&fs::read_to_string(path)?)?;
        config.validate()?;
        Ok(config)
    }

    pub fn write(&self, config: &PluginConfig) -> Result<()> {
        config.validate()?;
        fs::create_dir_all(self.paths.plugins_config_dir())?;
        write_json_atomically(&self.paths.plugin_settings_path(), config)
    }

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
