use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wait_timeout::ChildExt;
use wonder_of_u_core::{
    FeatureFlag, PermissionDecision, PermissionDecisionReason, PermissionRequest, Result,
    ShellSafetyIssue, ShellSafetyVerdict, TaskState, TaskStatus, Tool, ToolContext, ToolKind,
    ToolResult, ToolSchema, ToolSpec, ToolUseId, WonderError, evaluate_permission, resolve_path,
};
use wonder_of_u_storage::TaskStore;

use crate::{base_spec, display_path, parse_input, require_non_empty_path, require_non_empty_text};

const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// Represents bash input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BashInput {
    /// Stores the command
    pub command: String,
    /// Stores the cwd
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// Stores the timeout secs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Stores the timeout
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// Stores the description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(
        default,
        rename = "run_in_background",
        skip_serializing_if = "Option::is_none"
    )]
    /// Stores the run in background
    pub run_in_background: Option<bool>,
    #[serde(
        default,
        rename = "dangerouslyDisableSandbox",
        alias = "dangerously_disable_sandbox",
        skip_serializing_if = "Option::is_none"
    )]
    /// Stores the dangerously disable sandbox
    pub dangerously_disable_sandbox: Option<bool>,
}

impl BashInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("bash", "command", &self.command)?;
        if let Some(cwd) = &self.cwd {
            require_non_empty_path("bash", "cwd", cwd)?;
        }
        if self.timeout_secs == Some(0) {
            return Err(WonderError::validation(
                "bash timeout_secs must be greater than zero",
            ));
        }
        if self.timeout == Some(0) {
            return Err(WonderError::validation(
                "bash timeout must be greater than zero",
            ));
        }
        if self.timeout_secs.is_some() && self.timeout.is_some() {
            return Err(WonderError::validation(
                "bash accepts either `timeout_secs` or source-compatible `timeout`, not both",
            ));
        }
        // run_in_background is handled in execute(); dangerouslyDisableSandbox is
        // handled in permission_decision() so callers get a structured Deny reason.
        Ok(())
    }

    fn timeout(&self) -> u64 {
        self.timeout_secs
            .or_else(|| self.timeout.map(|timeout| timeout.div_ceil(1_000)))
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
    }
}
/// Represents bash tool
#[derive(Debug, Default)]
pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec("bash", "Run a local shell command", ToolKind::Shell)
            .with_input_schema(
                ToolSchema::object()
                    .property("command", ToolSchema::string("shell command to execute"))
                    .property(
                        "cwd",
                        ToolSchema::string(
                            "optional working directory relative to the session cwd",
                        ),
                    )
                    .property(
                        "timeout_secs",
                        ToolSchema::integer("optional command timeout in seconds"),
                    )
                    .property(
                        "timeout",
                        ToolSchema::integer(
                            "source-compatible timeout in milliseconds; converted to seconds",
                        ),
                    )
                    .property(
                        "description",
                        ToolSchema::string(
                            "optional source-compatible command description; ignored by the Rust runtime",
                        ),
                    )
                    .property(
                        "run_in_background",
                        ToolSchema::boolean(
                            "when true the command is spawned as a detached background task and \
                             a task_id is returned; use task_output / task_stop to manage it",
                        ),
                    )
                    .property(
                        "dangerouslyDisableSandbox",
                        ToolSchema::boolean(
                            "source-compatible sandbox override flag; denied by the Rust \
                             permission model — remove this flag and run the command normally",
                        ),
                    )
                    .required("command"),
            );
        spec.destructive = true;
        spec
    }

    /// Denies `dangerouslyDisableSandbox` in the permission layer so callers
    /// receive a structured `Deny` decision rather than a plain validation error.
    fn permission_decision(&self, context: &ToolContext, input: &Value) -> PermissionDecision {
        let sandbox_override = input
            .get("dangerouslyDisableSandbox")
            .or_else(|| input.get("dangerously_disable_sandbox"))
            .and_then(Value::as_bool)
            == Some(true);

        if sandbox_override {
            return PermissionDecision::deny(PermissionDecisionReason::ShellSafety {
                issue: ShellSafetyIssue {
                    verdict: ShellSafetyVerdict::Blocked,
                    message: "dangerouslyDisableSandbox is not permitted: the Rust runtime \
                              enforces sandboxing unconditionally; remove the flag and run \
                              the command normally"
                        .into(),
                },
            });
        }

        // Fall through to the standard evaluation (shell safety, path scope, rules).
        let spec = self.spec();
        let mut request = PermissionRequest::new(spec.name)
            .with_aliases(spec.aliases)
            .read_only(spec.read_only)
            .destructive(spec.destructive);
        if let Some(cmd) = input.get("command").and_then(Value::as_str) {
            request = request.with_shell_command(cmd.to_string());
        }
        evaluate_permission(&context.permission_context(), &request)
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<BashInput>("bash", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<BashInput>("bash", &input)?;
        input.validate()?;

        let cwd = input
            .cwd
            .as_deref()
            .map(|path| resolve_path(path, &context.cwd))
            .unwrap_or_else(|| context.cwd.clone());
        let metadata = fs::metadata(&cwd).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => {
                WonderError::not_found("directory", cwd.display().to_string())
            }
            _ => error.into(),
        })?;
        if !metadata.is_dir() {
            return Err(WonderError::validation(format!(
                "bash cwd is not a directory: {}",
                cwd.display()
            )));
        }

        let timeout_secs = input.timeout();
        let timeout = Duration::from_secs(timeout_secs);

        // --- Background task path ---
        if input.run_in_background == Some(true) {
            return run_in_background(use_id, &input.command, &cwd, &context);
        }

        // --- Persistent session path ---
        if let Some(store_lock) = &context.bash_session_store {
            let session_id = context.session_id;
            // Acquire the store mutex, run the command, then release.
            let result = {
                let mut store = store_lock
                    .lock()
                    .map_err(|_| WonderError::internal("bash session store mutex poisoned"))?;
                let session = store.get_or_create(session_id, &cwd)?;
                session.run(&input.command, timeout)
            };

            match result {
                Ok(out) => {
                    let content = render_output_persistent(&out.stdout, out.exit_code);
                    let success = out.exit_code == 0;
                    let mut tool_result = if success {
                        ToolResult::success(use_id, content)
                    } else {
                        ToolResult::failure(use_id, content)
                    };
                    tool_result.metadata = json!({
                        "cwd": cwd.display().to_string(),
                        "exit_code": out.exit_code,
                        "timed_out": false,
                        "persistent_session": true,
                    });
                    return Ok(tool_result);
                }
                Err(e) => {
                    // On error (e.g. timeout, EOF), fall through to the
                    // one-shot path so the command still has a chance to run.
                    // Also remove the dead session.
                    if let Ok(mut store) = store_lock.lock() {
                        store.remove(&session_id);
                    }
                    // If it was a timeout error, report it directly.
                    let msg = e.to_string();
                    if msg.contains("timed out") {
                        let content = format!("command timed out after {timeout_secs}s");
                        let mut tool_result = ToolResult::failure(use_id, content);
                        tool_result.metadata = json!({
                            "cwd": cwd.display().to_string(),
                            "exit_code": null,
                            "timed_out": true,
                            "persistent_session": true,
                        });
                        return Ok(tool_result);
                    }
                    // For other errors (e.g. session EOF), fall through to one-shot.
                }
            }
        }

        // --- One-shot fallback path (original implementation) ---
        let mut child = shell_command(&input.command);
        child
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = child.spawn()?;
        let timed_out = child.wait_timeout(timeout)?.is_none();
        if timed_out {
            match child.kill() {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {}
                Err(error) => return Err(error.into()),
            }
        }

        let output = child.wait_with_output()?;
        let exit_code = output.status.code();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let mut content = render_output(&stdout, &stderr, exit_code);
        if timed_out {
            let timeout_message = format!(
                "command timed out after {timeout_secs}s in {}",
                display_path(&cwd, &context.cwd)
            );
            content = if content.is_empty() {
                timeout_message
            } else {
                format!("{timeout_message}\n\n{content}")
            };
        }

        let mut result = if !timed_out && output.status.success() {
            ToolResult::success(use_id, content)
        } else {
            ToolResult::failure(use_id, content)
        };
        result.metadata = json!({
            "cwd": cwd.display().to_string(),
            "exit_code": exit_code,
            "timed_out": timed_out,
        });
        Ok(result)
    }
}

