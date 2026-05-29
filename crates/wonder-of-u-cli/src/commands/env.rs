//! Provides the top-level `env` CLI command.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use wonder_of_u_agent::SettingsStore;
use wonder_of_u_core::{Result, WonderError};

const MASKED_SYSTEM_ENV_VARS: [&str; 3] = ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GITHUB_TOKEN"];
const PLAIN_SYSTEM_ENV_VARS: [&str; 2] = ["WONDER_MODEL", "WONDER_PROVIDER"];

pub(crate) fn show(storage_dir: Option<&Path>) -> Result<String> {
    let mut lines = Vec::new();
    let mut persisted = persisted_env_vars(storage_dir)?
        .into_iter()
        .collect::<Vec<_>>();
    persisted.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));

    if persisted.is_empty() {
        lines.push("persisted_env_vars=none".to_string());
    } else {
        lines.extend(
            persisted
                .into_iter()
                .map(|(key, value)| format!("persisted.{key}={value}")),
        );
    }

    let system_lines = system_env_lines();
    if system_lines.is_empty() {
        lines.push("system_env_vars=none".to_string());
    } else {
        lines.extend(system_lines);
    }

    Ok(lines.join("\n"))
}

pub(crate) fn set_var(storage_dir: Option<&Path>, key: &str, value: &str) -> Result<String> {
    validate_key(key)?;
    let store = SettingsStore::new(require_storage_dir(storage_dir)?);
    let mut settings = store.read()?;
    settings.env_vars.insert(key.to_string(), value.to_string());
    store.write(&settings)?;
    Ok(format!(
        "env.{key}={value}\nstatus=environment variable updated"
    ))
}

pub(crate) fn unset_var(storage_dir: Option<&Path>, key: &str) -> Result<String> {
    validate_key(key)?;
    let store = SettingsStore::new(require_storage_dir(storage_dir)?);
    let mut settings = store.read()?;
    let removed = settings.env_vars.remove(key).is_some();
    store.write(&settings)?;
    let status = if removed {
        "environment variable removed"
    } else {
        "environment variable not set"
    };
    Ok(format!("env.{key}=unset\nstatus={status}"))
}

fn persisted_env_vars(storage_dir: Option<&Path>) -> Result<HashMap<String, String>> {
    let Some(storage_dir) = storage_dir else {
        return Ok(HashMap::new());
    };
    Ok(SettingsStore::new(storage_dir).read()?.env_vars)
}

#[must_use]
fn system_env_lines() -> Vec<String> {
    let mut lines = Vec::new();
    for name in MASKED_SYSTEM_ENV_VARS {
        if let Ok(value) = std::env::var(name) {
            lines.push(format!("system.{name}={}", mask_secret(&value)));
        }
    }
    for name in PLAIN_SYSTEM_ENV_VARS {
        if let Ok(value) = std::env::var(name) {
            lines.push(format!("system.{name}={value}"));
        }
    }
    lines
}

#[must_use]
fn mask_secret(value: &str) -> String {
    if value.is_empty() {
        "***".to_string()
    } else {
        format!("{}***", value.chars().take(8).collect::<String>())
    }
}

fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() {
        return Err(WonderError::validation(
            "environment variable name cannot be empty",
        ));
    }

    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return Err(WonderError::validation(
            "environment variable name cannot be empty",
        ));
    };

    if !(first.is_ascii_alphabetic() || first == '_')
        || !chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(WonderError::validation(format!(
            "invalid environment variable name: {key}"
        )));
    }

    Ok(())
}

fn require_storage_dir(storage_dir: Option<&Path>) -> Result<PathBuf> {
    storage_dir.map(Path::to_path_buf).ok_or_else(|| {
        WonderError::validation("env command requires --storage-dir or HOME/XDG_CONFIG_HOME")
    })
}

#[cfg(test)]
mod tests {
    use wonder_of_u_agent::{AgentSettings, SettingsStore};
    use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

    use super::*;

    #[test]
    fn env_show_works_with_storage_dir() {
        let _anthropic = EnvVarGuard::remove("ANTHROPIC_API_KEY");
        let _openai = EnvVarGuard::remove("OPENAI_API_KEY");
        let _github = EnvVarGuard::remove("GITHUB_TOKEN");
        let dir = unique_test_dir("cli-env-show");
        SettingsStore::new(&dir)
            .write(&AgentSettings {
                env_vars: HashMap::from([
                    ("ALPHA".into(), "one".into()),
                    ("BETA".into(), "two".into()),
                ]),
                ..AgentSettings::default()
            })
            .expect("write settings");

        let rendered = show(Some(dir.as_path())).expect("show env");

        assert_eq!(
            rendered,
            "persisted.ALPHA=one\npersisted.BETA=two\nsystem_env_vars=none"
        );
    }

    #[test]
    fn env_set_var_and_unset_var_persist_changes() {
        let dir = unique_test_dir("cli-env-set-unset");

        let set_output = set_var(Some(dir.as_path()), "WONDER_MODEL", "opus").expect("set env var");
        assert!(set_output.contains("env.WONDER_MODEL=opus"));
        assert_eq!(
            SettingsStore::new(&dir)
                .read()
                .expect("read settings")
                .env_vars
                .get("WONDER_MODEL"),
            Some(&"opus".to_string())
        );

        let unset_output = unset_var(Some(dir.as_path()), "WONDER_MODEL").expect("unset env var");
        assert!(unset_output.contains("status=environment variable removed"));
        assert!(
            !SettingsStore::new(&dir)
                .read()
                .expect("read settings")
                .env_vars
                .contains_key("WONDER_MODEL")
        );
    }

    #[test]
    fn env_show_masks_api_keys() {
        let dir = unique_test_dir("cli-env-mask");
        let _anthropic = EnvVarGuard::set("ANTHROPIC_API_KEY", "abcdefgh123456");
        let _openai = EnvVarGuard::set("OPENAI_API_KEY", "openai-abcdefgh");
        let _github = EnvVarGuard::set("GITHUB_TOKEN", "ghp_1234567890");

        let rendered = show(Some(dir.as_path())).expect("show env");

        assert!(rendered.contains("system.ANTHROPIC_API_KEY=abcdefgh***"));
        assert!(rendered.contains("system.OPENAI_API_KEY=openai-a***"));
        assert!(rendered.contains("system.GITHUB_TOKEN=ghp_1234***"));
    }

    #[test]
    fn env_set_var_rejects_invalid_key() {
        let dir = unique_test_dir("cli-env-invalid-key");

        let error = set_var(Some(dir.as_path()), "BAD-KEY", "value").expect_err("invalid key");

        assert!(
            error
                .to_string()
                .contains("invalid environment variable name")
        );
    }
}
