//! Agent task queue tool.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{
    AgentCatalog, AgentDefinitionSource, AgentLaunchSpec, FeatureFlag, FleetMemberRequest,
    PermissionMode, PermissionRuleBehavior, PermissionRuleSource, RemoteTaskState, RemoteTaskType,
    Result, TaskId, Tool, ToolContext, ToolEffect, ToolKind, ToolResult, ToolSchema, ToolSpec,
    ToolUseId, WONDER_OF_U_FORK_DEPTH_ENV, WonderError, WorktreeIsolation, WorktreeIsolationMode,
    agent_loader::AgentDefinitionLoader, get_git_root,
};
use wonder_of_u_storage::FleetStore;

use crate::{base_spec, parse_input, require_non_empty_text};
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

        // Validate mode: only "fork" is accepted; other values are rejected.
        match self.mode.as_deref() {
            None | Some("fork") => {}
            Some(other) => {
                return Err(WonderError::validation(format!(
                    "agent `mode` value `{other}` is not supported in the Rust runtime; \
                     accepted values: \"fork\""
                )));
            }
        }

        // fork + isolation is explicitly unsupported for now.
        if self.mode.as_deref() == Some("fork") && self.isolation.is_some() {
            return Err(WonderError::validation(
                "agent `mode=fork` combined with `isolation` is not currently supported; \
                 remove `isolation` to use fork mode",
            ));
        }

        // Only "worktree" isolation is accepted; any other value is rejected.
        if let Some(ref iso) = self.isolation {
            if iso != "worktree" {
                return Err(WonderError::validation(format!(
                    "agent `isolation` value `{iso}` is not supported; \
                     accepted value: \"worktree\""
                )));
            }
        }

        // team_name is still unsupported.
        if self.team_name.is_some() {
            return Err(WonderError::validation(
                "agent source-compatible `team_name` is not supported in the Rust runtime",
            ));
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
                            "fork-lite mode: `\"fork\"` inherits parent session context for the child subprocess",
                            ["fork"],
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

        // Fork-mode guards: check depth and context before building the request.
        let is_fork = input.mode.as_deref() == Some("fork");
        let is_worktree = input.isolation.as_deref() == Some("worktree");

        if is_fork {
            // Recursive fork guard: reject if already inside a fork subprocess.
            let parent_depth: u32 = std::env::var(WONDER_OF_U_FORK_DEPTH_ENV)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if parent_depth > 0 {
                return Err(WonderError::validation(format!(
                    "recursive fork rejected: {WONDER_OF_U_FORK_DEPTH_ENV}={parent_depth}; \
                     nested fork subagents are not supported"
                )));
            }
            // Fork context must be populated by the runtime in ToolContext.
            if context.fork_context.is_none() {
                return Err(WonderError::validation(
                    "mode=fork requires an active session with fork context; \
                     fork context was not populated in this runtime (no active prompt or TUI session)",
                ));
            }
        }

        // Bypass-propagation guard: prevent BypassPermissions from silently
        // escalating into spawned background/child agents.  A child agent
        // launched without any permission context would inherit an effectively
        // unrestricted posture, bypassing normal sandboxing for every tool it
        // calls.  We block the spawn unless the operator has placed an explicit
        // Policy-level allow rule on the "agent" tool — i.e. a deliberate,
        // auditable decision that bypass propagation is acceptable in this
        // deployment.
        if context.permission_mode == PermissionMode::BypassPermissions {
            let policy_allows = context.permission_rules.iter().any(|rule| {
                rule.source == PermissionRuleSource::Policy
                    && rule.behavior == PermissionRuleBehavior::Allow
                    && matches!(rule.tool.as_str(), "agent" | "Task" | "*")
            });
            if !policy_allows {
                return Err(WonderError::validation(
                    "BypassPermissions cannot propagate to spawned background agents; \
                     add a Policy-level allow rule for 'agent' to explicitly permit \
                     subagent launch under bypass mode",
                ));
            }
        }

        // Worktree isolation requires a git repo.  Fail explicitly here rather
        // than letting the error surface later in the dispatcher where it would
        // be harder to attribute.
        if is_worktree && get_git_root(&context.cwd).is_err() {
            return Err(WonderError::validation(format!(
                "agent isolation=worktree requires a git repository; \
                 `{}` is not inside one. Run from a git repo or remove `isolation`.",
                context.cwd.display()
            )));
        }

        // Build the full catalog from the project root when available so a tool
        // call from a nested cwd can still see repo-level `.claude/agents/`.
        let definition_root = get_git_root(&context.cwd).unwrap_or_else(|_| context.cwd.clone());
        let loader = AgentDefinitionLoader::new(&definition_root);
        let (catalog, warnings) = loader.build_catalog()?;

        // Build the fully-populated request (snapshot, lineage, tools) but do
        // NOT write any pending file here.  The runtime processes the returned
        // ToolEffect::LaunchAgentTask and decides whether to:
        //   a) directly call TaskManager::start_agent_task (preferred), or
        //   b) write a pending FleetMemberRequest file as fallback.
        let mut request = build_fleet_member_request_with_catalog(&input, &catalog)?;

        // Embed fork context into the request so the dispatcher can compose the
        // child system prompt and propagate WONDER_OF_U_FORK_DEPTH.
        if is_fork {
            request.fork_context = context.fork_context.clone();
        }

        let request_id = request.id.clone();

        // Pre-allocate a stable TaskId so callers can derive deterministic
        // output paths before the task is actually started.  The runtime
        // MUST honour this id when creating the task record (see
        // `AgentLaunchSpec::reserved_task_id`).
        let reserved_task_id = TaskId::new();

        // Compute the output paths from the app root.  Failures are
        // non-fatal: paths are best-effort metadata; the task still launches.
        let output_paths = crate::app_root()
            .ok()
            .map(|root| {
                let paths = wonder_of_u_storage::StoragePaths::new(&root);
                serde_json::json!({
                    "output_log": paths.task_log_path(reserved_task_id),
                    "result":     paths.task_result_path(reserved_task_id),
                })
            })
            .unwrap_or(serde_json::Value::Null);

        let effect = ToolEffect::LaunchAgentTask(AgentLaunchSpec {
            request,
            reserved_task_id: Some(reserved_task_id),
        });

        let mut result =
            ToolResult::success(use_id, format!("agent task launch requested: {request_id}"));
        result.metadata = serde_json::json!({
            "request_id": request_id,
            "reserved_task_id": reserved_task_id.to_string(),
            "output_paths": output_paths,
            "status": "launch_requested",
            "run_in_background": true,
            "mode": input.mode,
            "fork_mode": is_fork,
            "isolation": input.isolation,
            "worktree_isolation": is_worktree,
            "description": input.description,
            "dispatch_hint": "runtime will launch directly or fall back to `fleet dispatch`",
            "supports_send_message": false,
            "supports_team_name": false,
            "definition_root": definition_root,
            "agent_definition_warnings": warnings.iter().map(|w| w.message()).collect::<Vec<_>>(),
        });
        result.effects = vec![effect];
        Ok(result)
    }
}

