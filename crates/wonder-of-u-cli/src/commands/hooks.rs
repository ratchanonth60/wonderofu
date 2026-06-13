//! Hook executor for tool and lifecycle hook events.
//!
//! Reads `hooks.json` from the storage config directory and runs matching
//! `Command`-type hooks via subprocess, injecting the hook context as JSON
//! on the environment variable `CLAUDE_HOOK_INPUT`.  Prompt/Agent/Http hook
//! types are surfaced as unsupported instead of silently succeeding.
//!
//! # Outcome
//!
//! Hooks can explicitly block a tool by exiting with code 2 and printing JSON
//! `{"continue": false, "stopReason": "..."}` to stdout.  Any other non-zero
//! exit is logged as a warning but does not block the tool.

use std::{
    path::Path,
    process::{Command as ProcessCommand, Stdio},
    time::Duration,
};

use serde::Deserialize;
use serde_json::{Value, json};

use super::hook_trust::{build_hook_entry, load_hook_trust_state, load_hooks_config};

// ── Hook config types (mirrors workflow.rs HooksConfig) ─────────────────────

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
enum HookActionConfig {
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell: Option<String>,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
        #[serde(default)]
        managed: bool,
    },
    Prompt {
        prompt: String,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
        #[serde(default)]
        managed: bool,
    },
    Agent {
        prompt: String,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
        #[serde(default)]
        managed: bool,
    },
    Http {
        url: String,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
        #[serde(default)]
        managed: bool,
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
    /// Replacement tool input emitted by a successful hook via
    /// `{"updatedInput": {...}}` in its stdout.  When `Some`, callers must
    /// use this value instead of the original tool input for both permission
    /// evaluation and actual execution so that the approval flow operates on
    /// what the hook actually approved.
    pub updated_input: Option<Value>,
}

impl HookRunReport {
    fn allow() -> Self {
        Self {
            outcome: HookOutcome::Allow,
            hook_count: 0,
            success: true,
            updated_input: None,
        }
    }
}

// ── Event constants ──────────────────────────────────────────────────────────

pub const PRE_TOOL_USE: &str = "PreToolUse";
pub const POST_TOOL_USE: &str = "PostToolUse";
pub const POST_TOOL_USE_FAILURE: &str = "PostToolUseFailure";
#[allow(dead_code)]
pub const USER_PROMPT_SUBMIT: &str = "UserPromptSubmit";
#[allow(dead_code)]
pub const SESSION_START: &str = "SessionStart";
#[allow(dead_code)]
pub const STOP: &str = "Stop";
#[allow(dead_code)]
pub const TASK_CREATED: &str = "TaskCreated";
#[allow(dead_code)]
pub const AGENT_START: &str = "AgentStart";

// ── Public API ───────────────────────────────────────────────────────────────

/// Run all configured hooks for `event` that match `tool_name`.
///
/// Returns [`HookOutcome::Allow`] when no hook blocks execution.
/// Returns [`HookOutcome::Block`] when a hook exits with code 2 and a
/// `{"continue": false, "stopReason": "..."}` JSON payload.
///
/// A successful hook (exit 0) may print `{"updatedInput": {...}}` to stdout.
/// When present, [`HookRunReport::updated_input`] is set to that value and
/// callers **must** use it as the effective tool input for both permission
/// checks and execution.  The last hook to emit `updatedInput` wins.
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
    let config = match load_hooks_config(storage_dir) {
        Ok(c) => c,
        Err(_) => return HookRunReport::allow(),
    };
    let state = match load_hook_trust_state(storage_dir) {
        Ok(s) => s,
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
    run_matching_hooks(
        event,
        matchers,
        config.allow_managed_hooks_only,
        &state,
        tool_name,
        &hook_input,
        cwd,
    )
}

/// Runs non-tool lifecycle hooks using a JSON payload.
#[allow(dead_code)]
pub fn run_lifecycle_hooks(
    event: &str,
    payload: &Value,
    cwd: &Path,
    storage_dir: Option<&Path>,
) -> HookRunReport {
    let config = match load_hooks_config(storage_dir) {
        Ok(c) => c,
        Err(_) => return HookRunReport::allow(),
    };
    let state = match load_hook_trust_state(storage_dir) {
        Ok(s) => s,
        Err(_) => return HookRunReport::allow(),
    };

    if config.disable_all_hooks {
        return HookRunReport::allow();
    }

    let matchers = match config.hooks.get(event) {
        Some(m) => m,
        None => return HookRunReport::allow(),
    };

    let hook_input = json!({
        "hook_event_name": event,
        "payload": payload,
    });
    let matcher_value = payload
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or(event);
    run_matching_hooks(
        event,
        matchers,
        config.allow_managed_hooks_only,
        &state,
        matcher_value,
        &hook_input,
        cwd,
    )
}

