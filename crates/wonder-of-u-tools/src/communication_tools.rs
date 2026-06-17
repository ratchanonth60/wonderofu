//! Source-compatible agent/team communication tool shims.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{
    AgentMessageSpec, FeatureFlag, Result, Tool, ToolContext, ToolEffect, ToolKind, ToolResult,
    ToolSchema, ToolSpec, ToolUseId, WonderError,
};

use crate::{base_spec, parse_input, require_non_empty_text};

const TEAM_LEAD_NAME: &str = "team-lead";
const COMMUNICATION_RUNTIME_UNAVAILABLE: &str =
    "peer/team/message transport is not implemented in wonder-of-u-tools";
const PEER_DISCOVERY_UNAVAILABLE: &str =
    "live peer discovery is not implemented in wonder-of-u-tools";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecipientKind {
    Team,
    Uds,
    Bridge,
}

impl RecipientKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Team => "team",
            Self::Uds => "uds",
            Self::Bridge => "bridge",
        }
    }
}

fn recipient_kind(value: &str) -> RecipientKind {
    if value.starts_with("uds:") {
        RecipientKind::Uds
    } else if value.starts_with("bridge:") {
        RecipientKind::Bridge
    } else {
        RecipientKind::Team
    }
}

fn recipient_target(value: &str, kind: RecipientKind) -> &str {
    match kind {
        RecipientKind::Team => value,
        RecipientKind::Uds => value.trim_start_matches("uds:"),
        RecipientKind::Bridge => value.trim_start_matches("bridge:"),
    }
}

fn unsupported_result(
    use_id: ToolUseId,
    tool_name: &str,
    reason: &str,
    metadata: Value,
) -> ToolResult {
    let mut result = ToolResult::failure(use_id, format!("{tool_name} is unavailable: {reason}"));
    result.metadata = metadata;
    result
}

fn communication_spec(name: &str, description: &str) -> ToolSpec {
    let mut spec = base_spec(name, description, ToolKind::Agent);
    spec.required_features.insert(FeatureFlag::Agents);
    spec
}
/// Enumerates structured message
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StructuredMessage {
    /// Represents shutdown request
    ShutdownRequest {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        /// Stores the reason
        reason: Option<String>,
    },
    /// Represents shutdown response
    ShutdownResponse {
        /// Stores the request id
        request_id: String,
        /// Stores the approve
        approve: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        /// Stores the reason
        reason: Option<String>,
    },
    /// Represents plan approval response
    PlanApprovalResponse {
        /// Stores the request id
        request_id: String,
        /// Stores the approve
        approve: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        /// Stores the feedback
        feedback: Option<String>,
    },
}

impl StructuredMessage {
    fn validate(&self, to: &str) -> Result<()> {
        if to == "*" {
            return Err(WonderError::validation(
                "send_message structured messages cannot be broadcast (to: \"*\")",
            ));
        }

        match self {
            Self::ShutdownRequest { reason } => {
                if let Some(reason) = reason {
                    require_non_empty_text("send_message", "message.reason", reason)?;
                }
            }
            Self::ShutdownResponse {
                request_id,
                approve,
                reason,
            } => {
                require_non_empty_text("send_message", "message.request_id", request_id)?;
                if !to.eq_ignore_ascii_case(TEAM_LEAD_NAME) {
                    return Err(WonderError::validation(format!(
                        "send_message shutdown_response must be sent to \"{TEAM_LEAD_NAME}\""
                    )));
                }
                if !approve {
                    require_non_empty_text(
                        "send_message",
                        "message.reason",
                        reason.as_deref().unwrap_or(""),
                    )?;
                }
            }
            Self::PlanApprovalResponse {
                request_id,
                feedback,
                ..
            } => {
                require_non_empty_text("send_message", "message.request_id", request_id)?;
                if let Some(feedback) = feedback {
                    require_non_empty_text("send_message", "message.feedback", feedback)?;
                }
            }
        }

        Ok(())
    }

