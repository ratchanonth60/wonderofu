//! Markdown todo list management.

use std::{fs, path::Path};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{
    Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId, WonderError,
};

use crate::{base_spec, parse_input, require_non_empty_text};
/// Enumerates todo action
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoAction {
    /// Represents add
    Add,
    /// Represents remove
    Remove,
    /// Represents list
    List,
    /// Represents check
    Check,
    /// Represents uncheck
    Uncheck,
}
/// Represents todo input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodoInput {
    /// Stores the action
    pub action: TodoAction,
    /// Stores the text
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Stores the index
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}

impl TodoInput {
    fn validate(&self) -> Result<()> {
        match self.action {
            TodoAction::Add => {
                require_non_empty_text("todo", "text", self.text.as_deref().unwrap_or_default())
            }
            TodoAction::Remove | TodoAction::Check | TodoAction::Uncheck => {
                let index = self.index.ok_or_else(|| {
                    WonderError::validation("todo requires an `index` for this action")
                })?;
                if index == 0 {
                    return Err(WonderError::validation(
                        "todo index must be greater than zero",
                    ));
                }
                Ok(())
            }
            TodoAction::List => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SourceTodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceTodoItem {
    content: String,
    status: SourceTodoStatus,
    #[serde(rename = "activeForm", alias = "active_form")]
    active_form: String,
}

impl SourceTodoItem {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("todo", "todos[].content", &self.content)?;
        require_non_empty_text("todo", "todos[].activeForm", &self.active_form)?;
        if self.status == SourceTodoStatus::InProgress {
            return Err(WonderError::validation(
                "todo todos[].status `in_progress` is not supported in the markdown todo runtime",
            ));
        }
        Ok(())
    }

    fn to_markdown_item(&self) -> TodoItem {
        TodoItem {
            checked: self.status == SourceTodoStatus::Completed,
            text: self.content.trim().to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TodoWriteCompatInput {
    todos: Vec<SourceTodoItem>,
}

impl TodoWriteCompatInput {
    fn validate(&self) -> Result<()> {
        self.todos.iter().try_for_each(SourceTodoItem::validate)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ParsedTodoInput {
    Action(TodoInput),
    Replace(TodoWriteCompatInput),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TodoItem {
    checked: bool,
    text: String,
}
/// Represents todo tool
#[derive(Debug, Default)]
pub struct TodoTool;

#[async_trait]
impl Tool for TodoTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "todo",
            "Manage todos.md in the current directory",
            ToolKind::Planning,
        )
        .with_input_schema(ToolSchema::object());
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "action": ToolSchema::enumeration(
                    "todo operation to perform",
                    ["add", "remove", "list", "check", "uncheck"],
                ),
                "text": ToolSchema::string("todo text for add"),
                "index": ToolSchema::integer("1-indexed todo item position"),
                "todos": {
                    "type": "array",
                    "description": "source-compatible todo list replacement payload; use this instead of action/text/index",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": ToolSchema::string("todo text"),
                            "status": ToolSchema::enumeration(
                                "source-compatible todo status",
                                ["pending", "in_progress", "completed"],
                            ),
                            "activeForm": ToolSchema::string(
                                "source-compatible active-form progress label",
                            ),
                        },
                        "required": ["content", "status", "activeForm"],
                        "additionalProperties": false,
                    },
                },
            },
            "additionalProperties": false,
        });
        spec.aliases.push("TodoWrite".into());
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_todo_input(input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_todo_input(&input)?;
        input.validate()?;

        let path = context.cwd.join("todos.md");
        let content = match input {
            ParsedTodoInput::Action(input) => execute_todo_action(&path, &input)?,
            ParsedTodoInput::Replace(input) => execute_todo_replace(&path, &input)?,
        };
        Ok(ToolResult::success(use_id, content))
    }
}

impl ParsedTodoInput {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Action(input) => input.validate(),
            Self::Replace(input) => input.validate(),
        }
    }
}

