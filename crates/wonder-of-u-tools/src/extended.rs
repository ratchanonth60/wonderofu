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
    PermissionDecision, PermissionDecisionReason, PermissionRequest, Result, ShellSafetyIssue,
    ShellSafetyVerdict, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId,
    WonderError, evaluate_permission, resolve_path,
};

use crate::{base_spec, display_path, parse_input, require_non_empty_path, require_non_empty_text};

const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// Represents power shell input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerShellInput {
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
/// Represents power shell tool
#[derive(Debug, Default)]
pub struct PowerShellTool;

#[async_trait]
impl Tool for PowerShellTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec("powershell", "Run a PowerShell command", ToolKind::Shell)
            .with_input_schema(
                ToolSchema::object()
                    .property(
                        "command",
                        ToolSchema::string("PowerShell command to execute"),
                    )
                    .property("cwd", ToolSchema::string("optional working directory"))
                    .property(
                        "timeout_secs",
                        ToolSchema::integer("optional timeout in seconds"),
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
                            "source-compatible background flag; not supported for PowerShell — \
                             a structured unsupported result is returned when this flag is set",
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

    fn validate_input(&self, input: &Value) -> Result<()> {
        let input = parse_input::<PowerShellInput>("powershell", input)?;
        require_non_empty_text("powershell", "command", &input.command)?;
        if let Some(cwd) = &input.cwd {
            require_non_empty_path("powershell", "cwd", cwd)?;
        }
        if input.timeout_secs == Some(0) {
            return Err(WonderError::validation(
                "powershell timeout_secs must be greater than zero",
            ));
        }
        if input.timeout == Some(0) {
            return Err(WonderError::validation(
                "powershell timeout must be greater than zero",
            ));
        }
        if input.timeout_secs.is_some() && input.timeout.is_some() {
            return Err(WonderError::validation(
                "powershell accepts either `timeout_secs` or source-compatible `timeout`, not both",
            ));
        }
        Ok(())
    }

    /// Denies `dangerouslyDisableSandbox` in the permission layer, consistent
    /// with `BashTool`, so callers receive a structured `Deny` decision rather
    /// than a plain validation error.  Falls through to the standard shell
    /// safety evaluation for all other inputs.
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

        // Fall through to standard shell-safety and permission-rule evaluation.
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

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<PowerShellInput>("powershell", &input)?;
        self.validate_input(&serde_json::to_value(&input)?)?;

        // run_in_background is not implemented for PowerShell: background
        // process management (task store, watchdog, log files) is only wired up
        // in BashTool.  Return a structured failure so callers can distinguish
        // this unsupported-feature response from a genuine input error.
        if input.run_in_background == Some(true) {
            let mut result = ToolResult::failure(
                use_id,
                "powershell run_in_background is not supported in wonder-of-u-tools: \
                 background process management is only available for bash; \
                 use the bash tool with run_in_background instead",
            );
            result.metadata = json!({
                "supported": false,
                "unsupported_field": "run_in_background",
                "reason": "background process management is not implemented for PowerShell; use bash",
            });
            return Ok(result);
        }

        let cwd = input
            .cwd
            .as_deref()
            .map(|path| resolve_path(path, &context.cwd))
            .unwrap_or_else(|| context.cwd.clone());
        let shell = if cfg!(windows) { "powershell" } else { "pwsh" };
        let mut child = Command::new(shell)
            .arg("-NoProfile")
            .arg("-Command")
            .arg(&input.command)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                WonderError::validation(format!("failed to start {shell}: {error}"))
            })?;
        let timeout = Duration::from_secs(
            input
                .timeout_secs
                .or_else(|| input.timeout.map(|timeout| timeout.div_ceil(1_000)))
                .unwrap_or(DEFAULT_TIMEOUT_SECS),
        );
        let timed_out = child.wait_timeout(timeout)?.is_none();
        if timed_out {
            let _ = child.kill();
        }
        let output = child.wait_with_output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let content = format!(
            "stdout:\n{}\nstderr:\n{}\nexit_code: {}",
            stdout.trim_end(),
            stderr.trim_end(),
            output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".into())
        );
        let mut result = if !timed_out && output.status.success() {
            ToolResult::success(use_id, content)
        } else {
            ToolResult::failure(use_id, content)
        };
        result.metadata = json!({
            "cwd": display_path(&cwd, &context.cwd),
            "timed_out": timed_out,
            "shell": shell,
        });
        Ok(result)
    }
}
/// Represents notebook edit input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotebookEditInput {
    /// Stores the path
    pub path: PathBuf,
    /// Stores the cell index
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_index: Option<usize>,
    /// Stores the source
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}
/// Represents notebook edit tool
#[derive(Debug, Default)]
pub struct NotebookEditTool;