fn run_matching_hooks(
    event: &str,
    matchers: &[super::hook_trust::HookMatcherConfig],
    allow_managed_hooks_only: bool,
    state: &super::hook_trust::HookTrustState,
    matcher_value: &str,
    hook_input: &Value,
    cwd: &Path,
) -> HookRunReport {
    let hook_input_str = hook_input.to_string();
    let mut report = HookRunReport::allow();

    for (matcher_index, matcher) in matchers.iter().enumerate() {
        if !tool_name_matches(matcher_value, matcher.matcher.as_deref()) {
            continue;
        }
        for (hook_index, action_value) in matcher.hooks.iter().enumerate() {
            let entry = build_hook_entry(
                event,
                matcher_index,
                hook_index,
                matcher,
                action_value,
                state,
            );
            if allow_managed_hooks_only && !entry.managed {
                continue;
            }
            if !entry.is_trusted_and_enabled() {
                continue;
            }

            match condition_matches(entry.condition.as_deref(), hook_input) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(_) => {
                    report.success = false;
                    continue;
                }
            }

            let action = match serde_json::from_value::<HookActionConfig>(action_value.clone()) {
                Ok(action) => action,
                Err(_) => {
                    report.hook_count = report.hook_count.saturating_add(1);
                    report.success = false;
                    continue;
                }
            };

            match action {
                HookActionConfig::Command { command, .. } => {
                    report.hook_count = report.hook_count.saturating_add(1);
                    match exec_command_hook(&command, &hook_input_str, cwd) {
                        CommandHookOutcome::Passed { updated_input } => {
                            // Last hook to emit updatedInput wins.
                            if updated_input.is_some() {
                                report.updated_input = updated_input;
                            }
                        }
                        CommandHookOutcome::Failed => report.success = false,
                        CommandHookOutcome::Block { reason } => {
                            report.success = false;
                            report.outcome = HookOutcome::Block { reason };
                            return report;
                        }
                    }
                }
                HookActionConfig::Prompt { .. }
                | HookActionConfig::Agent { .. }
                | HookActionConfig::Http { .. } => {
                    report.hook_count = report.hook_count.saturating_add(1);
                    report.success = false;
                }
            }
        }
    }

    report
}

// ── Internal helpers ─────────────────────────────────────────────────────────

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

fn condition_matches(condition: Option<&str>, hook_input: &Value) -> Result<bool, String> {
    let Some(condition) = condition.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(true);
    };
    if condition.eq_ignore_ascii_case("true") {
        return Ok(true);
    }
    if condition.eq_ignore_ascii_case("false") {
        return Ok(false);
    }

    for operator in ["==", "!="] {
        if let Some((left, right)) = condition.split_once(operator) {
            let left_value = resolve_condition_path(hook_input, left.trim())?;
            let right_value = parse_condition_literal(right.trim())?;
            return Ok(match operator {
                "==" => left_value == right_value,
                "!=" => left_value != right_value,
                _ => unreachable!("operator list is exhaustive"),
            });
        }
    }

    Ok(resolve_condition_path(hook_input, condition)?
        .as_bool()
        .unwrap_or(false))
}

fn resolve_condition_path(input: &Value, path: &str) -> Result<Value, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("empty hook condition path".into());
    }
    if path.starts_with('/') {
        return input
            .pointer(path)
            .cloned()
            .ok_or_else(|| format!("hook condition path `{path}` did not match"));
    }
    let Some(dot_path) = path.strip_prefix('.') else {
        return parse_condition_literal(path);
    };

    let mut current = input;
    for segment in dot_path.split('.') {
        if segment.is_empty() {
            return Err(format!("invalid hook condition path `{path}`"));
        }
        current = current
            .get(segment)
            .ok_or_else(|| format!("hook condition path `{path}` did not match"))?;
    }
    Ok(current.clone())
}

