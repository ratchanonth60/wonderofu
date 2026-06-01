//! Cross-platform OS shell tool.
//!
//! [`ShellTool`] auto-detects the system shell at runtime:
//! - **Unix (Linux/macOS)**: uses `$SHELL`, falls back to `/bin/bash`, then `/bin/sh`
//! - **Windows**: uses `powershell.exe -NonInteractive -Command`

use std::{
    env, fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wait_timeout::ChildExt;
use wonder_of_u_core::{
    PermissionRequest, Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec,
    ToolUseId, WonderError, evaluate_permission, resolve_path,
};

use crate::{base_spec, display_path, parse_input, require_non_empty_path, require_non_empty_text};

const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Input for [`ShellTool`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellInput {
    /// Shell command to execute.
    pub command: String,
    /// Optional working directory (relative to session cwd).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// Optional timeout in seconds (default: 30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

impl ShellInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("shell", "command", &self.command)?;
        if let Some(cwd) = &self.cwd {
            require_non_empty_path("shell", "cwd", cwd)?;
        }
        if self.timeout_secs == Some(0) {
            return Err(WonderError::validation(
                "shell timeout_secs must be greater than zero",
            ));
        }
        Ok(())
    }

    fn timeout(&self) -> u64 {
        self.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS)
    }
}

/// Cross-platform shell execution tool.
///
/// Automatically selects the system shell (bash/zsh/sh on Unix,
/// PowerShell on Windows) so the AI can run OS commands without
/// knowing the target platform.
#[derive(Debug, Default)]
pub struct ShellTool;

#[async_trait]
impl Tool for ShellTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "shell",
            "Run a command using the system's native shell.\n\
             Automatically selects the appropriate shell: the user's $SHELL \
             (bash/zsh/fish/etc.) on Linux/macOS, PowerShell on Windows.\n\
             Use for any OS-level task: file operations, process management, \
             package managers, system info, etc.\n\
             For simple file reads prefer `file_read`; for pattern searches prefer `grep`.",
            ToolKind::Shell,
        )
        .with_input_schema(
            ToolSchema::object()
                .property("command", ToolSchema::string("shell command to execute"))
                .property(
                    "cwd",
                    ToolSchema::string("optional working directory relative to the session cwd"),
                )
                .property(
                    "timeout_secs",
                    ToolSchema::integer("optional command timeout in seconds (default: 30)"),
                )
                .required("command"),
        );
        spec.destructive = true;
        spec.aliases.push("RunCommand".into());
        spec
    }

    fn permission_decision(
        &self,
        context: &ToolContext,
        input: &Value,
    ) -> wonder_of_u_core::PermissionDecision {
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
        parse_input::<ShellInput>("shell", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<ShellInput>("shell", &input)?;
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
                "shell cwd is not a directory: {}",
                cwd.display()
            )));
        }

        let timeout_secs = input.timeout();
        let timeout = Duration::from_secs(timeout_secs);

        let mut child = native_shell_command(&input.command);
        child
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = child.spawn()?;

        let stdout_pipe = child.stdout.take().expect("stdout piped");
        let stderr_pipe = child.stderr.take().expect("stderr piped");
        let progress_tx = context.progress_tx.clone();

        let stdout_handle = std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let mut all = String::new();
            for line in BufReader::new(stdout_pipe).lines() {
                let line = line.unwrap_or_default();
                if let Some(ref tx) = progress_tx {
                    let _ = tx.try_send(line.clone());
                }
                all.push_str(&line);
                all.push('\n');
            }
            all
        });
        let stderr_handle = std::thread::spawn(move || {
            use std::io::Read;
            let mut s = String::new();
            let _ = stderr_pipe
                .take(crate::orchestration::MAX_TOOL_RESULT_BYTES as u64)
                .read_to_string(&mut s);
            s
        });

        let exit_status = child.wait_timeout(timeout)?;
        let timed_out = exit_status.is_none();
        if timed_out {
            match child.kill() {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {}
                Err(error) => return Err(error.into()),
            }
            let _ = child.wait();
        }

        let stdout = stdout_handle.join().unwrap_or_default();
        let stderr = stderr_handle.join().unwrap_or_default();
        let exit_code = exit_status.and_then(|s| s.code());
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

        let shell_name = detect_shell_name();
        let mut result = if !timed_out && exit_status.is_some_and(|s| s.success()) {
            ToolResult::success(use_id, content)
        } else {
            ToolResult::failure(use_id, content)
        };
        result.metadata = json!({
            "cwd": cwd.display().to_string(),
            "exit_code": exit_code,
            "timed_out": timed_out,
            "shell": shell_name,
        });
        Ok(result)
    }
}

