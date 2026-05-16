//! Fleet coordination tools: `fleet_results` and `fleet_wait`.
//!
//! These tools let a running agent inspect the results of its sub-agents and
//! block until the fleet finishes.  Both are gated on [`FeatureFlag::Fleet`] and
//! [`FeatureFlag::Agents`].

use std::{env, thread, time::Duration};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{
    FeatureFlag, FleetId, Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec,
    ToolUseId,
};
use wonder_of_u_storage::{AgentTaskResultStore, FleetInspector, MemberObservationClass};

use crate::{app_root, base_spec, parse_input};

// ── FleetResultsTool ─────────────────────────────────────────────────────────

/// Input for the `fleet_results` tool.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FleetResultsInput {
    /// Fleet run id. Falls back to `WONDER_OF_U_FLEET_ID` env var when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fleet_id: Option<String>,
    /// When set, only return results for these request ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_ids: Option<Vec<String>>,
    /// When true, include the full `output_text` field (may be large).
    #[serde(default)]
    pub include_logs: bool,
    /// Truncate each member's output to at most this many characters.
    #[serde(default = "default_max_output_chars")]
    pub max_output_chars: usize,
}

fn default_max_output_chars() -> usize {
    2000
}

/// Returns a formatted summary of fleet member results.
pub struct FleetResultsTool;

impl FleetResultsTool {
    fn make_spec() -> ToolSpec {
        let mut spec = base_spec(
            "fleet_results",
            "Return aggregated results from fleet sub-agent tasks. \
             Each member's status, output excerpt, and optional full output are included. \
             Resolves the fleet_id from the input or the WONDER_OF_U_FLEET_ID env var.",
            ToolKind::Task,
        )
        .with_input_schema(
            ToolSchema::object()
                .property(
                    "fleet_id",
                    ToolSchema::string(
                        "Fleet run UUID. Defaults to the WONDER_OF_U_FLEET_ID env var.",
                    ),
                )
                .property(
                    "request_ids",
                    json!({
                        "type": "array",
                        "description": "Optional filter: only these fleet request ids.",
                        "items": { "type": "string" }
                    }),
                )
                .property(
                    "include_logs",
                    ToolSchema::boolean("Include full output_text (may be large). Default false."),
                )
                .property(
                    "max_output_chars",
                    ToolSchema::integer(
                        "Truncate per-member output to this many chars. Default 2000.",
                    ),
                ),
        );
        spec.required_features.insert(FeatureFlag::Fleet);
        spec.required_features.insert(FeatureFlag::Agents);
        spec
    }
}

#[async_trait]
impl Tool for FleetResultsTool {
    fn spec(&self) -> ToolSpec {
        Self::make_spec()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input: FleetResultsInput = parse_input("fleet_results", &input)?;

        let fleet_id_str = match input
            .fleet_id
            .clone()
            .or_else(|| env::var("WONDER_OF_U_FLEET_ID").ok())
        {
            Some(s) => s,
            None => {
                return Ok(ToolResult::failure(
                    use_id,
                    "fleet_results: no fleet_id provided and WONDER_OF_U_FLEET_ID is not set",
                ));
            }
        };
        let fleet_id = match fleet_id_str.parse::<FleetId>() {
            Ok(id) => id,
            Err(e) => {
                return Ok(ToolResult::failure(
                    use_id,
                    format!("fleet_results: invalid fleet_id `{fleet_id_str}`: {e}"),
                ));
            }
        };

        let root = app_root()?;
        let inspector = FleetInspector::new(&root);
        let observation = match inspector.observe(fleet_id) {
            Ok(obs) => obs,
            Err(e) => return Ok(ToolResult::failure(use_id, e.to_string())),
        };

        let result_store = AgentTaskResultStore::new(&root);
        let mut lines: Vec<String> = Vec::new();
        lines.push(format!("fleet_id={fleet_id}"));
        lines.push(format!("members={}", observation.members.len()));
        lines.push(format!(
            "completed={}",
            observation.count_by_class(MemberObservationClass::Completed)
        ));
        lines.push(format!(
            "failed={}",
            observation.count_by_class(MemberObservationClass::Failed)
        ));
        lines.push(format!(
            "running={}",
            observation.count_by_class(MemberObservationClass::Running)
        ));
        lines.push(format!(
            "pending={}",
            observation.count_by_class(MemberObservationClass::Pending)
        ));
        lines.push(format!("all_terminal={}", observation.all_terminal()));
        lines.push(String::new());

        for member in &observation.members {
            // Resolve the request id from the result sidecar or task state.
            let request_id = member
                .result
                .as_ref()
                .and_then(|r| r.fleet_request_id.clone())
                .or_else(|| {
                    member
                        .task
                        .as_ref()
                        .and_then(|t| t.fleet_request_id.clone())
                })
                .unwrap_or_else(|| member.task_id.to_string());

            if let Some(filter) = &input.request_ids {
                if !filter.contains(&request_id) {
                    continue;
                }
            }

            lines.push(format!("--- task_id={} ---", member.task_id));
            lines.push(format!("status={}", member.class.label()));
            lines.push(format!("request_id={request_id}"));

            if let Some(result) = &member.result {
                if !result.output_excerpt.is_empty() {
                    lines.push(format!("excerpt={}", result.output_excerpt));
                }
                if input.include_logs {
                    if let Some(text) = &result.output_text {
                        let truncated = if text.len() > input.max_output_chars {
                            format!("{}…", &text[..input.max_output_chars])
                        } else {
                            text.clone()
                        };
                        lines.push(format!("output={truncated}"));
                    }
                }
            } else if let Some(task) = &member.task {
                // No result sidecar yet — surface the task's status message.
                if let Some(msg) = &task.status_message {
                    lines.push(format!("status_message={msg}"));
                }
                // Best-effort: read a result that may have just been written.
                if let Ok(full) = result_store.read_result(member.task_id) {
                    if !full.output_excerpt.is_empty() {
                        lines.push(format!("excerpt={}", full.output_excerpt));
                    }
                }
            }
            lines.push(String::new());
        }

        Ok(ToolResult::success(use_id, lines.join("\n")))
    }
}

