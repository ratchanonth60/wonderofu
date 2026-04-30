//! Plan file read and write tools.

use std::{fs, path::PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wonder_of_u_core::{
    Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId, resolve_path,
};

use crate::{base_spec, parse_input, require_non_empty_path};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanReadInput {
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanWriteInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
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

#[derive(Debug, Default)]
pub struct PlanReadTool;

#[derive(Debug, Default)]
pub struct PlanWriteTool;

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

fn resolve_plan_path(context: &ToolContext, path: Option<&std::path::Path>) -> PathBuf {
    path.map(|path| resolve_path(path, &context.cwd))
        .unwrap_or_else(|| context.cwd.join("plan.md"))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use futures::executor::block_on;
    use serde_json::json;
    use wonder_of_u_core::{FeatureSet, PermissionMode, SessionId, ToolContext, ToolUseId};
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
        }
    }

    #[test]
    fn plan_read_reads_default_plan_file() {
        let dir = unique_test_dir("tools-plan-read");
        fs::write(dir.join("plan.md"), "# plan\n").expect("write plan");
        let tool = PlanReadTool;

        let result = block_on(tool.execute(tool_context(dir), ToolUseId::new(), json!({})))
            .expect("read plan");

        assert_eq!(result.content, "# plan\n");
    }

    #[test]
    fn plan_write_overwrites_file() {
        let dir = unique_test_dir("tools-plan-write");
        let tool = PlanWriteTool;

        let result = block_on(tool.execute(
            tool_context(dir.clone()),
            ToolUseId::new(),
            json!({ "content": "# updated\n- done\n" }),
        ))
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
}
