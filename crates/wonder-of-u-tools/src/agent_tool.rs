//! Agent task queue tool.

use std::{fs, path::Path};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;
use wonder_of_u_core::{
    FeatureFlag, Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId,
};

use crate::{app_root, base_spec, parse_input, require_non_empty_text};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentInput {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
}

impl AgentInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("agent", "prompt", &self.prompt)
    }
}

#[derive(Debug, Default)]
pub struct AgentTool;

#[async_trait]
impl Tool for AgentTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec("agent", "Queue a background agent task", ToolKind::Agent)
            .with_input_schema(
                ToolSchema::object()
                    .property("prompt", ToolSchema::string("agent prompt to queue"))
                    .property("model", ToolSchema::string("optional model override"))
                    .property(
                        "tools",
                        json!({
                            "type": "array",
                            "description": "optional allowed tools",
                            "items": { "type": "string" }
                        }),
                    )
                    .required("prompt"),
            );
        spec.required_features.insert(FeatureFlag::Agents);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<AgentInput>("agent", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<AgentInput>("agent", &input)?;
        input.validate()?;

        let task_id = queue_agent_task(&app_root()?, &input)?;
        Ok(ToolResult::success(
            use_id,
            format!("agent task queued: {task_id}"),
        ))
    }
}

fn queue_agent_task(app_root: &Path, input: &AgentInput) -> Result<String> {
    let task_id = Uuid::new_v4().to_string();
    let task_dir = app_root.join("tasks").join(&task_id);
    fs::create_dir_all(&task_dir)?;
    fs::write(task_dir.join("prompt.txt"), &input.prompt)?;
    fs::write(task_dir.join("status.txt"), "queued\n")?;
    fs::write(
        task_dir.join("request.json"),
        serde_json::to_string_pretty(&json!({
            "prompt": input.prompt,
            "model": input.model,
            "tools": input.tools,
            "status": "queued",
        }))?,
    )?;
    Ok(task_id)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    #[test]
    fn agent_validation_rejects_empty_prompt() {
        let tool = AgentTool;
        let error = tool
            .validate_input(&json!({ "prompt": "   " }))
            .expect_err("empty prompt");

        assert!(error.to_string().contains("prompt"));
    }

    #[test]
    fn agent_queues_task_files() {
        let dir = unique_test_dir("tools-agent");
        let task_id = queue_agent_task(
            &dir,
            &AgentInput {
                prompt: "review the patch".into(),
                model: Some("demo".into()),
                tools: Some(vec!["bash".into()]),
            },
        )
        .expect("queue task");
        let task_dir = dir.join("tasks").join(&task_id);

        assert!(task_dir.join("prompt.txt").exists());
        assert_eq!(
            fs::read_to_string(task_dir.join("status.txt")).expect("status"),
            "queued\n"
        );
    }

    #[test]
    fn queued_agent_request_records_metadata() {
        let dir = unique_test_dir("tools-agent-request");
        let task_id = queue_agent_task(
            &dir,
            &AgentInput {
                prompt: "review".into(),
                model: None,
                tools: None,
            },
        )
        .expect("queue task");
        let request = fs::read_to_string(dir.join("tasks").join(task_id).join("request.json"))
            .expect("request");

        assert!(request.contains("\"status\": \"queued\""));
    }
}
