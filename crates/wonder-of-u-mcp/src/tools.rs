//! MCP resource tools backed by configured MCP servers.

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{
    FeatureFlag, Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId,
    WonderError,
};

use crate::{
    CallToolResult, McpCatalog, McpClient, McpConfigStore, McpResourceContents,
    McpResourceRegistration, McpSessionPool, McpToolRegistration,
};
/// Represents mcp resource list input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpResourceListInput {
    /// Stores the server
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
}

impl McpResourceListInput {
    fn validate(&self) -> Result<()> {
        if self
            .server
            .as_deref()
            .is_some_and(|server| server.trim().is_empty())
        {
            return Err(WonderError::validation(
                "mcp_resource_list server must be non-empty when provided",
            ));
        }
        Ok(())
    }
}
/// Input for reading a single MCP resource.
///
/// Two dispatch modes are supported (upstream `ReadMcpResourceTool` parity):
///
/// * **By qualified name** — supply `resource_name` (e.g. `mcp__demo__resource__readme`).
///   The tool discovers all configured servers and searches their catalogs.
/// * **By server + URI** — supply both `server` and `uri` for a direct read without
///   catalog discovery.  Exactly mirrors the upstream `{ server, uri }` input contract.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpResourceReadInput {
    /// Qualified MCP resource name searched across all configured servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_name: Option<String>,
    /// MCP server name — required when using `uri` for a direct resource read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    /// Resource URI for a direct read — required when using `server`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

impl McpResourceReadInput {
    fn validate(&self) -> Result<()> {
        match (&self.resource_name, &self.server, &self.uri) {
            // resource_name only
            (Some(name), None, None) => {
                if name.trim().is_empty() {
                    return Err(WonderError::validation(
                        "mcp_resource_read `resource_name` must be non-empty",
                    ));
                }
            }
            // server + uri (upstream ReadMcpResourceTool parity)
            (None, Some(server), Some(uri)) => {
                if server.trim().is_empty() {
                    return Err(WonderError::validation(
                        "mcp_resource_read `server` must be non-empty",
                    ));
                }
                if uri.trim().is_empty() {
                    return Err(WonderError::validation(
                        "mcp_resource_read `uri` must be non-empty",
                    ));
                }
            }
            // Nothing provided at all
            (None, None, None) => {
                return Err(WonderError::validation(
                    "mcp_resource_read requires either `resource_name` or both `server` and `uri`",
                ));
            }
            // Partial / mixed combination
            _ => {
                return Err(WonderError::validation(
                    "mcp_resource_read: provide either `resource_name` alone, or both `server` and `uri` together",
                ));
            }
        }
        Ok(())
    }
}
/// Represents mcp resource list tool
#[derive(Debug, Default)]
pub struct McpResourceListTool;
/// Represents mcp resource read tool
#[derive(Debug, Default)]
pub struct McpResourceReadTool;

#[async_trait]
impl Tool for McpResourceListTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = ToolSpec::new("mcp_resource_list", "List MCP resources", ToolKind::Mcp)
            .with_input_schema(ToolSchema::object().property(
                "server",
                ToolSchema::string("optional MCP server name filter"),
            ));
        annotate_resource_dispatch_schema(&mut spec);
        spec.aliases.push("ListMcpResourcesTool".into());
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec.required_features.insert(FeatureFlag::Tools);
        spec.required_features.insert(FeatureFlag::Mcp);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        serde_json::from_value::<McpResourceListInput>(input.clone())
            .map_err(|error| {
                WonderError::validation(format!("invalid mcp_resource_list input: {error}"))
            })?
            .validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input: McpResourceListInput = serde_json::from_value(input).map_err(|error| {
            WonderError::validation(format!("invalid mcp_resource_list input: {error}"))
        })?;
        input.validate()?;

        let content =
            list_resources_in_storage(&storage_root()?, &context.cwd, input.server.as_deref())?;
        Ok(ToolResult::success(use_id, content))
    }
}

