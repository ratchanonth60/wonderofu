//! MCP configuration, transport, discovery, and status foundations.
#![warn(missing_docs)]

mod catalog;
mod client;
mod config;
mod env_expand;
mod names;
mod project_config;
mod session_pool;
mod status;
mod tools;
mod types;

/// Re-exports items from `catalog`
pub use catalog::{
    McpCatalog, McpResourceRegistration, McpToolRegistration, discover_catalog_tools,
};
/// Re-exports items from `client`
pub use client::McpClient;
/// Re-exports items from `config`
pub use config::{
    DEFAULT_MCP_PROTOCOL_VERSION, MCP_CONFIG_SCHEMA_VERSION, McpClientIdentity, McpConfig,
    McpConfigStore, McpServerConfig,
};
/// Re-exports items from `env_expand`
pub use env_expand::expand_env_value;
/// Re-exports items from `names`
pub use names::{build_mcp_resource_name, build_mcp_tool_name, normalize_mcp_name};
/// Re-exports items from `project_config`
pub use project_config::{ProjectMcpConfig, ProjectMcpServerEntry};
/// Re-exports items from `session_pool`
pub use session_pool::McpSessionPool;
/// Re-exports items from `status`
pub use status::{McpServerState, McpServerStatus, McpStatusReport};
/// Re-exports items from `tools`
pub use tools::{
    DynamicMcpTool, McpResourceListInput, McpResourceListTool, McpResourceReadInput,
    McpResourceReadTool,
};
/// Re-exports items from `types`
pub use types::{
    CallToolParams, CallToolResult, ClientCapabilities, ClientInfo, InitializeParams,
    InitializeResult, JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
    ListChangedCapability, ListResourcesParams, ListResourcesResult, ListToolsParams,
    ListToolsResult, McpContent, McpResource, McpResourceContents, McpTool, McpToolAnnotations,
    ReadResourceParams, ReadResourceResult, ResourcesCapability, ServerCapabilities, ServerInfo,
    ToolsCapability,
};
