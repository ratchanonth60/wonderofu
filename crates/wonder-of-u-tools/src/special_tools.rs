//! Source-compatible special-tool shims and gated dev/test tools.

use std::{
    env, fs,
    io::{Error, ErrorKind},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use wonder_of_u_core::{
    FeatureFlag, PermissionDecision, PermissionDecisionReason, PermissionRule,
    PermissionRuleBehavior, PermissionRuleSource, RemoteTaskState, RemoteTaskType, Result, Tool,
    ToolContext, ToolKind, ToolResult, ToolSpec, ToolUseId, WonderError, resolve_path,
};
use wonder_of_u_mcp::{McpConfigStore, McpServerConfig};

use crate::{app_root, base_spec, parse_input, require_non_empty_path, require_non_empty_text};

const WEB_BROWSER_RUNTIME_UNAVAILABLE: &str = "web_browser is unavailable: interactive browser automation is not implemented in wonder-of-u-tools";
const MCP_AUTH_RUNTIME_UNAVAILABLE: &str =
    "mcp_auth: could not open browser for OAuth; check auth_url and open it manually";
const VERIFY_PLAN_RUNTIME_UNAVAILABLE: &str = "verify_plan_execution is unavailable: automated plan-verification hooks are not implemented in wonder-of-u-tools";
const SEND_USER_FILE_RUNTIME_UNAVAILABLE: &str =
    "send_user_file is unavailable: file delivery is not implemented in wonder-of-u-tools";
const SUGGEST_BACKGROUND_PR_RUNTIME_UNAVAILABLE: &str = "suggest_background_pr is unavailable: background PR suggestion workflows are not implemented in wonder-of-u-tools";
const MCP_TOOL_RUNTIME_UNAVAILABLE: &str =
    "mcp is unavailable: dynamic MCP tool invocation is not implemented in wonder-of-u-tools";
const MONITOR_RUNTIME_UNAVAILABLE: &str =
    "monitor is unavailable: runtime monitoring hooks are not implemented in wonder-of-u-tools";
const SUBSCRIBE_PR_RUNTIME_UNAVAILABLE: &str = "subscribe_pr is unavailable: PR webhook subscriptions are not implemented in wonder-of-u-tools";
const TUNGSTEN_RUNTIME_UNAVAILABLE: &str = "tungsten is unavailable: tmux-backed terminal orchestration is not implemented in wonder-of-u-tools";
const DEFAULT_SLEEP_MS: u64 = 1_000;
const MAX_SLEEP_MS: u64 = 300_000;

const REPL_PRIMITIVE_TOOLS: &[&str] = &[
    "file_read",
    "file_write",
    "file_edit",
    "glob",
    "grep",
    "bash",
    "notebook_edit",
    "agent",
];

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub struct ReplInput {
    #[serde(default)]
    pub command: Option<String>,
}

impl ReplInput {
    fn validate(&self) -> Result<()> {
        if let Some(command) = &self.command {
            require_non_empty_text("repl", "command", command)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub struct WebBrowserInput {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
}

impl WebBrowserInput {
    fn validate(&self) -> Result<()> {
        if let Some(url) = &self.url {
            require_non_empty_text("web_browser", "url", url)?;
        }
        if let Some(prompt) = &self.prompt {
            require_non_empty_text("web_browser", "prompt", prompt)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct McpAuthInput {
    pub server: String,
}

impl McpAuthInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("mcp_auth", "server", &self.server)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub struct VerifyPlanExecutionInput {
    #[serde(default)]
    pub prompt: Option<String>,
}

impl VerifyPlanExecutionInput {
    fn validate(&self) -> Result<()> {
        if let Some(prompt) = &self.prompt {
            require_non_empty_text("verify_plan_execution", "prompt", prompt)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct SendUserFileInput {
    pub files: Vec<PathBuf>,
}

impl SendUserFileInput {
    fn validate(&self) -> Result<()> {
        if self.files.is_empty() {
            return Err(WonderError::validation(
                "send_user_file requires at least one entry in `files`",
            ));
        }
        for path in &self.files {
            require_non_empty_path("send_user_file", "files", path)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub struct SuggestBackgroundPrInput {
    #[serde(default)]
    pub prompt: Option<String>,
}

impl SuggestBackgroundPrInput {
    fn validate(&self) -> Result<()> {
        if let Some(prompt) = &self.prompt {
            require_non_empty_text("suggest_background_pr", "prompt", prompt)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub struct SleepInput {
    #[serde(default, alias = "durationMs", alias = "milliseconds")]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub seconds: Option<u64>,
}

impl SleepInput {
    fn duration_ms(&self) -> Result<u64> {
        let millis = match (self.duration_ms, self.seconds) {
            (Some(_), Some(_)) => {
                return Err(WonderError::validation(
                    "sleep accepts either `duration_ms`/`milliseconds` or `seconds`, not both",
                ));
            }
            (Some(duration_ms), None) => duration_ms,
            (None, Some(seconds)) => seconds
                .checked_mul(1_000)
                .ok_or_else(|| WonderError::validation("sleep duration is too large"))?,
            (None, None) => DEFAULT_SLEEP_MS,
        };

        if millis == 0 {
            return Err(WonderError::validation(
                "sleep duration must be greater than zero",
            ));
        }
        if millis > MAX_SLEEP_MS {
            return Err(WonderError::validation(format!(
                "sleep duration must be at most {MAX_SLEEP_MS}ms"
            )));
        }
        Ok(millis)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct McpToolInput {
    #[serde(default, alias = "serverName")]
    pub server: Option<String>,
    #[serde(default, alias = "toolName")]
    pub tool: Option<String>,
    #[serde(default)]
    pub arguments: Value,
}

impl McpToolInput {
    fn validate(&self) -> Result<()> {
        if let Some(server) = &self.server {
            require_non_empty_text("mcp", "server", server)?;
        }
        if let Some(tool) = &self.tool {
            require_non_empty_text("mcp", "tool", tool)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct WorkflowInput {
    #[serde(default)]
    pub workflow: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub args: Value,
}

impl WorkflowInput {
    fn validate(&self) -> Result<()> {
        if let Some(workflow) = &self.workflow {
            require_non_empty_text("workflow", "workflow", workflow)?;
        }
        if let Some(command) = &self.command {
            require_non_empty_text("workflow", "command", command)?;
        }
        if let Some(prompt) = &self.prompt {
            require_non_empty_text("workflow", "prompt", prompt)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub struct PushNotificationInput {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, alias = "body")]
    pub message: Option<String>,
}

impl PushNotificationInput {
    fn validate(&self) -> Result<()> {
        if let Some(title) = &self.title {
            require_non_empty_text("push_notification", "title", title)?;
        }
        if let Some(message) = &self.message {
            require_non_empty_text("push_notification", "message", message)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub struct OverflowTestInput {
    #[serde(default = "default_overflow_chars")]
    pub chars: usize,
}

impl OverflowTestInput {
    fn validate(&self) -> Result<()> {
        if self.chars == 0 {
            return Err(WonderError::validation(
                "overflow_test requires `chars` to be greater than zero",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub struct TungstenInput {
    #[serde(default)]
    pub args: Vec<String>,
}

impl TungstenInput {
    fn validate(&self) -> Result<()> {
        for arg in &self.args {
            require_non_empty_text("tungsten", "args", arg)?;
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct ReplTool;

#[derive(Debug, Default)]
pub struct WebBrowserTool;

#[derive(Debug, Default)]
pub struct McpAuthTool;

#[derive(Debug, Default)]
pub struct StructuredOutputTool;

#[derive(Debug, Default)]
pub struct VerifyPlanExecutionTool;

#[derive(Debug, Default)]
pub struct SendUserFileTool;

#[derive(Debug, Default)]
pub struct SuggestBackgroundPrTool;

#[derive(Debug, Default)]
pub struct SleepTool;

#[derive(Debug, Default)]
pub struct McpTool;

#[derive(Debug, Default)]
pub struct WorkflowTool;

#[derive(Debug, Default)]
pub struct PushNotificationTool;

#[derive(Debug, Default)]
pub struct TestingPermissionTool;

#[derive(Debug, Default)]
pub struct OverflowTestTool;

#[derive(Debug, Default)]
pub struct CtxInspectTool;

#[derive(Debug, Default)]
pub struct MonitorTool;

#[derive(Debug, Default)]
pub struct SubscribePrTool;

#[derive(Debug, Default)]
pub struct SnipTool;

#[derive(Debug, Default)]
pub struct TungstenTool;

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolvedFile {
    path: String,
    size: u64,
    is_image: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct McpAuthStatus {
    server_name: String,
    enabled: bool,
    command_line: String,
    cwd: Option<String>,
    protocol_version: Option<String>,
}

#[async_trait]
impl Tool for ReplTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "repl",
            "Inspect source-compatible REPL mode state or execute a shell command in the current working directory",
            ToolKind::Interaction,
        );
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "optional REPL command to execute with `sh -c` in the current working directory"
                }
            },
            "additionalProperties": true
        });
        spec.read_only = false;
        spec.concurrency_safe = false;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<ReplInput>("repl", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<ReplInput>("repl", &input)?;
        input.validate()?;
        let mode_enabled = repl_mode_enabled();

        if let Some(command) = input.command {
            let output = Command::new("sh")
                .arg("-c")
                .arg(&command)
                .current_dir(&context.cwd)
                .output()?;
            let exit_code = output.status.code().unwrap_or(-1);

            if output.status.success() {
                let output = merge_command_output(&output.stdout, &output.stderr);
                let mut result =
                    ToolResult::success(use_id, render_command_result(exit_code, &output));
                result.metadata = json!({
                    "success": true,
                    "tool": "repl",
                    "command": command,
                    "exit_code": exit_code,
                    "mode_enabled": mode_enabled,
                    "primitive_tools": REPL_PRIMITIVE_TOOLS,
                });
                return Ok(result);
            }

            let stderr = trim_command_output(&output.stderr);
            let mut result = ToolResult::failure(use_id, render_command_result(exit_code, &stderr));
            result.metadata = json!({
                "success": false,
                "tool": "repl",
                "command": command,
                "exit_code": exit_code,
                "mode_enabled": mode_enabled,
                "primitive_tools": REPL_PRIMITIVE_TOOLS,
            });
            return Ok(result);
        }

        let mut result = ToolResult::success(
            use_id,
            format!(
                "mode_enabled={mode_enabled}\nprimitive_tools={}",
                REPL_PRIMITIVE_TOOLS.join(",")
            ),
        );
        result.metadata = json!({
            "success": true,
            "tool": "repl",
            "mode_enabled": mode_enabled,
            "primitive_tools": REPL_PRIMITIVE_TOOLS,
        });
        Ok(result)
    }
}

#[async_trait]
impl Tool for WebBrowserTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "web_browser",
            "Source-compatible WebBrowser alias; interactive browser control is unsupported",
            ToolKind::Web,
        );
        spec.aliases.push("WebBrowser".into());
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "optional URL to open in the browser panel"
                },
                "prompt": {
                    "type": "string",
                    "description": "optional browser instruction text"
                }
            },
            "additionalProperties": true
        });
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec.required_features.insert(FeatureFlag::WebTools);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<WebBrowserInput>("web_browser", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<WebBrowserInput>("web_browser", &input)?;
        input.validate()?;
        unsupported_result(
            use_id,
            WEB_BROWSER_RUNTIME_UNAVAILABLE,
            json!({
                "supported": false,
                "tool": "web_browser",
                "url": input.url,
                "prompt": input.prompt,
            }),
        )
    }
}

#[async_trait]
impl Tool for McpAuthTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "mcp_auth",
            "Inspect MCP server auth prerequisites; OAuth hand-off is unsupported",
            ToolKind::Mcp,
        );
        spec.aliases.push("McpAuth".into());
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "configured MCP server name"
                }
            },
            "required": ["server"],
            "additionalProperties": false
        });
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec.required_features.insert(FeatureFlag::Mcp);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<McpAuthInput>("mcp_auth", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<McpAuthInput>("mcp_auth", &input)?;
        input.validate()?;

        let storage_root = app_root()?;
        let status = read_mcp_auth_status(&storage_root, &input.server)?;
        let auth_url = status
            .enabled
            .then(|| auth_url_from_command_line(&status.command_line))
            .flatten();

        if let Some(url) = auth_url.as_deref()
            && open_url(url).is_ok()
        {
            let mut result = ToolResult::success(use_id, format!("auth_url={url}"));
            result.metadata = json!({
                "success": true,
                "tool": "mcp_auth",
                "server": status.server_name,
                "enabled": status.enabled,
                "command_line": status.command_line,
                "cwd": status.cwd,
                "protocol_version": status.protocol_version,
                "oauth_supported": true,
                "auth_url": url,
                "browser_opened": true,
            });
            return Ok(result);
        }

        let mut result =
            ToolResult::failure(use_id, render_mcp_auth_failure_message(auth_url.as_deref()));
        result.metadata = json!({
            "supported": false,
            "tool": "mcp_auth",
            "server": status.server_name,
            "enabled": status.enabled,
            "command_line": status.command_line,
            "cwd": status.cwd,
            "protocol_version": status.protocol_version,
            "oauth_supported": auth_url.is_some(),
            "auth_url": auth_url,
            "browser_opened": false,
        });
        Ok(result)
    }
}

#[async_trait]
impl Tool for StructuredOutputTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "structured_output",
            "Return structured output exactly as provided",
            ToolKind::Interaction,
        );
        spec.aliases.push("StructuredOutput".into());
        spec.aliases.push("SyntheticOutput".into());
        spec.input_schema = json!({
            "type": "object",
            "description": "structured JSON payload returned to the caller",
            "additionalProperties": true
        });
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        if input.is_object() {
            Ok(())
        } else {
            Err(WonderError::validation(
                "structured_output requires a JSON object input",
            ))
        }
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        self.validate_input(&input)?;
        let mut result = ToolResult::success(use_id, "Structured output provided successfully");
        result.metadata = json!({
            "success": true,
            "tool": "structured_output",
            "structured_output": input,
        });
        Ok(result)
    }
}

#[async_trait]
impl Tool for VerifyPlanExecutionTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "verify_plan_execution",
            "Source-compatible VerifyPlanExecution alias; automated verification is unsupported",
            ToolKind::Planning,
        );
        spec.aliases.push("VerifyPlanExecution".into());
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "optional verification prompt"
                }
            },
            "additionalProperties": true
        });
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<VerifyPlanExecutionInput>("verify_plan_execution", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<VerifyPlanExecutionInput>("verify_plan_execution", &input)?;
        input.validate()?;
        unsupported_result(
            use_id,
            VERIFY_PLAN_RUNTIME_UNAVAILABLE,
            json!({
                "supported": false,
                "tool": "verify_plan_execution",
                "prompt": input.prompt,
            }),
        )
    }
}

#[async_trait]
impl Tool for SendUserFileTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "send_user_file",
            "Validate user-facing file attachments without claiming delivery",
            ToolKind::Interaction,
        );
        spec.aliases.push("SendUserFile".into());
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "files": {
                    "type": "array",
                    "description": "paths to files the user should receive",
                    "items": { "type": "string" }
                }
            },
            "required": ["files"],
            "additionalProperties": true
        });
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<SendUserFileInput>("send_user_file", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<SendUserFileInput>("send_user_file", &input)?;
        input.validate()?;
        let files = resolve_files(&context.cwd, &input.files)?;
        unsupported_result(
            use_id,
            SEND_USER_FILE_RUNTIME_UNAVAILABLE,
            json!({
                "supported": false,
                "tool": "send_user_file",
                "files": files
                    .iter()
                    .map(|file| json!({
                        "path": file.path,
                        "size": file.size,
                        "is_image": file.is_image,
                    }))
                    .collect::<Vec<_>>(),
            }),
        )
    }
}

#[async_trait]
impl Tool for SuggestBackgroundPrTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "suggest_background_pr",
            "Source-compatible SuggestBackgroundPR alias; PR suggestion workflows are unsupported",
            ToolKind::Interaction,
        );
        spec.aliases.push("SuggestBackgroundPR".into());
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "optional PR suggestion context"
                }
            },
            "additionalProperties": true
        });
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<SuggestBackgroundPrInput>("suggest_background_pr", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<SuggestBackgroundPrInput>("suggest_background_pr", &input)?;
        input.validate()?;
        let backend = serde_json::to_value(RemoteTaskState::deferred(
            RemoteTaskType::BackgroundPr,
            None,
        ))?;
        unsupported_result(
            use_id,
            SUGGEST_BACKGROUND_PR_RUNTIME_UNAVAILABLE,
            json!({
                "supported": false,
                "tool": "suggest_background_pr",
                "prompt": input.prompt,
                "task_backend": backend,
            }),
        )
    }
}