/// Queues an agent request as a [`FleetMemberRequest`] pending file.
///
/// Used by tests that do not need project-level definition loading.
#[cfg(test)]
fn queue_fleet_member_request(app_root: &std::path::Path, input: &AgentInput) -> Result<String> {
    let catalog = AgentCatalog::builtin();
    queue_fleet_member_request_with_catalog(app_root, input, &catalog)
}

/// Builds a [`FleetMemberRequest`] from an `AgentInput` and catalog **without
/// writing any pending file to disk**.
///
/// This is the pure-data construction step used by:
/// - [`AgentTool::execute`] (returns the request as a [`ToolEffect`]),
/// - [`queue_fleet_member_request_with_catalog`] (builds then writes).
///
/// # Snapshot
///
/// When `input.subagent_type` resolves to a known definition an
/// [`AgentDefinitionSnapshot`] is embedded in the returned request.  If no
/// `subagent_type` is provided the `general-purpose` built-in is used as the
/// default and is snapshotted for the dispatcher.
pub fn build_fleet_member_request_with_catalog(
    input: &AgentInput,
    catalog: &AgentCatalog,
) -> Result<FleetMemberRequest> {
    validate_subagent_type(input, catalog)?;

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

    // Translate the input isolation string into the typed WorktreeIsolation.
    // Only "worktree" is accepted (validated earlier in validate()).
    if input.isolation.as_deref() == Some("worktree") {
        request.isolation = Some(WorktreeIsolation {
            mode: WorktreeIsolationMode::Worktree,
            branch: None,
        });
    }

    // Resolve subagent_type → definition + snapshot.
    let resolved_def = if let Some(ref subagent_type) = input.subagent_type {
        catalog.resolve_alias(subagent_type)
    } else {
        // Default to general-purpose for snapshot metadata when no type is given.
        catalog.get("general-purpose")
    };

    if let Some(def) = resolved_def {
        request.role = Some(def.id.clone());
        request.definition_snapshot = Some(def.snapshot());
        // For built-in roles honour the model override from the definition
        // if the caller didn't provide one.
        if request.model.is_none() && def.source == AgentDefinitionSource::Builtin {
            request.model = def.model.clone();
        }
    }

    Ok(request)
}

