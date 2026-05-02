use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
/// Represents json rpc request
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// Stores the jsonrpc
    pub jsonrpc: String,
    /// Stores the id
    pub id: u64,
    /// Stores the method
    pub method: String,
    /// Stores the params
    #[serde(default)]
    pub params: Value,
}

impl JsonRpcRequest {
    /// Creates a new value
    #[must_use]
    pub fn new(id: u64, method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        }
    }
}
/// Represents json rpc notification
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    /// Stores the jsonrpc
    pub jsonrpc: String,
    /// Stores the method
    pub method: String,
    /// Stores the params
    #[serde(default)]
    pub params: Value,
}

impl JsonRpcNotification {
    /// Creates a new value
    #[must_use]
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        }
    }
}
/// Represents json rpc response
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// Stores the jsonrpc
    pub jsonrpc: String,
    /// Stores the id
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    /// Stores the result
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Stores the error
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}
/// Describes json rpc error
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Stores the code
    pub code: i64,
    /// Stores the message
    pub message: String,
    /// Stores the data
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}
/// Represents client info
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClientInfo {
    /// Stores the name
    pub name: String,
    /// Stores the version
    pub version: String,
}
/// Represents client capabilities
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClientCapabilities {
    /// Stores the experimental
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub experimental: BTreeMap<String, Value>,
}
/// Represents initialize params
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InitializeParams {
    /// Stores the protocol version
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// Stores the capabilities
    #[serde(default)]
    pub capabilities: ClientCapabilities,
    /// Stores the client info
    #[serde(rename = "clientInfo")]
    pub client_info: ClientInfo,
}
/// Represents initialize result
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InitializeResult {
    /// Stores the protocol version
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// Stores the capabilities
    #[serde(default)]
    pub capabilities: ServerCapabilities,
    /// Stores the server info
    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,
    /// Stores the instructions
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}
/// Represents server info
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Stores the name
    pub name: String,
    /// Stores the version
    pub version: String,
}
/// Represents server capabilities
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerCapabilities {
    /// Stores the tools
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    /// Stores the resources
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    /// Stores the prompts
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompts: Option<ListChangedCapability>,
    /// Stores the logging
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<Value>,
    /// Stores the experimental
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub experimental: BTreeMap<String, Value>,
}

impl ServerCapabilities {
    /// Handles labels
    #[must_use]
    pub fn labels(&self) -> Vec<&'static str> {
        let mut labels = Vec::new();
        if self.tools.is_some() {
            labels.push("tools");
        }
        if self.resources.is_some() {
            labels.push("resources");
        }
        if self.prompts.is_some() {
            labels.push("prompts");
        }
        if self.logging.is_some() {
            labels.push("logging");
        }
        labels
    }
}
/// Represents tools capability
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolsCapability {
    /// Stores the list changed
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}
/// Represents resources capability
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResourcesCapability {
    /// Stores the subscribe
    #[serde(default)]
    pub subscribe: bool,
    /// Stores the list changed
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}
/// Represents list changed capability
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListChangedCapability {
    /// Stores the list changed
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}
/// Represents list tools params
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListToolsParams {
    /// Stores the cursor
    #[serde(default, rename = "cursor", skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}
/// Represents list resources params
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListResourcesParams {
    /// Stores the cursor
    #[serde(default, rename = "cursor", skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}
/// Represents call tool params
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CallToolParams {
    /// Stores the name
    pub name: String,
    /// Stores the arguments
    #[serde(default)]
    pub arguments: Value,
}
/// Represents read resource params
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReadResourceParams {
    /// Stores the uri
    pub uri: String,
}
/// Represents list tools result
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ListToolsResult {
    /// Stores the tools
    #[serde(default)]
    pub tools: Vec<McpTool>,
    #[serde(
        default,
        rename = "nextCursor",
        skip_serializing_if = "Option::is_none"
    )]
    /// Stores the next cursor
    pub next_cursor: Option<String>,
}
/// Represents list resources result
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ListResourcesResult {
    /// Stores the resources
    #[serde(default)]
    pub resources: Vec<McpResource>,
    #[serde(
        default,
        rename = "nextCursor",
        skip_serializing_if = "Option::is_none"
    )]
    /// Stores the next cursor
    pub next_cursor: Option<String>,
}
/// Represents call tool result
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CallToolResult {
    /// Stores the content
    #[serde(default)]
    pub content: Vec<McpContent>,
    /// Stores whether error
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}
/// Represents read resource result
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReadResourceResult {
    /// Stores the contents
    #[serde(default)]
    pub contents: Vec<McpResourceContents>,
}
/// Represents mcp content
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct McpContent {
    /// Stores the kind
    #[serde(rename = "type")]
    pub kind: String,
    /// Stores the text
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}
/// Represents mcp resource contents
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct McpResourceContents {
    /// Stores the uri
    pub uri: String,
    /// Stores the mime type
    #[serde(rename = "mimeType", default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Stores the text
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Stores the blob
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}
/// Represents mcp tool
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct McpTool {
    /// Stores the name
    pub name: String,
    /// Stores the description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Stores the input schema
    #[serde(rename = "inputSchema", default)]
    pub input_schema: Value,
    #[serde(
        rename = "outputSchema",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    /// Stores the output schema
    pub output_schema: Option<Value>,
}
/// Represents mcp resource
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct McpResource {
    /// Stores the uri
    pub uri: String,
    /// Stores the name
    pub name: String,
    /// Stores the description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Stores the mime type
    #[serde(rename = "mimeType", default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}