#[async_trait]
impl Tool for SleepTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "sleep",
            "Wait for a bounded duration without holding a shell process",
            ToolKind::Interaction,
        );
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "duration_ms": {
                    "type": "integer",
                    "description": "sleep duration in milliseconds; defaults to 1000ms"
                },
                "milliseconds": {
                    "type": "integer",
                    "description": "source-compatible alias for duration_ms"
                },
                "durationMs": {
                    "type": "integer",
                    "description": "camelCase alias for duration_ms"
                },
                "seconds": {
                    "type": "integer",
                    "description": "sleep duration in seconds"
                }
            },
            "additionalProperties": false
        });
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<SleepInput>("sleep", input)?
            .duration_ms()
            .map(|_| ())
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let duration_ms = parse_input::<SleepInput>("sleep", &input)?.duration_ms()?;
        thread::sleep(Duration::from_millis(duration_ms));
        let mut result = ToolResult::success(use_id, format!("Slept for {duration_ms}ms"));
        result.metadata = json!({
            "success": true,
            "tool": "sleep",
            "duration_ms": duration_ms,
        });
        Ok(result)
    }
}

#[async_trait]
impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "mcp",
            "Source-compatible MCP tool template; dynamic server calls are unsupported",
            ToolKind::Mcp,
        );
        spec.aliases.push("MCPTool".into());
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "optional MCP server name"
                },
                "serverName": {
                    "type": "string",
                    "description": "source-compatible server name alias"
                },
                "tool": {
                    "type": "string",
                    "description": "optional MCP tool name"
                },
                "toolName": {
                    "type": "string",
                    "description": "source-compatible tool name alias"
                },
                "arguments": {
                    "type": "object",
                    "description": "arguments intended for the dynamic MCP tool"
                }
            },
            "additionalProperties": true
        });
        spec.required_features.insert(FeatureFlag::Mcp);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<McpToolInput>("mcp", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<McpToolInput>("mcp", &input)?;
        input.validate()?;
        unsupported_result(
            use_id,
            MCP_TOOL_RUNTIME_UNAVAILABLE,
            json!({
                "supported": false,
                "tool": "mcp",
                "server": input.server,
                "tool_name": input.tool,
                "arguments": input.arguments,
            }),
        )
    }
}