/// Spawns `command` as a detached background task, records it in [`TaskStore`],
/// and returns a structured [`ToolResult`] with `background_task_id` in metadata.
///
/// The task's combined stdout+stderr is appended to the store log file so
/// `task_output` can read it.  The caller must have validated `cwd` exists.
fn run_in_background(
    use_id: ToolUseId,
    command: &str,
    cwd: &PathBuf,
    context: &ToolContext,
) -> Result<ToolResult> {
    // Respect the feature gate — mirrors upstream `isBackgroundTasksDisabled`.
    if !context.features.contains(FeatureFlag::BackgroundTasks) {
        let mut result = ToolResult::failure(
            use_id,
            "bash run_in_background requires the BackgroundTasks feature to be enabled",
        );
        result.metadata = json!({
            "supported": false,
            "unsupported_field": "run_in_background",
            "reason": "BackgroundTasks feature is disabled",
        });
        return Ok(result);
    }

    let app_root = crate::app_root()?;
    let store = TaskStore::new(&app_root);

    // Build the task record before spawning so we have the TaskId.
    let description = format!("bg: {}", command.chars().take(60).collect::<String>());
    let task = TaskState::pending_shell(&description, command, cwd);
    let task_id = task.id;

    store.ensure_layout()?;
    let log_path = store.paths().task_log_path(task_id);
    let mut task_with_log = task;
    task_with_log.output_log = Some(log_path.clone());
    store.write_task(&task_with_log)?;

    // Open log file; combined stdout+stderr for simple tail-reading.
    let log_file = fs::File::create(&log_path)?;
    let log_stderr = log_file.try_clone()?;

    let mut child = shell_command(command);
    child
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_stderr));
    let child = child.spawn()?;
    let pid = child.id();

    // Update task status to Running with the real PID.
    // We read back the stored task so any fields we didn't set are preserved.
    let mut running = store.read_task(task_id)?;
    running.mark_running(Some(pid), None, None, Some(description));
    store.write_task(&running)?;

    // Spawn a watchdog thread that waits on the child and reaps the OS process
    // entry once it exits.  Without this, the exited child becomes a zombie
    // until the parent process itself exits (because nobody called waitpid).
    // The thread also persists the final exit code and task status to TaskStore
    // so `task_output` / `task_stop` can observe the finished state.
    let watchdog_app_root = app_root.clone();
    std::thread::spawn(move || {
        // `child` is declared mut here so we can call the &mut self method wait().
        let mut child = child;
        let exit_status = child.wait();
        let store = TaskStore::new(&watchdog_app_root);
        if let Ok(mut task) = store.read_task(task_id) {
            let (task_status, code) = match exit_status {
                Ok(ref status) => {
                    let code = status.code();
                    let ts = if status.success() {
                        TaskStatus::Completed
                    } else {
                        TaskStatus::Failed
                    };
                    (ts, code)
                }
                Err(_) => (TaskStatus::Failed, None),
            };
            task.mark_finished(task_status, code, None);
            let _ = store.write_task(&task);
        }
    });

    let task_id_str = task_id.to_string();
    let content = format!(
        "command running in background with ID: {task_id_str}\n\
         Use task_output with task_id=\"{task_id_str}\" to read output, \
         or task_stop to cancel."
    );
    let mut result = ToolResult::success(use_id, content);
    result.metadata = json!({
        "background_task_id": task_id_str,
        "pid": pid,
        "cwd": cwd.display().to_string(),
        "run_in_background": true,
    });
    Ok(result)
}