fn parse_todo_input(input: &Value) -> Result<ParsedTodoInput> {
    let object = input
        .as_object()
        .ok_or_else(|| WonderError::validation("invalid todo input: expected an object"))?;
    let has_action = object.contains_key("action");
    let has_todos = object.contains_key("todos");

    if has_action && has_todos {
        return Err(WonderError::validation(
            "todo accepts either action-based input or source-compatible `todos`, not both",
        ));
    }

    if has_todos {
        return parse_input::<TodoWriteCompatInput>("todo", input).map(ParsedTodoInput::Replace);
    }

    parse_input::<TodoInput>("todo", input).map(ParsedTodoInput::Action)
}

fn execute_todo_action(path: &Path, input: &TodoInput) -> Result<String> {
    let mut todos = read_todos(path)?;
    match input.action {
        TodoAction::List => {}
        TodoAction::Add => {
            todos.push(TodoItem {
                checked: false,
                text: input.text.clone().unwrap_or_default().trim().to_string(),
            });
            write_todos(path, &todos)?;
        }
        TodoAction::Remove => {
            let index = todo_index(input.index, todos.len())?;
            todos.remove(index);
            write_todos(path, &todos)?;
        }
        TodoAction::Check => {
            let index = todo_index(input.index, todos.len())?;
            todos[index].checked = true;
            write_todos(path, &todos)?;
        }
        TodoAction::Uncheck => {
            let index = todo_index(input.index, todos.len())?;
            todos[index].checked = false;
            write_todos(path, &todos)?;
        }
    }
    Ok(render_todos(&todos))
}

fn execute_todo_replace(path: &Path, input: &TodoWriteCompatInput) -> Result<String> {
    let all_completed = !input.todos.is_empty()
        && input
            .todos
            .iter()
            .all(|todo| todo.status == SourceTodoStatus::Completed);
    let todos = if all_completed {
        Vec::new()
    } else {
        input
            .todos
            .iter()
            .map(SourceTodoItem::to_markdown_item)
            .collect()
    };
    write_todos(path, &todos)?;
    Ok(render_todos(&todos))
}

fn read_todos(path: &Path) -> Result<Vec<TodoItem>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)?;
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(line_number, line)| parse_todo_line(line, line_number + 1))
        .collect()
}

fn parse_todo_line(line: &str, line_number: usize) -> Result<TodoItem> {
    if let Some(text) = line.strip_prefix("- [ ] ") {
        return Ok(TodoItem {
            checked: false,
            text: text.to_string(),
        });
    }
    if let Some(text) = line.strip_prefix("- [x] ") {
        return Ok(TodoItem {
            checked: true,
            text: text.to_string(),
        });
    }
    Err(WonderError::validation(format!(
        "invalid todo item at line {line_number}: expected `- [ ]` or `- [x]`"
    )))
}

fn write_todos(path: &Path, todos: &[TodoItem]) -> Result<()> {
    let content = todos
        .iter()
        .map(|todo| format!("- [{}] {}", if todo.checked { "x" } else { " " }, todo.text))
        .collect::<Vec<_>>()
        .join("\n");
    if content.is_empty() {
        fs::write(path, "")?;
    } else {
        fs::write(path, format!("{content}\n"))?;
    }
    Ok(())
}