#[async_trait]
impl Tool for WorkflowTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = test_tool_spec(
            "workflow",
            "Source-compatible Workflow alias; workflow script execution is unsupported",
            ToolKind::Interaction,
        );
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "workflow": {
                    "type": "string",
                    "description": "workflow script name"
                },
                "command": {
                    "type": "string",
                    "description": "workflow command"
                },
                "prompt": {
                    "type": "string",
                    "description": "workflow prompt"
                },
                "args": {
                    "description": "source-compatible workflow arguments"
                }
            },
            "additionalProperties": true
        });
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<WorkflowInput>("workflow", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<WorkflowInput>("workflow", &input)?;
        input.validate()?;

        let workflow_name = match input.workflow.as_deref().filter(|s| !s.is_empty()) {
            Some(name) => name.to_owned(),
            None => {
                // No workflow name given — list available workflows.
                let scripts = find_workflow_scripts(&context.cwd);
                let list = if scripts.is_empty() {
                    format!(
                        "No workflow scripts found. Add shell scripts under \
                         {cwd}/.wonder/workflows/ (e.g. my-task.sh) and name \
                         them with the 'workflow' parameter.",
                        cwd = context.cwd.display()
                    )
                } else {
                    format!(
                        "Available workflows:\n{}",
                        scripts
                            .iter()
                            .map(|p| format!(
                                "  - {}",
                                p.file_stem().and_then(|s| s.to_str()).unwrap_or("?")
                            ))
                            .collect::<Vec<_>>()
                            .join("\n")
                    )
                };
                return Ok(ToolResult::success(use_id, list).with_metadata(json!({
                    "tool": "workflow",
                    "listed": true,
                })));
            }
        };

        let script_path = context
            .cwd
            .join(".wonder")
            .join("workflows")
            .join(format!("{workflow_name}.sh"));

        if !script_path.exists() {
            return Ok(ToolResult::failure(
                use_id,
                format!(
                    "Workflow script not found: {path}\n\
                     Create {path} to define this workflow.",
                    path = script_path.display()
                ),
            )
            .with_metadata(json!({
                "tool": "workflow",
                "workflow": workflow_name,
                "found": false,
            })));
        }

        let mut cmd = std::process::Command::new("sh");
        cmd.arg(&script_path).current_dir(&context.cwd);

        // Forward command/prompt/args as environment variables.
        if let Some(ref command) = input.command {
            cmd.env("WORKFLOW_COMMAND", command);
        }
        if let Some(ref prompt) = input.prompt {
            cmd.env("WORKFLOW_PROMPT", prompt);
        }
        if !input.args.is_null() {
            cmd.env(
                "WORKFLOW_ARGS",
                serde_json::to_string(&input.args).unwrap_or_default(),
            );
        }

        match cmd.output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = if stderr.is_empty() {
                    stdout.to_string()
                } else {
                    format!("{stdout}\n{stderr}")
                };
                let content = combined.trim().to_string();
                let result = if output.status.success() {
                    ToolResult::success(
                        use_id,
                        if content.is_empty() {
                            format!("Workflow '{workflow_name}' completed (exit 0).")
                        } else {
                            content
                        },
                    )
                } else {
                    let code = output.status.code().unwrap_or(-1);
                    ToolResult::failure(
                        use_id,
                        format!("Workflow '{workflow_name}' failed (exit {code}).\n{content}"),
                    )
                };
                Ok(result.with_metadata(json!({
                    "tool": "workflow",
                    "workflow": workflow_name,
                    "exit_code": output.status.code(),
                })))
            }
            Err(err) => Ok(ToolResult::failure(
                use_id,
                format!("Failed to execute workflow '{workflow_name}': {err}"),
            )
            .with_metadata(json!({
                "tool": "workflow",
                "workflow": workflow_name,
            }))),
        }
    }
}

