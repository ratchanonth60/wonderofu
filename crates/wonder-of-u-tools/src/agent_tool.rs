//! Agent task queue tool.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{
    AgentCatalog, AgentDefinitionSource, FeatureFlag, FleetMemberRequest,
    RemoteTaskState, RemoteTaskType, Result, TaskId, Tool, ToolContext, ToolKind, ToolResult,
    ToolSchema, ToolSpec, ToolUseId, WonderError,
    agent_loader::AgentDefinitionLoader,
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
    /// Task ids this agent should wait for before executing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<String>>,
}

impl AgentInput {
    /// Validates structural fields that can be checked without the agent catalog.
    ///
    /// `subagent_type` is intentionally **not** checked here because resolving
    /// it requires the full project-level catalog (including custom definitions
    /// from `.claude/agents/` and `agents/`), which requires filesystem access.
    /// Catalog resolution happens in the [`AgentTool::execute`] path via
    /// [`queue_fleet_member_request_with_catalog`].
    fn validate(&self) -> Result<()> {
        require_non_empty_text("agent", "prompt", &self.prompt)?;
        if let Some(description) = &self.description {
            require_non_empty_text("agent", "description", description)?;
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
                            "upstream-compatible agent type id or display name; resolved against \
                             the active agent catalog (built-ins + project definitions)",
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
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<AgentInput>("agent", &input)?;
        input.validate()?;

        // Build the full catalog: built-ins + project definitions from cwd.
        let loader = AgentDefinitionLoader::new(&context.cwd);
        let (catalog, _warnings) = loader.build_catalog()?;

        let request_id = queue_fleet_member_request_with_catalog(&app_root()?, &input, &catalog)?;
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

/// Queues an agent request as a [`FleetMemberRequest`] pending file, using
/// the built-in catalog only.
///
/// Used by tests that do not need project-level definition loading.
#[cfg(test)]
fn queue_fleet_member_request(app_root: &std::path::Path, input: &AgentInput) -> Result<String> {
    let catalog = AgentCatalog::builtin();
    queue_fleet_member_request_with_catalog(app_root, input, &catalog)
}

/// Queues an agent request as a [`FleetMemberRequest`] pending file.
///
/// Accepts an explicit `catalog` so the caller controls which definitions are
/// available for `subagent_type` resolution (built-ins only, or built-ins +
/// project).
///
/// # Snapshot
///
/// When `input.subagent_type` resolves to a known definition, an
/// [`AgentDefinitionSnapshot`] capturing the effective fields is stored on the
/// [`FleetMemberRequest`].  This ensures the dispatcher always has a stable
/// copy of the configuration even if the source file is later edited or deleted.
///
/// If no `subagent_type` is provided, the `general-purpose` built-in is used
/// as the default definition for the snapshot's `definition_id`, but its
/// system prompt is **not** prepended automatically — the caller-supplied
/// prompt is used verbatim.
///
/// Returns the request UUID string so callers can include it in tool metadata.
pub fn queue_fleet_member_request_with_catalog(
    app_root: &std::path::Path,
    input: &AgentInput,
    catalog: &AgentCatalog,
) -> Result<String> {
    let mut request = FleetMemberRequest::new(input.prompt.clone());
    request.description = input.description.clone();
    request.name = input.name.clone();
    request.model = input.model.clone();
    request.cwd = input.cwd.as_deref().map(std::path::PathBuf::from);

    // Inherit the running fleet context from env vars so child agents are
    // automatically associated with the same fleet run.
    if request.fleet_id.is_none() {
        if let Ok(fleet_id_str) = std::env::var("WONDER_OF_U_FLEET_ID") {
            if let Ok(fleet_id) = fleet_id_str.parse() {
                request.fleet_id = Some(fleet_id);
            }
        }
    }
    // Record the spawning task as the parent so the lineage can be traced.
    if let Ok(task_id_str) = std::env::var("WONDER_OF_U_TASK_ID") {
        if let Ok(task_id) = task_id_str.parse::<TaskId>() {
            request.parent_task_id = Some(task_id);
        }
    }

    // Forward the caller-specified tool list so the sub-agent is constrained
    // to the same (or a subset of) tools the parent agent was allowed.
    if let Some(tools) = input.tools.clone() {
        if !tools.is_empty() {
            request.allowed_tools = Some(tools);
        }
    }

    // Forward dependency list for scheduling.
    request.depends_on = input.depends_on.clone().unwrap_or_default();

    // Resolve subagent_type → definition + snapshot.
    //
    // When subagent_type is explicitly provided, it must resolve to a known
    // definition; unknown types are rejected with a helpful catalog listing.
    // When omitted, `general-purpose` is used for the snapshot metadata only
    // (the caller's prompt is used verbatim — no preamble is prepended).
    let resolved_def = if let Some(ref subagent_type) = input.subagent_type {
        match catalog.resolve_alias(subagent_type) {
            Some(def) => Some(def),
            None => {
                return Err(WonderError::validation(format!(
                    "agent `subagent_type` value `{subagent_type}` is not recognised; \
                     known definitions: {}",
                    catalog.known_ids_display()
                )));
            }
        }
    } else {
        // Default to general-purpose for snapshot metadata when no type is given.
        catalog.get("general-purpose")
    };

    if let Some(def) = resolved_def {
        // Store the stable role id on the request for the dispatcher.
        request.role = Some(def.id.clone());

        // Snapshot the effective definition so the dispatcher never needs to
        // re-resolve from disk.
        request.definition_snapshot = Some(def.snapshot());

        // For built-in roles that originated from FleetAgentRole, also honour
        // the model override from the definition if the caller didn't provide one.
        if request.model.is_none() && def.source == AgentDefinitionSource::Builtin {
            request.model = def.model.clone();
        }
    }

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
                depends_on: None,
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
                depends_on: None,
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
    fn agent_accepts_known_subagent_type_via_catalog() {
        let dir = unique_test_dir("tools-agent-known-type");
        let catalog = AgentCatalog::builtin();

        // Exact id.
        queue_fleet_member_request_with_catalog(
            &dir,
            &AgentInput {
                prompt: "implement the feature".into(),
                description: None,
                subagent_type: Some("rust-engineer".into()),
                model: None,
                run_in_background: None,
                name: None,
                team_name: None,
                mode: None,
                isolation: None,
                cwd: None,
                tools: None,
                depends_on: None,
            },
            &catalog,
        )
        .expect("known subagent_type should be accepted");
    }

    #[test]
    fn agent_accepts_display_name_alias_via_catalog() {
        let dir = unique_test_dir("tools-agent-display-name");
        let catalog = AgentCatalog::builtin();

        queue_fleet_member_request_with_catalog(
            &dir,
            &AgentInput {
                prompt: "review this diff".into(),
                description: None,
                subagent_type: Some("Code Reviewer".into()),
                model: None,
                run_in_background: None,
                name: None,
                team_name: None,
                mode: None,
                isolation: None,
                cwd: None,
                tools: None,
                depends_on: None,
            },
            &catalog,
        )
        .expect("display-name alias should be accepted");
    }

    #[test]
    fn agent_accepts_short_alias_via_catalog() {
        let dir = unique_test_dir("tools-agent-short-alias");
        let catalog = AgentCatalog::builtin();

        queue_fleet_member_request_with_catalog(
            &dir,
            &AgentInput {
                prompt: "review this diff".into(),
                description: None,
                subagent_type: Some("code-review".into()),
                model: None,
                run_in_background: None,
                name: None,
                team_name: None,
                mode: None,
                isolation: None,
                cwd: None,
                tools: None,
                depends_on: None,
            },
            &catalog,
        )
        .expect("short alias should be accepted");
    }

    #[test]
    fn agent_rejects_unknown_subagent_type_via_catalog() {
        let dir = unique_test_dir("tools-agent-unknown-type");
        let catalog = AgentCatalog::builtin();

        let error = queue_fleet_member_request_with_catalog(
            &dir,
            &AgentInput {
                prompt: "review".into(),
                description: None,
                subagent_type: Some("completely-unknown-agent".into()),
                model: None,
                run_in_background: None,
                name: None,
                team_name: None,
                mode: None,
                isolation: None,
                cwd: None,
                tools: None,
                depends_on: None,
            },
            &catalog,
        )
        .expect_err("unknown subagent_type should be rejected");

        let msg = error.to_string();
        assert!(
            msg.contains("completely-unknown-agent"),
            "error should mention the unknown value; got: {msg}"
        );
        // The error should list known roles so the caller can fix the input.
        assert!(
            msg.contains("rust-engineer"),
            "error should list known roles; got: {msg}"
        );
    }

    #[test]
    fn agent_accepts_custom_project_defined_type() {
        use std::fs;
        let root = unique_test_dir("tools-agent-custom-type");
        let agents_dir = root.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("my-scout.md"),
            "---\nname: My Scout\ndescription: Explores the codebase\n---\n\nYou are a scout.",
        )
        .unwrap();

        let loader = AgentDefinitionLoader::new(&root);
        let (catalog, _) = loader.build_catalog().unwrap();

        let dir = unique_test_dir("tools-agent-custom-type-store");
        queue_fleet_member_request_with_catalog(
            &dir,
            &AgentInput {
                prompt: "explore the repo".into(),
                description: None,
                subagent_type: Some("my-scout".into()),
                model: None,
                run_in_background: None,
                name: None,
                team_name: None,
                mode: None,
                isolation: None,
                cwd: None,
                tools: None,
                depends_on: None,
            },
            &catalog,
        )
        .expect("known custom subagent_type should be accepted");

        let store = FleetStore::new(&dir);
        let pending = store.list_pending_requests().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].role.as_deref(), Some("my-scout"));
        let snap = pending[0]
            .definition_snapshot
            .as_ref()
            .expect("snapshot should be set for resolved custom type");
        assert_eq!(snap.definition_id, "my-scout");
        assert_eq!(
            snap.source,
            Some(wonder_of_u_core::AgentDefinitionSource::Project)
        );
        assert!(snap.content_hash.is_some(), "project defs should have a content hash");
    }