#[async_trait]
impl Tool for McpResourceReadTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = ToolSpec::new(
            "mcp_resource_read",
            "Read an MCP resource",
            ToolKind::Mcp,
        )
        .with_input_schema(
            ToolSchema::object()
                .property(
                    "resource_name",
                    ToolSchema::string(
                        "qualified MCP resource name searched across all configured servers                              (e.g. `mcp__demo__resource__readme`)",
                    ),
                )
                .property(
                    "server",
                    ToolSchema::string(
                        "MCP server name — use together with `uri` for a direct resource                              read (parity with upstream ReadMcpResourceTool)",
                    ),
                )
                .property(
                    "uri",
                    ToolSchema::string(
                        "resource URI for a direct read — use together with `server`",
                    ),
                ),
            // No JSON-schema `required` array: validate() enforces the two valid patterns.
        );
        annotate_resource_dispatch_schema(&mut spec);
        spec.aliases.push("ReadMcpResourceTool".into());
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec.required_features.insert(FeatureFlag::Tools);
        spec.required_features.insert(FeatureFlag::Mcp);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        serde_json::from_value::<McpResourceReadInput>(input.clone())
            .map_err(|error| {
                WonderError::validation(format!("invalid mcp_resource_read input: {error}"))
            })?
            .validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input: McpResourceReadInput = serde_json::from_value(input).map_err(|error| {
            WonderError::validation(format!("invalid mcp_resource_read input: {error}"))
        })?;
        input.validate()?;

        let root = storage_root()?;
        let content = match (&input.resource_name, &input.server, &input.uri) {
            // Direct read by server + URI (upstream ReadMcpResourceTool parity).
            (None, Some(server), Some(uri)) => {
                read_resource_by_server_uri(&root, &context.cwd, server, uri)?
            }
            // Catalog search by qualified name.
            _ => {
                let name = input.resource_name.as_deref().expect("validated");
                read_resource_in_storage(&root, &context.cwd, name)?
            }
        };
        Ok(ToolResult::success(use_id, content))
    }
}

fn list_resources_in_storage(
    storage_root: &Path,
    cwd: &Path,
    server_filter: Option<&str>,
) -> Result<String> {
    let store = McpConfigStore::new(storage_root);
    let (config, _) = store.read_with_project(cwd, None)?;
    let mut lines = Vec::new();

    for server in config
        .servers
        .iter()
        .filter(|server| server.enabled)
        .filter(|server| server_filter.is_none_or(|filter| server.name == filter))
    {
        let (_, catalog) =
            McpClient::discover_server(server, &config.client, &config.protocol_version)?;
        lines.extend(format_resource_list(&catalog, Some(&server.name)));
    }

    Ok(if lines.is_empty() {
        "No MCP resources found.".into()
    } else {
        lines.join("\n")
    })
}

fn read_resource_in_storage(
    storage_root: &Path,
    cwd: &Path,
    resource_name: &str,
) -> Result<String> {
    let store = McpConfigStore::new(storage_root);
    let (config, _) = store.read_with_project(cwd, None)?;

    for server in config.servers.iter().filter(|server| server.enabled) {
        let mut client = McpClient::connect(server, &config.client, &config.protocol_version)?;
        let catalog = client.discover_catalog()?;
        if let Some(resource) = find_resource(&catalog, resource_name) {
            let result = client.read_resource(&resource.resource.uri)?;
            return Ok(format_resource_contents(&result.contents));
        }
    }

    Err(WonderError::not_found("mcp resource", resource_name))
}