#[async_trait]
impl Tool for PushNotificationTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = test_tool_spec(
            "push_notification",
            "Source-compatible PushNotification alias; notification delivery is unsupported",
            ToolKind::Interaction,
        );
        spec.aliases.push("PushNotification".into());
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "notification title"
                },
                "message": {
                    "type": "string",
                    "description": "notification message"
                },
                "body": {
                    "type": "string",
                    "description": "source-compatible message alias"
                }
            },
            "additionalProperties": true
        });
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<PushNotificationInput>("push_notification", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<PushNotificationInput>("push_notification", &input)?;
        input.validate()?;

        let title = input.title.as_deref().unwrap_or("wonder-of-u").to_string();
        let message = input
            .message
            .as_deref()
            .unwrap_or("Task complete")
            .to_string();

        let sent = send_os_notification_impl(&title, &message);
        let content = if sent {
            format!("Notification sent: {title} — {message}")
        } else {
            // Fallback: write to stderr as terminal bell.
            eprint!("\x07");
            format!(
                "Notification delivered via terminal bell (OS notification not available): \
                 {title} — {message}"
            )
        };

        Ok(ToolResult::success(use_id, content).with_metadata(json!({
            "tool": "push_notification",
            "title": title,
            "message": message,
            "sent_via_os": sent,
        })))
    }
}

