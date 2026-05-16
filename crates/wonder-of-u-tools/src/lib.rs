//! Provides wonder of u tools support
//!

#![warn(missing_docs)]

mod agent_tool;
mod ask_user_tool;
mod bash;
mod communication_tools;
mod cron_remote;
mod extended;
mod files;
mod orchestration;
mod plan_tool;
mod search;
mod source_compat;
mod special_tools;
mod task_tools;
mod todo_tool;
mod web;
mod worktree_tools;

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::de::DeserializeOwned;
use serde_json::Value;
use wonder_of_u_core::{FeatureFlag, Result, Tool, ToolKind, ToolRegistry, ToolSpec, WonderError};

/// Re-exports items from `agent_tool`
pub use agent_tool::{AgentInput, AgentTool};
/// Re-exports items from `ask_user_tool`
pub use ask_user_tool::{AskUserInput, AskUserTool};
/// Re-exports items from `bash`
pub use bash::{BashInput, BashTool};
/// Re-exports items from `communication_tools`
pub use communication_tools::{
    ListPeersInput, ListPeersTool, SendMessageInput, SendMessageTool, TeamCreateInput,
    TeamCreateTool, TeamDeleteInput, TeamDeleteTool,
};
/// Re-exports items from `cron_remote`
pub use cron_remote::{
    CronCreateInput, CronCreateTool, CronDeleteInput, CronDeleteTool, RemoteTriggerAction,
    RemoteTriggerInput, RemoteTriggerTool,
};
/// Re-exports items from `extended`
pub use extended::{
    CronListTool, NotebookEditInput, NotebookEditTool, PowerShellInput, PowerShellTool,
    TerminalCaptureTool, WorktreeListTool,
};
/// Re-exports items from `files`
pub use files::{
    FileEditInput, FileEditTool, FileReadInput, FileReadTool, FileWriteInput, FileWriteMode,
    FileWriteTool,
};
/// Re-exports items from `orchestration`
pub use orchestration::{
    BYTES_PER_TOKEN, DEFAULT_MAX_CONCURRENT_TOOL_USES, DEFAULT_MAX_RESULT_SIZE_CHARS,
    MAX_TOOL_RESULT_BYTES, MAX_TOOL_RESULT_TOKENS, MAX_TOOL_RESULTS_PER_MESSAGE_CHARS,
    PERSISTED_OUTPUT_CLOSING_TAG, PERSISTED_OUTPUT_TAG, PREVIEW_SIZE_BYTES,
    PersistedToolResultSummary, TOOL_RESULT_CLEARED_MESSAGE, TOOL_SUMMARY_MAX_LENGTH,
    ToolCancellationReason, ToolConcurrencyClass, ToolConcurrencyMetadata, ToolExecutionState,
    ToolProgressState, ToolRuntimeLimits, filter_tool_specs, merge_tool_specs, provider_tool_specs,
    statically_denied_by_rule, tool_is_allowed,
};
/// Re-exports items from `plan_tool`
pub use plan_tool::{
    EnterPlanModeInput, EnterPlanModeTool, ExitPlanModeInput, ExitPlanModeTool, PlanReadInput,
    PlanReadTool, PlanWriteInput, PlanWriteTool,
};
/// Re-exports items from `search`
pub use search::{GlobEntryType, GlobInput, GlobTool, GrepInput, GrepTool};
/// Re-exports items from `source_compat`
pub use source_compat::{
    BriefInput, BriefTool, ConfigInput, ConfigTool, LspInput, LspTool, SkillInput, SkillTool,
    ToolSearchInput, ToolSearchTool,
};
/// Re-exports items from `special_tools`
pub use special_tools::{
    CtxInspectTool, McpAuthInput, McpAuthTool, McpTool, McpToolInput, MonitorTool,
    OverflowTestInput, OverflowTestTool, PushNotificationInput, PushNotificationTool, ReplInput,
    ReplTool, SendUserFileInput, SendUserFileTool, SleepInput, SleepTool, SnipTool,
    StructuredOutputTool, SubscribePrTool, SuggestBackgroundPrInput, SuggestBackgroundPrTool,
    TestingPermissionTool, TungstenInput, TungstenTool, VerifyPlanExecutionInput,
    VerifyPlanExecutionTool, WebBrowserInput, WebBrowserTool, WorkflowInput, WorkflowTool,
};
/// Re-exports items from `task_tools`
pub use task_tools::{
    TaskCreateInput, TaskCreateTool, TaskGetInput, TaskGetTool, TaskListInput, TaskListTool,
    TaskOutputInput, TaskOutputTool, TaskStopInput, TaskStopTool, TaskUpdateInput,
    TaskUpdateStatus, TaskUpdateTool,
};
/// Re-exports items from `todo_tool`
pub use todo_tool::{TodoAction, TodoInput, TodoTool, TodoWriteTool};
/// Re-exports items from `web`
pub use web::{WebFetchInput, WebFetchTool, WebSearchInput, WebSearchTool};
/// Re-exports items from `wonder_of_u_mcp`
pub use wonder_of_u_mcp::{
    McpResourceListInput, McpResourceListTool, McpResourceReadInput, McpResourceReadTool,
};
/// Re-exports items from `worktree_tools`
pub use worktree_tools::{
    EnterWorktreeInput, EnterWorktreeTool, ExitWorktreeAction, ExitWorktreeInput, ExitWorktreeTool,
    WorktreeRuntimeAction, WorktreeRuntimeActionKind, WorktreeSessionState,
    create_fleet_agent_worktree, fleet_agent_worktree_slug, parse_worktree_runtime_action,
    validate_worktree_branch_name, validate_worktree_session_state,
};
/// Handles builtin tools
#[must_use]
pub fn builtin_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(BashTool),
        Arc::new(PowerShellTool),
        Arc::new(FileReadTool),
        Arc::new(FileWriteTool),
        Arc::new(FileEditTool),
        Arc::new(NotebookEditTool),
        Arc::new(GlobTool),
        Arc::new(GrepTool),
        Arc::new(LspTool),
        Arc::new(ConfigTool),
        Arc::new(ToolSearchTool),
        Arc::new(ReplTool),
        Arc::new(WebBrowserTool),
        Arc::new(McpAuthTool),
        Arc::new(McpTool),
        Arc::new(StructuredOutputTool),
        Arc::new(VerifyPlanExecutionTool),
        Arc::new(SendUserFileTool),
        Arc::new(SuggestBackgroundPrTool),
        Arc::new(SleepTool),
        Arc::new(WorkflowTool),
        Arc::new(PushNotificationTool),
        Arc::new(WorktreeListTool),
        Arc::new(EnterWorktreeTool),
        Arc::new(ExitWorktreeTool),
        Arc::new(TerminalCaptureTool),
        Arc::new(CronCreateTool),
        Arc::new(CronDeleteTool),
        Arc::new(CronListTool),
        Arc::new(RemoteTriggerTool),
        Arc::new(WebFetchTool),
        Arc::new(WebSearchTool),
        Arc::new(TodoTool),
        Arc::new(TodoWriteTool),
        Arc::new(AskUserTool),
        Arc::new(BriefTool),
        Arc::new(PlanReadTool),
        Arc::new(PlanWriteTool),
        Arc::new(EnterPlanModeTool),
        Arc::new(ExitPlanModeTool),
        Arc::new(TaskCreateTool),
        Arc::new(TaskGetTool),
        Arc::new(TaskListTool),
        Arc::new(TaskUpdateTool),
        Arc::new(TaskOutputTool),
        Arc::new(TaskStopTool),
        Arc::new(SkillTool),
        Arc::new(AgentTool),
        Arc::new(SendMessageTool),
        Arc::new(TeamCreateTool),
        Arc::new(TeamDeleteTool),
        Arc::new(ListPeersTool),
        Arc::new(McpResourceListTool),
        Arc::new(McpResourceReadTool),
        Arc::new(TestingPermissionTool),
        Arc::new(OverflowTestTool),
        Arc::new(CtxInspectTool),
        Arc::new(MonitorTool),
        Arc::new(SubscribePrTool),
        Arc::new(SnipTool),
        Arc::new(TungstenTool),
    ]
}

