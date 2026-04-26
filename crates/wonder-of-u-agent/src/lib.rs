//! Provider/auth configuration foundations for wonder-of-u.

mod auth;
mod config;
mod provider;
mod runtime;

pub use auth::{
    AuthMaterial, CopilotDeviceCode, CopilotOAuthToken, DEFAULT_COPILOT_API_BASE,
    StoredCredentials, poll_copilot_access_token, refresh_copilot_access_token,
    request_copilot_device_code,
};
pub use config::{
    AgentSettings, CredentialStore, ProviderOverride, SettingsStore, require_storage_dir,
};
pub use provider::{
    ModelDescriptor, ProviderDescriptor, ProviderRegistry, ProviderResolver, ProviderSelection,
    ProviderStatusReport, ResolvedProviderExecution,
};
pub use runtime::{
    CompletionRequest, CompletionResponse, ProviderRuntime, ProviderToolCall,
    ProviderToolResultMessage, ProviderToolSpec, ToolCallBatchResponse, ToolConversationRound,
    ToolUseRequest, ToolUseResponse,
};
pub use wonder_of_u_tools::{builtin_registry as builtin_tool_registry, builtin_tools};