#[async_trait]
impl Tool for TestingPermissionTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = test_tool_spec(
            "testing_permission",
            "Test-only permission shim that always requests explicit approval",
            ToolKind::Interaction,
        );
        spec.aliases.push("TestingPermission".into());
        spec.input_schema = json!({
            "type": "object",
            "additionalProperties": false
        });
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn permission_decision(&self, _context: &ToolContext, _input: &Value) -> PermissionDecision {
        PermissionDecision::ask(PermissionDecisionReason::Rule {
            rule: PermissionRule::new(
                "testing_permission",
                PermissionRuleBehavior::Ask,
                PermissionRuleSource::SessionRuntime,
            )
            .with_reason("testing_permission always asks for permission"),
        })
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        _input: Value,
    ) -> Result<ToolResult> {
        Ok(ToolResult::success(
            use_id,
            "TestingPermission executed successfully",
        ))
    }
}

#[async_trait]
impl Tool for OverflowTestTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = test_tool_spec(
            "overflow_test",
            "Test-only output generator for large-result handling",
            ToolKind::Interaction,
        );
        spec.aliases.push("OverflowTest".into());
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "chars": {
                    "type": "integer",
                    "description": "number of characters to emit"
                }
            },
            "additionalProperties": false
        });
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<OverflowTestInput>("overflow_test", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<OverflowTestInput>("overflow_test", &input)?;
        input.validate()?;
        let mut result = ToolResult::success(use_id, "x".repeat(input.chars));
        result.metadata = json!({
            "success": true,
            "tool": "overflow_test",
            "chars": input.chars,
        });
        Ok(result)
    }
}

#[async_trait]
impl Tool for CtxInspectTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = test_tool_spec(
            "ctx_inspect",
            "Test-only context snapshot for permissions and feature-gate debugging",
            ToolKind::Interaction,
        );
        spec.aliases.push("CtxInspect".into());
        spec.input_schema = json!({
            "type": "object",
            "additionalProperties": false
        });
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
        let snapshot = json!({
            "cwd": context.cwd.display().to_string(),
            "permission_mode": serde_json::to_value(context.permission_mode)?,
            "features": context
                .features
                .iter()
                .map(|feature| serde_json::to_string(&feature).expect("feature json").trim_matches('"').to_string())
                .collect::<Vec<_>>(),
            "additional_working_directories": context
                .additional_working_directories
                .iter()
                .map(|directory| directory.path.display().to_string())
                .collect::<Vec<_>>(),
            "permission_rules": context.permission_rules.len(),
        });
        let mut result = ToolResult::success(
            use_id,
            serde_json::to_string_pretty(&snapshot).expect("json"),
        );
        result.metadata = snapshot;
        Ok(result)
    }
}

#[async_trait]
impl Tool for MonitorTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = test_tool_spec(
            "monitor",
            "Test-only Monitor alias; runtime monitoring hooks are unsupported",
            ToolKind::Interaction,
        );
        spec.input_schema = json!({
            "type": "object",
            "additionalProperties": true
        });
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
        let backend =
            serde_json::to_value(RemoteTaskState::deferred(RemoteTaskType::AutofixPr, None))?;
        unsupported_result(
            use_id,
            MONITOR_RUNTIME_UNAVAILABLE,
            json!({
                "supported": false,
                "tool": "monitor",
                "task_backend": backend["monitor"].clone(),
            }),
        )
    }
}

#[async_trait]
impl Tool for SubscribePrTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = test_tool_spec(
            "subscribe_pr",
            "Test-only SubscribePR alias; webhook subscriptions are unsupported",
            ToolKind::Interaction,
        );
        spec.aliases.push("SubscribePR".into());
        spec.input_schema = json!({
            "type": "object",
            "additionalProperties": true
        });
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
        unsupported_result(
            use_id,
            SUBSCRIBE_PR_RUNTIME_UNAVAILABLE,
            json!({
                "supported": false,
                "tool": "subscribe_pr",
            }),
        )
    }
}

#[async_trait]
impl Tool for SnipTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = test_tool_spec(
            "snip",
            "Test-only Snip alias; history snipping is unsupported",
            ToolKind::Interaction,
        );
        spec.input_schema = json!({
            "type": "object",
            "additionalProperties": true
        });
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
        let mut result = ToolResult::success(use_id, "signal=snip");
        result.metadata = json!({
            "success": true,
            "tool": "snip",
            "signal": "snip",
        });
        Ok(result)
    }
}

#[async_trait]
impl Tool for TungstenTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = test_tool_spec(
            "tungsten",
            "Test-only Tungsten alias; tmux-backed terminal orchestration is unsupported",
            ToolKind::Shell,
        );
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "args": {
                    "type": "array",
                    "description": "source-compatible argument vector for Tungsten",
                    "items": { "type": "string" }
                }
            },
            "additionalProperties": true
        });
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<TungstenInput>("tungsten", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<TungstenInput>("tungsten", &input)?;
        input.validate()?;
        unsupported_result(
            use_id,
            TUNGSTEN_RUNTIME_UNAVAILABLE,
            json!({
                "supported": false,
                "tool": "tungsten",
                "args": input.args,
            }),
        )
    }
}

fn test_tool_spec(name: &str, description: &str, kind: ToolKind) -> ToolSpec {
    let mut spec = base_spec(name, description, kind);
    spec.required_features.insert(FeatureFlag::TestTools);
    spec
}

fn repl_mode_enabled() -> bool {
    repl_mode_enabled_from_env(|name| env::var(name).ok())
}

