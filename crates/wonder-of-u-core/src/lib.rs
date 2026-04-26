//! Core types shared by the wonder-of-u CLI, TUI, tools, and storage crates.
//!
//! This crate intentionally contains framework-neutral contracts and data
//! models. Later phases can add concrete TUI, provider, MCP, plugin, and tool
//! implementations without changing persisted transcripts or registry shapes.

pub mod app;
pub mod command;
pub mod error;
pub mod feature;
pub mod ids;
pub mod message;
pub mod permission;
pub mod tool;

pub use app::{
    AppState, InputMode, QueuePlacement, QueuedCommand, SessionState, StateStore, TaskState,
    TaskStatus,
};
pub use command::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandRegistry,
    CommandSource, CommandSpec, parse_slash_command,
};
pub use error::{Result, WonderError};
pub use feature::{FeatureFlag, FeatureSet};
pub use ids::{CommandId, MessageId, SessionId, TaskId, ToolUseId};
pub use message::{MESSAGE_SCHEMA_VERSION, MessageEnvelope, MessagePayload};
pub use permission::{
    PermissionDecision, PermissionMode, PermissionRule, PermissionRuleBehavior,
    PermissionRuleSource,
};
pub use tool::{
    Tool, ToolContext, ToolKind, ToolProgress, ToolRegistry, ToolResult, ToolSource, ToolSpec,
};
