//! Source-compatible cron and remote trigger tool shims.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use wonder_of_u_core::{
    FeatureFlag, PermissionDecision, PermissionRequest, Result, Tool, ToolContext, ToolKind,
    ToolResult, ToolSchema, ToolSpec, ToolUseId, WonderError, evaluate_permission,
};

use crate::{base_spec, parse_input, require_non_empty_text};

const SOURCE_CRON_RUNTIME_UNAVAILABLE: &str =
    "the Rust port does not implement source-compatible cron scheduling yet";
const REMOTE_TRIGGER_RUNTIME_UNAVAILABLE: &str =
    "the Rust port does not implement the claude.ai remote trigger service yet";
/// Represents cron create input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CronCreateInput {
    /// Stores the cron
    pub cron: String,
    /// Stores the prompt
    pub prompt: String,
    /// Stores the recurring
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurring: Option<bool>,
    /// Stores the durable
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durable: Option<bool>,
}

impl CronCreateInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("cron_create", "cron", &self.cron)?;
        require_non_empty_text("cron_create", "prompt", &self.prompt)?;
        validate_cron_expression("cron_create", "cron", &self.cron)
    }
}
/// Represents cron delete input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CronDeleteInput {
    /// Stores the id
    pub id: String,
}

impl CronDeleteInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("cron_delete", "id", &self.id)
    }
}
/// Enumerates remote trigger action
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteTriggerAction {
    /// Represents list
    List,
    /// Represents get
    Get,
    /// Represents create
    Create,
    /// Represents update
    Update,
    /// Represents run
    Run,
}

impl RemoteTriggerAction {
    const fn is_read_only(self) -> bool {
        matches!(self, Self::List | Self::Get)
    }
}
/// Represents remote trigger input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteTriggerInput {
    /// Stores the action
    pub action: RemoteTriggerAction,
    /// Stores the trigger identifier
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_id: Option<String>,
    /// Stores the body
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Map<String, Value>>,
}

impl RemoteTriggerInput {
    fn validate(&self) -> Result<()> {
        match self.action {
            RemoteTriggerAction::List => {}
            RemoteTriggerAction::Get | RemoteTriggerAction::Run => {
                validate_trigger_id(self.trigger_id.as_deref(), self.action)?
            }
            RemoteTriggerAction::Create => {
                validate_remote_trigger_body(self.body.as_ref(), self.action)?;
            }
            RemoteTriggerAction::Update => {
                validate_trigger_id(self.trigger_id.as_deref(), self.action)?;
                validate_remote_trigger_body(self.body.as_ref(), self.action)?;
            }
        }

        if let Some(body) = &self.body {
            validate_optional_body_name(body)?;
            validate_optional_body_schedule(body)?;
        }

        Ok(())
    }
}
/// Represents cron create tool
#[derive(Debug, Default)]
pub struct CronCreateTool;
/// Represents cron delete tool
#[derive(Debug, Default)]
pub struct CronDeleteTool;
/// Represents remote trigger tool
#[derive(Debug, Default)]
pub struct RemoteTriggerTool;

#[async_trait]
impl Tool for CronCreateTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "cron_create",
            "Schedule a recurring or one-shot prompt",
            ToolKind::Task,
        )
        .with_input_schema(
            ToolSchema::object()
                .property(
                    "cron",
                    ToolSchema::string(
                        "standard 5-field cron expression in local time: minute hour day-of-month month day-of-week",
                    ),
                )
                .property("prompt", ToolSchema::string("prompt to enqueue when the job fires"))
                .property(
                    "recurring",
                    ToolSchema::boolean(
                        "true to repeat on every cron match; false for a one-shot run",
                    ),
                )
                .property(
                    "durable",
                    ToolSchema::boolean(
                        "true to persist across sessions when durable cron storage exists",
                    ),
                )
                .required("cron")
                .required("prompt"),
        );
        spec.aliases.push("CronCreate".into());
        spec.destructive = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<CronCreateInput>("cron_create", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<CronCreateInput>("cron_create", &input)?;
        input.validate()?;
        Ok(unsupported_result(
            use_id,
            "cron_create",
            "source_cron",
            SOURCE_CRON_RUNTIME_UNAVAILABLE,
        ))
    }
}