    const fn kind(&self) -> &'static str {
        match self {
            Self::ShutdownRequest { .. } => "shutdown_request",
            Self::ShutdownResponse { .. } => "shutdown_response",
            Self::PlanApprovalResponse { .. } => "plan_approval_response",
        }
    }
}
/// Enumerates send message payload
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SendMessagePayload {
    /// Represents text
    Text(String),
    /// Represents structured
    Structured(StructuredMessage),
}

impl SendMessagePayload {
    fn validate(&self, to: &str, kind: RecipientKind, summary: Option<&str>) -> Result<()> {
        match self {
            Self::Text(message) => {
                require_non_empty_text("send_message", "message", message)?;
                if kind == RecipientKind::Team {
                    require_non_empty_text("send_message", "summary", summary.unwrap_or_default())?;
                }
            }
            Self::Structured(message) => {
                if kind != RecipientKind::Team {
                    return Err(WonderError::validation(
                        "send_message structured messages cannot be sent cross-session in the Rust runtime",
                    ));
                }
                message.validate(to)?;
            }
        }

        Ok(())
    }

    const fn kind(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::Structured(message) => message.kind(),
        }
    }
}
/// Represents send message input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendMessageInput {
    /// Stores the to
    pub to: String,
    /// Stores the summary
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Stores the message
    pub message: SendMessagePayload,
}

impl SendMessageInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("send_message", "to", &self.to)?;
        let kind = recipient_kind(&self.to);
        require_non_empty_text("send_message", "to", recipient_target(&self.to, kind))?;
        if self.to.contains('@') {
            return Err(WonderError::validation(
                "send_message `to` must be a bare teammate name or \"*\"",
            ));
        }
        self.message
            .validate(&self.to, kind, self.summary.as_deref())
    }
}
/// Represents send message tool
#[derive(Debug, Default)]
pub struct SendMessageTool;

#[async_trait]
impl Tool for SendMessageTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = communication_spec(
            "send_message",
            "Source-compatible SendMessage alias; team messages are persisted as advisory fleet steering records",
        )
        .with_input_schema(ToolSchema::object());
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "to": ToolSchema::string(
                    "recipient teammate name, \"*\" for broadcast, or source-compatible uds:/bridge: address",
                ),
                "summary": ToolSchema::string(
                    "5-10 word preview shown for plain-text teammate messages",
                ),
                "message": {
                    "description": "plain-text content or a source-compatible structured swarm message",
                    "oneOf": [
                        { "type": "string" },
                        {
                            "type": "object",
                            "oneOf": [
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": {
                                            "type": "string",
                                            "enum": ["shutdown_request"]
                                        },
                                        "reason": ToolSchema::string("optional shutdown reason")
                                    },
                                    "required": ["type"],
                                    "additionalProperties": false
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": {
                                            "type": "string",
                                            "enum": ["shutdown_response"]
                                        },
                                        "request_id": ToolSchema::string("request identifier"),
                                        "approve": ToolSchema::boolean("shutdown approval decision"),
                                        "reason": ToolSchema::string("required when approve is false")
                                    },
                                    "required": ["approve", "request_id", "type"],
                                    "additionalProperties": false
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": {
                                            "type": "string",
                                            "enum": ["plan_approval_response"]
                                        },
                                        "request_id": ToolSchema::string("request identifier"),
                                        "approve": ToolSchema::boolean("plan approval decision"),
                                        "feedback": ToolSchema::string("optional rejection feedback")
                                    },
                                    "required": ["approve", "request_id", "type"],
                                    "additionalProperties": false
                                }
                            ]
                        }
                    ]
                }
            },
            "required": ["message", "to"],
            "additionalProperties": false
        });
        spec.aliases.push("SendMessage".into());
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<SendMessageInput>("send_message", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<SendMessageInput>("send_message", &input)?;
        input.validate()?;

        let kind = recipient_kind(&input.to);
        match kind {
            // uds: and bridge: transports remain unsupported in this runtime.
            RecipientKind::Uds | RecipientKind::Bridge => Ok(unsupported_result(
                use_id,
                "send_message",
                COMMUNICATION_RUNTIME_UNAVAILABLE,
                json!({
                    "supported": false,
                    "tool": "send_message",
                    "tool_family": "agent_communication",
                    "delivery_attempted": false,
                    "live_delivery": false,
                    "recipient": input.to,
                    "recipient_kind": kind.as_str(),
                    "message_type": input.message.kind(),
                }),
            )),

            // Team recipients are persisted as advisory fleet steering messages.
            // The CLI runtime converts the effect into a FleetSteeringMessage.
            RecipientKind::Team => {
                let content = match &input.message {
                    SendMessagePayload::Text(text) => text.clone(),
                    // Structured messages are serialised to JSON as the prompt body.
                    SendMessagePayload::Structured(msg) => {
                        serde_json::to_string(msg).unwrap_or_else(|_| format!("{msg:?}"))
                    }
                };
                let message_kind = input.message.kind().to_owned();
                let spec = AgentMessageSpec {
                    to: input.to.clone(),
                    summary: input.summary.clone(),
                    content,
                    message_kind: message_kind.clone(),
                };
                let mut result = ToolResult::success(
                    use_id,
                    "message queued as advisory fleet steering \
                     (not live-delivered to any running task)",
                );
                result.metadata = json!({
                    "supported": true,
                    "tool": "send_message",
                    "tool_family": "agent_communication",
                    "delivery_attempted": false,
                    "live_delivery": false,
                    "delivery_note":
                        "persisted as advisory fleet message; \
                         not delivered to any running task",
                    "recipient": input.to,
                    "recipient_kind": kind.as_str(),
                    "message_type": message_kind,
                });
                result.effects = vec![ToolEffect::SendAgentMessage(spec)];
                Ok(result)
            }
        }
    }
}
/// Represents team create input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamCreateInput {
    /// Stores the team name
    pub team_name: String,
    /// Stores the description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Stores the agent type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
}