/// Read a resource directly by MCP server name and URI, without catalog discovery.
///
/// This mirrors the upstream `ReadMcpResourceTool { server, uri }` dispatch path and lets
/// callers skip the full catalog round-trip when they already know the resource address.
fn read_resource_by_server_uri(
    storage_root: &Path,
    cwd: &Path,
    server_name: &str,
    uri: &str,
) -> Result<String> {
    let store = McpConfigStore::new(storage_root);
    let (config, _) = store.read_with_project(cwd, None)?;

    let server = config
        .server(server_name)
        .ok_or_else(|| WonderError::not_found("mcp server", server_name))?;

    if !server.enabled {
        return Err(WonderError::validation(format!(
            "mcp server `{server_name}` is disabled"
        )));
    }

    let mut client = McpClient::connect(server, &config.client, &config.protocol_version)?;
    let result = client.read_resource(uri)?;
    Ok(format_resource_contents(&result.contents))
}

fn format_resource_list(catalog: &McpCatalog, server_name: Option<&str>) -> Vec<String> {
    catalog
        .resources
        .iter()
        .map(|resource| {
            let mut line = format!("{} -> {}", resource.qualified_name, resource.resource.uri);
            if let Some(server_name) = server_name {
                line.push_str(&format!(" ({server_name})"));
            }
            if let Some(description) = &resource.resource.description {
                line.push_str(&format!(" — {description}"));
            }
            line
        })
        .collect()
}

fn find_resource<'a>(
    catalog: &'a McpCatalog,
    resource_name: &str,
) -> Option<&'a McpResourceRegistration> {
    catalog.resources.iter().find(|resource| {
        resource.qualified_name == resource_name || resource.resource.name == resource_name
    })
}

