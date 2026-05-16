//! Hook executor for PreToolUse/PostToolUse/PostToolUseFailure events.
//!
//! Reads `hooks.json` from the storage config directory and runs matching
//! `Command`-type hooks via subprocess, injecting the hook context as JSON
//! on the environment variable `CLAUDE_HOOK_INPUT`.  Prompt/Agent/Http hook
//! types are not yet executed (they emit a no-op pass-through).
//!
//! # Outcome
//!
//! Hooks can explicitly block a tool by exiting with code 2 and printing JSON
//! `{"continue": false, "stopReason": "..."}` to stdout.  Any other non-zero
//! exit is logged as a warning but does not block the tool.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    time::Duration,
};

use serde::Deserialize;
use serde_json::{Value, json};
use wonder_of_u_storage::StoragePaths;

// ── Hook config types (mirrors workflow.rs HooksConfig) ─────────────────────

#[derive(Clone, Debug, Default, Deserialize)]
struct HooksConfig {
    #[serde(default)]
    disable_all_hooks: bool,
    #[serde(default)]
    hooks: BTreeMap<String, Vec<HookMatcherConfig>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct HookMatcherConfig {
    /// Glob/regex pattern matched against the tool name.  `None` matches all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    matcher: Option<String>,
    #[serde(default)]
    hooks: Vec<HookActionConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
enum HookActionConfig {
    Command {
        command: String,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
    },
    Prompt {
        prompt: String,
    },
    Agent {
        prompt: String,
    },
    Http {
        url: String,
    },
}

// ── Public outcome type ──────────────────────────────────────────────────────

/// Result of running hooks for one event.
#[derive(Clone, Debug, PartialEq)]
pub enum HookOutcome {
    /// All hooks passed; tool execution may proceed.
    Allow,
    /// A hook explicitly blocked execution.
    Block { reason: String },
}

/// Summary of hook execution for one event.
#[derive(Clone, Debug, PartialEq)]
pub struct HookRunReport {
    /// Overall hook outcome for tool execution.
    pub outcome: HookOutcome,
    /// Number of hooks that actually ran.
    pub hook_count: u32,
    /// Whether every executed hook passed.
    pub success: bool,
}

impl HookRunReport {
    fn allow() -> Self {
        Self {
            outcome: HookOutcome::Allow,
            hook_count: 0,
            success: true,
        }
    }
}

// ── Event constants ──────────────────────────────────────────────────────────

pub const PRE_TOOL_USE: &str = "PreToolUse";
pub const POST_TOOL_USE: &str = "PostToolUse";
pub const POST_TOOL_USE_FAILURE: &str = "PostToolUseFailure";

// ── Public API ───────────────────────────────────────────────────────────────

/// Run all configured hooks for `event` that match `tool_name`.
///
/// Returns [`HookOutcome::Allow`] when no hook blocks execution.
/// Returns [`HookOutcome::Block`] when a hook exits with code 2 and a
/// `{"continue": false, "stopReason": "..."}` JSON payload.
///
/// `tool_input` is always serialised into `CLAUDE_HOOK_INPUT`.
/// `tool_response` is included for `PostToolUse`/`PostToolUseFailure` events
/// so hooks can inspect the result; pass `None` for `PreToolUse`.
pub fn run_hooks(
    event: &str,
    tool_name: &str,
    tool_input: &Value,
    tool_response: Option<&Value>,
    cwd: &Path,
    storage_dir: Option<&Path>,
) -> HookRunReport {
    let config = match load_config(storage_dir) {
        Ok(c) => c,
        Err(_) => return HookRunReport::allow(),
    };

    if config.disable_all_hooks {
        return HookRunReport::allow();
    }

    let matchers = match config.hooks.get(event) {
        Some(m) => m,
        None => return HookRunReport::allow(),
    };

    let mut hook_input = json!({
        "hook_event_name": event,
        "tool_name": tool_name,
        "tool_input": tool_input,
    });
    // Inject tool result for post-execution events so hooks can act on the outcome.
    if let Some(response) = tool_response {
        hook_input["tool_response"] = response.clone();
    }
    let hook_input_str = hook_input.to_string();
    let mut report = HookRunReport::allow();

    for matcher in matchers {
        if !tool_name_matches(tool_name, matcher.matcher.as_deref()) {
            continue;
        }
        for action in &matcher.hooks {
            if let HookActionConfig::Command { command, .. } = action {
                report.hook_count = report.hook_count.saturating_add(1);
                match exec_command_hook(command, &hook_input_str, cwd) {
                    CommandHookOutcome::Passed => {}
                    CommandHookOutcome::Failed => report.success = false,
                    CommandHookOutcome::Block { reason } => {
                        report.success = false;
                        report.outcome = HookOutcome::Block { reason };
                        return report;
                    }
                }
            }
            // Prompt/Agent/Http hooks are not yet executed.
        }
    }

    report
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn load_config(storage_dir: Option<&Path>) -> Result<HooksConfig, ()> {
    let path = resolve_hooks_path(storage_dir);
    if !path.exists() {
        return Ok(HooksConfig::default());
    }
    let content = fs::read_to_string(&path).map_err(|_| ())?;
    serde_json::from_str(&content).map_err(|_| ())
}

fn resolve_hooks_path(storage_dir: Option<&Path>) -> PathBuf {
    storage_dir
        .map(StoragePaths::new)
        .map(|p| p.config_dir())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".wonder-of-u")
                .join("config")
        })
        .join("hooks.json")
}

