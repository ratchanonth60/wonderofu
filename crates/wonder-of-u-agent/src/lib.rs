//! Provider/auth configuration foundations for wonder-of-u.
#![warn(missing_docs)]

mod auth;
mod config;
mod provider;
mod runtime;

/// Re-exports items from `auth`
pub use auth::{
    AuthMaterial, CopilotDeviceCode, CopilotOAuthToken, DEFAULT_COPILOT_API_BASE,
    StoredCredentials, poll_copilot_access_token, refresh_copilot_access_token,
    request_copilot_device_code,
};
/// Re-exports items from `config`
pub use config::{
    AgentSettings, CredentialStore, ProviderOverride, SettingsHierarchy, SettingsLayer,
    SettingsStore, detect_shadowed_rules, require_storage_dir,
};
/// Re-exports items from `provider`
pub use provider::{
    ModelDescriptor, ProviderDescriptor, ProviderRegistry, ProviderResolver, ProviderSelection,
    ProviderStatusReport, ResolvedProviderExecution,
};
/// Re-exports items from `runtime`
pub use runtime::{
    CompletionRequest, CompletionResponse, ProviderRuntime, ProviderToolCall,
    ProviderToolResultMessage, ProviderToolSpec, ToolCallBatchResponse, ToolConversationRound,
    ToolUseRequest, ToolUseResponse,
};
/// Re-exports items from `wonder_of_u_tools`
pub use wonder_of_u_tools::{builtin_registry as builtin_tool_registry, builtin_tools};
