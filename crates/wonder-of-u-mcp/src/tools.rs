//! MCP resource tools backed by configured MCP servers.

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wonder_of_u_core::{
    FeatureFlag, Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId,
    WonderError,
};

use crate::{McpCatalog, McpClient, McpConfigStore, McpResourceContents, McpResourceRegistration};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpResourceListInput {
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpResourceReadInput {
    pub resource_name: String,
}

impl McpResourceReadInput {
    fn validate(&self) -> Result<()> {
        if self.resource_name.trim().is_empty() {
            return Err(WonderError::validation(
                "mcp_resource_read requires a non-empty `resource_name`",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct McpResourceListTool;

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
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input: McpResourceListInput = serde_json::from_value(input).map_err(|error| {
            WonderError::validation(format!("invalid mcp_resource_list input: {error}"))
        })?;
        input.validate()?;

        let content = list_resources_in_storage(&storage_root()?, input.server.as_deref())?;
        Ok(ToolResult::success(use_id, content))
    }
}

#[async_trait]
impl Tool for McpResourceReadTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = ToolSpec::new("mcp_resource_read", "Read an MCP resource", ToolKind::Mcp)
            .with_input_schema(
                ToolSchema::object()
                    .property(
                        "resource_name",
                        ToolSchema::string("qualified MCP resource name"),
                    )
                    .required("resource_name"),
            );
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
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input: McpResourceReadInput = serde_json::from_value(input).map_err(|error| {
            WonderError::validation(format!("invalid mcp_resource_read input: {error}"))
        })?;
        input.validate()?;

        let content = read_resource_in_storage(&storage_root()?, &input.resource_name)?;
        Ok(ToolResult::success(use_id, content))
    }
}

fn list_resources_in_storage(storage_root: &Path, server_filter: Option<&str>) -> Result<String> {
    let store = McpConfigStore::new(storage_root);
    let config = store.read()?;
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

fn read_resource_in_storage(storage_root: &Path, resource_name: &str) -> Result<String> {
    let store = McpConfigStore::new(storage_root);
    let config = store.read()?;

    for server in config.servers.iter().filter(|server| server.enabled) {
        let (_, catalog) =
            McpClient::discover_server(server, &config.client, &config.protocol_version)?;
        if let Some(resource) = find_resource(&catalog, resource_name) {
            let mut client = McpClient::connect(server, &config.client, &config.protocol_version)?;
            let result = client.read_resource(&resource.resource.uri)?;
            return Ok(format_resource_contents(&result.contents));
        }
    }

    Err(WonderError::not_found("mcp resource", resource_name))
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

    #[test]
    fn read_input_requires_resource_name() {
        let tool = McpResourceReadTool;
        let error = tool
            .validate_input(&json!({ "resource_name": "" }))
            .expect_err("empty resource name");

        assert!(error.to_string().contains("resource_name"));
    }

    #[test]
    fn read_spec_captures_permission_metadata_and_alias() {
        let spec = McpResourceReadTool.spec();

        assert!(spec.read_only);
        assert!(spec.concurrency_safe);
        assert!(spec.aliases.contains(&"ReadMcpResourceTool".to_string()));
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
