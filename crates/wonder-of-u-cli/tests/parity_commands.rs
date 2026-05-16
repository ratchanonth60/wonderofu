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

// ---------------------------------------------------------------------------
// Remote / cloud honest-stub parity tests
// ---------------------------------------------------------------------------

/// /teleport: spec description must advertise local-session listing and note
/// that remote teleport is not available.
#[test]
fn teleport_spec_is_local_first_honest() {
    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");
    let cmd = registry
        .resolve("teleport")
        .expect("teleport must be registered");
    let spec = cmd.spec();
    assert!(
        spec.description.to_lowercase().contains("local"),
        "teleport description must mention local sessions; got: {}",
        spec.description
    );
    assert!(
        spec.description.to_lowercase().contains("not available")
            || spec.description.to_lowercase().contains("unavailable"),
        "teleport description must note remote is not available; got: {}",
        spec.description
    );
}

/// /teleport: output header must contain `Local sessions` and the
/// `remote teleport: not available` note.
#[test]
fn teleport_output_header_is_local_sessions_with_unavailable_note() {
    use futures::executor::block_on;
    use wonder_of_u_core::{CommandInvocation, CommandOutput};

    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");
    let cmd = registry
        .resolve("teleport")
        .expect("teleport must be registered");

    let result = block_on(cmd.execute(
        parity_ctx(dir.path()),
        CommandInvocation {
            name: "teleport".into(),
            args: String::new(),
            raw: "/teleport".into(),
        },
    ));
    let output = result.expect("/teleport must succeed");
    let CommandOutput::Text(text) = output else {
        panic!("expected Text output");
    };
    assert!(
        text.to_lowercase().contains("local session"),
        "/teleport output must contain 'Local session'; got:\n{text}"
    );
    assert!(
        text.to_lowercase().contains("not available"),
        "/teleport output must note remote teleport is not available; got:\n{text}"
    );
}

/// /remote-env (no config stored): output must say remote sessions are not
/// available and point toward /status.
#[test]
fn remote_env_none_output_says_unavailable_and_references_status() {
    use futures::executor::block_on;
    use wonder_of_u_core::{CommandInvocation, CommandOutput};

    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");
    let cmd = registry
        .resolve("remote-env")
        .expect("remote-env must be registered");

    let result = block_on(cmd.execute(
        parity_ctx(dir.path()),
        CommandInvocation {
            name: "remote-env".into(),
            args: String::new(),
            raw: "/remote-env".into(),
        },
    ));
    let output = result.expect("/remote-env must succeed");
    let CommandOutput::Text(text) = output else {
        panic!("expected Text output");
    };
    assert!(
        text.to_lowercase().contains("not available"),
        "/remote-env (no config) must say remote is not available; got:\n{text}"
    );
    assert!(
        text.contains("/status"),
        "/remote-env (no config) must point to /status; got:\n{text}"
    );
}

/// /remote-env (config stored): output must show host/port/auth but mark
/// the transport as inactive / inert, not functional.
#[test]
fn remote_env_some_output_marks_transport_inactive() {
    use futures::executor::block_on;
    use wonder_of_u_core::{CommandInvocation, CommandOutput};

    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");
    let setup_cmd = registry
        .resolve("remote-setup")
        .expect("remote-setup must be registered");

    // Store a config entry first.
    block_on(setup_cmd.execute(
        parity_ctx(dir.path()),
        CommandInvocation {
            name: "remote-setup".into(),
            args: "example.com:22 ssh".into(),
            raw: "/remote-setup example.com:22 ssh".into(),
        },
    ))
    .expect("remote-setup must succeed");

    // Now query remote-env.
    let env_cmd = registry
        .resolve("remote-env")
        .expect("remote-env must be registered");
    let result = block_on(env_cmd.execute(
        parity_ctx(dir.path()),
        CommandInvocation {
            name: "remote-env".into(),
            args: String::new(),
            raw: "/remote-env".into(),
        },
    ));
    let output = result.expect("/remote-env must succeed");
    let CommandOutput::Text(text) = output else {
        panic!("expected Text output");
    };
    assert!(
        text.contains("example.com"),
        "/remote-env must show stored host; got:\n{text}"
    );
    assert!(
        text.to_lowercase().contains("inactive")
            || text.to_lowercase().contains("inert")
            || text.to_lowercase().contains("not available"),
        "/remote-env must mark transport as inactive; got:\n{text}"
    );
    // Must not claim the transport is live.
    assert!(
        !text.to_lowercase().contains("connected")
            && !text.to_lowercase().contains("active transport"),
        "/remote-env must not imply a live connection; got:\n{text}"
    );
}

