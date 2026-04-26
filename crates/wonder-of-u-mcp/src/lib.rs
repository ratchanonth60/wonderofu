//! MCP configuration, transport, discovery, and status foundations.

mod catalog;
mod client;
mod config;
mod names;
mod status;
mod types;

pub use catalog::{McpCatalog, McpResourceRegistration, McpToolRegistration};
pub use client::McpClient;
pub use config::{
    DEFAULT_MCP_PROTOCOL_VERSION, MCP_CONFIG_SCHEMA_VERSION, McpClientIdentity, McpConfig,
    McpConfigStore, McpServerConfig,
};
pub use names::{build_mcp_resource_name, build_mcp_tool_name, normalize_mcp_name};
pub use status::{McpServerState, McpServerStatus, McpStatusReport};
pub use types::{
    ClientCapabilities, ClientInfo, InitializeParams, InitializeResult, JsonRpcError,
    JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, ListChangedCapability,
    ListResourcesParams, ListResourcesResult, ListToolsParams, ListToolsResult, McpResource,
    McpTool, ResourcesCapability, ServerCapabilities, ServerInfo, ToolsCapability,
};
