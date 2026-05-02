//! XDG base directory helpers.

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
};

use thiserror::Error;

/// Errors produced while resolving XDG directories.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum XdgError {
    /// No home directory could be determined.
    #[error("home directory could not be determined")]
    MissingHomeDirectory,
}

/// Returns the XDG data home directory.
pub fn xdg_data_home() -> Result<PathBuf, XdgError> {
    resolve_xdg_home("XDG_DATA_HOME", [".local", "share"])
}

/// Returns the XDG config home directory.
pub fn xdg_config_home() -> Result<PathBuf, XdgError> {
    resolve_xdg_home("XDG_CONFIG_HOME", [".config"])
}

/// Returns the XDG cache home directory.
pub fn xdg_cache_home() -> Result<PathBuf, XdgError> {
    resolve_xdg_home("XDG_CACHE_HOME", [".cache"])
}

/// Resolves the XDG data home directory from explicit inputs.
pub fn xdg_data_home_from(env: &HashMap<String, String>, home: &Path) -> PathBuf {
    xdg_from(env, "XDG_DATA_HOME", home, [".local", "share"])
}

/// Resolves the XDG config home directory from explicit inputs.
pub fn xdg_config_home_from(env: &HashMap<String, String>, home: &Path) -> PathBuf {
    xdg_from(env, "XDG_CONFIG_HOME", home, [".config"])
}

/// Resolves the XDG cache home directory from explicit inputs.
pub fn xdg_cache_home_from(env: &HashMap<String, String>, home: &Path) -> PathBuf {
    xdg_from(env, "XDG_CACHE_HOME", home, [".cache"])
}

fn resolve_xdg_home<const N: usize>(key: &str, fallback: [&str; N]) -> Result<PathBuf, XdgError> {
    if let Some(value) = env::var_os(key).filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(value));
    }

    let home = home_dir().ok_or(XdgError::MissingHomeDirectory)?;
    Ok(join_all(home, fallback))
}

fn xdg_from<const N: usize>(
    env: &HashMap<String, String>,
    key: &str,
    home: &Path,
    fallback: [&str; N],
) -> PathBuf {
    env.get(key)
        .filter(|value| !value.is_empty())
        .map_or_else(|| join_all(home.to_path_buf(), fallback), PathBuf::from)
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
}

fn join_all<const N: usize>(mut base: PathBuf, parts: [&str; N]) -> PathBuf {
    for part in parts {
        base.push(part);
    }
    base
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::Path};

    use super::{xdg_cache_home_from, xdg_config_home_from, xdg_data_home_from};

    #[test]
    fn uses_env_overrides_when_present() {
        let env = HashMap::from([("XDG_DATA_HOME".to_owned(), "/tmp/data".to_owned())]);

        assert_eq!(
            xdg_data_home_from(&env, Path::new("/home/test")),
            Path::new("/tmp/data")
        );
    }

    #[test]
    fn uses_standard_fallbacks() {
        let env = HashMap::new();
        let home = Path::new("/home/test");

        assert_eq!(
            xdg_config_home_from(&env, home),
            Path::new("/home/test/.config")
        );
        assert_eq!(
            xdg_cache_home_from(&env, home),
            Path::new("/home/test/.cache")
        );
    }
}