/// `/remote-setup` must say it only stores local config and that remote/web
/// session transport is unavailable in this Rust build.
#[test]
fn remote_setup_output_marks_config_only_and_transport_unavailable() {
    use futures::executor::block_on;
    use wonder_of_u_core::{CommandInvocation, CommandOutput};

    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");
    let cmd = registry
        .resolve("remote-setup")
        .expect("remote-setup must be registered");

    let result = block_on(cmd.execute(
        parity_ctx(dir.path()),
        CommandInvocation {
            name: "remote-setup".into(),
            args: "myhost.example.com:2222 key".into(),
            raw: "/remote-setup myhost.example.com:2222 key".into(),
        },
    ));
    let output = result.expect("/remote-setup must succeed");
    let CommandOutput::Text(text) = output else {
        panic!("expected Text output");
    };
    assert!(
        text.to_lowercase().contains("unavailable"),
        "/remote-setup must state transport is unavailable; got:\n{text}"
    );
    assert!(
        text.to_lowercase().contains("config stored locally only")
            || text.to_lowercase().contains("stores config only"),
        "/remote-setup must mention config-only behavior; got:\n{text}"
    );
}

/// `/remote-setup` help/spec text must be explicit that this Rust build stores
/// config only and does not provide a remote/web session transport.
#[test]
fn remote_setup_spec_is_honest_about_config_only_transport() {
    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");
    let spec = registry
        .resolve("remote-setup")
        .expect("remote-setup must be registered")
        .spec();

    let description = spec.description.to_lowercase();
    assert!(
        description.contains("config"),
        "remote-setup description must mention config storage; got: {}",
        spec.description
    );
    assert!(
        description.contains("transport unavailable")
            || description.contains("transport unavailable in this rust build")
            || description.contains("transport unavailable in this build"),
        "remote-setup description must mention unavailable transport; got: {}",
        spec.description
    );
}

/// /bridge-kick: must always return an unavailability message and must NOT
/// say `restart required` or imply a real bridge process.
#[test]
fn bridge_kick_is_noop_unavailable_with_no_restart_language() {
    use futures::executor::block_on;
    use wonder_of_u_core::{CommandInvocation, CommandOutput};

    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");
    let cmd = registry
        .resolve("bridge-kick")
        .expect("bridge-kick must be registered");

    let result = block_on(cmd.execute(
        parity_ctx(dir.path()),
        CommandInvocation {
            name: "bridge-kick".into(),
            args: String::new(),
            raw: "/bridge-kick".into(),
        },
    ));
    let output = result.expect("/bridge-kick must succeed");
    let CommandOutput::Text(text) = output else {
        panic!("expected Text output");
    };
    assert!(
        text.to_lowercase().contains("not available")
            || text.to_lowercase().contains("not implemented"),
        "/bridge-kick must report unavailability; got:\n{text}"
    );
    assert!(
        !text.to_lowercase().contains("restart required"),
        "/bridge-kick must not say 'restart required'; got:\n{text}"
    );
    assert!(
        !text.to_lowercase().contains("restart the bridge process"),
        "/bridge-kick must not instruct restarting a bridge process; got:\n{text}"
    );
}

// ---------------------------------------------------------------------------
// Telemetry / experiments honest-omission tests  (ADR 0001)
// ---------------------------------------------------------------------------

