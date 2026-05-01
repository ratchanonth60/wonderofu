mod agent_tool;
mod ask_user_tool;
mod bash;
mod extended;
mod files;
mod plan_tool;
mod search;
mod task_tools;
mod todo_tool;
mod web;

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::de::DeserializeOwned;
use serde_json::Value;
use wonder_of_u_core::{FeatureFlag, Result, Tool, ToolKind, ToolRegistry, ToolSpec, WonderError};

pub use agent_tool::{AgentInput, AgentTool};
pub use ask_user_tool::{AskUserInput, AskUserTool};
pub use bash::{BashInput, BashTool};
pub use extended::{
    CronListTool, NotebookEditInput, NotebookEditTool, PowerShellInput, PowerShellTool,
    TerminalCaptureTool, WorktreeListTool,
};
pub use files::{
    FileEditInput, FileEditTool, FileReadInput, FileReadTool, FileWriteInput, FileWriteMode,
    FileWriteTool,
};
pub use plan_tool::{PlanReadInput, PlanReadTool, PlanWriteInput, PlanWriteTool};
pub use search::{GlobEntryType, GlobInput, GlobTool, GrepInput, GrepTool};
pub use task_tools::{
    TaskCreateInput, TaskCreateTool, TaskGetInput, TaskGetTool, TaskListInput, TaskListTool,
    TaskOutputInput, TaskOutputTool, TaskStopInput, TaskStopTool, TaskUpdateInput,
    TaskUpdateStatus, TaskUpdateTool,
};
pub use todo_tool::{TodoAction, TodoInput, TodoTool};
pub use web::{WebFetchInput, WebFetchTool, WebSearchInput, WebSearchTool};
pub use wonder_of_u_mcp::{
    McpResourceListInput, McpResourceListTool, McpResourceReadInput, McpResourceReadTool,
};

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
        Arc::new(WorktreeListTool),
        Arc::new(TerminalCaptureTool),
        Arc::new(CronListTool),
        Arc::new(WebFetchTool),
        Arc::new(WebSearchTool),
        Arc::new(TodoTool),
        Arc::new(AskUserTool),
        Arc::new(PlanReadTool),
        Arc::new(PlanWriteTool),
        Arc::new(TaskCreateTool),
        Arc::new(TaskGetTool),
        Arc::new(TaskListTool),
        Arc::new(TaskUpdateTool),
        Arc::new(TaskOutputTool),
        Arc::new(TaskStopTool),
        Arc::new(AgentTool),
        Arc::new(McpResourceListTool),
        Arc::new(McpResourceReadTool),
    ]
}

pub fn builtin_registry() -> Result<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    for tool in builtin_tools() {
        registry.register(tool)?;
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
                "worktree_list",
                "terminal_capture",
                "cron_list",
                "web_fetch",
                "web_search",
                "todo",
                "ask_user",
                "plan_read",
                "plan_write",
                "task_create",
                "task_get",
                "task_list",
                "task_update",
                "task_output",
                "task_stop",
                "agent",
                "mcp_resource_list",
                "mcp_resource_read"
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
}