    #[test]
    fn queued_request_snapshot_contains_effective_fields() {
        let dir = unique_test_dir("tools-agent-snapshot");
        let catalog = AgentCatalog::builtin();

        let request_id = queue_fleet_member_request_with_catalog(
            &dir,
            &AgentInput {
                prompt: "implement the feature".into(),
                description: None,
                subagent_type: Some("rust-engineer".into()),
                model: None,
                run_in_background: None,
                name: None,
                team_name: None,
                mode: None,
                isolation: None,
                cwd: None,
                tools: None,
                depends_on: None,
            },
            &catalog,
        )
        .expect("queue task");

        let store = FleetStore::new(&dir);
        let req = store.read_pending_request(&request_id).unwrap();
        let snap = req
            .definition_snapshot
            .as_ref()
            .expect("snapshot should be present for resolved subagent_type");
        assert_eq!(snap.definition_id, "rust-engineer");
        assert_eq!(
            snap.source,
            Some(wonder_of_u_core::AgentDefinitionSource::Builtin)
        );
        assert!(
            snap.system_prompt.as_ref().map(|p| !p.is_empty()).unwrap_or(false),
            "system_prompt should be non-empty"
        );
        // rust-engineer allows bash, file_read, etc.
        assert!(!snap.allowed_tools.is_empty(), "allowed_tools should be populated");
    }