/// `/status` output must include `analytics=unsupported` and
/// `experiments=unsupported`, confirming that no Datadog sink or GrowthBook
/// remote evaluation is wired up (see docs/adr/0001-telemetry-experiments-omission.md).
#[test]
fn status_output_includes_analytics_and_experiments_unsupported() {
    use futures::executor::block_on;
    use wonder_of_u_core::{CommandInvocation, CommandOutput};

    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");
    let cmd = registry
        .resolve("status")
        .expect("status must be registered");

    let result = block_on(cmd.execute(
        parity_ctx(dir.path()),
        CommandInvocation {
            name: "status".into(),
            args: String::new(),
            raw: "/status".into(),
        },
    ));
    let output = result.expect("/status must succeed");
    let CommandOutput::Text(text) = output else {
        panic!("expected Text output from /status");
    };

    assert!(
        text.contains("analytics=unsupported"),
        "/status must emit analytics=unsupported (ADR 0001); got:\n{text}"
    );
    assert!(
        text.contains("experiments=unsupported"),
        "/status must emit experiments=unsupported (ADR 0001); got:\n{text}"
    );
    // Must NOT claim analytics or experiments are active/ok, as that would
    // imply a telemetry pipeline that does not exist.
    assert!(
        !text.contains("analytics=ok"),
        "/status must not claim analytics=ok; got:\n{text}"
    );
    assert!(
        !text.contains("experiments=ok"),
        "/status must not claim experiments=ok; got:\n{text}"
    );
}

/// `/status` must clearly mark cloud-sync-adjacent surfaces as non-parity:
/// local-only settings, deferred remote managed settings, and unsupported team
/// memory sync.
#[test]
fn status_output_marks_cloud_sync_surfaces_as_non_parity() {
    use futures::executor::block_on;
    use wonder_of_u_core::{CommandInvocation, CommandOutput};

    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");
    let cmd = registry
        .resolve("status")
        .expect("status must be registered");

    let result = block_on(cmd.execute(
        parity_ctx(dir.path()),
        CommandInvocation {
            name: "status".into(),
            args: String::new(),
            raw: "/status".into(),
        },
    ));
    let output = result.expect("/status must succeed");
    let CommandOutput::Text(text) = output else {
        panic!("expected Text output from /status");
    };

    assert!(
        text.contains("settings_sync=local_only"),
        "/status must keep settings sync local-only; got:\n{text}"
    );
    assert!(
        text.contains("settings_sync_cloud=unsupported"),
        "/status must state settings sync cloud parity is unsupported; got:\n{text}"
    );
    assert!(
        text.contains("settings_sync_reason=")
            && text.contains("cloud upload/download backends are unavailable"),
        "/status must explain settings sync stays local; got:\n{text}"
    );
    assert!(
        text.contains("remote_managed_settings=deferred"),
        "/status must mark remote managed settings deferred; got:\n{text}"
    );
    assert!(
        text.contains("remote_managed_settings_reason=")
            && text.contains("deferred until a real policy backend exists"),
        "/status must explain remote managed settings are deferred; got:\n{text}"
    );
    assert!(
        text.contains("team_memory_sync=unsupported"),
        "/status must mark team memory sync unsupported; got:\n{text}"
    );
    assert!(
        text.contains("team_memory_sync_reason=")
            && text.contains("session memory remains local-only"),
        "/status must explain team memory sync is unsupported; got:\n{text}"
    );
}

/// `/ultraplan`: output must mention cloud backend unavailability and that the
/// flag is stored for local planning guidance only.
#[test]
fn ultraplan_output_says_cloud_unavailable_and_local_guidance_only() {
    use futures::executor::block_on;
    use wonder_of_u_core::{CommandInvocation, CommandOutput};

    let dir = tempfile::tempdir().expect("temp dir");
    let registry = build_command_registry(Some(dir.path().to_path_buf())).expect("build registry");
    let cmd = registry
        .resolve("ultraplan")
        .expect("ultraplan must be registered");

    let result = block_on(cmd.execute(
        parity_ctx(dir.path()),
        CommandInvocation {
            name: "ultraplan".into(),
            args: String::new(),
            raw: "/ultraplan".into(),
        },
    ));
    let output = result.expect("/ultraplan must succeed");
    let CommandOutput::Text(text) = output else {
        panic!("expected Text output");
    };
    assert!(
        text.to_lowercase().contains("not available"),
        "/ultraplan must say cloud backend is not available; got:\n{text}"
    );
    assert!(
        text.to_lowercase().contains("local"),
        "/ultraplan must clarify flag is for local planning guidance; got:\n{text}"
    );
}