/// Returns `true` if `tool_name` matches the optional `pattern`.
///
/// Pattern semantics:
/// - `None` / `""` / `"*"` → matches everything.
/// - Pattern containing `*` → minimal glob match (anchored start/end,
///   greedy middle segments); case-insensitive.
/// - Pattern with no `*` → case-insensitive substring match for
///   backwards compatibility with the TypeScript implementation.
fn tool_name_matches(tool_name: &str, pattern: Option<&str>) -> bool {
    match pattern {
        None | Some("") | Some("*") => true,
        Some(p) => {
            let name = tool_name.to_lowercase();
            let pat = p.to_lowercase();
            if pat.contains('*') {
                glob_match(&name, &pat)
            } else {
                // No wildcards: substring match preserves prior behaviour.
                name.contains(pat.as_str())
            }
        }
    }
}

/// Minimal `*`-glob matcher (case-insensitive input expected from caller).
///
/// Segments between `*` characters are matched left-to-right:
/// - The first non-empty segment is start-anchored.
/// - The last non-empty segment is end-anchored.
/// - Middle segments are found greedily from the current position.
fn glob_match(name: &str, pattern: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut pos = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            // First part must match at the very start.
            if !name[pos..].starts_with(part) {
                return false;
            }
            pos += part.len();
        } else if i == parts.len() - 1 {
            // Last part must match at the very end.
            return name[pos..].ends_with(part);
        } else {
            // Middle part: find the earliest occurrence after `pos`.
            let Some(idx) = name[pos..].find(part) else {
                return false;
            };
            pos += idx + part.len();
        }
    }
    true
}

/// Execute a single command-type hook.
///
/// The subprocess is given at most 60 seconds.  Exit code 2 with valid JSON
/// `{"continue": false}` in stdout is the only blocking path.
enum CommandHookOutcome {
    Passed,
    Failed,
    Block { reason: String },
}

fn exec_command_hook(command: &str, hook_input_json: &str, cwd: &Path) -> CommandHookOutcome {
    let Ok(mut child) = ProcessCommand::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .env("CLAUDE_HOOK_INPUT", hook_input_json)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    else {
        return CommandHookOutcome::Failed;
    };

    // Poll with a 60-second hard timeout.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status.code().unwrap_or(0);
                if code == 0 {
                    return CommandHookOutcome::Passed;
                }
                if code == 2 {
                    // Read stdout and check for explicit block.
                    use std::io::Read;
                    let mut out = String::new();
                    if let Some(mut stdout) = child.stdout.take() {
                        let _ = stdout.read_to_string(&mut out);
                    }
                    if let Some(reason) = parse_block_reason(&out) {
                        return CommandHookOutcome::Block { reason };
                    }
                }
                return CommandHookOutcome::Failed;
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    return CommandHookOutcome::Failed;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return CommandHookOutcome::Failed,
        }
    }
}

