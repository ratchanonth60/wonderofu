use async_trait::async_trait;
use serde_json::{Value, json};
use wonder_of_u_core::{
    PermissionDecision, PermissionDecisionReason, PermissionMode, Result, Tool, ToolContext,
    ToolKind, ToolResult, ToolSpec, ToolUseId,
};

/// Pseudo-tool used in YOLO (bypass-permissions) mode. The model calls this
/// tool to report its security classification (`safe` or `unsafe`) before
/// executing a proposed action.
#[derive(Debug, Default)]
pub struct ClassifyResultTool;

#[async_trait]
impl Tool for ClassifyResultTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = ToolSpec::new(
            "classify_result",
            "Report the security classification result for the agent action",
            ToolKind::Interaction,
        );
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "thinking": {
                    "type": "string",
                    "description": "Brief step-by-step reasoning."
                },
                "shouldBlock": {
                    "type": "boolean",
                    "description": "Whether the action should be blocked (true) or allowed (false)"
                },
                "reason": {
                    "type": "string",
                    "description": "Brief explanation of the classification decision"
                }
            },
            "required": ["thinking", "shouldBlock", "reason"]
        });
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn permission_decision(&self, _context: &ToolContext, _input: &Value) -> PermissionDecision {
        PermissionDecision::allow(PermissionDecisionReason::Mode {
            mode: PermissionMode::BypassPermissions,
            detail: "classify_result pseudo-tool is always allowed".into(),
        })
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let should_block = input
            .get("shouldBlock")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let reason = input
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("no reason provided");
        let verdict = if should_block { "unsafe" } else { "safe" };

        Ok(ToolResult::success(
            use_id,
            format!("Classification recorded: {verdict} — {reason}"),
        ))
    }
}
