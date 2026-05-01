//! Background task output and stop tools.

use std::{
    fs,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wonder_of_u_core::{
    FeatureFlag, Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId,
    WonderError,
};

use crate::{app_root, base_spec, parse_input, require_non_empty_text};

const DEFAULT_TASK_OUTPUT_LINES: u32 = 50;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskOutputInput {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<u32>,
}

impl TaskOutputInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("task_output", "task_id", &self.task_id)?;
        if self.lines == Some(0) {
            return Err(WonderError::validation(
                "task_output lines must be greater than zero",
            ));
        }
        Ok(())
    }

    fn lines(&self) -> usize {
        self.lines.unwrap_or(DEFAULT_TASK_OUTPUT_LINES) as usize
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskStopInput {
    pub task_id: String,
}

impl TaskStopInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("task_stop", "task_id", &self.task_id)
    }
}

#[derive(Debug, Default)]
pub struct TaskOutputTool;

#[derive(Debug, Default)]
pub struct TaskStopTool;

#[async_trait]
impl Tool for TaskOutputTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "task_output",
            "Read recent output from a background task",
            ToolKind::Task,
        )
        .with_input_schema(
            ToolSchema::object()
                .property("task_id", ToolSchema::string("background task id"))
                .property(
                    "lines",
                    ToolSchema::integer("number of trailing log lines to return"),
                )
                .required("task_id"),
        );
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec.required_features.insert(FeatureFlag::BackgroundTasks);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<TaskOutputInput>("task_output", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<TaskOutputInput>("task_output", &input)?;
        input.validate()?;

        let content = read_task_output(&app_root()?, &input.task_id, input.lines())?;
        Ok(ToolResult::success(use_id, content))
    }
}

#[async_trait]
impl Tool for TaskStopTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec("task_stop", "Request task cancellation", ToolKind::Task)
            .with_input_schema(
                ToolSchema::object()
                    .property("task_id", ToolSchema::string("background task id"))
                    .required("task_id"),
            );
        spec.destructive = true;
        spec.required_features.insert(FeatureFlag::BackgroundTasks);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<TaskStopInput>("task_stop", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<TaskStopInput>("task_stop", &input)?;
        input.validate()?;

        send_cancel_signal(&app_root()?, &input.task_id)?;
        Ok(ToolResult::success(use_id, "task cancel signal sent"))
    }
}

fn read_task_output(app_root: &Path, task_id: &str, lines: usize) -> Result<String> {
    let path = task_dir(app_root, task_id).join("output.log");
    if !path.exists() {
        return Err(WonderError::not_found("task", task_id));
    }
    let content = fs::read_to_string(path)?;
    let line_limit = lines.max(1);
    let mut lines = content.lines().collect::<Vec<_>>();
    if lines.len() > line_limit {
        lines.drain(0..lines.len() - line_limit);
    }
    Ok(lines.join("\n"))
}

fn send_cancel_signal(app_root: &Path, task_id: &str) -> Result<()> {
    let dir = task_dir(app_root, task_id);
    if !dir.exists() {
        return Err(WonderError::not_found("task", task_id));
    }
    fs::write(dir.join("cancel"), "cancel\n")?;
    Ok(())
}

fn task_dir(app_root: &Path, task_id: &str) -> PathBuf {
    app_root.join("tasks").join(task_id)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    #[test]
    fn task_output_validation_rejects_zero_lines() {
        let tool = TaskOutputTool;
        let error = tool
            .validate_input(&json!({ "task_id": "task-1", "lines": 0 }))
            .expect_err("invalid lines");

        assert!(error.to_string().contains("lines"));
    }

    #[test]
    fn task_output_reads_log_tail() {
        let dir = unique_test_dir("tools-task-output");
        let task_dir = dir.join("tasks").join("task-1");
        fs::create_dir_all(&task_dir).expect("task dir");
        fs::write(task_dir.join("output.log"), "one\ntwo\nthree\n").expect("log");

        let output = read_task_output(&dir, "task-1", 2).expect("tail");

        assert_eq!(output, "two\nthree");
    }

    #[test]
    fn task_stop_errors_for_missing_task() {
        let dir = unique_test_dir("tools-task-stop");
        let error = send_cancel_signal(&dir, "missing").expect_err("missing task");

        assert!(error.to_string().contains("task not found"));
    }
}
