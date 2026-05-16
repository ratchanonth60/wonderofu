//! Core types shared by the wonder-of-u CLI, TUI, tools, and storage crates.
//!
//! This crate intentionally contains framework-neutral contracts and data
//! models. Later phases can add concrete TUI, provider, MCP, plugin, and tool
//! implementations without changing persisted transcripts or registry shapes.
#![warn(missing_docs)]

/// Provides app support
pub mod app;
/// Provides command support
pub mod command;
/// Provides coordinator support
pub mod coordinator;
/// Provides cwd support
pub mod cwd;
/// Provides denial tracker and YOLO-mode classifier support
pub mod denial_tracker;
/// Provides env support
pub mod env;
/// Provides error support
pub mod error;
/// Provides feature support
pub mod feature;
/// Provides fingerprint support
pub mod fingerprint;
/// Fleet (multi-agent) run types.
pub mod fleet;
/// Fleet agent role catalog.
pub mod fleet_roles;
/// Provides generators support
pub mod generators;
/// Provides git support
pub mod git;
/// Provides ids support
pub mod ids;
/// Provides lockfile support
pub mod lockfile;
/// Provides memoize support
pub mod memoize;
/// Provides message support
pub mod message;
/// Provides permission support
pub mod permission;
/// Provides plans support
pub mod plans;
/// Provides prompt suggestion support
pub mod prompt_suggestion;
/// Provides provider support
pub mod provider;
/// Provides query support
pub mod query;
/// Provides sanitization support
pub mod sanitization;
/// Provides semver support
pub mod semver;
/// Provides sequential support
pub mod sequential;
/// Provides set support
pub mod set;
/// Provides shell session support
pub mod shell_session;
/// Provides tool support
pub mod tool;
/// Provides treeify support
pub mod treeify;
/// Provides xdg support
pub mod xdg;
/// Provides xml support
pub mod xml;
/// Provides yaml support
pub mod yaml;

/// Re-exports items from `app`
pub use app::{
    AgentRuntime, AgentTaskState, AppState, CostState, InputMode, PendingLocalToolCall,
    PendingProviderToolCall, PendingProviderToolResult, PendingToolApprovalState,
    PendingToolConversationRound, QueuePlacement, QueuedCommand, RemoteTaskMetadata,
    RemoteTaskState, RemoteTaskType, RuntimeWorktreeState, SessionState, StateStore,
    TaskBackendFlow, TaskBackendState, TaskBackendSupport, TaskKind, TaskState, TaskStatus,
    ThinkingEffort, TokenUsage, input_mode_label, permission_mode_label, session_footer_text,
    session_status_text,
};
/// Re-exports items from `command`
pub use command::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandQuery,
    CommandRegistry, CommandSource, CommandSpec, parse_slash_command,
};
/// Re-exports items from `coordinator`
pub use coordinator::{
    CoordinatorMode, CoordinatorState, CoordinatorSupport, CoordinatorSurfaceStatus,
};
/// Re-exports items from `cwd`
pub use cwd::get_cwd;
/// Re-exports items from `denial_tracker`
pub use denial_tracker::{
    DEFAULT_DENIAL_THRESHOLD, DenialRecord, DenialTracker, YoloClassifier, YoloVerdict,
};
/// Re-exports items from `env`
pub use env::{EnvVarError, get_bool_env, get_env_var, parse_bool_env_value, require_env_var};
/// Re-exports items from `error`
pub use error::{Result, WonderError};
/// Re-exports items from `feature`
pub use feature::{FeatureFlag, FeatureSet};
/// Re-exports items from `fingerprint`
pub use fingerprint::{fingerprint_json, fingerprint_str};
/// Re-exports items from `fleet`
pub use fleet::{FLEET_SCHEMA_VERSION, FleetMemberRequest, FleetRunState, FleetRunStatus};
/// Re-exports items from `fleet_roles`
pub use fleet_roles::{FleetAgentRole, FleetRoleCatalog};
/// Re-exports items from `generators`
pub use generators::{enumerate, take, zip};
/// Re-exports items from `git`
pub use git::{GitError, get_current_branch, get_diff, get_git_root, is_git_repo, read_gitignore};
/// Re-exports items from `ids`
pub use ids::{CommandId, FleetId, MessageId, SessionId, TaskId, ToolUseId};
/// Re-exports items from `lockfile`
pub use lockfile::LockFile;
/// Re-exports items from `memoize`
pub use memoize::MemoCache;
/// Re-exports items from `message`
pub use message::{MESSAGE_SCHEMA_VERSION, MessageEnvelope, MessagePayload};
/// Re-exports items from `permission`
pub use permission::{
    AdditionalWorkingDirectory, PermissionDecision, PermissionDecisionReason, PermissionMode,
    PermissionRequest, PermissionRule, PermissionRuleBehavior, PermissionRuleConstraint,
    PermissionRuleSource, ShellSafetyIssue, ShellSafetyVerdict, ToolPermissionContext,
    check_shell_safety, evaluate_permission, is_path_within, resolve_path,
};
/// Re-exports items from `plans`
pub use plans::{PLAN_FILE_CANDIDATES, read_plan_file, write_plan_file};
/// Re-exports items from `prompt_suggestion`
pub use prompt_suggestion::{
    PromptSuggestion, PromptSuggestionKind, best_prompt_suggestion, rank_prompt_suggestions,
    suggestion_filter_reason,
};
/// Re-exports items from `provider`
pub use provider::{AuthMaterialKind, AuthSource, AuthState, AuthStatus, ProviderReadiness};
/// Re-exports items from `query`
pub use query::{QueryPhase, QueryState};
/// Re-exports items from `sanitization`
pub use sanitization::{escape_html, sanitize_html};
/// Re-exports items from `semver`
pub use semver::{Version, VersionParseError, parse_version, version_gte};
/// Re-exports items from `sequential`
pub use sequential::run_sequential;
/// Re-exports items from `set`
pub use set::{difference, intersection, intersects, union};
/// Re-exports items from `shell_session`
pub use shell_session::{ShellOutput, ShellSession, ShellSessionStore};
/// Re-exports items from `tool`
pub use tool::{
    Tool, ToolContext, ToolKind, ToolProgress, ToolQuery, ToolRegistry, ToolResult, ToolSchema,
    ToolSource, ToolSpec,
};
/// Re-exports items from `treeify`
pub use treeify::render_path_tree;
/// Re-exports items from `xdg`
pub use xdg::{
    XdgError, xdg_cache_home, xdg_cache_home_from, xdg_config_home, xdg_config_home_from,
    xdg_data_home, xdg_data_home_from,
};
/// Re-exports items from `xml`
pub use xml::{extract_xml_block, parse_tag_contents};
/// Re-exports items from `yaml`
pub use yaml::parse_yaml_value;
