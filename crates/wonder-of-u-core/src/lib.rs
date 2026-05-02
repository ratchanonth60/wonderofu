//! Core types shared by the wonder-of-u CLI, TUI, tools, and storage crates.
//!
//! This crate intentionally contains framework-neutral contracts and data
//! models. Later phases can add concrete TUI, provider, MCP, plugin, and tool
//! implementations without changing persisted transcripts or registry shapes.

pub mod app;
pub mod command;
pub mod coordinator;
pub mod cwd;
pub mod env;
pub mod error;
pub mod feature;
pub mod fingerprint;
pub mod generators;
pub mod git;
pub mod ids;
pub mod lockfile;
pub mod memoize;
pub mod message;
pub mod permission;
pub mod plans;
pub mod prompt_suggestion;
pub mod provider;
pub mod query;
pub mod sanitization;
pub mod semver;
pub mod sequential;
pub mod set;
pub mod tool;
pub mod treeify;
pub mod xdg;
pub mod xml;
pub mod yaml;

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
pub use cwd::get_cwd;
pub use env::{EnvVarError, get_bool_env, get_env_var, parse_bool_env_value, require_env_var};
pub use error::{Result, WonderError};
pub use feature::{FeatureFlag, FeatureSet};
pub use fingerprint::{fingerprint_json, fingerprint_str};
pub use generators::{enumerate, take, zip};
pub use git::{GitError, get_current_branch, get_diff, get_git_root, is_git_repo, read_gitignore};
pub use ids::{CommandId, MessageId, SessionId, TaskId, ToolUseId};
pub use lockfile::LockFile;
pub use memoize::MemoCache;
pub use message::{MESSAGE_SCHEMA_VERSION, MessageEnvelope, MessagePayload};
pub use permission::{
    AdditionalWorkingDirectory, PermissionDecision, PermissionDecisionReason, PermissionMode,
    PermissionRequest, PermissionRule, PermissionRuleBehavior, PermissionRuleConstraint,
    PermissionRuleSource, ShellSafetyIssue, ShellSafetyVerdict, ToolPermissionContext,
    check_shell_safety, evaluate_permission, is_path_within, resolve_path,
};
pub use plans::{PLAN_FILE_CANDIDATES, read_plan_file, write_plan_file};
pub use prompt_suggestion::{
    PromptSuggestion, PromptSuggestionKind, best_prompt_suggestion, rank_prompt_suggestions,
    suggestion_filter_reason,
};
pub use provider::{AuthMaterialKind, AuthSource, AuthState, AuthStatus, ProviderReadiness};
pub use query::{QueryPhase, QueryState};
pub use sanitization::{escape_html, sanitize_html};
pub use semver::{Version, VersionParseError, parse_version, version_gte};
pub use sequential::run_sequential;
pub use set::{difference, intersection, intersects, union};
pub use tool::{
    Tool, ToolContext, ToolKind, ToolProgress, ToolQuery, ToolRegistry, ToolResult, ToolSchema,
    ToolSource, ToolSpec,
};
pub use treeify::render_path_tree;
pub use xdg::{
    XdgError, xdg_cache_home, xdg_cache_home_from, xdg_config_home, xdg_config_home_from,
    xdg_data_home, xdg_data_home_from,
};
pub use xml::{extract_xml_block, parse_tag_contents};
pub use yaml::parse_yaml_value;