/// Returns the name of the shell that will be used (for metadata/logging).
fn detect_shell_name() -> String {
    #[cfg(windows)]
    {
        "powershell.exe".into()
    }
    #[cfg(not(windows))]
    {
        env::var("SHELL")
            .ok()
            .and_then(|s| {
                PathBuf::from(&s)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "sh".into())
    }
}

/// Builds a `Command` using the platform-native shell.
///
/// - **Unix**: reads `$SHELL`, falls back to `/bin/bash`, then `/bin/sh`
/// - **Windows**: uses `powershell.exe -NonInteractive -Command`
#[cfg(not(windows))]
fn native_shell_command(command: &str) -> Command {
    let shell = env::var("SHELL").unwrap_or_else(|_| {
        if std::path::Path::new("/bin/bash").exists() {
            "/bin/bash".into()
        } else {
            "/bin/sh".into()
        }
    });
    let mut cmd = Command::new(&shell);
    cmd.arg("-c").arg(command);
    cmd
}

#[cfg(windows)]
fn native_shell_command(command: &str) -> Command {
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NonInteractive", "-Command", command]);
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use futures::executor::block_on;
    use serde_json::json;
    use wonder_of_u_core::{
        FeatureSet, PermissionDecision, PermissionMode, SessionId, ToolContext, ToolUseId,
    };
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

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
            progress_tx: None,
            fork_context: None,
        }
    }

    #[test]
    fn shell_validation_rejects_empty_command() {
        let tool = ShellTool;
        let error = tool
            .validate_input(&json!({ "command": "   " }))
            .expect_err("empty command");
        assert!(error.to_string().contains("non-empty `command`"));
    }

    #[test]
    fn shell_validation_rejects_zero_timeout() {
        let tool = ShellTool;
        let error = tool
            .validate_input(&json!({ "command": "echo hi", "timeout_secs": 0 }))
            .expect_err("zero timeout");
        assert!(error.to_string().contains("timeout_secs"));
    }

    #[test]
    fn shell_permission_blocks_obfuscated_commands() {
        let tool = ShellTool;
        let context = tool_context(PathBuf::from("/workspace"));
        let decision = tool.permission_decision(&context, &json!({ "command": "echo ${cmd@P}" }));
        assert!(matches!(decision, PermissionDecision::Deny { .. }));
    }

    #[test]
    fn shell_permission_allows_normal_command() {
        let tool = ShellTool;
        let context = tool_context(PathBuf::from("/workspace"));
        let decision = tool.permission_decision(&context, &json!({ "command": "printf 'hello'" }));
        assert!(
            !matches!(decision, PermissionDecision::Deny { .. }),
            "plain command should not be denied: {decision:?}"
        );
    }

    #[test]
    fn shell_execute_captures_output_and_exit_code() {
        let dir = unique_test_dir("tools-shell-success");
        let tool = ShellTool;
        let result = block_on(tool.execute(
            tool_context(dir),
            ToolUseId::new(),
            json!({ "command": "printf 'hello shell'" }),
        ))
        .expect("run shell tool");

        assert!(result.success);
        assert!(result.content.contains("hello shell"));
        assert_eq!(result.metadata["exit_code"], json!(0));
        assert!(result.metadata["shell"].as_str().is_some());
    }

    #[test]
    fn shell_execute_reports_failures() {
        let dir = unique_test_dir("tools-shell-failure");
        let tool = ShellTool;
        let result = block_on(tool.execute(
            tool_context(dir),
            ToolUseId::new(),
            json!({ "command": "printf 'oops' >&2; exit 5" }),
        ))
        .expect("run shell tool");

        assert!(!result.success);
        assert!(result.content.contains("stderr:"));
        assert_eq!(result.metadata["exit_code"], json!(5));
    }

    #[test]
    fn shell_metadata_includes_shell_name() {
        let dir = unique_test_dir("tools-shell-meta");
        let tool = ShellTool;
        let result = block_on(tool.execute(
            tool_context(dir),
            ToolUseId::new(),
            json!({ "command": "echo ok" }),
        ))
        .expect("run shell tool");

        let shell = result.metadata["shell"]
            .as_str()
            .expect("shell in metadata");
        assert!(!shell.is_empty());
    }

    #[test]
    fn shell_spec_has_expected_aliases() {
        let spec = ShellTool.spec();
        assert_eq!(spec.name, "shell");
        assert!(spec.aliases.contains(&"RunCommand".to_string()));
    }
}