fn repl_mode_enabled_from_env(getenv: impl Fn(&str) -> Option<String>) -> bool {
    if getenv("CLAUDE_CODE_REPL")
        .as_deref()
        .is_some_and(is_defined_falsy)
    {
        return false;
    }
    if getenv("CLAUDE_REPL_MODE").as_deref().is_some_and(is_truthy) {
        return true;
    }
    getenv("USER_TYPE").as_deref() == Some("ant")
        && getenv("CLAUDE_CODE_ENTRYPOINT").as_deref() == Some("cli")
}

fn is_defined_falsy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn unsupported_result(use_id: ToolUseId, message: &str, metadata: Value) -> Result<ToolResult> {
    let mut result = ToolResult::failure(use_id, message);
    result.metadata = metadata;
    Ok(result)
}

fn trim_command_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches(['\r', '\n'])
        .to_string()
}

fn merge_command_output(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = trim_command_output(stdout);
    let stderr = trim_command_output(stderr);
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => stdout,
        (true, false) => stderr,
        (false, false) => format!("{stdout}\n{stderr}"),
    }
}

fn render_command_result(exit_code: i32, output: &str) -> String {
    if output.is_empty() {
        format!("exit_code={exit_code}")
    } else {
        format!("exit_code={exit_code}\n{output}")
    }
}

fn resolve_files(cwd: &Path, paths: &[PathBuf]) -> Result<Vec<ResolvedFile>> {
    paths.iter().map(|path| resolve_file(cwd, path)).collect()
}

fn resolve_file(cwd: &Path, path: &Path) -> Result<ResolvedFile> {
    let resolved = resolve_path(path, cwd);
    let metadata = fs::metadata(&resolved).map_err(|error| match error.kind() {
        ErrorKind::NotFound => {
            WonderError::validation(format!("file does not exist: {}", path.display()))
        }
        _ => WonderError::validation(format!("unable to access file {}: {error}", path.display())),
    })?;
    if !metadata.is_file() {
        return Err(WonderError::validation(format!(
            "path is not a file: {}",
            path.display()
        )));
    }
    Ok(ResolvedFile {
        path: resolved.display().to_string(),
        size: metadata.len(),
        is_image: resolved
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg"
                )
            }),
    })
}

fn read_mcp_auth_status(storage_root: &Path, server_name: &str) -> Result<McpAuthStatus> {
    let store = McpConfigStore::new(storage_root);
    let config = store.read()?;
    let server = config
        .server(server_name)
        .ok_or_else(|| WonderError::not_found("mcp server", server_name))?;
    Ok(McpAuthStatus::from_server(server))
}

impl McpAuthStatus {
    fn from_server(server: &McpServerConfig) -> Self {
        Self {
            server_name: server.name.clone(),
            enabled: server.enabled,
            command_line: server.command_line(),
            cwd: server.cwd.as_ref().map(|path| path.display().to_string()),
            protocol_version: server.protocol_version.clone(),
        }
    }
}

fn auth_url_from_command_line(command_line: &str) -> Option<String> {
    command_line
        .split_whitespace()
        .map(|token| token.trim_matches(['"', '\'']))
        .find(|token| token.starts_with("http"))
        .map(str::to_owned)
}

fn render_mcp_auth_failure_message(auth_url: Option<&str>) -> String {
    match auth_url {
        Some(url) => format!("{MCP_AUTH_RUNTIME_UNAVAILABLE}\nauth_url={url}"),
        None => format!(
            "{MCP_AUTH_RUNTIME_UNAVAILABLE}\nauth_url=null\nnote=configure the MCP server command line with an http OAuth endpoint"
        ),
    }
}

fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    let status = Command::new("xdg-open").arg(url).status()?;

    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(url).status()?;

    #[cfg(target_os = "windows")]
    let status = Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(url)
        .status()?;

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = url;
        return Err(Error::other(
            "opening browser URLs is unsupported on this platform",
        ));
    }

    if status.success() {
        Ok(())
    } else {
        Err(Error::other(format!(
            "browser open command exited with {status}"
        )))
    }
}

const fn default_overflow_chars() -> usize {
    131_072
}

/// List .wonder/workflows/*.sh scripts under `cwd`.
fn find_workflow_scripts(cwd: &Path) -> Vec<PathBuf> {
    let dir = cwd.join(".wonder").join("workflows");
    match fs::read_dir(&dir) {
        Ok(entries) => {
            let mut scripts: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("sh"))
                .collect();
            scripts.sort();
            scripts
        }
        Err(_) => vec![],
    }
}