#[async_trait]
impl Tool for CronDeleteTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec("cron_delete", "Cancel a scheduled cron job", ToolKind::Task)
            .with_input_schema(
                ToolSchema::object()
                    .property("id", ToolSchema::string("job id returned by cron_create"))
                    .required("id"),
            );
        spec.aliases.push("CronDelete".into());
        spec.destructive = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<CronDeleteInput>("cron_delete", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<CronDeleteInput>("cron_delete", &input)?;
        input.validate()?;
        Ok(unsupported_result(
            use_id,
            "cron_delete",
            "source_cron",
            SOURCE_CRON_RUNTIME_UNAVAILABLE,
        ))
    }
}

#[async_trait]
impl Tool for RemoteTriggerTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "remote_trigger",
            "Manage scheduled remote Claude Code triggers",
            ToolKind::Task,
        )
        .with_input_schema(
            ToolSchema::object()
                .property(
                    "action",
                    ToolSchema::enumeration(
                        "remote trigger API action",
                        ["list", "get", "create", "update", "run"],
                    ),
                )
                .property(
                    "trigger_id",
                    ToolSchema::string("required for get, update, and run"),
                )
                .property(
                    "body",
                    json!({
                        "type": "object",
                        "description": "JSON body for create and update",
                        "additionalProperties": true,
                    }),
                )
                .required("action"),
        );
        spec.aliases.push("RemoteTrigger".into());
        spec.concurrency_safe = true;
        spec.required_features.insert(FeatureFlag::RemoteTriggers);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<RemoteTriggerInput>("remote_trigger", input)?.validate()
    }

    fn permission_decision(&self, context: &ToolContext, input: &Value) -> PermissionDecision {
        let spec = self.spec();
        let parsed = match parse_input::<RemoteTriggerInput>("remote_trigger", input) {
            Ok(parsed) => parsed,
            Err(_) => {
                return evaluate_permission(
                    &context.permission_context(),
                    &PermissionRequest::new(spec.name),
                );
            }
        };
        let request = PermissionRequest::new(spec.name)
            .with_aliases(spec.aliases)
            .read_only(parsed.action.is_read_only())
            .destructive(!parsed.action.is_read_only());
        evaluate_permission(&context.permission_context(), &request)
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<RemoteTriggerInput>("remote_trigger", &input)?;
        input.validate()?;
        Ok(unsupported_result(
            use_id,
            "remote_trigger",
            "remote_trigger",
            REMOTE_TRIGGER_RUNTIME_UNAVAILABLE,
        ))
    }
}

fn unsupported_result(
    use_id: ToolUseId,
    tool_name: &str,
    tool_family: &str,
    reason: &str,
) -> ToolResult {
    let mut result = ToolResult::failure(
        use_id,
        format!(
            "{tool_name} is unavailable: {reason}; only source-compatible input validation is implemented"
        ),
    );
    result.metadata = json!({
        "supported": false,
        "tool": tool_name,
        "tool_family": tool_family,
        "reason": reason,
    });
    result
}

fn validate_cron_expression(tool_name: &str, field: &str, expression: &str) -> Result<()> {
    let fields = expression.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 5 {
        return Err(WonderError::validation(format!(
            "{tool_name} requires `{field}` to use exactly 5 cron fields: minute hour day-of-month month day-of-week"
        )));
    }

    let ranges = [
        ("minute", 0_u32, 59_u32),
        ("hour", 0, 23),
        ("day-of-month", 1, 31),
        ("month", 1, 12),
        ("day-of-week", 0, 7),
    ];

    for (value, (name, min, max)) in fields.into_iter().zip(ranges) {
        validate_cron_field(tool_name, field, name, value, min, max)?;
    }

    Ok(())
}

fn validate_cron_field(
    tool_name: &str,
    field: &str,
    field_name: &str,
    value: &str,
    min: u32,
    max: u32,
) -> Result<()> {
    if value.is_empty() {
        return Err(WonderError::validation(format!(
            "{tool_name} `{field}` has an empty {field_name} field"
        )));
    }

    for segment in value.split(',') {
        validate_cron_segment(tool_name, field, field_name, segment, min, max)?;
    }

    Ok(())
}

