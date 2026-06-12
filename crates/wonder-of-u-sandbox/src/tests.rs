use std::path::PathBuf;

use super::*;

#[test]
fn workspace_policy_has_read_and_write() {
    let policy = SandboxPolicy::workspace_policy(PathBuf::from("/home/user/project"));
    assert!(!policy.readable_paths.is_empty());
    assert!(!policy.writable_paths.is_empty());
    assert!(!policy.allow_network);
}

#[test]
fn read_only_policy_has_no_writes() {
    let policy = SandboxPolicy::read_only_policy();
    assert!(!policy.readable_paths.is_empty());
    assert!(policy.writable_paths.is_empty());
}

#[test]
fn default_policy_is_empty() {
    let policy = SandboxPolicy::default();
    assert!(policy.readable_paths.is_empty());
    assert!(policy.writable_paths.is_empty());
}
