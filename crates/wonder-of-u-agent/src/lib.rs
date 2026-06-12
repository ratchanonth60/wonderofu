//! Provider/auth configuration foundations for wonder-of-u.
#![warn(missing_docs)]

mod agent_summary;
mod auth;
mod compact_summary;
mod config;
mod permissions;
mod protocol;
mod provider;
mod runtime;
/// Tool-use summary generation using a fast model.
pub mod tool_use_summary;

/// Re-exports items from `agent_summary`
pub use agent_summary::{generate_agent_summary, read_session_link};
/// Re-exports items from `auth`
pub use auth::{
    AuthMaterial, AwsBearerCredentials, AwsCredentials, AwsProfileCredentials, CopilotDeviceCode,
    CopilotOAuthToken, DEFAULT_COPILOT_API_BASE, GcpReadiness, StoredCredentials,
    poll_copilot_access_token, refresh_copilot_access_token, request_copilot_device_code,
    resolve_aws_bearer_from_env, resolve_aws_credentials_from_env, resolve_aws_profile_from_env,
    resolve_gcp_credentials_from_env,
};
/// Re-exports items from `compact_summary`
pub use compact_summary::{
    COMPACT_LLM_SUMMARY_ENV, extract_summary_block, generate_compact_summary,
    render_conversation_for_summary,
};
/// Re-exports items from `config`
pub use config::{
    AgentSettings, CredentialStore, ProviderOverride, SettingsHierarchy, SettingsLayer,
    SettingsStore, StatusLineConfig, detect_shadowed_rules, require_storage_dir,
};
/// Re-exports items from `permissions`
pub use permissions::{PermissionsLoader, persist_permission_rule, remove_permission_rule};
/// Re-exports items from `protocol`
pub use protocol::WireProtocol;
/// Re-exports items from `provider`
pub use provider::{
    DEFAULT_BEDROCK_API_BASE, ModelDescriptor, ProviderDescriptor, ProviderRegistry,
    ProviderResolver, ProviderSelection, ProviderStatusReport, ResolvedProviderExecution,
    openai_compat_gateway,
};
/// Re-exports items from `runtime`
pub use runtime::{
    CompletionRequest, CompletionResponse, ImageAttachment, ProviderRuntime, ProviderToolCall,
    ProviderToolResultMessage, ProviderToolSpec, ToolCallBatchResponse, ToolConversationRound,
    ToolUseRequest, ToolUseResponse,
};
/// Re-exports items from `wonder_of_u_tools`
pub use wonder_of_u_tools::{builtin_registry as builtin_tool_registry, builtin_tools};