/// Send an OS desktop notification. Returns `true` if successfully dispatched.
fn send_os_notification_impl(title: &str, message: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        if let Ok(output) = Command::new("notify-send").arg(title).arg(message).output() {
            return output.status.success();
        }
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            message.replace('"', "\\\""),
            title.replace('"', "\\\""),
        );
        if let Ok(output) = Command::new("osascript").args(["-e", &script]).output() {
            return output.status.success();
        }
        return false;
    }

    #[allow(unreachable_code)]
    false
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, sync::Arc};

    use futures::executor::block_on;
    use serde_json::json;
    use wonder_of_u_core::{
        AdditionalWorkingDirectory, FeatureSet, PermissionMode, SessionId, ToolQuery, ToolRegistry,
    };
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn tool_context(root: PathBuf) -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd: root,
            permission_mode: PermissionMode::AcceptEdits,
            additional_working_directories: Vec::new(),
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
        }
    }

    #[test]
    fn repl_mode_follows_source_environment_contract() {
        assert!(!repl_mode_enabled_from_env(|name| match name {
            "CLAUDE_CODE_REPL" => Some("0".into()),
            "CLAUDE_REPL_MODE" => Some("1".into()),
            "USER_TYPE" => Some("ant".into()),
            "CLAUDE_CODE_ENTRYPOINT" => Some("cli".into()),
            _ => None,
        }));
        assert!(repl_mode_enabled_from_env(|name| match name {
            "USER_TYPE" => Some("ant".into()),
            "CLAUDE_CODE_ENTRYPOINT" => Some("cli".into()),
            _ => None,
        }));
    }

    #[test]
    fn repl_returns_mode_metadata_and_executes_commands() {
        let tool = ReplTool;
        let result = block_on(tool.execute(
            tool_context(unique_test_dir("special-tools-repl")),
            ToolUseId::new(),
            json!({}),
        ))
        .expect("execute");

        assert!(result.success);
        assert!(result.content.contains("primitive_tools="));
        assert_eq!(result.metadata["tool"], "repl");

        let executed = block_on(tool.execute(
            tool_context(unique_test_dir("special-tools-repl-command")),
            ToolUseId::new(),
            json!({ "command": "printf 'stdout\\n'; printf 'stderr\\n' >&2" }),
        ))
        .expect("execute");
        assert!(executed.success);
        assert_eq!(executed.content, "exit_code=0\nstdout\nstderr");
        assert_eq!(executed.metadata["exit_code"], 0);
    }

    #[test]
    fn repl_returns_stderr_for_failed_commands() {
        let tool = ReplTool;
        let failed = block_on(tool.execute(
            tool_context(unique_test_dir("special-tools-repl-failure")),
            ToolUseId::new(),
            json!({ "command": "printf 'boom\\n' >&2; exit 7" }),
        ))
        .expect("execute");

        assert!(!failed.success);
        assert_eq!(failed.content, "exit_code=7\nboom");
        assert_eq!(failed.metadata["exit_code"], 7);
    }

    #[test]
    fn structured_output_echoes_the_json_payload() {
        let tool = StructuredOutputTool;
        let result = block_on(tool.execute(
            tool_context(unique_test_dir("special-tools-structured-output")),
            ToolUseId::new(),
            json!({ "status": "ok", "count": 2 }),
        ))
        .expect("execute");

        assert!(result.success);
        assert_eq!(result.metadata["structured_output"]["status"], "ok");
        assert_eq!(result.metadata["structured_output"]["count"], 2);
    }

    #[test]
    fn sleep_waits_for_bounded_duration_without_shell() {
        let tool = SleepTool;
        let result = block_on(tool.execute(
            tool_context(unique_test_dir("special-tools-sleep")),
            ToolUseId::new(),
            json!({ "duration_ms": 1 }),
        ))
        .expect("execute");

        assert!(result.success);
        assert_eq!(result.metadata["tool"], "sleep");
        assert_eq!(result.metadata["duration_ms"], 1);
        assert!(tool.validate_input(&json!({ "seconds": 301 })).is_err());
    }

    #[test]
    fn dynamic_source_tools_return_explicit_unsupported_failures() {
        let mcp = block_on(McpTool.execute(
            tool_context(unique_test_dir("special-tools-mcp")),
            ToolUseId::new(),
            json!({ "serverName": "demo", "toolName": "lookup", "arguments": { "q": "rust" } }),
        ))
        .expect("execute");
        assert!(!mcp.success);
        assert!(mcp.content.contains(MCP_TOOL_RUNTIME_UNAVAILABLE));
        assert_eq!(mcp.metadata["server"], "demo");
        assert_eq!(mcp.metadata["tool_name"], "lookup");

        let workflow = block_on(WorkflowTool.execute(
            tool_context(unique_test_dir("special-tools-workflow")),
            ToolUseId::new(),
            json!({ "workflow": "triage", "prompt": "summarize" }),
        ))
        .expect("execute");
        // Script doesn't exist in the test dir, so it should fail with not-found.
        assert!(!workflow.success);
        assert!(workflow.content.contains("triage"));
        assert_eq!(workflow.metadata["workflow"], "triage");

        let push = block_on(PushNotificationTool.execute(
            tool_context(unique_test_dir("special-tools-push")),
            ToolUseId::new(),
            json!({ "title": "Done", "body": "Work finished" }),
        ))
        .expect("execute");
        // push_notification always succeeds (OS notify or terminal bell fallback).
        assert!(push.success);
        assert!(push.content.contains("Work finished"));
        assert_eq!(push.metadata["title"], "Done");
        assert_eq!(push.metadata["message"], "Work finished");
    }

    #[test]
    fn send_user_file_reports_resolved_files_without_claiming_delivery() {
        let dir = unique_test_dir("special-tools-send-user-file");
        let screenshot = dir.join("shot.png");
        fs::write(&screenshot, [1_u8, 2, 3]).expect("attachment");
        let tool = SendUserFileTool;

        let result = block_on(tool.execute(
            tool_context(dir),
            ToolUseId::new(),
            json!({ "files": [screenshot.display().to_string()] }),
        ))
        .expect("execute");

        assert!(!result.success);
        assert_eq!(result.metadata["files"][0]["is_image"], true);
        assert!(result.content.contains(SEND_USER_FILE_RUNTIME_UNAVAILABLE));
    }

    #[test]
    fn mcp_auth_reads_server_configuration_without_claiming_oauth_support() {
        let dir = unique_test_dir("special-tools-mcp-auth");
        let store = McpConfigStore::new(&dir);
        store
            .write(&wonder_of_u_mcp::McpConfig {
                servers: vec![McpServerConfig {
                    name: "demo".into(),
                    command: "demo-server".into(),
                    args: vec!["--stdio".into()],
                    env: Default::default(),
                    enabled: true,
                    cwd: Some(dir.join("workspace")),
                    protocol_version: Some("2024-11-05".into()),
                }],
                ..Default::default()
            })
            .expect("config");

        let status = read_mcp_auth_status(&dir, "demo").expect("status");
        assert_eq!(status.server_name, "demo");
        assert_eq!(status.command_line, "demo-server --stdio");
    }

    #[test]
    fn mcp_auth_extracts_http_urls_from_command_lines() {
        assert_eq!(
            auth_url_from_command_line("demo-server --oauth http://localhost:4317/auth"),
            Some("http://localhost:4317/auth".into())
        );
        assert_eq!(auth_url_from_command_line("demo-server --stdio"), None);
        assert!(render_mcp_auth_failure_message(None).contains("auth_url=null"));
    }

    #[test]
    fn snip_returns_supported_signal() {
        let result = block_on(SnipTool.execute(
            tool_context(unique_test_dir("special-tools-snip")),
            ToolUseId::new(),
            json!({}),
        ))
        .expect("execute");

        assert!(result.success);
        assert_eq!(result.content, "signal=snip");
        assert_eq!(result.metadata["signal"], "snip");
    }

    #[test]
    fn testing_permission_always_requests_approval() {
        let tool = TestingPermissionTool;
        let decision = tool.permission_decision(
            &tool_context(unique_test_dir("special-tools-permission")),
            &json!({}),
        );

        assert!(matches!(decision, PermissionDecision::Ask { .. }));
    }

    #[test]
    fn overflow_test_emits_requested_output_size() {
        let tool = OverflowTestTool;
        let result = block_on(tool.execute(
            tool_context(unique_test_dir("special-tools-overflow")),
            ToolUseId::new(),
            json!({ "chars": 16 }),
        ))
        .expect("execute");

        assert!(result.success);
        assert_eq!(result.content.len(), 16);
        assert_eq!(result.metadata["chars"], 16);
    }

    #[test]
    fn ctx_inspect_reports_context_details() {
        let dir = unique_test_dir("special-tools-ctx");
        let extra = dir.join("shared");
        let mut context = tool_context(dir.clone());
        context
            .additional_working_directories
            .push(AdditionalWorkingDirectory::new(
                &extra,
                PermissionRuleSource::Local,
            ));

        let result = block_on(CtxInspectTool.execute(context, ToolUseId::new(), json!({})))
            .expect("execute");

        assert!(result.success);
        assert_eq!(result.metadata["cwd"], dir.display().to_string());
        assert!(result.content.contains("permission_mode"));
    }

    #[test]
    fn unsupported_dev_tools_return_explicit_failures() {
        let monitor = block_on(MonitorTool.execute(
            tool_context(unique_test_dir("special-tools-monitor")),
            ToolUseId::new(),
            json!({}),
        ))
        .expect("execute");
        assert!(!monitor.success);
        assert!(monitor.content.contains(MONITOR_RUNTIME_UNAVAILABLE));
        assert_eq!(monitor.metadata["task_backend"]["flow"], "monitor");
        assert_eq!(monitor.metadata["task_backend"]["support"], "unsupported");

        let tungsten = block_on(TungstenTool.execute(
            tool_context(unique_test_dir("special-tools-tungsten")),
            ToolUseId::new(),
            json!({ "args": ["test"] }),
        ))
        .expect("execute");
        assert!(!tungsten.success);
        assert_eq!(tungsten.metadata["args"][0], "test");

        let background_pr = block_on(SuggestBackgroundPrTool.execute(
            tool_context(unique_test_dir("special-tools-background-pr")),
            ToolUseId::new(),
            json!({ "prompt": "summarize" }),
        ))
        .expect("execute");
        assert!(!background_pr.success);
        assert_eq!(
            background_pr.metadata["task_backend"]["task_type"],
            "background-pr"
        );
        assert_eq!(
            background_pr.metadata["task_backend"]["checker"]["support"],
            "unsupported"
        );
    }

    #[test]
    fn registry_hides_test_tools_until_feature_is_enabled() {
        let mut registry = ToolRegistry::new();
        registry
            .register(Arc::new(TestingPermissionTool))
            .expect("testing permission");
        registry
            .register(Arc::new(StructuredOutputTool))
            .expect("structured output");

        let default_query = ToolQuery::new(FeatureSet::first_release());
        let mut with_tests = FeatureSet::first_release();
        with_tests.enable(FeatureFlag::TestTools);
        let test_query = ToolQuery::new(with_tests);

        assert!(
            registry
                .resolve_enabled("StructuredOutput", &default_query)
                .is_some()
        );
        assert!(
            registry
                .resolve_enabled("TestingPermission", &default_query)
                .is_none()
        );
        assert!(
            registry
                .resolve_enabled("testing_permission", &test_query)
                .is_some()
        );
    }

    #[test]
    fn builtin_registry_keeps_dev_tools_hidden_by_default_specs() {
        let registry = crate::builtin_registry().expect("registry");
        let default_specs = registry
            .enabled_specs(&FeatureSet::first_release())
            .into_iter()
            .map(|spec| spec.name)
            .collect::<BTreeSet<_>>();
        assert!(!default_specs.contains("testing_permission"));
        assert!(!default_specs.contains("ctx_inspect"));

        let mut features = FeatureSet::first_release();
        features.enable(FeatureFlag::TestTools);
        let enabled = registry
            .enabled_specs(&features)
            .into_iter()
            .map(|spec| spec.name)
            .collect::<BTreeSet<_>>();
        assert!(enabled.contains("testing_permission"));
        assert!(enabled.contains("ctx_inspect"));
    }

    #[test]
    fn special_tool_specs_expose_expected_aliases_and_schema_fields() {
        let repl = ReplTool.spec();
        assert!(repl.aliases.is_empty());
        assert!(!repl.read_only);
        assert!(!repl.concurrency_safe);
        assert!(
            repl.input_schema
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|properties| properties.contains_key("command"))
        );

        let send_user_file = SendUserFileTool.spec();
        assert_eq!(send_user_file.aliases, vec!["SendUserFile".to_string()]);
        assert!(
            send_user_file
                .input_schema
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|properties| properties.contains_key("files"))
        );

        let sleep = SleepTool.spec();
        assert!(sleep.aliases.is_empty());
        assert!(sleep.concurrency_safe);
        assert!(
            sleep
                .input_schema
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|properties| properties.contains_key("duration_ms"))
        );

        let mcp = McpTool.spec();
        assert_eq!(mcp.aliases, vec!["MCPTool".to_string()]);
        assert!(
            mcp.input_schema
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|properties| properties.contains_key("toolName"))
        );

        let tungsten = TungstenTool.spec();
        assert!(tungsten.aliases.is_empty());
        assert!(
            tungsten
                .input_schema
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|properties| properties.contains_key("args"))
        );
    }
}