/// Handles builtin registry
pub fn builtin_registry() -> Result<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    for tool in builtin_tools() {
        registry.register(tool)?;
    }
    Ok(registry)
}

/// Builds the built-in registry and registers any MCP catalog tools discovered from
/// enabled servers under `storage_root`.
///
/// Each enabled MCP server is contacted once to fetch its tool list.  Servers that fail
/// to connect are silently skipped so a misconfigured server does not break the session.
/// Discovered tools use namespaced names (e.g. `mcp__github__create_issue`) and therefore
/// cannot shadow built-in tools.
///
/// If `storage_root` contains no MCP config, or if all servers are unreachable, the
/// returned registry is identical to [`builtin_registry`].
pub fn builtin_registry_with_mcp_catalog(storage_root: &Path) -> Result<ToolRegistry> {
    let mut registry = builtin_registry()?;
    if let Ok(config) = wonder_of_u_mcp::McpConfigStore::new(storage_root).read() {
        for tool in wonder_of_u_mcp::discover_catalog_tools(&config) {
            // Namespacing guarantees no collisions with built-ins; soft-fail just in case.
            let _ = registry.register(Arc::new(tool));
        }
    }
    Ok(registry)
}

fn base_spec(name: &str, description: &str, kind: ToolKind) -> ToolSpec {
    let mut spec = ToolSpec::new(name, description, kind);
    spec.required_features.insert(FeatureFlag::Tools);
    spec
}