fn validate_cron_segment(
    tool_name: &str,
    field: &str,
    field_name: &str,
    segment: &str,
    min: u32,
    max: u32,
) -> Result<()> {
    if segment.is_empty() {
        return Err(WonderError::validation(format!(
            "{tool_name} `{field}` has an empty {field_name} segment"
        )));
    }

    let (base, step) = match segment.split_once('/') {
        Some((base, step)) => (base, Some(step)),
        None => (segment, None),
    };

    if let Some(step) = step {
        let step = parse_cron_number(tool_name, field, field_name, step)?;
        if step == 0 {
            return Err(WonderError::validation(format!(
                "{tool_name} `{field}` has a zero step in the {field_name} field"
            )));
        }
    }

    if base == "*" {
        return Ok(());
    }

    if let Some((start, end)) = base.split_once('-') {
        let start = parse_cron_number(tool_name, field, field_name, start)?;
        let end = parse_cron_number(tool_name, field, field_name, end)?;
        validate_cron_number(tool_name, field, field_name, start, min, max)?;
        validate_cron_number(tool_name, field, field_name, end, min, max)?;
        if start > end {
            return Err(WonderError::validation(format!(
                "{tool_name} `{field}` has a descending range in the {field_name} field"
            )));
        }
        return Ok(());
    }

    let number = parse_cron_number(tool_name, field, field_name, base)?;
    validate_cron_number(tool_name, field, field_name, number, min, max)
}

fn parse_cron_number(tool_name: &str, field: &str, field_name: &str, value: &str) -> Result<u32> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(WonderError::validation(format!(
            "{tool_name} `{field}` has an invalid token `{value}` in the {field_name} field"
        )));
    }

    value.parse::<u32>().map_err(|error| {
        WonderError::validation(format!(
            "{tool_name} `{field}` could not parse `{value}` in the {field_name} field: {error}"
        ))
    })
}

fn validate_cron_number(
    tool_name: &str,
    field: &str,
    field_name: &str,
    number: u32,
    min: u32,
    max: u32,
) -> Result<()> {
    if (min..=max).contains(&number) {
        Ok(())
    } else {
        Err(WonderError::validation(format!(
            "{tool_name} `{field}` value {number} is out of range for the {field_name} field ({min}-{max})"
        )))
    }
}

fn validate_trigger_id(trigger_id: Option<&str>, action: RemoteTriggerAction) -> Result<()> {
    let trigger_id = trigger_id.ok_or_else(|| {
        WonderError::validation(format!(
            "remote_trigger action `{}` requires `trigger_id`",
            remote_trigger_action_name(action)
        ))
    })?;
    require_non_empty_text("remote_trigger", "trigger_id", trigger_id)?;
    if trigger_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        Ok(())
    } else {
        Err(WonderError::validation(
            "remote_trigger `trigger_id` must contain only ASCII letters, digits, `_`, or `-`",
        ))
    }
}

fn validate_remote_trigger_body(
    body: Option<&Map<String, Value>>,
    action: RemoteTriggerAction,
) -> Result<()> {
    body.ok_or_else(|| {
        WonderError::validation(format!(
            "remote_trigger action `{}` requires `body`",
            remote_trigger_action_name(action)
        ))
    })?;
    Ok(())
}

fn validate_optional_body_name(body: &Map<String, Value>) -> Result<()> {
    let Some(name) = body.get("name") else {
        return Ok(());
    };
    let name = name
        .as_str()
        .ok_or_else(|| WonderError::validation("remote_trigger body `name` must be a string"))?;
    require_non_empty_text("remote_trigger", "body.name", name)
}

fn validate_optional_body_schedule(body: &Map<String, Value>) -> Result<()> {
    let Some(schedule) = body.get("schedule") else {
        return Ok(());
    };
    let schedule = schedule.as_str().ok_or_else(|| {
        WonderError::validation("remote_trigger body `schedule` must be a string")
    })?;
    require_non_empty_text("remote_trigger", "body.schedule", schedule)?;
    validate_cron_expression("remote_trigger", "body.schedule", schedule)
}

