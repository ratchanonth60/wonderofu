//! Plan file read and write tools.

use std::{fs, path::PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{
    Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId, WonderError,
    resolve_path,
};

use crate::{base_spec, parse_input, require_non_empty_path};
/// Represents plan read input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanReadInput {
    /// Stores the path
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

impl PlanReadInput {
    fn validate(&self) -> Result<()> {
        if let Some(path) = &self.path {
            require_non_empty_path("plan_read", "path", path)?;
        }
        Ok(())
    }
}
/// Represents plan write input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanWriteInput {
    /// Stores the path
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Stores the content
    pub content: String,
}

impl PlanWriteInput {
    fn validate(&self) -> Result<()> {
        if let Some(path) = &self.path {
            require_non_empty_path("plan_write", "path", path)?;
        }
        Ok(())
    }
}
/// Represents enter plan mode input
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnterPlanModeInput {}
/// Represents exit plan mode allowed prompt
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitPlanModeAllowedPrompt {
    /// Stores the tool
    pub tool: String,
    /// Stores the prompt
    pub prompt: String,
}
/// Represents exit plan mode input
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitPlanModeInput {
    /// Stores the allowed prompts
    #[serde(default, rename = "allowedPrompts", alias = "allowed_prompts")]
    pub allowed_prompts: Option<Vec<ExitPlanModeAllowedPrompt>>,
    /// Stores the plan
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(
        default,
        rename = "planFilePath",
        alias = "plan_file_path",
        skip_serializing_if = "Option::is_none"
    )]
    /// Stores the plan file path
    pub plan_file_path: Option<PathBuf>,
}

impl ExitPlanModeInput {
    fn validate(&self) -> Result<()> {
        if self.allowed_prompts.is_some() {
            return Err(WonderError::validation(
                "exit_plan_mode source-compatible `allowedPrompts` is not supported in wonder-of-u-tools",
            ));
        }
        if self.plan.is_some() {
            return Err(WonderError::validation(
                "exit_plan_mode source-compatible `plan` injection is not supported in wonder-of-u-tools",
            ));
        }
        if self.plan_file_path.is_some() {
            return Err(WonderError::validation(
                "exit_plan_mode source-compatible `planFilePath` is not supported in wonder-of-u-tools",
            ));
        }
        Ok(())
    }
}
/// Represents plan read tool
#[derive(Debug, Default)]
pub struct PlanReadTool;
/// Represents plan write tool
#[derive(Debug, Default)]
pub struct PlanWriteTool;
/// Represents enter plan mode tool
#[derive(Debug, Default)]
pub struct EnterPlanModeTool;
/// Represents exit plan mode tool
#[derive(Debug, Default)]
pub struct ExitPlanModeTool;

#[async_trait]
impl Tool for PlanReadTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec("plan_read", "Read a plan file", ToolKind::Planning)
            .with_input_schema(
                ToolSchema::object()
                    .property("path", ToolSchema::string("optional path to the plan file")),
            );
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<PlanReadInput>("plan_read", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<PlanReadInput>("plan_read", &input)?;
        input.validate()?;

        let path = resolve_plan_path(&context, input.path.as_deref());
        let content = fs::read_to_string(path)?;
        Ok(ToolResult::success(use_id, content))
    }
}

#[async_trait]
impl Tool for PlanWriteTool {
    fn spec(&self) -> ToolSpec {
        base_spec("plan_write", "Write a plan file", ToolKind::Planning).with_input_schema(
            ToolSchema::object()
                .property("path", ToolSchema::string("optional path to the plan file"))
                .property("content", ToolSchema::string("plan content to write"))
                .required("content"),
        )
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<PlanWriteInput>("plan_write", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<PlanWriteInput>("plan_write", &input)?;
        input.validate()?;

        let path = resolve_plan_path(&context, input.path.as_deref());
        fs::write(path, input.content)?;
        Ok(ToolResult::success(use_id, "plan written"))
    }
}

#[async_trait]
impl Tool for EnterPlanModeTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "enter_plan_mode",
            "Source-compatible EnterPlanMode alias; runtime plan mode is unsupported in wonder-of-u-tools",
            ToolKind::Planning,
        )
        .with_input_schema(ToolSchema::object());
        spec.aliases.push("EnterPlanMode".into());
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<EnterPlanModeInput>("enter_plan_mode", input)?;
        Ok(())
    }

    async fn execute(
        &self,
        _context: ToolContext,
        _use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        parse_input::<EnterPlanModeInput>("enter_plan_mode", &input)?;
        Err(WonderError::validation(
            "enter_plan_mode is not supported in wonder-of-u-tools because the Rust runtime does not implement interactive plan mode",
        ))
    }
}