// ── FleetWaitTool ─────────────────────────────────────────────────────────────

/// Input for the `fleet_wait` tool.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FleetWaitInput {
    /// Fleet run id. Falls back to `WONDER_OF_U_FLEET_ID` env var when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fleet_id: Option<String>,
    /// Maximum seconds to wait before returning with a timeout status.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// How often to poll (seconds).
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
}

fn default_timeout_secs() -> u64 {
    120
}
fn default_poll_interval_secs() -> u64 {
    2
}

/// Blocks until all fleet member tasks reach a terminal state or a timeout elapses.
pub struct FleetWaitTool;

impl FleetWaitTool {
    fn make_spec() -> ToolSpec {
        let mut spec = base_spec(
            "fleet_wait",
            "Block until all tasks in a fleet run reach a terminal state (completed or failed), \
             or until the timeout expires.  Returns a summary of the final observation.",
            ToolKind::Task,
        )
        .with_input_schema(
            ToolSchema::object()
                .property(
                    "fleet_id",
                    ToolSchema::string("Fleet run UUID. Defaults to WONDER_OF_U_FLEET_ID env var."),
                )
                .property(
                    "timeout_secs",
                    ToolSchema::integer("Seconds before giving up. Default 120."),
                )
                .property(
                    "poll_interval_secs",
                    ToolSchema::integer("Poll interval in seconds. Default 2."),
                ),
        );
        spec.required_features.insert(FeatureFlag::Fleet);
        spec.required_features.insert(FeatureFlag::Agents);
        spec
    }
}

#[async_trait]
impl Tool for FleetWaitTool {
    fn spec(&self) -> ToolSpec {
        Self::make_spec()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input: FleetWaitInput = parse_input("fleet_wait", &input)?;

        let fleet_id_str = match input
            .fleet_id
            .clone()
            .or_else(|| env::var("WONDER_OF_U_FLEET_ID").ok())
        {
            Some(s) => s,
            None => {
                return Ok(ToolResult::failure(
                    use_id,
                    "fleet_wait: no fleet_id provided and WONDER_OF_U_FLEET_ID is not set",
                ));
            }
        };
        let fleet_id = match fleet_id_str.parse::<FleetId>() {
            Ok(id) => id,
            Err(e) => {
                return Ok(ToolResult::failure(
                    use_id,
                    format!("fleet_wait: invalid fleet_id `{fleet_id_str}`: {e}"),
                ));
            }
        };

        let root = app_root()?;
        let inspector = FleetInspector::new(&root);

        let interval = Duration::from_secs(input.poll_interval_secs.max(1));
        let deadline = std::time::Instant::now() + Duration::from_secs(input.timeout_secs);

        loop {
            let observation = match inspector.observe(fleet_id) {
                Ok(obs) => obs,
                Err(e) => return Ok(ToolResult::failure(use_id, e.to_string())),
            };

            if observation.all_terminal() {
                let text = format!(
                    "fleet_id={fleet_id}\ntimed_out=false\nmembers={}\ncompleted={}\nfailed={}",
                    observation.members.len(),
                    observation.count_by_class(MemberObservationClass::Completed),
                    observation.count_by_class(MemberObservationClass::Failed),
                );
                return Ok(ToolResult::success(use_id, text));
            }

            if std::time::Instant::now() >= deadline {
                // One final read for accurate counts.
                let observation = inspector.observe(fleet_id).unwrap_or(observation);
                let text = format!(
                    "fleet_id={fleet_id}\ntimed_out=true\nmembers={}\ncompleted={}\nfailed={}\npending={}\nrunning={}",
                    observation.members.len(),
                    observation.count_by_class(MemberObservationClass::Completed),
                    observation.count_by_class(MemberObservationClass::Failed),
                    observation.count_by_class(MemberObservationClass::Pending),
                    observation.count_by_class(MemberObservationClass::Running),
                );
                return Ok(ToolResult::success(use_id, text));
            }

            // `std::thread::sleep` is safe here: the CLI uses `futures::executor::block_on`
            // rather than a multithreaded async runtime, so blocking the thread is fine.
            thread::sleep(interval);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn fleet_results_tool_spec_has_fleet_and_agents_features() {
        let spec = FleetResultsTool::make_spec();
        assert!(spec.required_features.contains(&FeatureFlag::Fleet));
        assert!(spec.required_features.contains(&FeatureFlag::Agents));
        assert_eq!(spec.name, "fleet_results");
    }

    #[test]
    fn fleet_wait_tool_spec_has_fleet_and_agents_features() {
        let spec = FleetWaitTool::make_spec();
        assert!(spec.required_features.contains(&FeatureFlag::Fleet));
        assert!(spec.required_features.contains(&FeatureFlag::Agents));
        assert_eq!(spec.name, "fleet_wait");
    }

    #[test]
    fn fleet_results_input_defaults() {
        let input: FleetResultsInput = serde_json::from_value(json!({})).expect("default parse");
        assert!(!input.include_logs);
        assert_eq!(input.max_output_chars, 2000);
    }

    #[test]
    fn fleet_wait_input_defaults() {
        let input: FleetWaitInput = serde_json::from_value(json!({})).expect("default parse");
        assert_eq!(input.timeout_secs, 120);
        assert_eq!(input.poll_interval_secs, 2);
    }
}