fn format_resource_contents(contents: &[McpResourceContents]) -> String {
    if contents.is_empty() {
        return "No content returned.".into();
    }
    contents
        .iter()
        .map(|content| {
            let body = content
                .text
                .clone()
                .or_else(|| content.blob.clone())
                .unwrap_or_default();
            match &content.mime_type {
                Some(mime) => format!("{} ({mime})\n{}", content.uri, body),
                None => format!("{}\n{}", content.uri, body),
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// A live MCP tool discovered from a running server.
///
/// Created by [`crate::discover_catalog_tools`] for each tool advertised by an enabled MCP
/// server.  The qualified name (e.g. `mcp__github__create_issue`) is already baked into the
/// spec, so the model calls it directly — no generic wrapper needed.
///
/// # Dispatch
///
/// On execution the tool re-reads the MCP config from `WONDER_OF_U_STORAGE_DIR` (or the
/// XDG / HOME fallbacks), connects a fresh `McpClient`, and forwards the call.  The config
/// read happens *per execution* so that server enable/disable changes take effect without
/// restarting the session.
#[derive(Clone, Debug)]
pub struct DynamicMcpTool {
    spec: ToolSpec,
    server_name: String,
    /// Raw tool name as the server advertised it (used in `tools/call`).
    raw_tool_name: String,
    pool: Option<Arc<Mutex<McpSessionPool>>>,
}

impl DynamicMcpTool {
    /// Builds a `DynamicMcpTool` from a catalog registration.
    #[must_use]
    pub fn from_registration(registration: &McpToolRegistration) -> Self {
        Self {
            spec: registration.tool_spec(),
            server_name: registration.server_name.clone(),
            raw_tool_name: registration.tool.name.clone(),
            pool: None,
        }
    }

    /// Builds a `DynamicMcpTool` that reuses the provided session pool.
    #[must_use]
    pub fn from_registration_with_pool(
        registration: &McpToolRegistration,
        pool: Arc<Mutex<McpSessionPool>>,
    ) -> Self {
        Self {
            spec: registration.tool_spec(),
            server_name: registration.server_name.clone(),
            raw_tool_name: registration.tool.name.clone(),
            pool: Some(pool),
        }
    }

    /// Returns the name of the MCP server this tool belongs to.
    #[must_use]
    pub fn server_name(&self) -> &str {
        &self.server_name
    }
}

#[async_trait]
impl Tool for DynamicMcpTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        if input.is_object() {
            Ok(())
        } else {
            Err(WonderError::validation(format!(
                "{}: input must be a JSON object",
                self.spec.name
            )))
        }
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let storage_root = storage_root()?;
        let (config, _) =
            McpConfigStore::new(&storage_root).read_with_project(&context.cwd, None)?;

        let Some(server) = config.server(&self.server_name) else {
            return Ok(ToolResult::failure(
                use_id,
                format!(
                    "mcp server `{}` is not configured; add it with `mcp add`",
                    self.server_name
                ),
            ));
        };

        if !server.enabled {
            return Ok(ToolResult::failure(
                use_id,
                format!(
                    "mcp server `{}` is disabled; enable it with `mcp enable {}`",
                    self.server_name, self.server_name
                ),
            ));
        }

        let call_result = if let Some(pool) = &self.pool {
            pool.lock()
                .map_err(|_| WonderError::internal("mcp session pool mutex poisoned"))?
                .with_client(server, &config.client, &config.protocol_version, |client| {
                    client.call_tool(&self.raw_tool_name, input.clone())
                })?
        } else {
            let mut client = McpClient::connect(server, &config.client, &config.protocol_version)?;
            client.call_tool(&self.raw_tool_name, input)?
        };
        let content = format_call_result(&call_result);

        let mut result = if call_result.is_error {
            ToolResult::failure(
                use_id,
                if content.is_empty() {
                    format!(
                        "mcp tool `{}` on `{}` returned an error",
                        self.raw_tool_name, self.server_name
                    )
                } else {
                    content
                },
            )
        } else {
            ToolResult::success(use_id, content)
        };

        result.metadata = serde_json::json!({
            "tool": self.spec.name,
            "server": self.server_name,
            "tool_name": self.raw_tool_name,
            "is_error": call_result.is_error,
            "content_items": call_result.content.len(),
        });
        Ok(result)
    }
}

/// Joins all `text`-typed content items from a `CallToolResult` into a single string.
fn format_call_result(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|item| item.text.as_deref())
        .collect::<Vec<_>>()
        .join("\n")
}

fn annotate_resource_dispatch_schema(spec: &mut ToolSpec) {
    if let Some(object) = spec.input_schema.as_object_mut() {
        object.insert(
            "x-mcp-resource-capability-dependent".into(),
            Value::Bool(true),
        );
        object.insert(
            "x-mcp-resource-dispatch".into(),
            json!("always_registered_runtime_checked"),
        );
    }
}

fn storage_root() -> Result<PathBuf> {
    storage_root_from_env(|name| env::var_os(name))
}

fn storage_root_from_env(getenv: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf> {
    if let Some(path) = getenv("WONDER_OF_U_STORAGE_DIR").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = getenv("XDG_CONFIG_HOME").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path).join("wonder-of-u"));
    }
    if let Some(path) = getenv("HOME").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path).join(".config").join("wonder-of-u"));
    }
    Err(WonderError::validation(
        "unable to resolve wonder-of-u storage directory from WONDER_OF_U_STORAGE_DIR, XDG_CONFIG_HOME, or HOME",
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::McpResource;

    #[test]
    fn list_input_rejects_empty_server_name() {
        let tool = McpResourceListTool;
        let error = tool
            .validate_input(&json!({ "server": "   " }))
            .expect_err("empty server");

        assert!(error.to_string().contains("server"));
    }

    #[test]
    fn format_resource_list_renders_catalog_entries() {
        let catalog = McpCatalog::from_server(
            "demo",
            Vec::new(),
            vec![McpResource {
                uri: "file:///demo".into(),
                name: "Demo".into(),
                description: Some("Example".into()),
                mime_type: Some("text/plain".into()),
            }],
        );

        let rendered = format_resource_list(&catalog, Some("demo"));

        assert_eq!(rendered.len(), 1);
        assert!(rendered[0].contains("mcp__demo__resource__demo"));
    }

    // --- McpResourceReadInput validation ---

    #[test]
    fn read_input_requires_resource_name_non_empty() {
        let tool = McpResourceReadTool;
        let error = tool
            .validate_input(&json!({ "resource_name": "" }))
            .expect_err("empty resource name");

        assert!(error.to_string().contains("resource_name"));
    }

    #[test]
    fn read_input_rejects_empty_object() {
        let error = McpResourceReadInput {
            resource_name: None,
            server: None,
            uri: None,
        }
        .validate()
        .expect_err("empty input");

        assert!(error.to_string().contains("resource_name"));
    }

    #[test]
    fn read_input_accepts_server_and_uri() {
        McpResourceReadInput {
            resource_name: None,
            server: Some("demo".into()),
            uri: Some("file:///workspace/Cargo.toml".into()),
        }
        .validate()
        .expect("server + uri is valid");
    }

    #[test]
    fn read_input_rejects_server_without_uri() {
        let error = McpResourceReadInput {
            resource_name: None,
            server: Some("demo".into()),
            uri: None,
        }
        .validate()
        .expect_err("server alone is invalid");

        assert!(error.to_string().contains("server") || error.to_string().contains("uri"));
    }

    #[test]
    fn read_input_rejects_uri_without_server() {
        let error = McpResourceReadInput {
            resource_name: None,
            server: None,
            uri: Some("file:///workspace/Cargo.toml".into()),
        }
        .validate()
        .expect_err("uri alone is invalid");

        assert!(error.to_string().contains("server") || error.to_string().contains("uri"));
    }

    #[test]
    fn read_input_rejects_mixing_resource_name_with_server_uri() {
        let error = McpResourceReadInput {
            resource_name: Some("mcp__demo__resource__readme".into()),
            server: Some("demo".into()),
            uri: Some("file:///workspace/README.md".into()),
        }
        .validate()
        .expect_err("mixing modes is invalid");

        assert!(
            error.to_string().contains("resource_name")
                || error.to_string().contains("server")
                || error.to_string().contains("uri")
        );
    }

    #[test]
    fn read_input_rejects_empty_server_in_server_uri_mode() {
        let error = McpResourceReadInput {
            resource_name: None,
            server: Some("   ".into()),
            uri: Some("file:///workspace/Cargo.toml".into()),
        }
        .validate()
        .expect_err("empty server in server+uri mode");

        assert!(error.to_string().contains("server"));
    }

    #[test]
    fn read_spec_captures_permission_metadata_and_alias() {
        let spec = McpResourceReadTool.spec();

        assert!(spec.read_only);
        assert!(spec.concurrency_safe);
        assert!(spec.aliases.contains(&"ReadMcpResourceTool".to_string()));
    }

    #[test]
    fn read_spec_exposes_server_and_uri_properties() {
        let spec = McpResourceReadTool.spec();
        let schema = &spec.input_schema;

        // All three dispatch fields must be described in the JSON schema.
        assert!(schema["properties"]["resource_name"].is_object());
        assert!(schema["properties"]["server"].is_object());
        assert!(schema["properties"]["uri"].is_object());
    }

    #[test]
    fn resource_specs_document_runtime_capability_dependency() {
        let list_spec = McpResourceListTool.spec();
        let read_spec = McpResourceReadTool.spec();

        assert_eq!(
            list_spec.input_schema["x-mcp-resource-capability-dependent"],
            json!(true)
        );
        assert_eq!(
            list_spec.input_schema["x-mcp-resource-dispatch"],
            json!("always_registered_runtime_checked")
        );
        assert_eq!(
            read_spec.input_schema["x-mcp-resource-capability-dependent"],
            json!(true)
        );
        assert_eq!(
            read_spec.input_schema["x-mcp-resource-dispatch"],
            json!("always_registered_runtime_checked")
        );
    }

    #[test]
    fn format_resource_contents_prefers_text_payloads() {
        let rendered = format_resource_contents(&[McpResourceContents {
            uri: "file:///demo".into(),
            mime_type: Some("text/plain".into()),
            text: Some("hello".into()),
            blob: Some("ignored".into()),
        }]);

        assert!(rendered.contains("hello"));
        assert!(rendered.contains("text/plain"));
    }
}
