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

use crate::auth::{AuthMaterial, StoredCredentials};

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

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_model: Option<String>,
    #[serde(default)]
    pub fast_mode: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_level: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub providers: BTreeMap<String, ProviderOverride>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProviderOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SettingsStore {
    paths: StoragePaths,
}

impl SettingsStore {
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

    pub fn read(&self) -> Result<AgentSettings> {
        let path = self.paths.settings_path();
        if !path.exists() {
            return Ok(AgentSettings::default());
        }

        serde_json::from_str(&fs::read_to_string(path)?).map_err(Into::into)
    }

    pub fn write(&self, settings: &AgentSettings) -> Result<()> {
        fs::create_dir_all(self.paths.config_dir())?;
        write_json_atomically(&self.paths.settings_path(), settings)
    }
}

#[derive(Clone, Debug)]
pub struct CredentialStore {
    paths: StoragePaths,
}

impl CredentialStore {
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
        }
    }

    pub fn read(&self) -> Result<StoredCredentials> {
        let path = self.paths.credentials_path();
        if !path.exists() {
            return Ok(StoredCredentials::default());
        }

        serde_json::from_str(&fs::read_to_string(path)?).map_err(Into::into)
    }

    pub fn write(&self, credentials: &StoredCredentials) -> Result<()> {
        fs::create_dir_all(self.paths.config_dir())?;
        write_json_atomically(&self.paths.credentials_path(), credentials)
    }

    pub fn set_api_key(&self, provider: &str, api_key: impl Into<String>) -> Result<()> {
        let mut credentials = self.read()?;
        credentials.providers.insert(
            provider.to_string(),
            AuthMaterial::ApiKey {
                key: api_key.into(),
            },
        );
        self.write(&credentials)
    }

    pub fn set_oauth_token(
        &self,
        provider: &str,
        access_token: impl Into<String>,
        refresh_token: Option<String>,
        expires_at: Option<time::OffsetDateTime>,
    ) -> Result<()> {
        let mut credentials = self.read()?;
        credentials.providers.insert(
            provider.to_string(),
            AuthMaterial::OAuth {
                access_token: Some(access_token.into()),
                refresh_token,
                expires_at,
            },
        );
        self.write(&credentials)
    }

    pub fn remove(&self, provider: &str) -> Result<bool> {
        let mut credentials = self.read()?;
        let removed = credentials.providers.remove(provider).is_some();
        self.write(&credentials)?;
        Ok(removed)
    }
}

pub fn require_storage_dir(storage_dir: Option<PathBuf>) -> Result<PathBuf> {
    storage_dir.ok_or_else(|| {
        WonderError::validation(
            "this command requires --storage-dir so provider settings can be persisted",
        )
    })
}

#[cfg(test)]
mod tests {
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    #[test]
    fn settings_store_round_trips_selection() {
        let dir = unique_test_dir("agent-settings");
        let store = SettingsStore::new(&dir);
        let settings = AgentSettings {
            selected_provider: Some("openai".into()),
            selected_model: Some("gpt-4.1".into()),
            ..AgentSettings::default()
        };

        store.write(&settings).expect("write settings");

        assert_eq!(store.read().expect("read settings"), settings);
    }

    #[test]
    fn credential_store_round_trips_api_keys() {
        let dir = unique_test_dir("agent-credentials");
        let store = CredentialStore::new(&dir);

        store.set_api_key("openai", "secret").expect("store key");

        let loaded = store.read().expect("read credentials");
        assert!(matches!(
            loaded.providers.get("openai"),
            Some(AuthMaterial::ApiKey { key }) if key == "secret"
        ));
    }

    #[test]
    fn credential_store_round_trips_oauth_tokens() {
        let dir = unique_test_dir("agent-oauth-credentials");
        let store = CredentialStore::new(&dir);

        store
            .set_oauth_token("copilot", "oauth-secret", None, None)
            .expect("store oauth token");

        let loaded = store.read().expect("read credentials");
        assert!(matches!(
            loaded.providers.get("copilot"),
            Some(AuthMaterial::OAuth {
                access_token: Some(token),
                refresh_token: None,
                ..
            }) if token == "oauth-secret"
        ));
    }

    #[test]
    fn require_storage_dir_rejects_missing_paths() {
        let error = require_storage_dir(None).expect_err("missing storage dir");
        assert!(error.to_string().contains("--storage-dir"));
    }
}
