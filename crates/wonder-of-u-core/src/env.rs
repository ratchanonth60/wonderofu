//! Environment variable helpers.

use std::env::{self, VarError};

use thiserror::Error;

/// Errors produced by required environment variable lookups.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EnvVarError {
    /// The requested variable was not present.
    #[error("environment variable `{0}` is not set")]
    Missing(String),
    /// The requested variable was present but not valid UTF-8.
    #[error("environment variable `{0}` is not valid unicode")]
    NotUnicode(String),
}

/// Returns an environment variable or the provided default value.
pub fn get_env_var(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}

/// Parses a boolean-ish environment variable value.
pub fn parse_bool_env_value(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Returns a parsed boolean environment variable when present and valid.
pub fn get_bool_env(key: &str) -> Option<bool> {
    env::var(key).ok().as_deref().and_then(parse_bool_env_value)
}

/// Returns an environment variable or an error when it is missing.
pub fn require_env_var(key: &str) -> Result<String, EnvVarError> {
    match env::var(key) {
        Ok(value) => Ok(value),
        Err(VarError::NotPresent) => Err(EnvVarError::Missing(key.to_owned())),
        Err(VarError::NotUnicode(_)) => Err(EnvVarError::NotUnicode(key.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::{EnvVarError, get_bool_env, get_env_var, parse_bool_env_value, require_env_var};

    #[test]
    fn falls_back_to_default_for_missing_values() {
        assert_eq!(
            get_env_var("WONDER_OF_U_TEST_MISSING_ENV_VAR", "fallback"),
            "fallback"
        );
    }

    #[test]
    fn parses_boolean_values() {
        assert_eq!(parse_bool_env_value("true"), Some(true));
        assert_eq!(parse_bool_env_value("OFF"), Some(false));
        assert_eq!(parse_bool_env_value("sometimes"), None);
    }

    #[test]
    fn returns_none_for_missing_bool_env() {
        assert_eq!(get_bool_env("WONDER_OF_U_TEST_MISSING_BOOL_ENV_VAR"), None);
    }

    #[test]
    fn errors_for_missing_required_value() {
        assert_eq!(
            require_env_var("WONDER_OF_U_TEST_MISSING_REQUIRED_ENV_VAR"),
            Err(EnvVarError::Missing(
                "WONDER_OF_U_TEST_MISSING_REQUIRED_ENV_VAR".to_owned()
            ))
        );
    }
}
