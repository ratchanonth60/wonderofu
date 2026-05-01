//! MCP configuration, transport, discovery, and status foundations.

mod catalog;
mod client;
mod config;
mod names;
mod status;
mod tools;
mod types;

pub use catalog::{McpCatalog, McpResourceRegistration, McpToolRegistration};
pub use client::McpClient;
pub use config::{
    DEFAULT_MCP_PROTOCOL_VERSION, MCP_CONFIG_SCHEMA_VERSION, McpClientIdentity, McpConfig,
    McpConfigStore, McpServerConfig,
};
pub use names::{build_mcp_resource_name, build_mcp_tool_name, normalize_mcp_name};
pub use status::{McpServerState, McpServerStatus, McpStatusReport};
pub use tools::{
    McpResourceListInput, McpResourceListTool, McpResourceReadInput, McpResourceReadTool,
};
pub use types::{
    CallToolParams, CallToolResult, ClientCapabilities, ClientInfo, InitializeParams,
    InitializeResult, JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
    ListChangedCapability, ListResourcesParams, ListResourcesResult, ListToolsParams,
    ListToolsResult, McpContent, McpResource, McpResourceContents, McpTool, ReadResourceParams,
    ReadResourceResult, ResourcesCapability, ServerCapabilities, ServerInfo, ToolsCapability,
};
