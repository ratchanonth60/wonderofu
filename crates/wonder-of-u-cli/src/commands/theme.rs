//! Provides the top-level `theme` CLI command.

use std::path::{Path, PathBuf};

use wonder_of_u_agent::SettingsStore;
use wonder_of_u_core::{Result, WonderError};

pub(crate) struct ThemeEntry {
    pub(crate) name: &'static str,
    #[allow(dead_code)]
    pub(crate) description: &'static str,
}

pub(crate) const THEME_CATALOG: [ThemeEntry; 3] = [
    ThemeEntry {
        name: "default",
        description: "Dark shell with blue borders and cyan prompt accents.",
    },
    ThemeEntry {
        name: "midnight",
        description: "Deeper dark background with brighter cyan and magenta emphasis.",
    },
    ThemeEntry {
        name: "light",
        description: "Light background with dark text and blue status accents.",
    },
];

pub(crate) fn parse_theme_name(value: &str) -> Result<&'static str> {
    THEME_CATALOG
        .iter()
        .find(|t| t.name == value)
        .map(|t| t.name)
        .ok_or_else(|| WonderError::validation(format!("unknown theme: {value}")))
}

#[must_use]
pub(crate) fn theme_name(value: Option<&str>) -> &'static str {
    match value {
        Some("midnight") => "midnight",
        Some("light") => "light",
        _ => "default",
    }
}

pub(crate) fn show(storage_dir: Option<&Path>) -> Result<String> {
    let current = theme_name(persisted_theme(storage_dir)?.as_deref());
    Ok(format!("current_theme={current}"))
}

#[must_use]
pub(crate) fn list() -> String {
    THEME_CATALOG
        .iter()
        .map(|t| t.name)
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn set(storage_dir: Option<&Path>, name: &str) -> Result<String> {
    let theme = parse_theme_name(name)?;
    let store = SettingsStore::new(require_storage_dir(storage_dir)?);
    let mut settings = store.read()?;
    settings.theme = (theme != "default").then(|| theme.to_string());
    store.write(&settings)?;
    Ok(format!("theme={theme}\nstatus=theme updated"))
}

fn persisted_theme(storage_dir: Option<&Path>) -> Result<Option<String>> {
    let Some(storage_dir) = storage_dir else {
        return Ok(None);
    };
    Ok(SettingsStore::new(storage_dir).read()?.theme)
}

fn require_storage_dir(storage_dir: Option<&Path>) -> Result<PathBuf> {
    storage_dir.map(Path::to_path_buf).ok_or_else(|| {
        WonderError::validation("theme command requires --storage-dir or HOME/XDG_CONFIG_HOME")
    })
}

#[cfg(test)]
mod tests {
    use wonder_of_u_agent::SettingsStore;
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    #[test]
    fn theme_list_contains_all_themes() {
        let output = list();

        assert!(output.lines().any(|l| l == "default"));
        assert!(output.lines().any(|l| l == "midnight"));
        assert!(output.lines().any(|l| l == "light"));
    }

    #[test]
    fn theme_set_persists_valid_theme() {
        let dir = unique_test_dir("cli-theme-set");

        let output = set(Some(dir.as_path()), "midnight").expect("set theme");

        assert!(output.contains("theme=midnight"));
        let settings = SettingsStore::new(&dir).read().expect("read settings");
        assert_eq!(settings.theme.as_deref(), Some("midnight"));
        assert_eq!(
            show(Some(dir.as_path())).expect("show theme"),
            "current_theme=midnight"
        );
    }

    #[test]
    fn theme_show_defaults_to_default_when_not_set() {
        let dir = unique_test_dir("cli-theme-show-default");

        assert_eq!(
            show(Some(dir.as_path())).expect("show theme"),
            "current_theme=default"
        );
    }

    #[test]
    fn theme_set_rejects_unknown_theme() {
        let dir = unique_test_dir("cli-theme-invalid");

        let err = set(Some(dir.as_path()), "dracula").expect_err("unknown theme");

        assert!(err.to_string().contains("unknown theme: dracula"));
    }

    #[test]
    fn parse_theme_name_rejects_unknown() {
        assert!(parse_theme_name("solarized").is_err());
    }

    #[test]
    fn theme_name_falls_back_to_default() {
        assert_eq!(theme_name(None), "default");
        assert_eq!(theme_name(Some("unknown")), "default");
    }
}