impl TeamCreateInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("team_create", "team_name", &self.team_name)?;
        if let Some(agent_type) = &self.agent_type {
            require_non_empty_text("team_create", "agent_type", agent_type)?;
        }
        Ok(())
    }
}
/// Represents team create tool
#[derive(Debug, Default)]
pub struct TeamCreateTool;

#[async_trait]
impl Tool for TeamCreateTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = communication_spec(
            "team_create",
            "Source-compatible TeamCreate alias; runtime team creation is unsupported",
        )
        .with_input_schema(
            ToolSchema::object()
                .property(
                    "team_name",
                    ToolSchema::string("name for the new team to create"),
                )
                .property(
                    "description",
                    ToolSchema::string("optional team description or purpose"),
                )
                .property(
                    "agent_type",
                    ToolSchema::string("optional lead agent role or type"),
                )
                .required("team_name"),
        );
        spec.aliases.push("TeamCreate".into());
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<TeamCreateInput>("team_create", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<TeamCreateInput>("team_create", &input)?;
        input.validate()?;

        Ok(unsupported_result(
            use_id,
            "team_create",
            COMMUNICATION_RUNTIME_UNAVAILABLE,
            json!({
                "supported": false,
                "tool": "team_create",
                "tool_family": "agent_communication",
                "team_name": input.team_name,
                "team_created": false,
            }),
        ))
    }
}
/// Represents team delete input
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamDeleteInput {}
/// Represents team delete tool
#[derive(Debug, Default)]
pub struct TeamDeleteTool;

#[async_trait]
impl Tool for TeamDeleteTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = communication_spec(
            "team_delete",
            "Source-compatible TeamDelete alias; runtime team deletion is unsupported",
        )
        .with_input_schema(ToolSchema::object());
        spec.aliases.push("TeamDelete".into());
        spec.destructive = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<TeamDeleteInput>("team_delete", input)?;
        Ok(())
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        parse_input::<TeamDeleteInput>("team_delete", &input)?;

        Ok(unsupported_result(
            use_id,
            "team_delete",
            COMMUNICATION_RUNTIME_UNAVAILABLE,
            json!({
                "supported": false,
                "tool": "team_delete",
                "tool_family": "agent_communication",
                "team_deleted": false,
            }),
        ))
    }
}
/// Represents list peers input
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListPeersInput {}
/// Represents list peers tool
#[derive(Debug, Default)]
pub struct ListPeersTool;