const fn remote_trigger_action_name(action: RemoteTriggerAction) -> &'static str {
    match action {
        RemoteTriggerAction::List => "list",
        RemoteTriggerAction::Get => "get",
        RemoteTriggerAction::Create => "create",
        RemoteTriggerAction::Update => "update",
        RemoteTriggerAction::Run => "run",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use futures::executor::block_on;
    use serde_json::json;
    use wonder_of_u_core::{
        FeatureSet, PermissionDecision, PermissionMode, SessionId, ToolContext, ToolUseId,
    };
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
    fn cron_create_validation_rejects_invalid_field_count() {
        let error = CronCreateTool
            .validate_input(&json!({
                "cron": "*/5 * * *",
                "prompt": "check the deploy",
            }))
            .expect_err("invalid cron");

        assert!(error.to_string().contains("exactly 5 cron fields"));
    }

    #[test]
    fn cron_create_validation_rejects_out_of_range_values() {
        let error = CronCreateTool
            .validate_input(&json!({
                "cron": "60 * * * *",
                "prompt": "check the deploy",
            }))
            .expect_err("out of range cron");

        assert!(error.to_string().contains("out of range"));
    }

    #[test]
    fn cron_delete_validation_rejects_blank_id() {
        let error = CronDeleteTool
            .validate_input(&json!({ "id": "   " }))
            .expect_err("blank id");

        assert!(error.to_string().contains("non-empty `id`"));
    }

    #[test]
    fn cron_create_execute_reports_unsupported_runtime() {
        let result = block_on(CronCreateTool.execute(
            tool_context(unique_test_dir("tools-cron-create-unsupported")),
            ToolUseId::new(),
            json!({
                "cron": "*/5 * * * *",
                "prompt": "check the deploy",
                "recurring": true,
            }),
        ))
        .expect("result");

        assert!(!result.success);
        assert!(result.content.contains("cron_create is unavailable"));
        assert_eq!(result.metadata["supported"], json!(false));
        assert_eq!(result.metadata["tool"], json!("cron_create"));
    }

    #[test]
    fn cron_delete_execute_reports_unsupported_runtime() {
        let result = block_on(CronDeleteTool.execute(
            tool_context(unique_test_dir("tools-cron-delete-unsupported")),
            ToolUseId::new(),
            json!({ "id": "job-1" }),
        ))
        .expect("result");

        assert!(!result.success);
        assert!(result.content.contains("cron_delete is unavailable"));
        assert_eq!(result.metadata["tool_family"], json!("source_cron"));
    }

    #[test]
    fn remote_trigger_validation_requires_trigger_id_for_run() {
        let error = RemoteTriggerTool
            .validate_input(&json!({ "action": "run" }))
            .expect_err("missing trigger id");

        assert!(error.to_string().contains("requires `trigger_id`"));
    }

    #[test]
    fn remote_trigger_validation_requires_body_for_update() {
        let error = RemoteTriggerTool
            .validate_input(&json!({
                "action": "update",
                "trigger_id": "trigger-1",
            }))
            .expect_err("missing body");

        assert!(error.to_string().contains("requires `body`"));
    }

    #[test]
    fn remote_trigger_validation_rejects_invalid_schedule_in_body() {
        let error = RemoteTriggerTool
            .validate_input(&json!({
                "action": "create",
                "body": {
                    "name": "daily sync",
                    "schedule": "61 * * * *",
                },
            }))
            .expect_err("invalid schedule");

        assert!(error.to_string().contains("body.schedule"));
    }

    #[test]
    fn remote_trigger_list_is_treated_as_read_only_for_permissions() {
        let decision = RemoteTriggerTool.permission_decision(
            &tool_context(PathBuf::from("/workspace")),
            &json!({ "action": "list" }),
        );

        assert!(matches!(decision, PermissionDecision::Allow { .. }));
    }

    #[test]
    fn remote_trigger_create_requires_confirmation_for_permissions() {
        let decision = RemoteTriggerTool.permission_decision(
            &tool_context(PathBuf::from("/workspace")),
            &json!({
                "action": "create",
                "body": { "name": "daily sync" },
            }),
        );

        assert!(matches!(decision, PermissionDecision::Ask { .. }));
    }

    #[test]
    fn remote_trigger_execute_reports_unsupported_runtime() {
        let result = block_on(RemoteTriggerTool.execute(
            tool_context(unique_test_dir("tools-remote-trigger-unsupported")),
            ToolUseId::new(),
            json!({
                "action": "create",
                "body": {
                    "name": "daily sync",
                    "schedule": "7 * * * *",
                },
            }),
        ))
        .expect("result");

        assert!(!result.success);
        assert!(result.content.contains("remote_trigger is unavailable"));
        assert_eq!(result.metadata["supported"], json!(false));
        assert_eq!(result.metadata["tool"], json!("remote_trigger"));
    }
}