    #[test]
    fn no_subagent_type_defaults_to_general_purpose_snapshot() {
        let dir = unique_test_dir("tools-agent-default-snap");
        let catalog = AgentCatalog::builtin();

        let request_id = queue_fleet_member_request_with_catalog(
            &dir,
            &AgentInput {
                prompt: "do something".into(),
                description: None,
                subagent_type: None,
                model: None,
                run_in_background: None,
                name: None,
                team_name: None,
                mode: None,
                isolation: None,
                cwd: None,
                tools: None,
                depends_on: None,
            },
            &catalog,
        )
        .expect("queue task");

        let store = FleetStore::new(&dir);
        let req = store.read_pending_request(&request_id).unwrap();
        let snap = req
            .definition_snapshot
            .as_ref()
            .expect("snapshot should default to general-purpose");
        assert_eq!(snap.definition_id, "general-purpose");
    }

    #[test]
    fn subagent_type_alias_stored_as_role_id() {
        let dir = unique_test_dir("tools-agent-subagent-type");
        let catalog = AgentCatalog::builtin();

        let request_id = queue_fleet_member_request_with_catalog(
            &dir,
            &AgentInput {
                prompt: "review the patch".into(),
                description: None,
                subagent_type: Some("Rust Engineer".into()),
                model: None,
                run_in_background: None,
                name: None,
                team_name: None,
                mode: None,
                isolation: None,
                cwd: None,
                tools: None,
                depends_on: None,
            },
            &catalog,
        )
        .expect("queue task");

        let store = FleetStore::new(&dir);
        let req = store.read_pending_request(&request_id).expect("read");
        // Alias "Rust Engineer" should resolve to role id "rust-engineer".
        assert_eq!(req.role.as_deref(), Some("rust-engineer"));
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