#[async_trait]
impl Tool for ExitPlanModeTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "exit_plan_mode",
            "Source-compatible ExitPlanMode alias; runtime plan approval is unsupported in wonder-of-u-tools",
            ToolKind::Planning,
        )
        .with_input_schema(ToolSchema::object());
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "allowedPrompts": {
                    "type": "array",
                    "description": "source-compatible prompt permission requests; unsupported in wonder-of-u-tools",
                    "items": {
                        "type": "object",
                        "properties": {
                            "tool": ToolSchema::string("tool name"),
                            "prompt": ToolSchema::string("semantic prompt description"),
                        },
                        "required": ["tool", "prompt"],
                        "additionalProperties": false,
                    },
                },
                "plan": ToolSchema::string(
                    "source-compatible plan content injection; unsupported in wonder-of-u-tools",
                ),
                "planFilePath": ToolSchema::string(
                    "source-compatible plan file path injection; unsupported in wonder-of-u-tools",
                ),
            },
            "additionalProperties": false,
        });
        spec.aliases.push("ExitPlanMode".into());
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<ExitPlanModeInput>("exit_plan_mode", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        _use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<ExitPlanModeInput>("exit_plan_mode", &input)?;
        input.validate()?;
        Err(WonderError::validation(
            "exit_plan_mode is not supported in wonder-of-u-tools because the Rust runtime does not implement interactive plan approval or mode switching",
        ))
    }
}

fn resolve_plan_path(context: &ToolContext, path: Option<&std::path::Path>) -> PathBuf {
    path.map(|path| resolve_path(path, &context.cwd))
        .unwrap_or_else(|| context.cwd.join("plan.md"))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use serde_json::json;
    use wonder_of_u_core::{FeatureSet, PermissionMode, SessionId, ToolContext, ToolUseId};
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
            interaction_rx: None,
            fork_context: None,
            file_checkpointer: None,
            network_policy: None,
        }
    }

    #[tokio::test]
    async fn plan_read_reads_default_plan_file() {
        let dir = unique_test_dir("tools-plan-read");
        fs::write(dir.join("plan.md"), "# plan\n").expect("write plan");
        let tool = PlanReadTool;

        let result = tool
            .execute(tool_context(dir), ToolUseId::new(), json!({}))
            .await
            .expect("read plan");

        assert_eq!(result.content, "# plan\n");
    }

    #[tokio::test]
    async fn plan_write_overwrites_file() {
        let dir = unique_test_dir("tools-plan-write");
        let tool = PlanWriteTool;

        let result = tool
            .execute(
                tool_context(dir.clone()),
                ToolUseId::new(),
                json!({ "content": "# updated\n- done\n" }),
            )
            .await
            .expect("write plan");

        assert!(result.success);
        assert_eq!(
            fs::read_to_string(dir.join("plan.md")).expect("read written plan"),
            "# updated\n- done\n"
        );
    }

    #[test]
    fn plan_read_validation_rejects_empty_path() {
        let tool = PlanReadTool;
        let error = tool
            .validate_input(&json!({ "path": "" }))
            .expect_err("empty path");

        assert!(error.to_string().contains("path"));
    }

    #[tokio::test]
    async fn enter_plan_mode_is_explicitly_unsupported() {
        let dir = unique_test_dir("tools-enter-plan-mode");
        let tool = EnterPlanModeTool;
        let error = tool
            .execute(tool_context(dir), ToolUseId::new(), json!({}))
            .await
            .expect_err("unsupported enter plan mode");

        assert!(error.to_string().contains("not supported"));
    }

    #[test]
    fn exit_plan_mode_rejects_source_allowed_prompts() {
        let tool = ExitPlanModeTool;
        let error = tool
            .validate_input(&json!({
                "allowedPrompts": [
                    { "tool": "Bash", "prompt": "run tests" }
                ]
            }))
            .expect_err("unsupported allowedPrompts");

        assert!(error.to_string().contains("allowedPrompts"));
    }

    #[tokio::test]
    async fn exit_plan_mode_is_explicitly_unsupported_without_source_fields() {
        let dir = unique_test_dir("tools-exit-plan-mode");
        let tool = ExitPlanModeTool;
        let error = tool
            .execute(tool_context(dir), ToolUseId::new(), json!({}))
            .await
            .expect_err("unsupported exit plan mode");

        assert!(error.to_string().contains("not supported"));
    }

    #[test]
    fn plan_mode_specs_expose_source_aliases() {
        let enter = EnterPlanModeTool;
        let exit = ExitPlanModeTool;

        assert_eq!(enter.spec().aliases, vec!["EnterPlanMode".to_string()]);
        assert_eq!(exit.spec().aliases, vec!["ExitPlanMode".to_string()]);
    }
}
