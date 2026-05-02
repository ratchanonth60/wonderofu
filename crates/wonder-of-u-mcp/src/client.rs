use std::{
    io::{BufRead, BufReader, BufWriter, Read, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    time::Duration,
};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use wait_timeout::ChildExt;
use wonder_of_u_core::{Result, WonderError};

use crate::{
    CallToolParams, CallToolResult, ClientCapabilities, ClientInfo, InitializeParams,
    InitializeResult, JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
    ListResourcesParams, ListResourcesResult, ListToolsParams, ListToolsResult, McpCatalog,
    McpClientIdentity, McpResource, McpServerConfig, McpTool, ReadResourceParams,
    ReadResourceResult,
};

const PROCESS_EXIT_TIMEOUT: Duration = Duration::from_millis(250);
/// Represents mcp client
#[derive(Debug)]
pub struct McpClient {
    config: McpServerConfig,
    child: Option<Child>,
    reader: Option<BufReader<ChildStdout>>,
    writer: Option<BufWriter<ChildStdin>>,
    next_request_id: u64,
    initialize_result: InitializeResult,
}

impl McpClient {
    /// Handles connect
    pub fn connect(
        config: &McpServerConfig,
        client: &McpClientIdentity,
        fallback_protocol: &str,
    ) -> Result<Self> {
        config.validate()?;
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }
        command.envs(&config.env);

        let mut child = command.spawn().map_err(|error| {
            WonderError::internal(format!(
                "failed to spawn mcp server `{}`: {error}",
                config.name
            ))
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            WonderError::internal(format!("mcp server `{}` did not expose stdin", config.name))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            WonderError::internal(format!(
                "mcp server `{}` did not expose stdout",
                config.name
            ))
        })?;

        let mut client_instance = Self {
            config: config.clone(),
            child: Some(child),
            reader: Some(BufReader::new(stdout)),
            writer: Some(BufWriter::new(stdin)),
            next_request_id: 1,
            initialize_result: InitializeResult::default(),
        };
        client_instance.initialize_result =
            client_instance.initialize(client, fallback_protocol)?;
        Ok(client_instance)
    }
    /// Handles initialize result
    #[must_use]
    pub fn initialize_result(&self) -> &InitializeResult {
        &self.initialize_result
    }

    /// Handles discover catalog
    pub fn discover_catalog(&mut self) -> Result<McpCatalog> {
        let tools = if self.initialize_result.capabilities.tools.is_some() {
            self.list_tools()?
        } else {
            Vec::new()
        };
        let resources = if self.initialize_result.capabilities.resources.is_some() {
            self.list_resources()?
        } else {
            Vec::new()
        };
        Ok(McpCatalog::from_server(&self.config.name, tools, resources))
    }

    /// Handles discover server
    pub fn discover_server(
        config: &McpServerConfig,
        client: &McpClientIdentity,
        fallback_protocol: &str,
    ) -> Result<(InitializeResult, McpCatalog)> {
        let mut connection = Self::connect(config, client, fallback_protocol)?;
        let initialize_result = connection.initialize_result().clone();
        let catalog = connection.discover_catalog()?;
        connection.shutdown()?;
        Ok((initialize_result, catalog))
    }

    /// Handles list tools
    pub fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        let mut tools = Vec::new();
        let mut cursor = None;
        loop {
            let result: ListToolsResult = self.request(
                "tools/list",
                &ListToolsParams {
                    cursor: cursor.clone(),
                },
            )?;
            tools.extend(result.tools);
            match result.next_cursor {
                Some(next_cursor) => cursor = Some(next_cursor),
                None => return Ok(tools),
            }
        }
    }

    /// Handles list resources
    pub fn list_resources(&mut self) -> Result<Vec<McpResource>> {
        let mut resources = Vec::new();
        let mut cursor = None;
        loop {
            let result: ListResourcesResult = self.request(
                "resources/list",
                &ListResourcesParams {
                    cursor: cursor.clone(),
                },
            )?;
            resources.extend(result.resources);
            match result.next_cursor {
                Some(next_cursor) => cursor = Some(next_cursor),
                None => return Ok(resources),
            }
        }
    }

    /// Handles call tool
    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Result<CallToolResult> {
        self.request(
            "tools/call",
            &CallToolParams {
                name: name.into(),
                arguments,
            },
        )
    }

    /// Reads resource
    pub fn read_resource(&mut self, uri: &str) -> Result<ReadResourceResult> {
        self.request("resources/read", &ReadResourceParams { uri: uri.into() })
    }

    /// Handles shutdown
    pub fn shutdown(&mut self) -> Result<()> {
        self.reader.take();
        self.writer.take();

        let Some(mut child) = self.child.take() else {
            return Ok(());
        };

        if child.try_wait()?.is_none() && child.wait_timeout(PROCESS_EXIT_TIMEOUT)?.is_none() {
            child.kill()?;
        }
        let _ = child.wait();
        Ok(())
    }

    fn initialize(
        &mut self,
        client: &McpClientIdentity,
        fallback_protocol: &str,
    ) -> Result<InitializeResult> {
        let requested_protocol = self
            .config
            .protocol_version
            .as_deref()
            .unwrap_or(fallback_protocol)
            .to_string();
        let result: InitializeResult = self.request(
            "initialize",
            &InitializeParams {
                protocol_version: requested_protocol,
                capabilities: ClientCapabilities::default(),
                client_info: ClientInfo {
                    name: client.name.clone(),
                    version: client.version.clone(),
                },
            },
        )?;
        self.notify("notifications/initialized", json!({}))?;
        Ok(result)
    }

    fn request<T: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: &T,
    ) -> Result<R> {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        let request = JsonRpcRequest::new(request_id, method, serde_json::to_value(params)?);
        self.write_message(&request)?;
        let response = self.read_response(request_id)?;
        if let Some(error) = response.error {
            return Err(WonderError::internal(format!(
                "mcp request `{method}` failed: {} ({})",
                error.message, error.code
            )));
        }
        let result = response.result.ok_or_else(|| {
            WonderError::internal(format!("mcp request `{method}` returned no result"))
        })?;
        serde_json::from_value(result).map_err(Into::into)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let notification = JsonRpcNotification::new(method, params);
        self.write_message(&notification)
    }

    fn write_message<T: Serialize>(&mut self, message: &T) -> Result<()> {
        let body = serde_json::to_vec(message)?;
        let writer = self.writer.as_mut().ok_or_else(|| {
            WonderError::internal(format!(
                "mcp server `{}` writer is closed",
                self.config.name
            ))
        })?;
        write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
        writer.write_all(&body)?;
        writer.flush()?;
        Ok(())
    }

    fn read_response(&mut self, request_id: u64) -> Result<JsonRpcResponse> {
        loop {
            let envelope = MessageEnvelope::from_value(self.read_message()?)?;
            if envelope.is_response() {
                if envelope.id == Some(request_id) {
                    return Ok(envelope.into_response());
                }
                return Err(WonderError::internal(format!(
                    "received out-of-order mcp response for id {:?} while waiting for {request_id}",
                    envelope.id
                )));
            }
        }
    }

    fn read_message(&mut self) -> Result<Value> {
        let reader = self.reader.as_mut().ok_or_else(|| {
            WonderError::internal(format!(
                "mcp server `{}` reader is closed",
                self.config.name
            ))
        })?;

        let mut content_length = None;
        let mut line = String::new();
        loop {
            line.clear();
            let bytes = reader.read_line(&mut line)?;
            if bytes == 0 {
                return Err(WonderError::internal(format!(
                    "unexpected EOF from mcp server `{}`",
                    self.config.name
                )));
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            let (name, value) = trimmed.split_once(':').ok_or_else(|| {
                WonderError::internal(format!(
                    "invalid mcp header from server `{}`: {trimmed}",
                    self.config.name
                ))
            })?;
            if name.eq_ignore_ascii_case("content-length") {
                content_length = Some(value.trim().parse::<usize>().map_err(|error| {
                    WonderError::internal(format!(
                        "invalid content length from mcp server `{}`: {error}",
                        self.config.name
                    ))
                })?);
            }
        }

        let content_length = content_length.ok_or_else(|| {
            WonderError::internal(format!(
                "missing content length from mcp server `{}`",
                self.config.name
            ))
        })?;
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body)?;
        serde_json::from_slice(&body).map_err(Into::into)
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[derive(Debug)]
struct MessageEnvelope {
    jsonrpc: String,
    id: Option<u64>,
    method: Option<String>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

impl MessageEnvelope {
    fn from_value(value: Value) -> Result<Self> {
        let object = value
            .as_object()
            .ok_or_else(|| WonderError::internal("mcp message must be a JSON object"))?;
        let jsonrpc = object
            .get("jsonrpc")
            .and_then(Value::as_str)
            .ok_or_else(|| WonderError::internal("mcp message missing jsonrpc version"))?
            .to_string();
        let id = object.get("id").and_then(Value::as_u64);
        let method = object
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string);
        let result = object.get("result").cloned();
        let error = object
            .get("error")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?;
        Ok(Self {
            jsonrpc,
            id,
            method,
            result,
            error,
        })
    }

    fn is_response(&self) -> bool {
        self.method.is_none() && (self.result.is_some() || self.error.is_some())
    }

    fn into_response(self) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: self.jsonrpc,
            id: self.id,
            result: self.result,
            error: self.error,
        }
    }
}
