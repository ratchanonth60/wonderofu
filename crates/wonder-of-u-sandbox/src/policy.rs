use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A filesystem permission entry for a path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileSystemEntry {
    /// Absolute or project-relative path.
    pub path: PathBuf,
    /// Access decision for this path and its children.
    pub decision: PolicyDecision,
}

/// Access decision for a filesystem path.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyDecision {
    /// Full read+write access.
    ReadWrite,
    /// Read-only access.
    ReadOnly,
    /// No access (path is invisible/excluded).
    Denied,
}

/// Sandbox policy defining what the sandboxed process may access.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SandboxPolicy {
    /// Filesystem entries the process may read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readable_paths: Vec<FileSystemEntry>,
    /// Filesystem entries the process may write to.
    /// These paths are also readable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writable_paths: Vec<FileSystemEntry>,
    /// Whether network access is allowed.
    #[serde(default = "default_allow_network")]
    pub allow_network: bool,
}

fn default_allow_network() -> bool {
    false
}

impl SandboxPolicy {
    /// Build a policy suitable for a typical project workspace.
    /// Gives read access to the whole filesystem and write access to
    /// the project root and standard temp directories.
    #[must_use]
    pub fn workspace_policy(project_root: PathBuf) -> Self {
        let readable = vec![
            FileSystemEntry {
                path: PathBuf::from("/"),
                decision: PolicyDecision::ReadOnly,
            },
        ];
        let writable = vec![
            FileSystemEntry {
                path: project_root,
                decision: PolicyDecision::ReadWrite,
            },
            FileSystemEntry {
                path: PathBuf::from("/tmp"),
                decision: PolicyDecision::ReadWrite,
            },
            FileSystemEntry {
                path: PathBuf::from("/dev/shm"),
                decision: PolicyDecision::ReadWrite,
            },
        ];
        Self {
            readable_paths: readable,
            writable_paths: writable,
            allow_network: false,
        }
    }

    /// Build a strict read-only policy with no write access.
    #[must_use]
    pub fn read_only_policy() -> Self {
        Self {
            readable_paths: vec![FileSystemEntry {
                path: PathBuf::from("/"),
                decision: PolicyDecision::ReadOnly,
            }],
            writable_paths: Vec::new(),
            allow_network: false,
        }
    }
}