fn parse_input<T>(tool_name: &str, input: &Value) -> Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_value(input.clone())
        .map_err(|error| WonderError::validation(format!("invalid {tool_name} input: {error}")))
}

fn require_non_empty_path(tool_name: &str, field: &str, path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Err(WonderError::validation(format!(
            "{tool_name} requires a non-empty `{field}`"
        )));
    }
    Ok(())
}

fn require_non_empty_text(tool_name: &str, field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(WonderError::validation(format!(
            "{tool_name} requires a non-empty `{field}`"
        )));
    }
    Ok(())
}

fn schema_with_aliases(mut schema: Value, aliases: &[&str]) -> Value {
    if aliases.is_empty() {
        return schema;
    }

    if let Some(object) = schema.as_object_mut() {
        object.insert(
            "x-aliases".into(),
            Value::Array(
                aliases
                    .iter()
                    .map(|alias| Value::String((*alias).to_string()))
                    .collect(),
            ),
        );
    }
    schema
}

fn display_path(path: &Path, cwd: &Path) -> String {
    path.strip_prefix(cwd)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .unwrap_or(path)
        .display()
        .to_string()
}

fn app_root() -> Result<PathBuf> {
    app_root_from_env(|name| env::var_os(name))
}

fn app_root_from_env(getenv: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf> {
    if let Some(path) = getenv("WONDER_OF_U_STORAGE_DIR").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = getenv("XDG_CONFIG_HOME").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path).join("wonder-of-u"));
    }
    if let Some(path) = getenv("HOME").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path).join(".config").join("wonder-of-u"));
    }
    Err(WonderError::validation(
        "unable to resolve wonder-of-u storage directory from WONDER_OF_U_STORAGE_DIR, XDG_CONFIG_HOME, or HOME",
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use serde_json::Value;
    use wonder_of_u_core::FeatureSet;

    use super::*;

    #[test]
    fn builtin_registry_registers_expected_order() {
        let registry = builtin_registry().expect("registry");
        let names = registry
            .all_specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "bash",
                "powershell",
                "file_read",
                "file_write",
                "file_edit",
                "notebook_edit",
                "glob",
                "grep",
                "lsp",
                "config",
                "tool_search",
                "repl",
                "web_browser",
                "mcp_auth",
                "mcp",
                "structured_output",
                "verify_plan_execution",
                "send_user_file",
                "suggest_background_pr",
                "sleep",
                "workflow",
                "push_notification",
                "worktree_list",
                "enter_worktree",
                "exit_worktree",
                "terminal_capture",
                "cron_create",
                "cron_delete",
                "cron_list",
                "remote_trigger",
                "web_fetch",
                "web_search",
                "todo",
                "todo_write",
                "ask_user",
                "send_user_message",
                "plan_read",
                "plan_write",
                "enter_plan_mode",
                "exit_plan_mode",
                "task_create",
                "task_get",
                "task_list",
                "task_update",
                "task_output",
                "task_stop",
                "skill",
                "agent",
                "send_message",
                "team_create",
                "team_delete",
                "list_peers",
                "mcp_resource_list",
                "mcp_resource_read",
                "testing_permission",
                "overflow_test",
                "ctx_inspect",
                "monitor",
                "subscribe_pr",
                "snip",
                "tungsten"
            ]
        );
    }

    #[test]
    fn app_root_prefers_explicit_storage_dir() {
        let root = app_root_from_env(|name| match name {
            "WONDER_OF_U_STORAGE_DIR" => Some("/tmp/wonder".into()),
            _ => None,
        })
        .expect("root");

        assert_eq!(root, PathBuf::from("/tmp/wonder"));
    }

    #[test]
    fn app_root_falls_back_to_xdg_config_home() {
        let root = app_root_from_env(|name| match name {
            "XDG_CONFIG_HOME" => Some("/config".into()),
            _ => None,
        })
        .expect("root");

        assert_eq!(root, PathBuf::from("/config/wonder-of-u"));
    }

    #[test]
    fn builtin_registry_resolves_source_aliases() {
        let registry = builtin_registry().expect("registry");

        for alias in [
            "Bash",
            "PowerShell",
            "Read",
            "Write",
            "Edit",
            "Glob",
            "Grep",
            "LSP",
            "Config",
            "ToolSearch",
            "REPL",
            "WebBrowser",
            "McpAuth",
            "MCPTool",
            "StructuredOutput",
            "SyntheticOutput",
            "VerifyPlanExecution",
            "SendUserFile",
            "SuggestBackgroundPR",
            "Sleep",
            "Workflow",
            "PushNotification",
            "EnterWorktree",
            "ExitWorktree",
            "WebFetch",
            "WebSearch",
            "TodoWrite",
            "AskUserQuestion",
            "SendUserMessage",
            "Brief",
            "EnterPlanMode",
            "ExitPlanMode",
            "CronCreate",
            "CronDelete",
            "CronList",
            "RemoteTrigger",
            "TaskCreate",
            "TaskGet",
            "TaskList",
            "TaskUpdate",
            "TaskOutput",
            "AgentOutputTool",
            "BashOutputTool",
            "Task",
            "Skill",
            "SendMessage",
            "TeamCreate",
            "TeamDelete",
            "ListPeers",
            "TaskStop",
            "KillShell",
            "ListMcpResourcesTool",
            "ReadMcpResourceTool",
            "TestingPermission",
            "OverflowTest",
            "CtxInspect",
            "Monitor",
            "SubscribePR",
            "Snip",
            "Tungsten",
        ] {
            assert!(registry.resolve(alias).is_some(), "missing alias {alias}");
        }
    }

    #[test]
    fn builtin_specs_include_schema_alias_metadata() {
        let registry = builtin_registry().expect("registry");
        let specs = registry
            .all_specs()
            .into_iter()
            .map(|spec| (spec.name.clone(), spec))
            .collect::<BTreeMap<_, _>>();

        let file_read = specs.get("file_read").expect("file_read");
        assert_eq!(file_read.aliases, vec!["Read".to_string()]);
        assert_eq!(
            property_aliases(file_read, "path"),
            BTreeSet::from(["file_path".to_string()])
        );
        assert_eq!(
            required_properties(file_read),
            BTreeSet::from(["path".into()])
        );
        assert!(has_property(file_read, "offset"));
        assert!(has_property(file_read, "limit"));
        assert!(has_property(file_read, "pages"));

        let file_write = specs.get("file_write").expect("file_write");
        assert_eq!(file_write.aliases, vec!["Write".to_string()]);
        assert_eq!(
            property_aliases(file_write, "path"),
            BTreeSet::from(["file_path".to_string()])
        );
        assert_eq!(
            required_properties(file_write),
            BTreeSet::from(["content".into(), "path".into()])
        );

        let file_edit = specs.get("file_edit").expect("file_edit");
        assert_eq!(file_edit.aliases, vec!["Edit".to_string()]);
        assert_eq!(
            property_aliases(file_edit, "old_text"),
            BTreeSet::from(["old_string".to_string()])
        );
        assert_eq!(
            property_aliases(file_edit, "new_text"),
            BTreeSet::from(["new_string".to_string()])
        );

        let task_output = specs.get("task_output").expect("task_output");
        assert_eq!(
            task_output.aliases,
            vec![
                "TaskOutput".to_string(),
                "AgentOutputTool".to_string(),
                "BashOutputTool".to_string(),
            ]
        );
        assert_eq!(
            required_properties(task_output),
            BTreeSet::from(["task_id".into()])
        );
        assert!(has_property(task_output, "block"));
        assert!(has_property(task_output, "timeout"));

        let task_stop = specs.get("task_stop").expect("task_stop");
        assert_eq!(
            task_stop.aliases,
            vec!["TaskStop".to_string(), "KillShell".to_string()]
        );
        assert!(has_property(task_stop, "shell_id"));

        let agent = specs.get("agent").expect("agent");
        assert_eq!(agent.aliases, vec!["Task".to_string()]);
        assert!(has_property(agent, "description"));
        assert!(has_property(agent, "subagent_type"));
        assert!(has_property(agent, "run_in_background"));
        assert!(has_property(agent, "name"));
        assert!(has_property(agent, "team_name"));
        assert!(has_property(agent, "mode"));
        assert!(has_property(agent, "isolation"));
        assert!(has_property(agent, "cwd"));

        let send_message = specs.get("send_message").expect("send_message");
        assert_eq!(send_message.aliases, vec!["SendMessage".to_string()]);
        assert_eq!(
            required_properties(send_message),
            BTreeSet::from(["message".into(), "to".into()])
        );
        assert!(has_property(send_message, "summary"));

        let team_create = specs.get("team_create").expect("team_create");
        assert_eq!(team_create.aliases, vec!["TeamCreate".to_string()]);
        assert_eq!(
            required_properties(team_create),
            BTreeSet::from(["team_name".into()])
        );
        assert!(has_property(team_create, "description"));
        assert!(has_property(team_create, "agent_type"));

        let team_delete = specs.get("team_delete").expect("team_delete");
        assert_eq!(team_delete.aliases, vec!["TeamDelete".to_string()]);

        let list_peers = specs.get("list_peers").expect("list_peers");
        assert_eq!(list_peers.aliases, vec!["ListPeers".to_string()]);

        let todo = specs.get("todo").expect("todo");
        assert!(todo.aliases.is_empty());
        assert!(has_property(todo, "todos"));

        let todo_write = specs.get("todo_write").expect("todo_write");
        assert_eq!(todo_write.aliases, vec!["TodoWrite".to_string()]);
        assert_eq!(
            required_properties(todo_write),
            BTreeSet::from(["todos".into()])
        );

        let ask_user = specs.get("ask_user").expect("ask_user");
        assert_eq!(ask_user.aliases, vec!["AskUserQuestion".to_string()]);
        assert!(has_property(ask_user, "questions"));

        let lsp = specs.get("lsp").expect("lsp");
        assert!(lsp.aliases.is_empty());
        assert_eq!(
            property_aliases(lsp, "file_path"),
            BTreeSet::from(["filePath".to_string()])
        );

        let config = specs.get("config").expect("config");
        assert!(config.aliases.is_empty());
        assert!(has_property(config, "setting"));
        assert!(has_property(config, "value"));

        let tool_search = specs.get("tool_search").expect("tool_search");
        assert_eq!(tool_search.aliases, vec!["ToolSearch".to_string()]);
        assert!(has_property(tool_search, "query"));
        assert!(has_property(tool_search, "max_results"));

        let repl = specs.get("repl").expect("repl");
        assert!(repl.aliases.is_empty());
        assert!(has_property(repl, "command"));

        let web_browser = specs.get("web_browser").expect("web_browser");
        assert_eq!(web_browser.aliases, vec!["WebBrowser".to_string()]);
        assert!(has_property(web_browser, "url"));
        assert!(has_property(web_browser, "prompt"));

        let mcp_auth = specs.get("mcp_auth").expect("mcp_auth");
        assert_eq!(mcp_auth.aliases, vec!["McpAuth".to_string()]);
        assert_eq!(
            required_properties(mcp_auth),
            BTreeSet::from(["server".into()])
        );

        let structured_output = specs.get("structured_output").expect("structured_output");
        assert_eq!(
            structured_output.aliases,
            vec![
                "StructuredOutput".to_string(),
                "SyntheticOutput".to_string()
            ]
        );

        let verify_plan_execution = specs
            .get("verify_plan_execution")
            .expect("verify_plan_execution");
        assert_eq!(
            verify_plan_execution.aliases,
            vec!["VerifyPlanExecution".to_string()]
        );
        assert!(has_property(verify_plan_execution, "prompt"));

        let send_user_file = specs.get("send_user_file").expect("send_user_file");
        assert_eq!(send_user_file.aliases, vec!["SendUserFile".to_string()]);
        assert_eq!(
            required_properties(send_user_file),
            BTreeSet::from(["files".into()])
        );
        assert!(has_property(send_user_file, "files"));

        let suggest_background_pr = specs
            .get("suggest_background_pr")
            .expect("suggest_background_pr");
        assert_eq!(
            suggest_background_pr.aliases,
            vec!["SuggestBackgroundPR".to_string()]
        );
        assert!(has_property(suggest_background_pr, "prompt"));

        let brief = specs.get("send_user_message").expect("send_user_message");
        assert_eq!(
            brief.aliases,
            vec!["SendUserMessage".to_string(), "Brief".to_string()]
        );
        assert!(has_property(brief, "message"));
        assert!(has_property(brief, "attachments"));
        assert!(has_property(brief, "status"));

        let skill = specs.get("skill").expect("skill");
        assert!(skill.aliases.is_empty());
        assert!(has_property(skill, "skill"));
        assert!(has_property(skill, "args"));

        let enter_plan_mode = specs.get("enter_plan_mode").expect("enter_plan_mode");
        assert_eq!(enter_plan_mode.aliases, vec!["EnterPlanMode".to_string()]);

        let exit_plan_mode = specs.get("exit_plan_mode").expect("exit_plan_mode");
        assert_eq!(exit_plan_mode.aliases, vec!["ExitPlanMode".to_string()]);
        assert!(has_property(exit_plan_mode, "allowedPrompts"));

        let cron_create = specs.get("cron_create").expect("cron_create");
        assert_eq!(cron_create.aliases, vec!["CronCreate".to_string()]);
        assert_eq!(
            required_properties(cron_create),
            BTreeSet::from(["cron".into(), "prompt".into()])
        );
        assert!(has_property(cron_create, "recurring"));
        assert!(has_property(cron_create, "durable"));

        let cron_delete = specs.get("cron_delete").expect("cron_delete");
        assert_eq!(cron_delete.aliases, vec!["CronDelete".to_string()]);
        assert_eq!(
            required_properties(cron_delete),
            BTreeSet::from(["id".into()])
        );

        let remote_trigger = specs.get("remote_trigger").expect("remote_trigger");
        assert_eq!(remote_trigger.aliases, vec!["RemoteTrigger".to_string()]);
        assert_eq!(
            required_properties(remote_trigger),
            BTreeSet::from(["action".into()])
        );
        assert!(has_property(remote_trigger, "trigger_id"));
        assert!(has_property(remote_trigger, "body"));

        let testing_permission = specs.get("testing_permission").expect("testing_permission");
        assert_eq!(
            testing_permission.aliases,
            vec!["TestingPermission".to_string()]
        );

        let overflow_test = specs.get("overflow_test").expect("overflow_test");
        assert_eq!(overflow_test.aliases, vec!["OverflowTest".to_string()]);
        assert!(has_property(overflow_test, "chars"));

        let ctx_inspect = specs.get("ctx_inspect").expect("ctx_inspect");
        assert_eq!(ctx_inspect.aliases, vec!["CtxInspect".to_string()]);

        let monitor = specs.get("monitor").expect("monitor");
        assert!(monitor.aliases.is_empty());

        let subscribe_pr = specs.get("subscribe_pr").expect("subscribe_pr");
        assert_eq!(subscribe_pr.aliases, vec!["SubscribePR".to_string()]);

        let snip = specs.get("snip").expect("snip");
        assert!(snip.aliases.is_empty());

        let tungsten = specs.get("tungsten").expect("tungsten");
        assert!(tungsten.aliases.is_empty());
        assert!(has_property(tungsten, "args"));
    }

    #[test]
    fn builtin_specs_capture_feature_gates() {
        let registry = builtin_registry().expect("registry");
        let specs = registry
            .all_specs()
            .into_iter()
            .map(|spec| (spec.name.clone(), spec))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            feature_names(specs.get("todo_write").expect("todo_write")),
            BTreeSet::from(["legacy-todo-write".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("task_create").expect("task_create")),
            BTreeSet::from(["todo-v2".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("task_get").expect("task_get")),
            BTreeSet::from(["todo-v2".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("task_list").expect("task_list")),
            BTreeSet::from(["todo-v2".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("task_update").expect("task_update")),
            BTreeSet::from(["todo-v2".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("web_fetch").expect("web_fetch")),
            BTreeSet::from(["tools".to_string(), "web-tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("todo").expect("todo")),
            BTreeSet::from(["tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("task_output").expect("task_output")),
            BTreeSet::from(["background-tasks".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("agent").expect("agent")),
            BTreeSet::from(["agents".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("send_message").expect("send_message")),
            BTreeSet::from(["agents".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("team_create").expect("team_create")),
            BTreeSet::from(["agents".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("team_delete").expect("team_delete")),
            BTreeSet::from(["agents".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("list_peers").expect("list_peers")),
            BTreeSet::from(["agents".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("task_stop").expect("task_stop")),
            BTreeSet::from(["background-tasks".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("skill").expect("skill")),
            BTreeSet::from(["skills".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("mcp_resource_list").expect("mcp_resource_list")),
            BTreeSet::from(["mcp".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("mcp_auth").expect("mcp_auth")),
            BTreeSet::from(["mcp".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("remote_trigger").expect("remote_trigger")),
            BTreeSet::from(["remote-triggers".to_string(), "tools".to_string()])
        );
        assert_eq!(
            feature_names(specs.get("testing_permission").expect("testing_permission")),
            BTreeSet::from(["test-tools".to_string(), "tools".to_string()])
        );
    }

    #[test]
    fn builtin_specs_gate_remote_trigger_and_todo_v2_tooling_until_features_are_enabled() {
        let registry = builtin_registry().expect("registry");

        let default_enabled = registry
            .enabled_specs(&FeatureSet::first_release())
            .into_iter()
            .map(|spec| spec.name)
            .collect::<BTreeSet<_>>();
        assert!(default_enabled.contains("todo_write"));
        assert!(!default_enabled.contains("task_create"));
        assert!(!default_enabled.contains("task_get"));
        assert!(!default_enabled.contains("task_list"));
        assert!(!default_enabled.contains("task_update"));
        assert!(!default_enabled.contains("remote_trigger"));
        assert!(!default_enabled.contains("testing_permission"));

        let mut features = FeatureSet::first_release();
        features.enable(FeatureFlag::RemoteTriggers);
        features.enable(FeatureFlag::TestTools);
        features.enable(FeatureFlag::TodoV2);
        features.disable(FeatureFlag::LegacyTodoWrite);
        let enabled = registry
            .enabled_specs(&features)
            .into_iter()
            .map(|spec| spec.name)
            .collect::<BTreeSet<_>>();
        assert!(!enabled.contains("todo_write"));
        assert!(enabled.contains("task_create"));
        assert!(enabled.contains("task_get"));
        assert!(enabled.contains("task_list"));
        assert!(enabled.contains("task_update"));
        assert!(enabled.contains("remote_trigger"));
        assert!(enabled.contains("testing_permission"));
    }

    #[test]
    fn builtin_specs_capture_permission_metadata() {
        let registry = builtin_registry().expect("registry");
        let specs = registry
            .all_specs()
            .into_iter()
            .map(|spec| (spec.name.clone(), spec))
            .collect::<BTreeMap<_, _>>();

        let web_fetch = specs.get("web_fetch").expect("web_fetch");
        assert!(web_fetch.read_only);
        assert!(web_fetch.concurrency_safe);

        let cron_list = specs.get("cron_list").expect("cron_list");
        assert!(cron_list.read_only);
        assert!(cron_list.concurrency_safe);

        let cron_create = specs.get("cron_create").expect("cron_create");
        assert!(cron_create.destructive);
        assert!(!cron_create.read_only);

        let cron_delete = specs.get("cron_delete").expect("cron_delete");
        assert!(cron_delete.destructive);
        assert!(!cron_delete.read_only);

        let remote_trigger = specs.get("remote_trigger").expect("remote_trigger");
        assert!(remote_trigger.concurrency_safe);
        assert!(!remote_trigger.read_only);
        assert!(!remote_trigger.destructive);

        let list_peers = specs.get("list_peers").expect("list_peers");
        assert!(list_peers.read_only);
        assert!(list_peers.concurrency_safe);

        let structured_output = specs.get("structured_output").expect("structured_output");
        assert!(structured_output.read_only);
        assert!(structured_output.concurrency_safe);

        let repl = specs.get("repl").expect("repl");
        assert!(!repl.read_only);
        assert!(!repl.concurrency_safe);

        let team_delete = specs.get("team_delete").expect("team_delete");
        assert!(team_delete.destructive);
        assert!(!team_delete.read_only);

        let mcp_resource_read = specs.get("mcp_resource_read").expect("mcp_resource_read");
        assert!(mcp_resource_read.read_only);
        assert!(mcp_resource_read.concurrency_safe);
        assert!(
            mcp_resource_read
                .aliases
                .contains(&"ReadMcpResourceTool".to_string())
        );

        let testing_permission = specs.get("testing_permission").expect("testing_permission");
        assert!(testing_permission.read_only);
        assert!(testing_permission.concurrency_safe);

        let tungsten = specs.get("tungsten").expect("tungsten");
        assert!(tungsten.read_only);
        assert!(tungsten.concurrency_safe);

        let mcp_resource_list = specs.get("mcp_resource_list").expect("mcp_resource_list");
        assert_eq!(
            mcp_resource_list.input_schema["x-mcp-resource-capability-dependent"],
            serde_json::json!(true)
        );
        assert_eq!(
            mcp_resource_list.input_schema["x-mcp-resource-dispatch"],
            serde_json::json!("always_registered_runtime_checked")
        );
    }

    fn has_property(spec: &ToolSpec, property: &str) -> bool {
        spec.input_schema
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|properties| properties.contains_key(property))
    }

    fn property_aliases(spec: &ToolSpec, property: &str) -> BTreeSet<String> {
        spec.input_schema
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(property))
            .and_then(|schema| schema.get("x-aliases"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    fn required_properties(spec: &ToolSpec) -> BTreeSet<String> {
        spec.input_schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    fn feature_names(spec: &ToolSpec) -> BTreeSet<String> {
        spec.required_features
            .iter()
            .map(|feature| serde_json::to_string(feature).expect("feature json"))
            .map(|feature| feature.trim_matches('"').to_string())
            .collect()
    }

    /// A nonexistent storage root must not panic; it should return only the
    /// built-in tools because `McpConfigStore` will find no config file.
    #[test]
    fn builtin_registry_with_mcp_catalog_no_panic_for_missing_dir() {
        let result = builtin_registry_with_mcp_catalog(Path::new("/no/such/storage/root"));
        let registry = result.expect("registry should succeed even without mcp config");
        // Every built-in should still be present.
        assert!(
            registry.resolve("bash").is_some(),
            "bash tool must be present"
        );
        assert!(
            registry.resolve("file_read").is_some(),
            "file_read tool must be present"
        );
    }
}
