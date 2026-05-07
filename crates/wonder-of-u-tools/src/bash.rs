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
    Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId, WonderError,
    resolve_path,
};

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
        if self.run_in_background == Some(true) {
            return Err(WonderError::validation(
                "bash run_in_background is not supported in wonder-of-u-tools",
            ));
        }
        if self.dangerously_disable_sandbox == Some(true) {
            return Err(WonderError::validation(
                "bash dangerouslyDisableSandbox is not supported in wonder-of-u-tools",
            ));
        }
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
                            "source-compatible background flag; currently unsupported",
                        ),
                    )
                    .property(
                        "dangerouslyDisableSandbox",
                        ToolSchema::boolean(
                            "source-compatible sandbox override flag; currently unsupported",
                        ),
                    )
                    .required("command"),
            );
        spec.destructive = true;
        spec
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
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use futures::executor::block_on;
    use serde_json::json;
    use wonder_of_u_core::{
        FeatureSet, PermissionDecision, PermissionMode, SessionId, ShellSessionStore, ToolContext,
        ToolUseId,
    };
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn tool_context(cwd: PathBuf) -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: None,
        }
    }

    fn tool_context_with_session(cwd: PathBuf) -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: Some(Arc::new(Mutex::new(ShellSessionStore::new()))),
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
    fn bash_validation_rejects_unsupported_background_execution() {
        let tool = BashTool;
        let error = tool
            .validate_input(&json!({
                "command": "echo hi",
                "run_in_background": true,
            }))
            .expect_err("unsupported background execution");

        assert!(error.to_string().contains("run_in_background"));
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
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: Some(Arc::clone(&store)),
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
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: Some(Arc::clone(&store)),
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
}