#[async_trait]
impl Tool for NotebookEditTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "notebook_edit",
            "Inspect or replace a Jupyter notebook cell",
            ToolKind::FileWrite,
        )
        .with_input_schema(
            ToolSchema::object()
                .property("path", ToolSchema::string("notebook .ipynb path"))
                .property(
                    "cell_index",
                    ToolSchema::integer("zero-based cell index to edit"),
                )
                .property(
                    "source",
                    ToolSchema::string("replacement cell source; omit to inspect"),
                )
                .required("path"),
        );
        spec.destructive = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let input = parse_input::<NotebookEditInput>("notebook_edit", input)?;
        require_non_empty_path("notebook_edit", "path", &input.path)
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<NotebookEditInput>("notebook_edit", &input)?;
        self.validate_input(&serde_json::to_value(&input)?)?;
        let path = resolve_path(&input.path, &context.cwd);
        let mut notebook: Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
        let cells = notebook
            .get_mut("cells")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| WonderError::validation("notebook is missing a cells array"))?;
        if let Some(index) = input.cell_index {
            let cell = cells.get_mut(index).ok_or_else(|| {
                WonderError::validation(format!("notebook cell {index} not found"))
            })?;
            if let Some(source) = input.source {
                cell["source"] = Value::Array(
                    source
                        .lines()
                        .map(|line| json!(format!("{line}\n")))
                        .collect(),
                );
                fs::write(&path, serde_json::to_string_pretty(&notebook)?)?;
                return Ok(ToolResult::success(
                    use_id,
                    format!(
                        "updated cell {index} in {}",
                        display_path(&path, &context.cwd)
                    ),
                ));
            }
        }
        Ok(ToolResult::success(
            use_id,
            format!(
                "notebook {} has {} cells",
                display_path(&path, &context.cwd),
                cells.len()
            ),
        ))
    }
}
/// Represents worktree list tool
#[derive(Debug, Default)]
pub struct WorktreeListTool;

#[async_trait]
impl Tool for WorktreeListTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "worktree_list",
            "List git worktrees for the current repository",
            ToolKind::Search,
        );
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        _input: Value,
    ) -> Result<ToolResult> {
        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&context.cwd)
            .output()?;
        let content = String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string();
        Ok(if output.status.success() {
            ToolResult::success(use_id, content)
        } else {
            ToolResult::failure(use_id, String::from_utf8_lossy(&output.stderr).to_string())
        })
    }
}
/// Represents terminal capture tool
#[derive(Debug, Default)]
pub struct TerminalCaptureTool;

#[async_trait]
impl Tool for TerminalCaptureTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "terminal_capture",
            "Capture terminal environment metadata",
            ToolKind::Search,
        );
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        _input: Value,
    ) -> Result<ToolResult> {
        let content = json!({
            "term": std::env::var("TERM").ok(),
            "cols": std::env::var("COLUMNS").ok(),
            "rows": std::env::var("LINES").ok(),
            "tty": std::env::var("TTY").ok(),
        });
        Ok(ToolResult::success(use_id, content.to_string()))
    }
}
/// Represents cron list tool
#[derive(Debug, Default)]
pub struct CronListTool;