fn render_todos(todos: &[TodoItem]) -> String {
    if todos.is_empty() {
        "No todos.".into()
    } else {
        todos
            .iter()
            .enumerate()
            .map(|(index, todo)| {
                format!(
                    "{}. [{}] {}",
                    index + 1,
                    if todo.checked { "x" } else { " " },
                    todo.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn todo_index(index: Option<u32>, len: usize) -> Result<usize> {
    let index = index.unwrap_or_default() as usize;
    if index == 0 || index > len {
        return Err(WonderError::validation(format!(
            "todo index {index} is out of range for {len} item(s)"
        )));
    }
    Ok(index - 1)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, fs, path::PathBuf};

    use futures::executor::block_on;
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
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: None,
        }
    }

    #[test]
    fn todo_validation_requires_text_for_add() {
        let tool = TodoTool;
        let error = tool
            .validate_input(&json!({ "action": "add" }))
            .expect_err("missing text");

        assert!(error.to_string().contains("text"));
    }

    #[test]
    fn todo_adds_and_lists_items() {
        let dir = unique_test_dir("tools-todo-add");
        let tool = TodoTool;

        let result = block_on(tool.execute(
            tool_context(dir.clone()),
            ToolUseId::new(),
            json!({ "action": "add", "text": "ship release" }),
        ))
        .expect("add todo");

        assert!(result.success);
        assert_eq!(result.content, "1. [ ] ship release");
        assert_eq!(
            fs::read_to_string(dir.join("todos.md")).expect("todo file"),
            "- [ ] ship release\n"
        );
    }

    #[test]
    fn todo_source_write_alias_replaces_existing_list() {
        let dir = unique_test_dir("tools-todo-source-write");
        let tool = TodoTool;

        block_on(tool.execute(
            tool_context(dir.clone()),
            ToolUseId::new(),
            json!({ "action": "add", "text": "draft release notes" }),
        ))
        .expect("seed todo");

        let result = block_on(tool.execute(
            tool_context(dir.clone()),
            ToolUseId::new(),
            json!({
                "todos": [
                    {
                        "content": "draft release notes",
                        "status": "completed",
                        "activeForm": "drafting release notes"
                    },
                    {
                        "content": "ship release",
                        "status": "pending",
                        "activeForm": "shipping release"
                    }
                ]
            }),
        ))
        .expect("replace todos");

        assert_eq!(
            result.content,
            "1. [x] draft release notes\n2. [ ] ship release"
        );
        assert_eq!(
            block_on(tool.execute(
                tool_context(dir.clone()),
                ToolUseId::new(),
                json!({ "action": "list" }),
            ))
            .expect("list todos")
            .content,
            "1. [x] draft release notes\n2. [ ] ship release"
        );
    }

    #[test]
    fn todo_source_write_clears_when_everything_is_completed() {
        let dir = unique_test_dir("tools-todo-source-clear");
        let tool = TodoTool;

        let result = block_on(tool.execute(
            tool_context(dir.clone()),
            ToolUseId::new(),
            json!({
                "todos": [
                    {
                        "content": "ship release",
                        "status": "completed",
                        "activeForm": "shipping release"
                    }
                ]
            }),
        ))
        .expect("replace todos");

        assert_eq!(result.content, "No todos.");
        assert_eq!(
            fs::read_to_string(dir.join("todos.md")).expect("todo file"),
            ""
        );
    }

    #[test]
    fn todo_source_validation_rejects_in_progress_status() {
        let tool = TodoTool;
        let error = tool
            .validate_input(&json!({
                "todos": [
                    {
                        "content": "ship release",
                        "status": "in_progress",
                        "activeForm": "shipping release"
                    }
                ]
            }))
            .expect_err("unsupported status");

        assert!(error.to_string().contains("in_progress"));
    }

    #[test]
    fn todo_validation_rejects_mixed_action_and_todos_inputs() {
        let tool = TodoTool;
        let error = tool
            .validate_input(&json!({
                "action": "list",
                "todos": [],
            }))
            .expect_err("mixed todo inputs");

        assert!(
            error
                .to_string()
                .contains("either action-based input or source-compatible `todos`")
        );
    }

    #[test]
    fn todo_rejects_out_of_range_index() {
        let dir = unique_test_dir("tools-todo-range");
        let error = execute_todo_action(
            &dir.join("todos.md"),
            &TodoInput {
                action: TodoAction::Check,
                text: None,
                index: Some(1),
            },
        )
        .expect_err("invalid index");

        assert!(error.to_string().contains("out of range"));
    }

    #[test]
    fn todo_spec_exposes_source_alias() {
        let tool = TodoTool;
        let aliases = tool.spec().aliases.into_iter().collect::<BTreeSet<_>>();

        assert_eq!(aliases, BTreeSet::from(["TodoWrite".to_string()]));
    }
}
