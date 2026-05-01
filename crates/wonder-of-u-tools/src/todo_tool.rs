//! Markdown todo list management.

use std::{fs, path::Path};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wonder_of_u_core::{
    Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId, WonderError,
};

use crate::{base_spec, parse_input, require_non_empty_text};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoAction {
    Add,
    Remove,
    List,
    Check,
    Uncheck,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodoInput {
    pub action: TodoAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct TodoItem {
    checked: bool,
    text: String,
}

#[derive(Debug, Default)]
pub struct TodoTool;

#[async_trait]
impl Tool for TodoTool {
    fn spec(&self) -> ToolSpec {
        base_spec(
            "todo",
            "Manage todos.md in the current directory",
            ToolKind::Planning,
        )
        .with_input_schema(
            ToolSchema::object()
                .property(
                    "action",
                    ToolSchema::enumeration(
                        "todo operation to perform",
                        ["add", "remove", "list", "check", "uncheck"],
                    ),
                )
                .property("text", ToolSchema::string("todo text for add"))
                .property("index", ToolSchema::integer("1-indexed todo item position"))
                .required("action"),
        )
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<TodoInput>("todo", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<TodoInput>("todo", &input)?;
        input.validate()?;

        let path = context.cwd.join("todos.md");
        let content = execute_todo_action(&path, &input)?;
        Ok(ToolResult::success(use_id, content))
    }
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
}
