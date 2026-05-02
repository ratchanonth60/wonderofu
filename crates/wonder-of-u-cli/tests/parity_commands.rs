//! Parity verification: the Rust command registry must cover every command
//! surface in `claude-leak/commands/`.

use std::collections::HashSet;

use wonder_of_u_cli::build_command_registry;

/// All command names (and their canonical display names) from `claude-leak/commands/`.
/// Excludes TypeScript helpers that are not standalone commands.
const CLAUDE_LEAK_COMMANDS: &[&str] = &[
    "add-dir",
    "advisor",
    "agents",
    "ant-trace",
    "autofix-pr",
    "backfill-sessions",
    "branch",
    "break-cache",
    "bridge",
    "bridge-kick",
    "brief",
    "btw",
    "bughunter",
    "chrome",
    "clear",
    "color",
    "commit",
    "commit-push-pr",
    "compact",
    "config",
    "context",
    "copy",
    "cost",
    "ctx-viz",
    "debug-tool-call",
    "desktop",
    "diff",
    "doctor",
    "effort",
    "env",
    "exit",
    "export",
    "extra-usage",
    "fast",
    "feedback",
    "files",
    "good-claude",
    "heapdump",
    "help",
    "hooks",
    "ide",
    "init",
    "init-verifiers",
    "insights",
    "install",
    "install-github-app",
    "install-slack-app",
    "issue",
    "keybindings",
    "login",
    "logout",
    "mcp",
    "memory",
    "mobile",
    "mock-limits",
    "model",
    "oauth-refresh",
    "onboarding",
    "output-style",
    "passes",
    "perf-issue",
    "permissions",
    "plan",
    "plugin",
    "pr-comments",
    "privacy-settings",
    "rate-limit-options",
    "release-notes",
    "reload-plugins",
    "remote-env",
    "remote-setup",
    "rename",
    "reset-limits",
    "resume",
    "review",
    "rewind",
    "sandbox-toggle",
    "security-review",
    "session",
    "share",
    "skills",
    "stats",
    "status",
    "stickers",
    "summary",
    "tag",
    "tasks",
    "teleport",
    "terminal-setup",
    "theme",
    "thinkback",
    "thinkback-play",
    "ultraplan",
    "upgrade",
    "usage",
    "version",
    "vim",
    "voice",
];

fn registry_names() -> HashSet<String> {
    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");

    let mut names: HashSet<String> = HashSet::new();
    for spec in registry.all_specs() {
        names.insert(spec.name.clone());
        for alias in &spec.aliases {
            names.insert(alias.clone());
        }
    }
    names
}

#[test]
fn all_claude_leak_commands_are_registered() {
    let registered = registry_names();

    let mut gaps: Vec<&str> = Vec::new();
    for &cmd in CLAUDE_LEAK_COMMANDS {
        // Command names may be stored with or without a "/" prefix.
        let with_slash = format!("/{cmd}");
        if !registered.contains(&with_slash) && !registered.contains(cmd) {
            gaps.push(cmd);
        }
    }

    assert!(
        gaps.is_empty(),
        "The following claude-leak commands are NOT in the Rust registry:\n  {}",
        gaps.join("\n  ")
    );
}

#[test]
fn registry_has_no_duplicate_names() {
    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");

    let mut seen: HashSet<String> = HashSet::new();
    let mut duplicates: Vec<String> = Vec::new();
    for spec in registry.all_specs() {
        for name in std::iter::once(&spec.name).chain(spec.aliases.iter()) {
            if !seen.insert(name.clone()) {
                duplicates.push(name.clone());
            }
        }
    }

    assert!(
        duplicates.is_empty(),
        "Duplicate command names/aliases: {:?}",
        duplicates
    );
}
