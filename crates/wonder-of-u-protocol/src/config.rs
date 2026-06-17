//! Session-scoped configuration types: approval policy, sandbox policy,
//! network access.
//!
//! These mirror codex's `AskForApproval` / `SandboxPolicy` / `NetworkAccess`
//! closely but without the upstream `JsonSchema` / `TS` derives — Phase 0
//! keeps this crate minimal (serde + Clone + Debug + PartialEq). Phase 5 will
//! bring in schema generation if needed.

use serde::{Deserialize, Serialize};

/// Determines under which conditions the user is consulted before running a
/// command proposed by the agent.
///
/// Mirrors codex's `AskForApproval` (sans the deprecated `OnFailure` variant,
/// which we collapse into `OnRequest` per Phase 1's executor policy).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AskForApproval {
    /// Only "known safe" read-only commands are auto-approved. Everything
    /// else prompts.
    #[serde(rename = "untrusted")]
    UnlessTrusted,
    /// The model decides when to ask. Default.
    #[default]
    OnRequest,
    /// Fine-grained per-category controls.
    Granular(GranularApprovalConfig),
    /// Never ask; failures are returned to the model.
    Never,
}

/// Per-category switches for `AskForApproval::Granular`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GranularApprovalConfig {
    /// Allow shell command approval requests.
    pub sandbox_approval: bool,
    /// Allow prompts triggered by execpolicy rules.
    pub rules: bool,
    /// Allow prompts triggered by skill scripts.
    #[serde(default)]
    pub skill_approval: bool,
    /// Allow prompts triggered by the `request_permissions` tool.
    #[serde(default)]
    pub request_permissions: bool,
    /// Allow MCP elicitation prompts.
    pub mcp_elicitations: bool,
}

/// Outbound network access availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkAccess {
    /// Outbound network is restricted (default).
    #[default]
    Restricted,
    /// Outbound network is enabled.
    Enabled,
}

impl NetworkAccess {
    /// True if network access is enabled.
    pub fn is_enabled(self) -> bool {
        matches!(self, NetworkAccess::Enabled)
    }
}

/// Tool-shell execution restrictions.
///
/// Mirrors codex's `SandboxPolicy`. Phase 0 carries the shape so the wire
/// contract is complete; the executor policy engine behind `wonder-of-u-exec`
/// implements it in Phase 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SandboxPolicy {
    /// No restrictions whatsoever. Use with caution.
    #[serde(rename = "danger-full-access")]
    DangerFullAccess,
    /// Read-only access; no writes anywhere.
    #[serde(rename = "read-only")]
    ReadOnly {
        /// When true, outbound network access is allowed.
        #[serde(default)]
        network_access: bool,
    },
    /// The process is already in an external sandbox; honor the network
    /// setting but otherwise allow full disk access.
    #[serde(rename = "external-sandbox")]
    ExternalSandbox {
        /// Whether the external sandbox permits outbound network traffic.
        #[serde(default)]
        network_access: NetworkAccess,
    },
    /// Read-only everywhere except `writable_roots` plus the cwd / `/tmp`.
    #[serde(rename = "workspace-write")]
    WorkspaceWrite {
        /// Additional writable folders beyond the defaults.
        #[serde(default)]
        writable_roots: Vec<String>,
        /// When true, outbound network access is allowed.
        #[serde(default)]
        network_access: bool,
        /// When true, do not include the per-user `TMPDIR` in writable roots.
        #[serde(default)]
        exclude_tmpdir_env_var: bool,
        /// When true, do not include `/tmp` in writable roots on Unix.
        #[serde(default)]
        exclude_slash_tmp: bool,
    },
}

impl SandboxPolicy {
    /// Construct a default read-only policy.
    pub fn new_read_only_policy() -> Self {
        SandboxPolicy::ReadOnly {
            network_access: false,
        }
    }

    /// Construct a workspace-write policy (read-only disk + cwd / `/tmp` writes).
    pub fn new_workspace_write_policy() -> Self {
        SandboxPolicy::WorkspaceWrite {
            writable_roots: Vec::new(),
            network_access: false,
            exclude_tmpdir_env_var: false,
            exclude_slash_tmp: false,
        }
    }

    /// True if the policy permits full disk writes.
    pub fn has_full_disk_write_access(&self) -> bool {
        matches!(
            self,
            SandboxPolicy::DangerFullAccess | SandboxPolicy::ExternalSandbox { .. }
        )
    }

    /// True if the policy permits outbound network traffic.
    pub fn has_full_network_access(&self) -> bool {
        match self {
            SandboxPolicy::DangerFullAccess => true,
            SandboxPolicy::ExternalSandbox { network_access } => network_access.is_enabled(),
            SandboxPolicy::ReadOnly { network_access, .. } => *network_access,
            SandboxPolicy::WorkspaceWrite { network_access, .. } => *network_access,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_for_approval_roundtrip() {
        for value in [
            AskForApproval::UnlessTrusted,
            AskForApproval::OnRequest,
            AskForApproval::Never,
            AskForApproval::Granular(GranularApprovalConfig {
                sandbox_approval: true,
                rules: false,
                skill_approval: false,
                request_permissions: false,
                mcp_elicitations: true,
            }),
        ] {
            let json = serde_json::to_string(&value).unwrap();
            let parsed: AskForApproval = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, value);
        }
    }

    #[test]
    fn sandbox_policy_roundtrip() {
        for value in [
            SandboxPolicy::new_read_only_policy(),
            SandboxPolicy::new_workspace_write_policy(),
            SandboxPolicy::DangerFullAccess,
            SandboxPolicy::ExternalSandbox {
                network_access: NetworkAccess::Enabled,
            },
            SandboxPolicy::WorkspaceWrite {
                writable_roots: vec!["/data".to_string()],
                network_access: true,
                exclude_tmpdir_env_var: false,
                exclude_slash_tmp: false,
            },
        ] {
            let json = serde_json::to_string(&value).unwrap();
            let parsed: SandboxPolicy = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, value);
        }
    }

    #[test]
    fn sandbox_policy_helpers() {
        assert!(SandboxPolicy::DangerFullAccess.has_full_disk_write_access());
        assert!(SandboxPolicy::DangerFullAccess.has_full_network_access());
        assert!(!SandboxPolicy::new_read_only_policy().has_full_disk_write_access());
        assert!(!SandboxPolicy::new_read_only_policy().has_full_network_access());
        assert!(
            SandboxPolicy::ExternalSandbox {
                network_access: NetworkAccess::Enabled,
            }
            .has_full_network_access()
        );
        assert!(
            !SandboxPolicy::ExternalSandbox {
                network_access: NetworkAccess::Restricted,
            }
            .has_full_network_access()
        );
    }

    #[test]
    fn network_access_roundtrip() {
        for value in [NetworkAccess::Restricted, NetworkAccess::Enabled] {
            let json = serde_json::to_string(&value).unwrap();
            let parsed: NetworkAccess = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, value);
        }
    }
}