#[async_trait]
impl Tool for CronListTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "cron_list",
            "List current user cron entries",
            ToolKind::Task,
        );
        spec.aliases.push("CronList".into());
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        _input: Value,
    ) -> Result<ToolResult> {
        let output = Command::new("crontab").arg("-l").output();
        match output {
            Ok(output) if output.status.success() => Ok(ToolResult::success(
                use_id,
                String::from_utf8_lossy(&output.stdout).to_string(),
            )),
            Ok(output) => Ok(ToolResult::failure(
                use_id,
                String::from_utf8_lossy(&output.stderr).to_string(),
            )),
            Err(error) => Ok(ToolResult::failure(
                use_id,
                format!("failed to run crontab -l: {error}"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;
    use wonder_of_u_core::{
        FeatureSet, PermissionDecision, PermissionMode, SessionId, ToolContext,
    };

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
        }
    }

    #[test]
    fn powershell_validation_rejects_empty_cwd() {
        let tool = PowerShellTool;
        let error = tool
            .validate_input(&json!({ "command": "Get-ChildItem", "cwd": "" }))
            .expect_err("empty cwd");

        assert!(error.to_string().contains("non-empty `cwd`"));
    }

    #[test]
    fn powershell_permission_requires_review_for_encoded_commands() {
        let tool = PowerShellTool;
        let context = tool_context(PathBuf::from("/workspace"));
        let decision = tool.permission_decision(
            &context,
            &json!({ "command": "pwsh –EncodedCommand ZQBjAGgAbwA=" }),
        );

        assert!(matches!(decision, PermissionDecision::Ask { .. }));
        assert!(decision.reason().to_string().contains("encoded"));
    }

    #[test]
    fn powershell_permission_requires_review_for_invoke_expression() {
        let tool = PowerShellTool;
        let context = tool_context(PathBuf::from("/workspace"));
        let decision = tool.permission_decision(
            &context,
            &json!({ "command": "Invoke-Expression $payload" }),
        );

        assert!(matches!(decision, PermissionDecision::Ask { .. }));
        assert!(decision.reason().to_string().contains("PowerShell"));
    }

    // ── dangerouslyDisableSandbox — permission_decision Deny ─────────────────

    #[test]
    fn powershell_permission_denies_dangerously_disable_sandbox_camel_case() {
        // dangerouslyDisableSandbox must be denied in permission_decision() with
        // a structured ShellSafety reason, not a plain validation error.
        let tool = PowerShellTool;
        let context = tool_context(PathBuf::from("/workspace"));
        let decision = tool.permission_decision(
            &context,
            &json!({ "command": "Get-ChildItem", "dangerouslyDisableSandbox": true }),
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
    fn powershell_permission_denies_dangerously_disable_sandbox_snake_case_alias() {
        let tool = PowerShellTool;
        let context = tool_context(PathBuf::from("/workspace"));
        let decision = tool.permission_decision(
            &context,
            &json!({ "command": "Get-ChildItem", "dangerously_disable_sandbox": true }),
        );
        assert!(matches!(decision, PermissionDecision::Deny { .. }));
    }

    #[test]
    fn powershell_permission_allows_normal_command_without_sandbox_flag() {
        let tool = PowerShellTool;
        let context = tool_context(PathBuf::from("/workspace"));
        let decision = tool.permission_decision(&context, &json!({ "command": "Get-ChildItem" }));
        assert!(
            !matches!(decision, PermissionDecision::Deny { .. }),
            "plain command should not be denied: {decision:?}"
        );
    }

    #[test]
    fn powershell_validation_no_longer_rejects_dangerously_disable_sandbox() {
        // validate_input() no longer rejects dangerouslyDisableSandbox: the
        // check was moved to permission_decision() so callers receive a
        // structured Deny with metadata rather than a hard validation error.
        let tool = PowerShellTool;
        tool.validate_input(&json!({
            "command": "Get-ChildItem",
            "dangerouslyDisableSandbox": true,
        }))
        .expect("validate_input should not reject dangerouslyDisableSandbox");
    }

    // ── run_in_background — structured ToolResult failure ────────────────────

    #[test]
    fn powershell_run_in_background_returns_structured_failure() {
        // run_in_background is unsupported for PowerShell; execute() must return
        // a structured ToolResult failure (not Err) so callers can distinguish
        // unsupported-feature responses from genuine input mistakes.
        use futures::executor::block_on;
        use wonder_of_u_test_support::unique_test_dir;

        let tool = PowerShellTool;
        let dir = unique_test_dir("tools-ps-run-in-bg");
        let result = block_on(tool.execute(
            tool_context(dir),
            wonder_of_u_core::ToolUseId::new(),
            json!({ "command": "Get-ChildItem", "run_in_background": true }),
        ))
        .expect("execute should not return Err");

        assert!(!result.success, "expected failure result");
        assert!(
            result.content.contains("run_in_background"),
            "content should mention `run_in_background`: {:?}",
            result.content
        );
        assert_eq!(result.metadata["supported"], serde_json::json!(false));
        assert_eq!(
            result.metadata["unsupported_field"],
            serde_json::json!("run_in_background")
        );
    }

    #[test]
    fn powershell_validation_no_longer_rejects_run_in_background() {
        // validate_input() must accept run_in_background; the structured failure
        // is now deferred to execute() so callers receive a proper ToolResult.
        let tool = PowerShellTool;
        tool.validate_input(&json!({
            "command": "Get-ChildItem",
            "run_in_background": true,
        }))
        .expect("validate_input should not reject run_in_background");
    }
}
