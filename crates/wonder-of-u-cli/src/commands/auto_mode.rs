//! `auto-mode` subcommand — inspect and manage the yolo/auto-mode classifier rules.
//!
//! Mirrors the three `claude auto-mode` subcommands so existing scripts are portable:
//!
//! - `defaults` → print the built-in heuristic safe/deny lists as JSON
//! - `config`   → print the effective config (user overrides where present, built-in otherwise)

use std::path::Path;

use wonder_of_u_agent::{AutoModeRules, SettingsStore};
use wonder_of_u_core::Result;

// ── Built-in defaults (derived from YoloClassifier constants in denial_tracker.rs) ──

const BUILTIN_ALLOW: &[&str] = &[
    "ls",
    "ll",
    "la",
    "dir",
    "pwd",
    "cd",
    "cat",
    "head",
    "tail",
    "less",
    "more",
    "file",
    "stat",
    "wc",
    "grep",
    "rg",
    "ag",
    "find",
    "locate",
    "which",
    "whereis",
    "type",
    "echo",
    "printf",
    "date",
    "uname",
    "hostname",
    "whoami",
    "id",
    "git",
    "hg",
    "svn",
    "cargo",
    "make",
    "cmake",
    "env",
    "printenv",
    "ps",
    "top",
    "htop",
    "df",
    "du",
    "ping",
    "traceroute",
    "nslookup",
    "dig",
    "host",
    "man",
    "help",
    "true",
    "false",
    "test",
    "[",
];

const BUILTIN_SOFT_DENY: &[&str] = &[
    "iptables",
    "ip6tables",
    "nft",
    "ufw",
    "firewall-cmd",
    "route",
    "ifconfig",
    "iwconfig",
    "nmcli",
    "networksetup",
    "arp",
    "arping",
    "tc",
    "ssh",
    "scp",
    "sftp",
    "rsync",
    "ftp",
    "nc",
    "ncat",
    "netcat",
    "socat",
    "telnet",
    "curl",
    "wget",
    "fetch",
    "httpie",
    "http",
];

/// Returns the built-in default `AutoModeRules` derived from `YoloClassifier`.
pub fn default_rules() -> AutoModeRules {
    AutoModeRules {
        allow: BUILTIN_ALLOW.iter().map(|s| s.to_string()).collect(),
        soft_deny: BUILTIN_SOFT_DENY.iter().map(|s| s.to_string()).collect(),
        environment: Vec::new(),
    }
}

/// `auto-mode defaults` — print built-in rules as JSON.
pub fn defaults() -> Result<String> {
    let rules = default_rules();
    let value = &rules;
    Ok(serde_json::to_string_pretty(value)?)
}

/// `auto-mode config` — print effective rules (user overrides or built-in fallback).
///
/// Per-section replace semantics: a non-empty user section replaces that
/// section's defaults entirely; an empty/absent section falls through.
pub fn config(storage_dir: Option<&Path>) -> Result<String> {
    let user_rules = storage_dir
        .and_then(|dir| SettingsStore::new(dir).read().ok())
        .and_then(|s| s.auto_mode);

    let defaults = default_rules();

    let effective = match user_rules {
        None => defaults,
        Some(u) => AutoModeRules {
            allow: if u.allow.is_empty() {
                defaults.allow
            } else {
                u.allow
            },
            soft_deny: if u.soft_deny.is_empty() {
                defaults.soft_deny
            } else {
                u.soft_deny
            },
            environment: u.environment,
        },
    };

    Ok(serde_json::to_string_pretty(&effective)?)
}
