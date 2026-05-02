//! Lightweight semantic version helpers.

use std::num::ParseIntError;

use thiserror::Error;

/// A parsed `major.minor.patch` version tuple.
pub type Version = (u64, u64, u64);

/// Errors produced while parsing semantic versions.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum VersionParseError {
    /// The version did not contain three numeric components.
    #[error("invalid semantic version: {0}")]
    InvalidFormat(String),
    /// A numeric component failed to parse.
    #[error(transparent)]
    InvalidNumber(#[from] ParseIntError),
}

/// Parses a version string into `(major, minor, patch)`.
pub fn parse_version(input: &str) -> Result<Version, VersionParseError> {
    let core = input
        .trim()
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .ok_or_else(|| VersionParseError::InvalidFormat(input.to_owned()))?;
    let mut parts = core.split('.');

    let major = parts
        .next()
        .ok_or_else(|| VersionParseError::InvalidFormat(input.to_owned()))?
        .parse()?;
    let minor = parts
        .next()
        .ok_or_else(|| VersionParseError::InvalidFormat(input.to_owned()))?
        .parse()?;
    let patch = parts
        .next()
        .ok_or_else(|| VersionParseError::InvalidFormat(input.to_owned()))?
        .parse()?;

    if parts.next().is_some() {
        return Err(VersionParseError::InvalidFormat(input.to_owned()));
    }

    Ok((major, minor, patch))
}

/// Returns whether version `a` is greater than or equal to version `b`.
pub fn version_gte(a: &str, b: &str) -> Result<bool, VersionParseError> {
    Ok(parse_version(a)? >= parse_version(b)?)
}

#[cfg(test)]
mod tests {
    use super::{parse_version, version_gte};

    #[test]
    fn parses_versions_with_common_prefixes_and_suffixes() {
        assert_eq!(parse_version("v1.2.3-alpha+1").unwrap(), (1, 2, 3));
    }

    #[test]
    fn compares_versions() {
        assert!(version_gte("1.2.3", "1.2.0").unwrap());
        assert!(!version_gte("1.1.9", "1.2.0").unwrap());
    }
}