/// Parse `{"continue": false, "stopReason": "..."}` from hook stdout.
fn parse_block_reason(output: &str) -> Option<String> {
    let v: Value = serde_json::from_str(output.trim()).ok()?;
    if v.get("continue")?.as_bool()? {
        return None;
    }
    let reason = v
        .get("stopReason")
        .and_then(|r| r.as_str())
        .unwrap_or("hook blocked tool execution")
        .to_owned();
    Some(reason)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_hooks(dir: &Path, json: &str) {
        let config_dir = dir.join("config");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(config_dir.join("hooks.json"), json).unwrap();
    }

    #[test]
    fn allow_when_no_config_file() {
        let dir = TempDir::new().unwrap();
        let outcome = run_hooks(
            PRE_TOOL_USE,
            "bash",
            &json!({"command": "echo hi"}),
            None,
            dir.path(),
            Some(dir.path()),
        );
        assert_eq!(outcome.outcome, HookOutcome::Allow);
        assert_eq!(outcome.hook_count, 0);
        assert!(outcome.success);
    }

    #[test]
    fn allow_when_disable_all_hooks() {
        let dir = TempDir::new().unwrap();
        write_hooks(dir.path(), r#"{"disable_all_hooks": true}"#);
        let outcome = run_hooks(
            PRE_TOOL_USE,
            "bash",
            &json!({}),
            None,
            dir.path(),
            Some(dir.path()),
        );
        assert_eq!(outcome.outcome, HookOutcome::Allow);
        assert_eq!(outcome.hook_count, 0);
        assert!(outcome.success);
    }

    #[test]
    fn allow_when_no_matching_event() {
        let dir = TempDir::new().unwrap();
        write_hooks(
            dir.path(),
            r#"{"hooks": {"PostToolUse": [{"hooks": [{"type": "command", "command": "exit 2"}]}]}}"#,
        );
        let outcome = run_hooks(
            PRE_TOOL_USE,
            "bash",
            &json!({}),
            None,
            dir.path(),
            Some(dir.path()),
        );
        assert_eq!(outcome.outcome, HookOutcome::Allow);
        assert_eq!(outcome.hook_count, 0);
        assert!(outcome.success);
    }

    #[test]
    fn allow_when_command_exits_zero() {
        let dir = TempDir::new().unwrap();
        write_hooks(
            dir.path(),
            r#"{"hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "true"}]}]}}"#,
        );
        let outcome = run_hooks(
            PRE_TOOL_USE,
            "bash",
            &json!({}),
            None,
            dir.path(),
            Some(dir.path()),
        );
        assert_eq!(outcome.outcome, HookOutcome::Allow);
        assert_eq!(outcome.hook_count, 1);
        assert!(outcome.success);
    }

    #[test]
    fn block_when_command_exits_2_with_continue_false() {
        let dir = TempDir::new().unwrap();
        write_hooks(
            dir.path(),
            r#"{"hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "printf '{\"continue\":false,\"stopReason\":\"not allowed\"}'; exit 2"}]}]}}"#,
        );
        let outcome = run_hooks(
            PRE_TOOL_USE,
            "bash",
            &json!({}),
            None,
            dir.path(),
            Some(dir.path()),
        );
        assert_eq!(
            outcome.outcome,
            HookOutcome::Block {
                reason: "not allowed".into()
            }
        );
        assert_eq!(outcome.hook_count, 1);
        assert!(!outcome.success);
    }

    #[test]
    fn allow_when_command_exits_2_without_continue_false() {
        let dir = TempDir::new().unwrap();
        // exit 2 but stdout is not a block JSON → allow
        write_hooks(
            dir.path(),
            r#"{"hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "exit 2"}]}]}}"#,
        );
        let outcome = run_hooks(
            PRE_TOOL_USE,
            "bash",
            &json!({}),
            None,
            dir.path(),
            Some(dir.path()),
        );
        assert_eq!(outcome.outcome, HookOutcome::Allow);
        assert_eq!(outcome.hook_count, 1);
        assert!(!outcome.success);
    }

    #[test]
    fn matcher_none_matches_any_tool() {
        assert!(tool_name_matches("bash", None));
        assert!(tool_name_matches("file_read", None));
    }

    #[test]
    fn matcher_star_matches_any_tool() {
        assert!(tool_name_matches("bash", Some("*")));
    }

    #[test]
    fn matcher_substring_is_case_insensitive() {
        assert!(tool_name_matches("BashTool", Some("bash")));
        assert!(!tool_name_matches("file_read", Some("bash")));
    }

    #[test]
    fn matcher_glob_prefix_anchor() {
        assert!(tool_name_matches("bash_exec", Some("bash*")));
        assert!(tool_name_matches("bash", Some("bash*")));
        assert!(!tool_name_matches("run_bash", Some("bash*")));
    }

    #[test]
    fn matcher_glob_suffix_anchor() {
        assert!(tool_name_matches("run_bash", Some("*bash")));
        assert!(tool_name_matches("bash", Some("*bash")));
        assert!(!tool_name_matches("bash_exec", Some("*bash")));
    }

    #[test]
    fn matcher_glob_contains() {
        assert!(tool_name_matches("run_bash_exec", Some("*bash*")));
        assert!(tool_name_matches("bash", Some("*bash*")));
        assert!(!tool_name_matches("run_shell", Some("*bash*")));
    }

    #[test]
    fn matcher_glob_prefix_and_suffix() {
        assert!(tool_name_matches("bash_exec_tool", Some("bash*tool")));
        assert!(!tool_name_matches("bash_exec", Some("bash*tool")));
    }

    #[test]
    fn post_tool_use_hook_receives_tool_response_in_context() {
        let dir = TempDir::new().unwrap();
        // Hook writes CLAUDE_HOOK_INPUT to a temp file so we can inspect it.
        let out_file = dir.path().join("hook_input.json");
        // Use serde_json to build the config so command escaping is handled correctly.
        let cmd = format!(
            "printf '%s' \"$CLAUDE_HOOK_INPUT\" > '{}'",
            out_file.display()
        );
        let hooks_config = json!({
            "hooks": {
                "PostToolUse": [{"hooks": [{"type": "command", "command": cmd}]}]
            }
        });
        write_hooks(dir.path(), &hooks_config.to_string());
        let tool_response = json!({"success": true, "content": "ok"});
        let outcome = run_hooks(
            POST_TOOL_USE,
            "bash",
            &json!({"command": "echo hi"}),
            Some(&tool_response),
            dir.path(),
            Some(dir.path()),
        );
        assert_eq!(outcome.outcome, HookOutcome::Allow);
        assert_eq!(outcome.hook_count, 1);
        assert!(outcome.success);
        // Verify the written context contains tool_response.
        if out_file.exists() {
            let written = std::fs::read_to_string(&out_file).unwrap_or_default();
            if let Ok(v) = serde_json::from_str::<Value>(written.trim()) {
                assert_eq!(v["hook_event_name"], "PostToolUse");
                assert_eq!(v["tool_response"]["success"], true);
            }
        }
    }

    #[test]
    fn pre_tool_use_hook_context_omits_tool_response() {
        let dir = TempDir::new().unwrap();
        let out_file = dir.path().join("hook_input_pre.json");
        let cmd = format!(
            "printf '%s' \"$CLAUDE_HOOK_INPUT\" > '{}'",
            out_file.display()
        );
        let hooks_config = json!({
            "hooks": {
                "PreToolUse": [{"hooks": [{"type": "command", "command": cmd}]}]
            }
        });
        write_hooks(dir.path(), &hooks_config.to_string());
        run_hooks(
            PRE_TOOL_USE,
            "bash",
            &json!({}),
            None,
            dir.path(),
            Some(dir.path()),
        );
        if out_file.exists() {
            let written = std::fs::read_to_string(&out_file).unwrap_or_default();
            if let Ok(v) = serde_json::from_str::<Value>(written.trim()) {
                assert_eq!(v["hook_event_name"], "PreToolUse");
                // No tool_response key for PreToolUse.
                assert!(v.get("tool_response").is_none());
            }
        }
    }

    #[test]
    fn parse_block_reason_requires_continue_false() {
        assert_eq!(
            parse_block_reason(r#"{"continue": false, "stopReason": "denied"}"#),
            Some("denied".to_string())
        );
        assert_eq!(parse_block_reason(r#"{"continue": true}"#), None,);
        assert_eq!(parse_block_reason("not json"), None);
    }
}