#[cfg(not(windows))]
fn shell_command(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-lc").arg(command);
    cmd
}

#[cfg(windows)]
fn shell_command(command: &str) -> Command {
    let mut cmd = Command::new("cmd");
    cmd.arg("/C").arg(command);
    cmd
}

fn render_output(stdout: &str, stderr: &str, exit_code: Option<i32>) -> String {
    let mut sections = Vec::new();
    if !stdout.is_empty() {
        sections.push(format!("stdout:\n{}", stdout.trim_end_matches('\n')));
    }
    if !stderr.is_empty() {
        sections.push(format!("stderr:\n{}", stderr.trim_end_matches('\n')));
    }
    sections.push(match exit_code {
        Some(code) => format!("exit_code: {code}"),
        None => "exit_code: terminated by signal".into(),
    });
    sections.join("\n\n")
}

/// Render output for the persistent-session path (no separate stderr field).
fn render_output_persistent(stdout: &str, exit_code: i32) -> String {
    let mut sections = Vec::new();
    if !stdout.is_empty() {
        sections.push(format!("stdout:\n{}", stdout.trim_end_matches('\n')));
    }
    sections.push(format!("exit_code: {exit_code}"));
    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    use futures::executor::block_on;
    use serde_json::json;
    use wonder_of_u_core::{
        FeatureSet, PermissionDecision, PermissionMode, SessionId, ShellSessionStore, ToolContext,
        ToolUseId,
    };
    use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

    use super::*;

    fn storage_env_guard(path: &Path) -> EnvVarGuard {
        EnvVarGuard::set("WONDER_OF_U_STORAGE_DIR", path.as_os_str())
    }

    fn tool_context(cwd: PathBuf) -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd,
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            provider: None,
            model: None,
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: None,
            fork_context: None,
        }
    }

    fn tool_context_with_session(cwd: PathBuf) -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd,
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            provider: None,
            model: None,
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: Some(Arc::new(Mutex::new(ShellSessionStore::new()))),
            fork_context: None,
        }
    }

    #[test]
    fn bash_validation_rejects_empty_command() {
        let tool = BashTool;
        let error = tool
            .validate_input(&json!({ "command": "   " }))
            .expect_err("empty command");

        assert!(error.to_string().contains("non-empty `command`"));
    }

    #[test]
    fn bash_permission_blocks_obfuscated_commands() {
        let tool = BashTool;
        let context = tool_context(PathBuf::from("/workspace"));
        let decision = tool.permission_decision(&context, &json!({ "command": "echo ${cmd@P}" }));

        assert!(matches!(decision, PermissionDecision::Deny { .. }));
    }

    #[test]
    fn bash_validation_accepts_run_in_background_flag() {
        // run_in_background is no longer rejected at validation time; it is
        // handled in execute() via the background task path.
        let tool = BashTool;
        tool.validate_input(&json!({
            "command": "echo hi",
            "run_in_background": true,
        }))
        .expect("run_in_background should pass validation");
    }

    #[test]
    fn bash_permission_denies_dangerously_disable_sandbox() {
        // dangerouslyDisableSandbox is denied in the permission layer, not
        // validation, so callers receive a structured Deny with a clear reason.
        let tool = BashTool;
        let context = tool_context(PathBuf::from("/workspace"));

        let decision = tool.permission_decision(
            &context,
            &json!({ "command": "echo hi", "dangerouslyDisableSandbox": true }),
        );
        assert!(
            matches!(decision, PermissionDecision::Deny { .. }),
            "expected Deny, got {decision:?}"
        );
        assert!(
            decision
                .reason()
                .to_string()
                .contains("dangerouslyDisableSandbox"),
            "reason should mention the flag: {}",
            decision.reason()
        );
    }

    #[test]
    fn bash_permission_denies_dangerously_disable_sandbox_snake_case_alias() {
        let tool = BashTool;
        let context = tool_context(PathBuf::from("/workspace"));

        let decision = tool.permission_decision(
            &context,
            &json!({ "command": "echo hi", "dangerously_disable_sandbox": true }),
        );
        assert!(matches!(decision, PermissionDecision::Deny { .. }));
    }

    #[test]
    fn bash_permission_allows_normal_command_without_sandbox_flag() {
        let tool = BashTool;
        let context = tool_context(PathBuf::from("/workspace"));

        let decision = tool.permission_decision(&context, &json!({ "command": "printf 'hello'" }));
        // Should not be unconditionally denied just because it's a shell command.
        assert!(
            !matches!(decision, PermissionDecision::Deny { .. }),
            "plain command should not be denied: {decision:?}"
        );
    }

    #[test]
    fn bash_run_in_background_spawns_task_and_returns_id() {
        let storage_dir = unique_test_dir("tools-bash-background-storage");

        let dir = unique_test_dir("tools-bash-background");
        let tool = BashTool;
        let result = {
            let _storage_env = storage_env_guard(&storage_dir);
            block_on(tool.execute(
                tool_context(dir),
                ToolUseId::new(),
                json!({ "command": "echo background_ok", "run_in_background": true }),
            ))
            .expect("run background bash tool")
        };

        assert!(
            result.success,
            "expected success, got: {:?}",
            result.content
        );
        assert_eq!(result.metadata["run_in_background"], json!(true));
        assert!(
            result.metadata["background_task_id"].as_str().is_some(),
            "metadata should contain background_task_id"
        );
        assert!(
            result.metadata["pid"].as_u64().is_some(),
            "metadata should contain pid"
        );
    }

    #[test]
    fn bash_validation_accepts_source_timeout_metadata() {
        let tool = BashTool;

        tool.validate_input(&json!({
            "command": "echo hi",
            "description": "Print a greeting",
            "timeout": 1_500,
        }))
        .expect("source-compatible bash input");
    }

    #[test]
    fn bash_execute_captures_output_and_exit_code() {
        let dir = unique_test_dir("tools-bash-success");
        let tool = BashTool;
        let result = block_on(tool.execute(
            tool_context(dir),
            ToolUseId::new(),
            json!({ "command": "printf 'hello world'" }),
        ))
        .expect("run bash tool");

        assert!(result.success);
        assert!(result.content.contains("hello world"));
        assert_eq!(result.metadata["exit_code"], json!(0));
    }

    #[test]
    fn bash_execute_reports_failures() {
        let dir = unique_test_dir("tools-bash-failure");
        let tool = BashTool;
        let result = block_on(tool.execute(
            tool_context(dir),
            ToolUseId::new(),
            json!({ "command": "printf 'oops' >&2; exit 7" }),
        ))
        .expect("run bash tool");

        assert!(!result.success);
        assert!(result.content.contains("stderr:"));
        assert_eq!(result.metadata["exit_code"], json!(7));
    }

    #[test]
    fn bash_persistent_session_captures_output() {
        let dir = unique_test_dir("tools-bash-persistent");
        let tool = BashTool;
        let result = block_on(tool.execute(
            tool_context_with_session(dir),
            ToolUseId::new(),
            json!({ "command": "printf 'hello persistent'" }),
        ))
        .expect("run bash tool with persistent session");

        assert!(result.success);
        assert!(result.content.contains("hello persistent"));
        assert_eq!(result.metadata["exit_code"], json!(0));
        assert_eq!(result.metadata["persistent_session"], json!(true));
    }

    #[test]
    fn bash_persistent_session_preserves_env() {
        let dir = unique_test_dir("tools-bash-persistent-env");
        let store = Arc::new(Mutex::new(ShellSessionStore::new()));
        let session_id = SessionId::new();
        let tool = BashTool;

        // Set a variable in the first call.
        let ctx1 = ToolContext {
            session_id,
            cwd: dir.clone(),
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            provider: None,
            model: None,
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: Some(Arc::clone(&store)),
            fork_context: None,
        };
        block_on(tool.execute(
            ctx1,
            ToolUseId::new(),
            json!({ "command": "export WONDER_PERSIST_TEST=hello_persist" }),
        ))
        .expect("set var");

        // Read it back in the second call using the same session_id + store.
        let ctx2 = ToolContext {
            session_id,
            cwd: dir,
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            provider: None,
            model: None,
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: Some(Arc::clone(&store)),
            fork_context: None,
        };
        let result = block_on(tool.execute(
            ctx2,
            ToolUseId::new(),
            json!({ "command": "printf '%s' \"$WONDER_PERSIST_TEST\"" }),
        ))
        .expect("read var");

        assert!(
            result.content.contains("hello_persist"),
            "env not preserved: {:?}",
            result.content
        );
    }

    /// Verifies that the watchdog thread reaps a fast-exiting background
    /// process and transitions the task status to `Completed` with exit code 0.
    #[test]
    fn bash_run_in_background_watchdog_marks_task_completed() {
        let storage_dir = unique_test_dir("tools-bash-watchdog-storage");

        let dir = unique_test_dir("tools-bash-watchdog");
        let tool = BashTool;
        let result = {
            let _storage_env = storage_env_guard(&storage_dir);
            block_on(tool.execute(
                tool_context(dir),
                ToolUseId::new(),
                json!({ "command": "exit 0", "run_in_background": true }),
            ))
            .expect("run background bash tool")
        };

        assert!(result.success, "expected success: {:?}", result.content);
        let task_id_str = result.metadata["background_task_id"]
            .as_str()
            .expect("background_task_id must be present");

        // Parse the TaskId and poll the store until the watchdog updates the
        // state or we time out.  The spawned process exits nearly immediately
        // so a 2-second cap is more than enough on any CI machine.
        let task_id: wonder_of_u_core::TaskId = task_id_str.parse().expect("valid task id");
        let store = wonder_of_u_storage::TaskStore::new(&storage_dir);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let task = store.read_task(task_id).expect("task must exist in store");
            if matches!(task.status, wonder_of_u_core::TaskStatus::Completed) {
                assert_eq!(task.exit_code, Some(0), "exit_code must be 0");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "watchdog did not mark task Completed within 2 s; status={:?}",
                task.status
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// Verifies that a background command that exits with a non-zero code is
    /// marked `Failed` by the watchdog, not left in `Running`.
    #[test]
    fn bash_run_in_background_watchdog_marks_task_failed_on_nonzero_exit() {
        let storage_dir = unique_test_dir("tools-bash-watchdog-fail-storage");

        let dir = unique_test_dir("tools-bash-watchdog-fail");
        let tool = BashTool;
        let result = {
            let _storage_env = storage_env_guard(&storage_dir);
            block_on(tool.execute(
                tool_context(dir),
                ToolUseId::new(),
                json!({ "command": "exit 42", "run_in_background": true }),
            ))
            .expect("run background bash tool")
        };

        assert!(
            result.success,
            "spawn itself must succeed: {:?}",
            result.content
        );
        let task_id_str = result.metadata["background_task_id"]
            .as_str()
            .expect("background_task_id must be present");

        let task_id: wonder_of_u_core::TaskId = task_id_str.parse().expect("valid task id");
        let store = wonder_of_u_storage::TaskStore::new(&storage_dir);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let task = store.read_task(task_id).expect("task must exist in store");
            if matches!(task.status, wonder_of_u_core::TaskStatus::Failed) {
                assert_eq!(task.exit_code, Some(42), "exit_code must be 42");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "watchdog did not mark task Failed within 2 s; status={:?}",
                task.status
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}