#[async_trait]
impl Tool for ListPeersTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = communication_spec(
            "list_peers",
            "Source-compatible ListPeers alias; runtime peer discovery is unsupported",
        )
        .with_input_schema(ToolSchema::object());
        spec.aliases.push("ListPeers".into());
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<ListPeersInput>("list_peers", input)?;
        Ok(())
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        parse_input::<ListPeersInput>("list_peers", &input)?;

        Ok(unsupported_result(
            use_id,
            "list_peers",
            PEER_DISCOVERY_UNAVAILABLE,
            json!({
                "supported": false,
                "tool": "list_peers",
                "tool_family": "agent_communication",
                "peers": [],
            }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;
    use wonder_of_u_core::{FeatureSet, PermissionMode, SessionId};
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn tool_context(cwd: PathBuf) -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd,
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            provider: None,
            model: None,
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: None,
            progress_tx: None,
            interaction_rx: None,
            fork_context: None,
            file_checkpointer: None,
            network_policy: None,
        }
    }

    #[test]
    fn send_message_validation_requires_summary_for_team_text_messages() {
        let tool = SendMessageTool;
        let error = tool
            .validate_input(&json!({
                "to": "reviewer",
                "message": "please check the patch",
            }))
            .expect_err("missing summary");

        assert!(error.to_string().contains("summary"));
    }

    #[test]
    fn send_message_validation_accepts_cross_session_plain_text_without_summary() {
        let tool = SendMessageTool;

        tool.validate_input(&json!({
            "to": "uds:/tmp/demo.sock",
            "message": "hello",
        }))
        .expect("cross-session plain text");
    }

    #[test]
    fn send_message_validation_rejects_structured_cross_session_messages() {
        let tool = SendMessageTool;
        let error = tool
            .validate_input(&json!({
                "to": "bridge:session-123",
                "message": {
                    "type": "shutdown_request",
                },
            }))
            .expect_err("structured cross-session");

        assert!(error.to_string().contains("cross-session"));
    }

    #[test]
    fn send_message_validation_requires_team_lead_for_shutdown_responses() {
        let tool = SendMessageTool;
        let error = tool
            .validate_input(&json!({
                "to": "reviewer",
                "message": {
                    "type": "shutdown_response",
                    "request_id": "req-1",
                    "approve": true,
                },
            }))
            .expect_err("invalid shutdown target");

        assert!(error.to_string().contains(TEAM_LEAD_NAME));
    }

    #[tokio::test]
    async fn send_message_execute_team_returns_success_with_send_effect() {
        let dir = unique_test_dir("tools-send-message-team-effect");
        let tool = SendMessageTool;
        let result = tool
            .execute(
                tool_context(dir.clone()),
                ToolUseId::new(),
                json!({
                    "to": "reviewer",
                    "summary": "review request",
                    "message": "please check the patch",
                }),
            )
            .await
            .expect("execute");

        // Team recipients: success with a SendAgentMessage effect.
        assert!(
            result.success,
            "team send should succeed; got: {}",
            result.content
        );
        assert_eq!(result.effects.len(), 1, "should have one effect");
        match &result.effects[0] {
            ToolEffect::SendAgentMessage(spec) => {
                assert_eq!(spec.to, "reviewer");
                assert_eq!(spec.summary.as_deref(), Some("review request"));
                assert_eq!(spec.content, "please check the patch");
                assert_eq!(spec.message_kind, "text");
            }
            other => panic!("unexpected effect: {other:?}"),
        }
        // Result must be honest: no live delivery, not live_delivery=true.
        assert_eq!(result.metadata["live_delivery"], false);
        // No files written by the tool itself.
        assert!(!dir.join("fleet").exists());
    }

    #[tokio::test]
    async fn send_message_execute_uds_returns_unsupported() {
        let dir = unique_test_dir("tools-send-message-uds-unsupported");
        let tool = SendMessageTool;
        let result = tool
            .execute(
                tool_context(dir.clone()),
                ToolUseId::new(),
                json!({
                    "to": "uds:/tmp/demo.sock",
                    "message": "hello",
                }),
            )
            .await
            .expect("execute");

        assert!(!result.success, "uds send should remain unsupported");
        assert!(result.content.contains("send_message is unavailable"));
        assert_eq!(result.metadata["supported"], false);
        assert_eq!(result.metadata["delivery_attempted"], false);
        assert!(!dir.join("fleet").exists());
    }

    #[tokio::test]
    async fn send_message_execute_broadcast_returns_send_effect() {
        let dir = unique_test_dir("tools-send-message-broadcast-effect");
        let tool = SendMessageTool;
        let result = tool
            .execute(
                tool_context(dir.clone()),
                ToolUseId::new(),
                json!({
                    "to": "*",
                    "summary": "heads up",
                    "message": "pivoting to auth module",
                }),
            )
            .await
            .expect("execute");

        assert!(
            result.success,
            "broadcast send should succeed; got: {}",
            result.content
        );
        assert_eq!(result.effects.len(), 1);
        match &result.effects[0] {
            ToolEffect::SendAgentMessage(spec) => {
                assert_eq!(spec.to, "*");
                assert_eq!(spec.summary.as_deref(), Some("heads up"));
            }
            other => panic!("unexpected effect: {other:?}"),
        }
        assert_eq!(result.metadata["live_delivery"], false);
    }

    #[tokio::test]
    async fn send_message_execute_structured_team_returns_send_effect() {
        let dir = unique_test_dir("tools-send-message-structured-effect");
        let tool = SendMessageTool;
        let result = tool
            .execute(
                tool_context(dir.clone()),
                ToolUseId::new(),
                json!({
                    "to": "team-lead",
                    "message": {
                        "type": "shutdown_response",
                        "request_id": "req-42",
                        "approve": true,
                    },
                }),
            )
            .await
            .expect("execute");

        assert!(
            result.success,
            "structured send should succeed; got: {}",
            result.content
        );
        assert_eq!(result.effects.len(), 1);
        match &result.effects[0] {
            ToolEffect::SendAgentMessage(spec) => {
                assert_eq!(spec.message_kind, "shutdown_response");
                // Content is the JSON serialisation of the structured message.
                assert!(
                    spec.content.contains("shutdown_response"),
                    "got: {}",
                    spec.content
                );
            }
            other => panic!("unexpected effect: {other:?}"),
        }
    }

    #[test]
    fn team_create_validation_rejects_blank_team_name() {
        let tool = TeamCreateTool;
        let error = tool
            .validate_input(&json!({ "team_name": "   " }))
            .expect_err("blank team name");

        assert!(error.to_string().contains("team_name"));
    }

    #[tokio::test]
    async fn team_create_execute_returns_unsupported_without_creating_team_state() {
        let dir = unique_test_dir("tools-team-create-unsupported");
        let tool = TeamCreateTool;
        let result = tool
            .execute(
                tool_context(dir.clone()),
                ToolUseId::new(),
                json!({
                    "team_name": "reviewers",
                    "description": "parallel reviewers",
                }),
            )
            .await
            .expect("execute");

        assert!(!result.success);
        assert!(result.content.contains("team_create is unavailable"));
        assert_eq!(result.metadata["team_created"], false);
        assert!(!dir.join("teams").exists());
        assert!(!dir.join("tasks").exists());
    }

    #[tokio::test]
    async fn team_delete_execute_returns_unsupported_without_cleanup_side_effects() {
        let dir = unique_test_dir("tools-team-delete-unsupported");
        let tool = TeamDeleteTool;
        let result = tool
            .execute(tool_context(dir.clone()), ToolUseId::new(), json!({}))
            .await
            .expect("execute");

        assert!(!result.success);
        assert!(result.content.contains("team_delete is unavailable"));
        assert_eq!(result.metadata["team_deleted"], false);
        assert!(!dir.join("teams").exists());
    }

    #[tokio::test]
    async fn list_peers_execute_returns_explicit_unsupported_failure() {
        let dir = unique_test_dir("tools-list-peers-unsupported");
        let tool = ListPeersTool;
        let result = tool
            .execute(tool_context(dir), ToolUseId::new(), json!({}))
            .await
            .expect("execute");

        assert!(!result.success);
        assert!(result.content.contains("list_peers is unavailable"));
        assert_eq!(result.metadata["supported"], false);
        assert_eq!(result.metadata["peers"], json!([]));
    }
}