fn validate_subagent_type(input: &AgentInput, catalog: &AgentCatalog) -> Result<()> {
    if let Some(ref subagent_type) = input.subagent_type {
        if catalog.resolve_alias(subagent_type).is_none() {
            return Err(WonderError::validation(format!(
                "agent `subagent_type` value `{subagent_type}` is not recognised; \
                 known definitions: {}",
                catalog.known_ids_display()
            )));
        }
    }
    Ok(())
}

/// Queues an agent request as a [`FleetMemberRequest`] pending file.
///
/// Accepts an explicit `catalog` so the caller controls which definitions are
/// available for `subagent_type` resolution (built-ins only, or built-ins +
/// project).
///
/// When `input.subagent_type` is explicitly provided and does **not** resolve
/// to a known definition the call returns a validation error with a catalog
/// listing hint.
///
/// Returns the request UUID string so callers can include it in tool metadata.
pub fn queue_fleet_member_request_with_catalog(
    app_root: &std::path::Path,
    input: &AgentInput,
    catalog: &AgentCatalog,
) -> Result<String> {
    let request = build_fleet_member_request_with_catalog(input, catalog)?;
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
        assert!(
            snap.content_hash.is_some(),
            "project defs should have a content hash"
        );
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
            snap.system_prompt
                .as_ref()
                .map(|p| !p.is_empty())
                .unwrap_or(false),
            "system_prompt should be non-empty"
        );
        // rust-engineer allows bash, file_read, etc.
        assert!(
            !snap.allowed_tools.is_empty(),
            "allowed_tools should be populated"
        );
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

    // ── AgentTool::execute returns ToolEffect::LaunchAgentTask ────────────────

    /// Verify that execute() returns a LaunchAgentTask effect and does NOT
    /// write any pending file.  This is the core contract of the new design:
    /// the tool is pure – it delegates side-effects to the runtime.
    #[test]
    fn agent_execute_returns_launch_effect_without_writing_pending_file() {
        use wonder_of_u_core::{FeatureSet, PermissionMode, SessionId, ToolEffect};

        let dir = unique_test_dir("tools-agent-execute-effect");
        // ToolContext with cwd pointing at our temp dir (not a git repo).
        let context = wonder_of_u_core::ToolContext {
            session_id: SessionId::new(),
            cwd: dir.clone(),
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: vec![],
            provider: None,
            model: None,
            permission_rules: vec![],
            features: FeatureSet::first_release(),
            bash_session_store: None,
            fork_context: None,
        };
        let use_id = wonder_of_u_core::ToolUseId::new();
        let input = json!({ "prompt": "review this code" });
        let tool = AgentTool;

        let result = futures::executor::block_on(tool.execute(context, use_id, input))
            .expect("execute should succeed");

        // Must succeed and carry exactly one LaunchAgentTask effect.
        assert!(result.success, "result should be success");
        assert_eq!(result.effects.len(), 1, "expected exactly one effect");
        let ToolEffect::LaunchAgentTask(ref spec) = result.effects[0] else {
            panic!(
                "expected LaunchAgentTask effect, got: {:?}",
                result.effects[0]
            );
        };
        assert_eq!(spec.request.prompt, "review this code");

        // Metadata status must be "launch_requested", not "pending_dispatch".
        let status = result.metadata["status"].as_str().unwrap_or("");
        assert_eq!(
            status, "launch_requested",
            "metadata.status should be launch_requested, got: {status}"
        );

        // No pending file should have been written — no FleetStore on dir.
        let store = FleetStore::new(&dir);
        assert!(
            store.list_pending_requests().unwrap().is_empty(),
            "execute() must not write a pending file"
        );
    }

    /// build_fleet_member_request_with_catalog constructs the request but does
    /// not touch the filesystem — a FleetStore on the same dir stays empty.
    #[test]
    fn build_helper_does_not_write_pending_file() {
        let dir = unique_test_dir("tools-agent-build-no-write");
        let catalog = AgentCatalog::builtin();
        let input = AgentInput {
            prompt: "just build".into(),
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
        };

        let request =
            build_fleet_member_request_with_catalog(&input, &catalog).expect("build request");
        assert_eq!(request.prompt, "just build");

        // definition_snapshot defaults to general-purpose.
        let snap = request.definition_snapshot.as_ref().unwrap();
        assert_eq!(snap.definition_id, "general-purpose");

        // No file written.
        let store = FleetStore::new(&dir);
        assert!(store.list_pending_requests().unwrap().is_empty());
    }

    /// The queue helper (used for fallback and backward-compat) still writes a
    /// file even after the refactor.
    #[test]
    fn queue_helper_still_writes_pending_file_after_refactor() {
        let dir = unique_test_dir("tools-agent-queue-still-writes");
        let catalog = AgentCatalog::builtin();
        let input = AgentInput {
            prompt: "queue me".into(),
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
        };

        let request_id =
            queue_fleet_member_request_with_catalog(&dir, &input, &catalog).expect("queue");

        let store = FleetStore::new(&dir);
        let pending = store.list_pending_requests().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, request_id);
    }

    #[test]
    fn agent_execute_rejects_unknown_subagent_type() {
        use wonder_of_u_core::{FeatureSet, PermissionMode, SessionId};

        let dir = unique_test_dir("tools-agent-execute-unknown-type");
        let context = wonder_of_u_core::ToolContext {
            session_id: SessionId::new(),
            cwd: dir,
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: vec![],
            provider: None,
            model: None,
            permission_rules: vec![],
            features: FeatureSet::first_release(),
            bash_session_store: None,
            fork_context: None,
        };
        let tool = AgentTool;
        let error = futures::executor::block_on(tool.execute(
            context,
            wonder_of_u_core::ToolUseId::new(),
            json!({
                "prompt": "review",
                "subagent_type": "does-not-exist",
            }),
        ))
        .expect_err("unknown subagent_type should be rejected");

        let message = error.to_string();
        assert!(message.contains("does-not-exist"), "got: {message}");
        assert!(message.contains("known definitions"), "got: {message}");
    }

    // ── Fork mode validation and execute tests ────────────────────────────────

    /// validate() accepts mode=fork.
    #[test]
    fn agent_validation_accepts_fork_mode() {
        let tool = AgentTool;
        tool.validate_input(&json!({
            "prompt": "do something as a fork",
            "mode": "fork",
        }))
        .expect("mode=fork should be accepted by validate_input");
    }

    /// The machine-readable schema must not advertise source modes that the
    /// Rust runtime rejects.
    #[test]
    fn agent_schema_mode_enum_only_advertises_fork() {
        let spec = AgentTool.spec();
        let mode_enum = spec.input_schema["properties"]["mode"]["enum"]
            .as_array()
            .expect("mode enum should be an array");

        let values = mode_enum
            .iter()
            .map(|value| value.as_str().expect("enum values are strings"))
            .collect::<Vec<_>>();
        assert_eq!(values, vec!["fork"]);
    }

    /// validate() rejects unknown mode values.
    #[test]
    fn agent_validation_rejects_unknown_mode() {
        let tool = AgentTool;
        let error = tool
            .validate_input(&json!({
                "prompt": "review",
                "mode": "acceptEdits",
            }))
            .expect_err("unsupported mode should be rejected");

        let msg = error.to_string();
        assert!(
            msg.contains("acceptEdits"),
            "error should mention mode value; got: {msg}"
        );
        assert!(
            msg.contains("fork"),
            "error should hint at accepted values; got: {msg}"
        );
    }

    /// validate() rejects mode=fork combined with isolation.
    #[test]
    fn agent_validation_rejects_fork_plus_isolation() {
        let tool = AgentTool;
        let error = tool
            .validate_input(&json!({
                "prompt": "review",
                "mode": "fork",
                "isolation": "worktree",
            }))
            .expect_err("fork+isolation should be rejected");

        let msg = error.to_string();
        assert!(
            msg.contains("isolation"),
            "error should mention isolation; got: {msg}"
        );
    }

    /// execute() rejects mode=fork when ToolContext has no fork_context.
    #[test]
    fn agent_execute_rejects_fork_without_context() {
        use wonder_of_u_core::{FeatureSet, PermissionMode, SessionId};

        let dir = unique_test_dir("tools-agent-fork-no-ctx");
        let context = wonder_of_u_core::ToolContext {
            session_id: SessionId::new(),
            cwd: dir,
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: vec![],
            provider: None,
            model: None,
            permission_rules: vec![],
            features: FeatureSet::first_release(),
            bash_session_store: None,
            fork_context: None, // no fork context provided
        };
        let tool = AgentTool;
        let error = futures::executor::block_on(tool.execute(
            context,
            wonder_of_u_core::ToolUseId::new(),
            json!({ "prompt": "fork task", "mode": "fork" }),
        ))
        .expect_err("fork without context should fail");

        let msg = error.to_string();
        assert!(
            msg.contains("fork context was not populated"),
            "error should mention missing fork context; got: {msg}"
        );
    }

    /// execute() with mode=fork and a populated fork_context embeds it in the
    /// LaunchAgentTask effect's request.
    #[test]
    fn agent_execute_fork_mode_embeds_fork_context() {
        use wonder_of_u_core::{
            FeatureSet, ForkContextSnapshot, PermissionMode, SessionId, ToolEffect,
        };

        let dir = unique_test_dir("tools-agent-fork-embeds-ctx");
        let fork_ctx = ForkContextSnapshot {
            parent_session_id: "test-session-123".into(),
            fork_depth: 0,
            parent_system_prompt: Some("You are a helpful assistant.".into()),
            conversation_summary: Some("User asked to refactor the auth module.".into()),
            parent_entrypoint: Some("prompt".into()),
        };
        let context = wonder_of_u_core::ToolContext {
            session_id: SessionId::new(),
            cwd: dir,
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: vec![],
            provider: None,
            model: None,
            permission_rules: vec![],
            features: FeatureSet::first_release(),
            bash_session_store: None,
            fork_context: Some(fork_ctx.clone()),
        };
        let tool = AgentTool;
        let result = futures::executor::block_on(tool.execute(
            context,
            wonder_of_u_core::ToolUseId::new(),
            json!({ "prompt": "continue the refactor as a fork", "mode": "fork" }),
        ))
        .expect("fork with context should succeed");

        assert!(
            result.success,
            "should succeed; content: {}",
            result.content
        );
        assert_eq!(result.effects.len(), 1, "should have exactly one effect");
        let ToolEffect::LaunchAgentTask(ref spec) = result.effects[0] else {
            panic!("expected LaunchAgentTask effect");
        };
        // fork_context must be embedded in the request.
        let embedded = spec
            .request
            .fork_context
            .as_ref()
            .expect("fork_context must be set in the request");
        assert_eq!(embedded.parent_session_id, fork_ctx.parent_session_id);
        assert_eq!(embedded.parent_system_prompt, fork_ctx.parent_system_prompt);
        assert_eq!(embedded.conversation_summary, fork_ctx.conversation_summary);

        // metadata should reflect fork mode.
        assert_eq!(result.metadata["fork_mode"], true);
    }

    /// execute() with mode=fork rejects recursive forks via env var.
    #[test]
    fn agent_execute_rejects_recursive_fork_via_env_depth() {
        use wonder_of_u_core::{FeatureSet, ForkContextSnapshot, PermissionMode, SessionId};
        use wonder_of_u_test_support::EnvVarGuard;

        let dir = unique_test_dir("tools-agent-fork-recursive");
        let fork_ctx = ForkContextSnapshot {
            parent_session_id: "nested-session".into(),
            fork_depth: 1,
            parent_system_prompt: None,
            conversation_summary: None,
            parent_entrypoint: None,
        };
        let context = wonder_of_u_core::ToolContext {
            session_id: SessionId::new(),
            cwd: dir,
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: vec![],
            provider: None,
            model: None,
            permission_rules: vec![],
            features: FeatureSet::first_release(),
            bash_session_store: None,
            fork_context: Some(fork_ctx),
        };

        // Simulate being inside a fork subprocess: depth=1.
        let _guard = EnvVarGuard::set("WONDER_OF_U_FORK_DEPTH", "1");

        let tool = AgentTool;
        let error = futures::executor::block_on(tool.execute(
            context,
            wonder_of_u_core::ToolUseId::new(),
            json!({ "prompt": "nested fork attempt", "mode": "fork" }),
        ))
        .expect_err("recursive fork should be rejected");

        let msg = error.to_string();
        assert!(
            msg.contains("recursive fork rejected"),
            "error should mention recursive fork; got: {msg}"
        );
    }

    // ── Output-path metadata ──────────────────────────────────────────────────

    /// `execute()` includes `reserved_task_id` in both the metadata JSON and in
    /// the `AgentLaunchSpec` carried by the `LaunchAgentTask` effect, and the
    /// two values agree.
    #[test]
    fn agent_execute_includes_reserved_task_id_in_metadata_and_spec() {
        use wonder_of_u_core::{FeatureSet, PermissionMode, SessionId, ToolEffect};

        let dir = unique_test_dir("tools-agent-reserved-task-id");
        let context = wonder_of_u_core::ToolContext {
            session_id: SessionId::new(),
            cwd: dir,
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: vec![],
            provider: None,
            model: None,
            permission_rules: vec![],
            features: FeatureSet::first_release(),
            bash_session_store: None,
            fork_context: None,
        };

        let result = futures::executor::block_on(AgentTool.execute(
            context,
            wonder_of_u_core::ToolUseId::new(),
            json!({ "prompt": "test task" }),
        ))
        .expect("execute should succeed");

        // There must be exactly one LaunchAgentTask effect.
        assert_eq!(result.effects.len(), 1);
        let ToolEffect::LaunchAgentTask(ref spec) = result.effects[0] else {
            panic!("expected LaunchAgentTask effect");
        };

        // The spec must carry a reserved_task_id.
        let spec_task_id = spec
            .reserved_task_id
            .expect("spec must have a reserved_task_id");

        // The metadata must also carry the same id as a string.
        let meta_task_id_str = result.metadata["reserved_task_id"]
            .as_str()
            .expect("metadata.reserved_task_id must be a string");

        assert_eq!(
            spec_task_id.to_string(),
            meta_task_id_str,
            "reserved_task_id in spec and metadata must match"
        );
    }

    // ── Bypass-propagation guard ──────────────────────────────────────────────

    /// execute() rejects agent spawn when the parent runs under BypassPermissions
    /// and no Policy-level allow rule is present.
    ///
    /// This is the core of the reference-bypass-guard: a child agent should
    /// never silently inherit unrestricted permission posture from its parent.
    #[test]
    fn agent_execute_rejects_bypass_mode_without_policy_allow() {
        use wonder_of_u_core::{FeatureSet, PermissionMode, SessionId};

        let dir = unique_test_dir("tools-agent-bypass-no-policy");
        let context = wonder_of_u_core::ToolContext {
            session_id: SessionId::new(),
            cwd: dir,
            session_worktree: None,
            permission_mode: PermissionMode::BypassPermissions,
            additional_working_directories: vec![],
            provider: None,
            model: None,
            permission_rules: vec![],
            features: FeatureSet::first_release(),
            bash_session_store: None,
            fork_context: None,
        };
        let tool = AgentTool;
        let error = futures::executor::block_on(tool.execute(
            context,
            wonder_of_u_core::ToolUseId::new(),
            json!({ "prompt": "do something in bypass" }),
        ))
        .expect_err("bypass mode without policy allow should be rejected");

        let msg = error.to_string();
        assert!(
            msg.contains("BypassPermissions cannot propagate"),
            "error must explain bypass propagation risk; got: {msg}"
        );
        assert!(
            msg.contains("Policy-level allow rule"),
            "error must mention how to explicitly permit it; got: {msg}"
        );
    }

    /// execute() succeeds when the parent runs under BypassPermissions AND a
    /// Policy-level allow rule for "agent" is explicitly present.
    ///
    /// This is the designated safe-policy exception: an operator who has
    /// deliberately set a Policy allow rule acknowledges bypass propagation.
    #[test]
    fn agent_execute_allows_bypass_mode_with_policy_allow_for_agent() {
        use wonder_of_u_core::{
            FeatureSet, PermissionMode, PermissionRule, PermissionRuleBehavior,
            PermissionRuleSource, SessionId, ToolEffect,
        };

        let dir = unique_test_dir("tools-agent-bypass-with-policy");
        let policy_rule = PermissionRule::new(
            "agent",
            PermissionRuleBehavior::Allow,
            PermissionRuleSource::Policy,
        );
        let context = wonder_of_u_core::ToolContext {
            session_id: SessionId::new(),
            cwd: dir,
            session_worktree: None,
            permission_mode: PermissionMode::BypassPermissions,
            additional_working_directories: vec![],
            provider: None,
            model: None,
            permission_rules: vec![policy_rule],
            features: FeatureSet::first_release(),
            bash_session_store: None,
            fork_context: None,
        };

        let result = futures::executor::block_on(AgentTool.execute(
            context,
            wonder_of_u_core::ToolUseId::new(),
            json!({ "prompt": "do something in bypass with explicit policy" }),
        ))
        .expect("bypass with explicit Policy-level allow rule should succeed");

        assert!(result.success, "result should be success");
        assert_eq!(result.effects.len(), 1);
        assert!(
            matches!(result.effects[0], ToolEffect::LaunchAgentTask(_)),
            "should return LaunchAgentTask effect"
        );
    }

    /// execute() also accepts a wildcard `"*"` Policy allow rule as the safe
    /// exception since it explicitly covers all tools.
    #[test]
    fn agent_execute_allows_bypass_mode_with_wildcard_policy_allow() {
        use wonder_of_u_core::{
            FeatureSet, PermissionMode, PermissionRule, PermissionRuleBehavior,
            PermissionRuleSource, SessionId, ToolEffect,
        };

        let dir = unique_test_dir("tools-agent-bypass-wildcard-policy");
        let policy_rule = PermissionRule::new(
            "*",
            PermissionRuleBehavior::Allow,
            PermissionRuleSource::Policy,
        );
        let context = wonder_of_u_core::ToolContext {
            session_id: SessionId::new(),
            cwd: dir,
            session_worktree: None,
            permission_mode: PermissionMode::BypassPermissions,
            additional_working_directories: vec![],
            provider: None,
            model: None,
            permission_rules: vec![policy_rule],
            features: FeatureSet::first_release(),
            bash_session_store: None,
            fork_context: None,
        };
        let tool = AgentTool;
        let result = futures::executor::block_on(tool.execute(
            context,
            wonder_of_u_core::ToolUseId::new(),
            json!({ "prompt": "task with wildcard policy" }),
        ))
        .expect("wildcard Policy allow should satisfy bypass guard");

        assert!(result.success);
        assert!(matches!(result.effects[0], ToolEffect::LaunchAgentTask(_)));
    }

    /// execute() with a non-Policy allow rule (e.g. CliArg) does NOT satisfy
    /// the bypass guard — only Policy-sourced rules count.
    #[test]
    fn agent_execute_rejects_bypass_mode_with_non_policy_allow() {
        use wonder_of_u_core::{
            FeatureSet, PermissionMode, PermissionRule, PermissionRuleBehavior,
            PermissionRuleSource, SessionId,
        };

        let dir = unique_test_dir("tools-agent-bypass-cli-allow");
        // CliArg is not a Policy — must not bypass the guard.
        let cli_rule = PermissionRule::new(
            "agent",
            PermissionRuleBehavior::Allow,
            PermissionRuleSource::CliArg,
        );
        let context = wonder_of_u_core::ToolContext {
            session_id: SessionId::new(),
            cwd: dir,
            session_worktree: None,
            permission_mode: PermissionMode::BypassPermissions,
            additional_working_directories: vec![],
            provider: None,
            model: None,
            permission_rules: vec![cli_rule],
            features: FeatureSet::first_release(),
            bash_session_store: None,
            fork_context: None,
        };
        let tool = AgentTool;
        let error = futures::executor::block_on(tool.execute(
            context,
            wonder_of_u_core::ToolUseId::new(),
            json!({ "prompt": "bypass via cli-arg allow" }),
        ))
        .expect_err("CliArg allow must not satisfy bypass guard; only Policy does");

        let msg = error.to_string();
        assert!(
            msg.contains("BypassPermissions cannot propagate"),
            "error should reference bypass propagation; got: {msg}"
        );
    }

    /// Non-bypass permission modes (Default, AcceptEdits, DontAsk) are never
    /// subject to the guard and always proceed normally.
    #[test]
    fn agent_execute_non_bypass_modes_are_never_blocked() {
        use wonder_of_u_core::{FeatureSet, PermissionMode, SessionId, ToolEffect};

        for mode in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::DontAsk,
        ] {
            let dir = unique_test_dir("tools-agent-non-bypass-mode");
            let context = wonder_of_u_core::ToolContext {
                session_id: SessionId::new(),
                cwd: dir,
                session_worktree: None,
                permission_mode: mode,
                additional_working_directories: vec![],
                provider: None,
                model: None,
                permission_rules: vec![],
                features: FeatureSet::first_release(),
                bash_session_store: None,
                fork_context: None,
            };
            let tool = AgentTool;
            let result = futures::executor::block_on(tool.execute(
                context,
                wonder_of_u_core::ToolUseId::new(),
                json!({ "prompt": "task in normal mode" }),
            ))
            .unwrap_or_else(|e| panic!("mode {mode:?} should not be blocked; got: {e}"));

            assert!(
                result.success,
                "mode {mode:?} should produce a success result"
            );
            assert!(
                matches!(result.effects[0], ToolEffect::LaunchAgentTask(_)),
                "mode {mode:?} should yield a LaunchAgentTask effect"
            );
        }
    }

    /// `isolation=worktree` is accepted in validate(); it no longer returns an
    /// error for this valid value.
    #[test]
    fn agent_validation_accepts_isolation_worktree() {
        let tool = AgentTool;
        tool.validate_input(&json!({
            "prompt": "refactor the module",
            "isolation": "worktree",
        }))
        .expect("isolation=worktree should be accepted");
    }

    /// `isolation=remote` is still rejected because the Rust runtime has no
    /// remote execution transport.
    #[test]
    fn agent_validation_rejects_isolation_remote() {
        let tool = AgentTool;
        let err = tool
            .validate_input(&json!({
                "prompt": "do something",
                "isolation": "remote",
            }))
            .expect_err("isolation=remote should be rejected");
        assert!(
            err.to_string().to_lowercase().contains("remote"),
            "error should mention 'remote'; got: {err}"
        );
    }

    /// Unknown isolation values are rejected with a clear message listing
    /// accepted options.
    #[test]
    fn agent_validation_rejects_unknown_isolation_value() {
        let tool = AgentTool;
        let err = tool
            .validate_input(&json!({
                "prompt": "do something",
                "isolation": "container",
            }))
            .expect_err("unknown isolation should be rejected");
        assert!(
            err.to_string().contains("container"),
            "error should echo the unknown value; got: {err}"
        );
    }

    /// `mode=fork` combined with `isolation` is still rejected.
    #[test]
    fn agent_validation_rejects_fork_with_isolation() {
        let tool = AgentTool;
        let err = tool
            .validate_input(&json!({
                "prompt": "do something",
                "mode": "fork",
                "isolation": "worktree",
            }))
            .expect_err("fork + isolation should be rejected");
        assert!(
            err.to_string().contains("fork"),
            "error should mention fork; got: {err}"
        );
    }

    /// `build_fleet_member_request_with_catalog` sets `request.isolation` to
    /// `WorktreeIsolation { mode: Worktree }` when `input.isolation="worktree"`.
    #[test]
    fn build_request_sets_isolation_for_worktree() {
        let catalog = AgentCatalog::builtin();
        let input = AgentInput {
            prompt: "do some work".into(),
            description: None,
            subagent_type: None,
            model: None,
            run_in_background: None,
            name: None,
            team_name: None,
            mode: None,
            isolation: Some("worktree".into()),
            cwd: None,
            tools: None,
            depends_on: None,
        };
        let request =
            build_fleet_member_request_with_catalog(&input, &catalog).expect("build request");
        let iso = request.isolation.expect("isolation should be set");
        assert_eq!(
            iso.mode,
            WorktreeIsolationMode::Worktree,
            "isolation mode should be Worktree"
        );
        assert!(iso.branch.is_none(), "branch override should be None");
    }

    /// When `input.isolation` is `None` the request carries no isolation.
    #[test]
    fn build_request_no_isolation_when_not_requested() {
        let catalog = AgentCatalog::builtin();
        let input = AgentInput {
            prompt: "do some work".into(),
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
        };
        let request =
            build_fleet_member_request_with_catalog(&input, &catalog).expect("build request");
        assert!(
            request.isolation.is_none(),
            "isolation should be None when not requested"
        );
    }
}
