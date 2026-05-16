//! Agent task queue tool.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{
    FeatureFlag, FleetMemberRequest, RemoteTaskState, RemoteTaskType, Result, Tool, ToolContext,
    ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId, WonderError,
};
use wonder_of_u_storage::FleetStore;

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
    /// Agent name (fleet-compatible; mapped to `FleetMemberRequest::name`).
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
    /// Working directory override (fleet-compatible; mapped to
    /// `FleetMemberRequest::cwd`).
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
            return Err(WonderError::validation(
                "agent source-compatible `subagent_type` is not supported in the Rust runtime",
            ));
        }
        if matches!(self.run_in_background, Some(false)) {
            return Err(WonderError::validation(
                "agent source-compatible `run_in_background=false` is not supported because the Rust runtime only queues background agent tasks",
            ));
        }

        if self.isolation.as_deref() == Some("remote") {
            return Err(WonderError::validation(
                RemoteTaskState::deferred(RemoteTaskType::RemoteAgent, None).start_error_message(),
            ));
        }

        // team_name, mode, and non-remote isolation are still unsupported.
        for (field, is_present) in [
            ("team_name", self.team_name.is_some()),
            ("mode", self.mode.is_some()),
            ("isolation", self.isolation.is_some()),
        ] {
            if is_present {
                return Err(WonderError::validation(format!(
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
                            "agent name forwarded to the fleet dispatch request",
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
                            "working directory override forwarded to the fleet dispatch request",
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

        let request_id = queue_fleet_member_request(&app_root()?, &input)?;
        let mut result = ToolResult::success(
            use_id,
            format!("agent task queued for fleet dispatch: {request_id}"),
        );
        result.metadata = json!({
            "request_id": request_id,
            "status": "pending_dispatch",
            "run_in_background": true,
            "description": input.description,
            "dispatch_hint": "run `fleet dispatch` to launch this agent",
            "supports_send_message": false,
            "supports_team_name": false,
        });
        Ok(result)
    }
}

/// Queues an agent request as a [`FleetMemberRequest`] pending file.
///
/// Returns the request UUID string so callers can include it in tool metadata.
fn queue_fleet_member_request(app_root: &std::path::Path, input: &AgentInput) -> Result<String> {
    let mut request = FleetMemberRequest::new(input.prompt.clone());
    request.description = input.description.clone();
    request.name = input.name.clone();
    request.model = input.model.clone();
    request.cwd = input.cwd.as_deref().map(std::path::PathBuf::from);

    let store = FleetStore::new(app_root);
    store.queue_member_request(&request)?;
    Ok(request.id)
}

#[cfg(test)]
mod tests {
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
    fn agent_queues_pending_request_file() {
        let dir = unique_test_dir("tools-agent");
        let request_id = queue_fleet_member_request(
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

        let store = FleetStore::new(&dir);
        let pending = store.list_pending_requests().expect("list");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, request_id);
        assert_eq!(pending[0].prompt, "review the patch");
        assert_eq!(pending[0].model.as_deref(), Some("demo"));
    }

    #[test]
    fn queued_agent_request_records_metadata() {
        let dir = unique_test_dir("tools-agent-request");
        let request_id = queue_fleet_member_request(
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

        let store = FleetStore::new(&dir);
        let req = store.read_pending_request(&request_id).expect("read");
        assert_eq!(req.description.as_deref(), Some("review task"));
        assert_eq!(req.prompt, "review");
    }

    #[test]
    fn agent_name_and_cwd_are_now_accepted() {
        let tool = AgentTool;
        // name and cwd are forwarded to the fleet request — they should no
        // longer raise a validation error.
        tool.validate_input(&json!({
            "prompt": "review",
            "name": "reviewer",
            "cwd": "/tmp/project",
        }))
        .expect("name and cwd should be accepted");
    }

    #[test]
    fn agent_validation_rejects_unsupported_source_fields() {
        let tool = AgentTool;
        let error = tool
            .validate_input(&json!({
                "prompt": "review",
                "team_name": "alpha",
            }))
            .expect_err("unsupported team_name");

        assert!(error.to_string().contains("team_name"));
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
