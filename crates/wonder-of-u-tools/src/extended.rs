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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerShellInput {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

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
                    .required("command"),
            );
        spec.destructive = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let input = parse_input::<PowerShellInput>("powershell", input)?;
        require_non_empty_text("powershell", "command", &input.command)?;
        if input.timeout_secs == Some(0) {
            return Err(WonderError::validation(
                "powershell timeout_secs must be greater than zero",
            ));
        }
        Ok(())
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<PowerShellInput>("powershell", &input)?;
        self.validate_input(&serde_json::to_value(&input)?)?;
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
        let timeout = Duration::from_secs(input.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS));
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotebookEditInput {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

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
        spec.read_only = true;
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