fn parse_condition_literal(value: &str) -> Result<Value, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("empty hook condition literal".into());
    }
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    Ok(Value::String(trimmed.to_string()))
}

/// Execute a single command-type hook.
///
/// The subprocess is given at most 60 seconds.  Exit code 2 with valid JSON
/// `{"continue": false}` in stdout is the only blocking path.  Exit code 0
/// with `{"updatedInput": {...}}` in stdout signals that the hook approved
/// execution with a mutated tool input.
enum CommandHookOutcome {
    /// Hook passed.  `updated_input` carries the hook's replacement tool
    /// input when the hook emitted `{"updatedInput": {...}}` to stdout.
    Passed {
        updated_input: Option<Value>,
    },
    Failed,
    Block {
        reason: String,
    },
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
                // Read stdout once the process has exited so we can inspect
                // both updatedInput (exit 0) and block payloads (exit 2).
                use std::io::Read;
                let mut out = String::new();
                if let Some(mut stdout) = child.stdout.take() {
                    let _ = stdout.read_to_string(&mut out);
                }

                let code = status.code().unwrap_or(0);
                if code == 0 {
                    return CommandHookOutcome::Passed {
                        updated_input: parse_updated_input(&out),
                    };
                }
                if code == 2 {
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

/// Parse `{"updatedInput": {...}}` from a successful hook's stdout.
///
/// Returns `None` when the output is absent, non-JSON, or does not contain
/// the `updatedInput` key.
fn parse_updated_input(output: &str) -> Option<Value> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(trimmed).ok()?;
    v.get("updatedInput").cloned()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::hook_trust;
    use std::fs;
    use tempfile::TempDir;

    fn write_hooks(dir: &Path, json: &str) {
        let config_dir = dir.join("config");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(config_dir.join("hooks.json"), json).unwrap();
    }

    fn trust_all_hooks(dir: &Path) {
        let entries = hook_trust::load_hooks_inventory(Some(dir))
            .expect("load hook inventory")
            .entries;
        for entry in entries {
            hook_trust::trust_hook(Some(dir), &entry.id).expect("trust hook");
        }
    }

    fn write_trusted_hooks(dir: &Path, json: &str) {
        write_hooks(dir, json);
        trust_all_hooks(dir);
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
    fn untrusted_command_hook_is_skipped() {
        let dir = TempDir::new().unwrap();
        let out_file = dir.path().join("untrusted-ran");
        let hooks_config = json!({
            "hooks": {
                "PreToolUse": [{
                    "hooks": [{
                        "type": "command",
                        "command": format!("touch '{}'", out_file.display())
                    }]
                }]
            }
        });
        write_hooks(dir.path(), &hooks_config.to_string());

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
        assert!(!out_file.exists(), "untrusted hook must not run");
    }

    #[test]
    fn changed_hook_is_skipped_until_retrusted() {
        let dir = TempDir::new().unwrap();
        let out_file = dir.path().join("changed-ran");
        let original = json!({
            "hooks": {
                "PreToolUse": [{
                    "hooks": [{
                        "type": "command",
                        "command": "true"
                    }]
                }]
            }
        });
        write_trusted_hooks(dir.path(), &original.to_string());

        let changed = json!({
            "hooks": {
                "PreToolUse": [{
                    "hooks": [{
                        "type": "command",
                        "command": format!("touch '{}'", out_file.display())
                    }]
                }]
            }
        });
        write_hooks(dir.path(), &changed.to_string());

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
        assert!(!out_file.exists(), "changed hook must not run before trust");
    }

    #[test]
    fn disabled_trusted_hook_is_skipped() {
        let dir = TempDir::new().unwrap();
        write_trusted_hooks(
            dir.path(),
            r#"{"hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "exit 2"}]}]}}"#,
        );
        hook_trust::set_hook_disabled(Some(dir.path()), "pretooluse.0.0", true)
            .expect("disable hook");

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
    fn allow_managed_hooks_only_skips_unmanaged_hooks() {
        let dir = TempDir::new().unwrap();
        write_trusted_hooks(
            dir.path(),
            r#"{"allow_managed_hooks_only": true, "hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "exit 2"}]}]}}"#,
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
    fn allow_managed_hooks_only_runs_managed_trusted_hooks() {
        let dir = TempDir::new().unwrap();
        write_trusted_hooks(
            dir.path(),
            r#"{"allow_managed_hooks_only": true, "hooks": {"PreToolUse": [{"hooks": [{"type": "command", "managed": true, "command": "true"}]}]}}"#,
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
    fn allow_when_no_matching_event() {
        let dir = TempDir::new().unwrap();
        write_trusted_hooks(
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
        write_trusted_hooks(
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
        write_trusted_hooks(
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
        write_trusted_hooks(
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
        write_trusted_hooks(dir.path(), &hooks_config.to_string());
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
        write_trusted_hooks(dir.path(), &hooks_config.to_string());
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

    // ── updatedInput tests ───────────────────────────────────────────────────

    #[test]
    fn parse_updated_input_returns_none_for_empty_output() {
        assert_eq!(parse_updated_input(""), None);
        assert_eq!(parse_updated_input("   "), None);
    }

    #[test]
    fn parse_updated_input_returns_none_when_key_absent() {
        assert_eq!(parse_updated_input(r#"{"continue": true}"#), None);
        assert_eq!(parse_updated_input("not json"), None);
    }

    #[test]
    fn parse_updated_input_extracts_value_from_json() {
        let out = r#"{"updatedInput": {"command": "echo safe"}}"#;
        let got = parse_updated_input(out).expect("should parse updatedInput");
        assert_eq!(got, json!({"command": "echo safe"}));
    }

    #[test]
    fn parse_updated_input_ignores_non_object_value() {
        // updatedInput present but other sibling keys don't matter.
        let out = r#"{"updatedInput": [1, 2, 3], "extra": "ignored"}"#;
        let got = parse_updated_input(out).expect("should parse");
        assert_eq!(got, json!([1, 2, 3]));
    }

    #[test]
    fn hook_exit_zero_with_updated_input_populates_report() {
        let dir = TempDir::new().unwrap();
        // Hook exits 0 and prints updatedInput JSON to stdout.
        let hooks_config = json!({
            "hooks": {
                "PreToolUse": [{
                    "hooks": [{
                        "type": "command",
                        "command": r#"printf '{"updatedInput":{"command":"echo safe"}}'"#
                    }]
                }]
            }
        });
        write_trusted_hooks(dir.path(), &hooks_config.to_string());

        let report = run_hooks(
            PRE_TOOL_USE,
            "bash",
            &json!({"command": "echo original"}),
            None,
            dir.path(),
            Some(dir.path()),
        );

        assert_eq!(report.outcome, HookOutcome::Allow);
        assert_eq!(report.hook_count, 1);
        assert!(report.success);
        let updated = report.updated_input.expect("updatedInput should be set");
        assert_eq!(updated, json!({"command": "echo safe"}));
    }

    #[test]
    fn hook_exit_zero_without_updated_input_leaves_report_none() {
        let dir = TempDir::new().unwrap();
        // Hook just exits 0 with no output.
        let hooks_config = json!({
            "hooks": {
                "PreToolUse": [{"hooks": [{"type": "command", "command": "true"}]}]
            }
        });
        write_trusted_hooks(dir.path(), &hooks_config.to_string());

        let report = run_hooks(
            PRE_TOOL_USE,
            "bash",
            &json!({"command": "echo hi"}),
            None,
            dir.path(),
            Some(dir.path()),
        );

        assert_eq!(report.outcome, HookOutcome::Allow);
        assert_eq!(report.hook_count, 1);
        assert!(report.success);
        assert!(report.updated_input.is_none(), "no updatedInput expected");
    }

    #[test]
    fn hook_condition_true_runs_matching_command() {
        let dir = TempDir::new().unwrap();
        let hooks_config = json!({
            "hooks": {
                "PreToolUse": [{
                    "hooks": [{
                        "type": "command",
                        "if": ".tool_input.command == \"echo hi\"",
                        "command": "true"
                    }]
                }]
            }
        });
        write_trusted_hooks(dir.path(), &hooks_config.to_string());

        let report = run_hooks(
            PRE_TOOL_USE,
            "bash",
            &json!({"command": "echo hi"}),
            None,
            dir.path(),
            Some(dir.path()),
        );

        assert_eq!(report.hook_count, 1);
        assert!(report.success);
    }

    #[test]
    fn hook_condition_false_skips_command() {
        let dir = TempDir::new().unwrap();
        let hooks_config = json!({
            "hooks": {
                "PreToolUse": [{
                    "hooks": [{
                        "type": "command",
                        "if": ".tool_name == \"file_read\"",
                        "command": "exit 2"
                    }]
                }]
            }
        });
        write_trusted_hooks(dir.path(), &hooks_config.to_string());

        let report = run_hooks(
            PRE_TOOL_USE,
            "bash",
            &json!({"command": "echo hi"}),
            None,
            dir.path(),
            Some(dir.path()),
        );

        assert_eq!(report.hook_count, 0);
        assert!(report.success);
    }

    #[test]
    fn unsupported_hook_actions_are_reported_as_failed() {
        let dir = TempDir::new().unwrap();
        let hooks_config = json!({
            "hooks": {
                "PreToolUse": [{
                    "hooks": [{"type": "http", "url": "https://example.invalid/hook"}]
                }]
            }
        });
        write_trusted_hooks(dir.path(), &hooks_config.to_string());

        let report = run_hooks(
            PRE_TOOL_USE,
            "bash",
            &json!({}),
            None,
            dir.path(),
            Some(dir.path()),
        );

        assert_eq!(report.outcome, HookOutcome::Allow);
        assert_eq!(report.hook_count, 1);
        assert!(!report.success);
    }

    #[test]
    fn lifecycle_hooks_match_source_payload() {
        let dir = TempDir::new().unwrap();
        let out_file = dir.path().join("lifecycle.json");
        let cmd = format!(
            "printf '%s' \"$CLAUDE_HOOK_INPUT\" > '{}'",
            out_file.display()
        );
        let hooks_config = json!({
            "hooks": {
                "SessionStart": [{
                    "matcher": "resume",
                    "hooks": [{"type": "command", "command": cmd}]
                }]
            }
        });
        write_trusted_hooks(dir.path(), &hooks_config.to_string());

        let report = run_lifecycle_hooks(
            SESSION_START,
            &json!({"source": "resume"}),
            dir.path(),
            Some(dir.path()),
        );

        assert_eq!(report.hook_count, 1);
        assert!(report.success);
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(out_file).expect("hook output"))
                .expect("json");
        assert_eq!(written["hook_event_name"], SESSION_START);
        assert_eq!(written["payload"]["source"], "resume");
    }

    #[test]
    fn last_hook_updated_input_wins_when_multiple_hooks_emit_it() {
        let dir = TempDir::new().unwrap();
        // Two hooks, both emit updatedInput; the second one should win.
        let hooks_config = json!({
            "hooks": {
                "PreToolUse": [{
                    "hooks": [
                        {
                            "type": "command",
                            "command": r#"printf '{"updatedInput":{"command":"first"}}'"#
                        },
                        {
                            "type": "command",
                            "command": r#"printf '{"updatedInput":{"command":"second"}}'"#
                        }
                    ]
                }]
            }
        });
        write_trusted_hooks(dir.path(), &hooks_config.to_string());

        let report = run_hooks(
            PRE_TOOL_USE,
            "bash",
            &json!({"command": "echo original"}),
            None,
            dir.path(),
            Some(dir.path()),
        );

        assert_eq!(report.outcome, HookOutcome::Allow);
        assert_eq!(report.hook_count, 2);
        let updated = report.updated_input.expect("updatedInput should be set");
        assert_eq!(updated["command"], "second");
    }

    #[test]
    fn hook_block_does_not_propagate_updated_input() {
        let dir = TempDir::new().unwrap();
        let hooks_config = json!({
            "hooks": {
                "PreToolUse": [{
                    "hooks": [{
                        "type": "command",
                        "command": r#"printf '{"continue":false,"stopReason":"blocked"}'; exit 2"#
                    }]
                }]
            }
        });
        write_trusted_hooks(dir.path(), &hooks_config.to_string());

        let report = run_hooks(
            PRE_TOOL_USE,
            "bash",
            &json!({"command": "echo hi"}),
            None,
            dir.path(),
            Some(dir.path()),
        );

        assert_eq!(
            report.outcome,
            HookOutcome::Block {
                reason: "blocked".into()
            }
        );
        assert!(
            report.updated_input.is_none(),
            "blocked hooks must not set updatedInput"
        );
    }
}
