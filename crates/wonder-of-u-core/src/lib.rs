//! Core types shared by the wonder-of-u CLI, TUI, tools, and storage crates.
//!
//! This crate intentionally contains framework-neutral contracts and data
//! models. Later phases can add concrete TUI, provider, MCP, plugin, and tool
//! implementations without changing persisted transcripts or registry shapes.

pub mod app;
pub mod command;
pub mod coordinator;
pub mod error;
pub mod feature;
pub mod ids;
pub mod message;
pub mod permission;
pub mod prompt_suggestion;
pub mod provider;
pub mod query;
pub mod tool;

pub use app::{
    AgentRuntime, AgentTaskState, AppState, CostState, InputMode, PendingLocalToolCall,
    PendingProviderToolCall, PendingProviderToolResult, PendingToolApprovalState,
    PendingToolConversationRound, QueuePlacement, QueuedCommand, RemoteTaskMetadata,
    RemoteTaskState, RemoteTaskType, SessionState, StateStore, TaskBackendFlow, TaskBackendState,
    TaskBackendSupport, TaskKind, TaskState, TaskStatus, TokenUsage, input_mode_label,
    permission_mode_label, session_footer_text, session_status_text,
};
pub use command::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandQuery,
    CommandRegistry, CommandSource, CommandSpec, parse_slash_command,
};
pub use coordinator::{
    CoordinatorMode, CoordinatorState, CoordinatorSupport, CoordinatorSurfaceStatus,
};
pub use error::{Result, WonderError};
pub use feature::{FeatureFlag, FeatureSet};
pub use ids::{CommandId, MessageId, SessionId, TaskId, ToolUseId};
pub use message::{MESSAGE_SCHEMA_VERSION, MessageEnvelope, MessagePayload};
pub use permission::{
    AdditionalWorkingDirectory, PermissionDecision, PermissionDecisionReason, PermissionMode,
    PermissionRequest, PermissionRule, PermissionRuleBehavior, PermissionRuleConstraint,
    PermissionRuleSource, ShellSafetyIssue, ShellSafetyVerdict, ToolPermissionContext,
    check_shell_safety, evaluate_permission, is_path_within, resolve_path,
};
pub use prompt_suggestion::{
    PromptSuggestion, PromptSuggestionKind, best_prompt_suggestion, rank_prompt_suggestions,
    suggestion_filter_reason,
};
pub use provider::{AuthMaterialKind, AuthSource, AuthState, AuthStatus, ProviderReadiness};
pub use query::{QueryPhase, QueryState};
pub use tool::{
    Tool, ToolContext, ToolKind, ToolProgress, ToolQuery, ToolRegistry, ToolResult, ToolSchema,
    ToolSource, ToolSpec,
};
