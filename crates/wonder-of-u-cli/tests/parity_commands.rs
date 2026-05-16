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

// ---------------------------------------------------------------------------
// P0/P1 parity behaviour tests
// ---------------------------------------------------------------------------

/// Shared helper to build a `CommandContext` for parity tests.
fn parity_ctx(dir: &std::path::Path) -> wonder_of_u_core::CommandContext {
    use wonder_of_u_core::{CommandContext, FeatureSet, PermissionMode, SessionId};
    CommandContext {
        session_id: SessionId::new(),
        cwd: dir.to_path_buf(),
        features: FeatureSet::first_release(),
        authenticated: false,
        interactive: false,
        permission_mode: PermissionMode::Default,
        theme: None,
        session_color: None,
        effort_level: None,
        brief_mode: false,
        fast_mode: false,
        optimize_token_mode: false,
        session_tags: Vec::new(),
        additional_working_directories: Vec::new(),
    }
}

/// /login: env-guided providers (AWS, GCP) must return Ok guidance, not a hard
/// error, so callers can display actionable text to the user.
#[test]
fn login_env_providers_return_guidance_not_error() {
    use futures::executor::block_on;
    use wonder_of_u_core::{CommandInvocation, CommandOutput};

    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");
    let login = registry.resolve("login").expect("login must be registered");

    // bedrock is an AWS SigV4 provider — must return Ok env guidance.
    let result = block_on(login.execute(
        parity_ctx(dir.path()),
        CommandInvocation {
            name: "login".into(),
            args: "--provider bedrock".into(),
            raw: "/login --provider bedrock".into(),
        },
    ));
    assert!(
        result.is_ok(),
        "login bedrock must return Ok, not Err; got: {:?}",
        result
    );
    let CommandOutput::Text(text) = result.unwrap() else {
        panic!("expected Text output");
    };
    assert!(
        text.contains("auth_mode=env"),
        "bedrock guidance must contain auth_mode=env; got:\n{text}"
    );

    // 'vertex' is the GCP OAuth2 provider (Google Vertex AI) — must return Ok env guidance.
    let result = block_on(login.execute(
        parity_ctx(dir.path()),
        CommandInvocation {
            name: "login".into(),
            args: "--provider vertex".into(),
            raw: "/login --provider vertex".into(),
        },
    ));
    assert!(
        result.is_ok(),
        "login vertex must return Ok, not Err; got: {:?}",
        result
    );
    let CommandOutput::Text(text) = result.unwrap() else {
        panic!("expected Text output");
    };
    assert!(
        text.contains("auth_mode=env"),
        "vertex guidance must contain auth_mode=env; got:\n{text}"
    );
}

/// /login: bare invocation (no --provider) must return Ok with a guide.
#[test]
fn login_bare_invocation_returns_auth_mode_guide() {
    use futures::executor::block_on;
    use wonder_of_u_core::{CommandInvocation, CommandOutput};

    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");
    let login = registry.resolve("login").expect("login must be registered");

    let result = block_on(login.execute(
        parity_ctx(dir.path()),
        CommandInvocation {
            name: "login".into(),
            args: String::new(),
            raw: "/login".into(),
        },
    ));
    let output = result.expect("bare /login must succeed");
    let CommandOutput::Text(text) = output else {
        panic!("expected Text output");
    };
    assert!(
        text.contains("api-key") || text.contains("auth"),
        "bare /login guide must mention auth modes; got:\n{text}"
    );
}

/// /login: local no-auth provider must return Ok guidance that mentions
/// /model set, not a hard error.
#[test]
fn login_local_no_auth_provider_returns_guidance() {
    use futures::executor::block_on;
    use wonder_of_u_core::{CommandInvocation, CommandOutput};

    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");
    let login = registry.resolve("login").expect("login must be registered");

    let result = block_on(login.execute(
        parity_ctx(dir.path()),
        CommandInvocation {
            name: "login".into(),
            args: "--provider local".into(),
            raw: "/login --provider local".into(),
        },
    ));
    let output = result.expect("local no-auth provider must return Ok");
    let CommandOutput::Text(text) = output else {
        panic!("expected Text output");
    };
    assert!(
        text.contains("auth_mode=none"),
        "local guidance must contain auth_mode=none; got:\n{text}"
    );
    assert!(
        text.contains("/model set"),
        "local guidance must suggest /model set; got:\n{text}"
    );
}

/// /terminal-setup: output must include terminal_type and setup_needed fields.
#[test]
fn terminal_setup_output_includes_required_parity_fields() {
    use futures::executor::block_on;
    use wonder_of_u_core::{CommandInvocation, CommandOutput};

    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");
    let cmd = registry
        .resolve("terminal-setup")
        .expect("terminal-setup must be registered");

    let result = block_on(cmd.execute(
        parity_ctx(dir.path()),
        CommandInvocation {
            name: "terminal-setup".into(),
            args: String::new(),
            raw: "/terminal-setup".into(),
        },
    ));
    let output = result.expect("terminal-setup must succeed");
    let CommandOutput::Text(text) = output else {
        panic!("expected Text output");
    };
    assert!(
        text.contains("terminal_type="),
        "output must include terminal_type field; got:\n{text}"
    );
    assert!(
        text.contains("setup_needed="),
        "output must include setup_needed field; got:\n{text}"
    );
    assert!(
        text.contains("## Terminal Setup"),
        "output must start with the heading; got:\n{text}"
    );
}

/// /compact: the command spec description must mention custom instructions.
#[test]
fn compact_spec_description_mentions_custom_instructions() {
    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");
    let compact = registry
        .resolve("compact")
        .expect("compact must be registered");
    let spec = compact.spec();
    assert!(
        spec.description.contains("instructions") || spec.description.contains("hint"),
        "compact spec description should mention custom instructions; got: {}",
        spec.description
    );
}

/// /output-style: must remain registered (even if hidden/deprecated).
#[test]
fn output_style_command_is_registered() {
    let registered = registry_names();
    assert!(
        registered.contains("output-style"),
        "output-style must remain registered for backward compatibility"
    );
}

/// /setup and its 'settings' alias must both be registered.
#[test]
fn setup_command_and_alias_are_registered() {
    let registered = registry_names();
    assert!(
        registered.contains("setup"),
        "setup command must be registered"
    );
    assert!(
        registered.contains("settings"),
        "settings alias for setup must be registered"
    );
}

/// /keybindings must be registered.
#[test]
fn keybindings_command_is_registered() {
    let registered = registry_names();
    assert!(
        registered.contains("keybindings"),
        "keybindings command must be registered"
    );
}

/// /terminal-setup must be registered under its canonical kebab-case name and
/// the upstream-compatible camelCase alias.
#[test]
fn terminal_setup_camel_alias_is_registered() {
    let registered = registry_names();
    assert!(
        registered.contains("terminal-setup"),
        "terminal-setup must be registered under its canonical kebab-case name"
    );
    // The alias terminalSetup is registered for upstream compatibility.
    assert!(
        registered.contains("terminalSetup"),
        "terminalSetup alias must be registered for upstream parity"
    );
}
