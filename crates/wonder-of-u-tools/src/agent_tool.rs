//! Agent task queue tool.

use std::{fs, path::Path};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;
use wonder_of_u_core::{
    FeatureFlag, RemoteTaskState, RemoteTaskType, Result, Tool, ToolContext, ToolKind, ToolResult,
    ToolSchema, ToolSpec, ToolUseId,
};

use crate::{app_root, base_spec, parse_input, require_non_empty_text};
/// Represents agent input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentInput {
    /// Stores the prompt
    pub prompt: String,
    /// Stores the description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Stores the subagent type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_type: Option<String>,
    /// Stores the model
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Stores the run in background
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_in_background: Option<bool>,
    /// Stores the name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Stores the team name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_name: Option<String>,
    /// Stores the mode
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Stores the isolation
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<String>,
    /// Stores the cwd
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Stores the tools
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
}

impl AgentInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("agent", "prompt", &self.prompt)?;
        if let Some(description) = &self.description {
            require_non_empty_text("agent", "description", description)?;
        }
        if self.subagent_type.is_some() {
            return Err(wonder_of_u_core::WonderError::validation(
                "agent source-compatible `subagent_type` is not supported in the Rust runtime",
            ));
        }
        if matches!(self.run_in_background, Some(false)) {
            return Err(wonder_of_u_core::WonderError::validation(
                "agent source-compatible `run_in_background=false` is not supported because the Rust runtime only queues background agent tasks",
            ));
        }

        if self.isolation.as_deref() == Some("remote") {
            return Err(wonder_of_u_core::WonderError::validation(
                RemoteTaskState::deferred(RemoteTaskType::RemoteAgent, None).start_error_message(),
            ));
        }

        for (field, is_present) in [
            ("name", self.name.is_some()),
            ("team_name", self.team_name.is_some()),
            ("mode", self.mode.is_some()),
            ("isolation", self.isolation.is_some()),
            ("cwd", self.cwd.is_some()),
        ] {
            if is_present {
                return Err(wonder_of_u_core::WonderError::validation(format!(
                    "agent source-compatible `{field}` is not supported in the Rust runtime"
                )));
            }
        }

        Ok(())
    }
}
/// Represents agent tool
#[derive(Debug, Default)]
pub struct AgentTool;

#[async_trait]
impl Tool for AgentTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec("agent", "Queue a background agent task", ToolKind::Agent)
            .with_input_schema(
                ToolSchema::object()
                    .property(
                        "description",
                        ToolSchema::string("optional short task description for source compatibility"),
                    )
                    .property("prompt", ToolSchema::string("agent prompt to queue"))
                    .property(
                        "subagent_type",
                        ToolSchema::string(
                            "source-compatible specialized agent type; currently unsupported",
                        ),
                    )
                    .property("model", ToolSchema::string("optional model override"))
                    .property(
                        "run_in_background",
                        ToolSchema::boolean(
                            "source-compatible background toggle; false is unsupported because the Rust runtime always queues",
                        ),
                    )
                    .property(
                        "name",
                        ToolSchema::string(
                            "source-compatible teammate name; currently unsupported",
                        ),
                    )
                    .property(
                        "team_name",
                        ToolSchema::string(
                            "source-compatible swarm team name; currently unsupported",
                        ),
                    )
                    .property(
                        "mode",
                        ToolSchema::enumeration(
                            "source-compatible permission mode; currently unsupported",
                            [
                                "default",
                                "acceptEdits",
                                "bypassPermissions",
                                "dontAsk",
                                "plan",
                            ],
                        ),
                    )
                    .property(
                        "isolation",
                        ToolSchema::enumeration(
                            "source-compatible isolation mode; currently unsupported",
                            ["worktree", "remote"],
                        ),
                    )
                    .property(
                        "cwd",
                        ToolSchema::string(
                            "source-compatible working directory override; currently unsupported",
                        ),
                    )
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
        spec.aliases.push("Task".into());
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
        let mut result = ToolResult::success(use_id, format!("agent task queued: {task_id}"));
        result.metadata = json!({
            "task_id": task_id,
            "status": "queued",
            "run_in_background": true,
            "description": input.description,
            "supports_send_message": false,
            "supports_team_name": false,
        });
        Ok(result)
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
            "description": input.description,
            "prompt": input.prompt,
            "model": input.model,
            "run_in_background": input.run_in_background.unwrap_or(true),
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
                description: None,
                subagent_type: None,
                model: Some("demo".into()),
                run_in_background: None,
                name: None,
                team_name: None,
                mode: None,
                isolation: None,
                cwd: None,
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
                description: Some("review task".into()),
                subagent_type: None,
                model: None,
                run_in_background: Some(true),
                name: None,
                team_name: None,
                mode: None,
                isolation: None,
                cwd: None,
                tools: None,
            },
        )
        .expect("queue task");
        let request = fs::read_to_string(dir.join("tasks").join(task_id).join("request.json"))
            .expect("request");

        assert!(request.contains("\"status\": \"queued\""));
        assert!(request.contains("\"description\": \"review task\""));
        assert!(request.contains("\"run_in_background\": true"));
    }

    #[test]
    fn agent_validation_rejects_unsupported_source_fields() {
        let tool = AgentTool;
        let error = tool
            .validate_input(&json!({
                "prompt": "review",
                "name": "reviewer",
            }))
            .expect_err("unsupported name");

        assert!(error.to_string().contains("name"));
    }

    #[test]
    fn agent_validation_rejects_remote_isolation_backend() {
        let tool = AgentTool;
        let error = tool
            .validate_input(&json!({
                "prompt": "review",
                "isolation": "remote",
            }))
            .expect_err("remote isolation");

        assert!(error.to_string().contains("cannot start remote-agent task"));
    }

    #[test]
    fn agent_validation_rejects_foreground_source_mode() {
        let tool = AgentTool;
        let error = tool
            .validate_input(&json!({
                "prompt": "review",
                "run_in_background": false,
            }))
            .expect_err("foreground unsupported");

        assert!(error.to_string().contains("run_in_background"));
    }
}
