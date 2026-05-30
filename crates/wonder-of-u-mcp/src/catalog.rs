use std::sync::{Arc, Mutex};

use serde_json::Value;
use wonder_of_u_core::{FeatureFlag, ToolKind, ToolSource, ToolSpec};

use crate::{
    McpConfig, McpResource, McpSessionPool, McpTool, build_mcp_resource_name, build_mcp_tool_name,
};
/// Stores mcp catalog
#[derive(Clone, Debug, Default, PartialEq)]
pub struct McpCatalog {
    /// Stores the tools
    pub tools: Vec<McpToolRegistration>,
    /// Stores the resources
    pub resources: Vec<McpResourceRegistration>,
}

impl McpCatalog {
    /// Handles from server
    #[must_use]
    pub fn from_server(
        server_name: &str,
        tools: Vec<McpTool>,
        resources: Vec<McpResource>,
    ) -> Self {
        Self {
            tools: tools
                .into_iter()
                .map(|tool| McpToolRegistration::new(server_name, tool))
                .collect(),
            resources: resources
                .into_iter()
                .map(|resource| McpResourceRegistration::new(server_name, resource))
                .collect(),
        }
    }
    /// Handles tool specs
    #[must_use]
    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        self.tools
            .iter()
            .map(McpToolRegistration::tool_spec)
            .collect()
    }
}
/// Represents mcp tool registration
#[derive(Clone, Debug, PartialEq)]
pub struct McpToolRegistration {
    /// Stores the server name
    pub server_name: String,
    /// Stores the qualified name
    pub qualified_name: String,
    /// Stores the tool
    pub tool: McpTool,
}

impl McpToolRegistration {
    /// Creates a new value
    #[must_use]
    pub fn new(server_name: &str, tool: McpTool) -> Self {
        Self {
            server_name: server_name.into(),
            qualified_name: build_mcp_tool_name(server_name, &tool.name),
            tool,
        }
    }
    /// Handles tool spec
    #[must_use]
    pub fn tool_spec(&self) -> ToolSpec {
        let mut spec = ToolSpec::new(
            self.qualified_name.clone(),
            self.tool.description.clone().unwrap_or_else(|| {
                format!("MCP tool `{}` from `{}`", self.tool.name, self.server_name)
            }),
            ToolKind::Mcp,
        );
        spec.source = ToolSource::Mcp;
        // Dynamic catalog tools require both the Tools and Mcp feature flags,
        // matching the gating applied to the static resource tools.
        spec.required_features.insert(FeatureFlag::Tools);
        spec.required_features.insert(FeatureFlag::Mcp);
        if self
            .tool
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.destructive_hint)
            == Some(true)
        {
            spec.destructive = true;
        } else if self
            .tool
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.read_only_hint)
            == Some(true)
        {
            spec.read_only = true;
        }
        spec.input_schema = annotated_tool_schema(&self.tool);
        spec
    }
}

fn annotated_tool_schema(tool: &McpTool) -> Value {
    let mut schema = tool.input_schema.clone();
    let Some(object) = schema.as_object_mut() else {
        return schema;
    };

    if let Some(title) = tool
        .annotations
        .as_ref()
        .and_then(|annotations| annotations.title.as_ref())
    {
        object
            .entry("title")
            .or_insert_with(|| Value::String(title.clone()));
    }
    if let Some(output_schema) = &tool.output_schema {
        object.insert("x-mcp-output-schema".into(), output_schema.clone());
    }
    if let Some(annotations) = &tool.annotations {
        if let Ok(value) = serde_json::to_value(annotations) {
            object.insert("x-mcp-tool-annotations".into(), value);
        }
        if annotations.open_world_hint == Some(true) {
            object.insert("x-mcp-open-world-hint".into(), Value::Bool(true));
        }
        if annotations.idempotent_hint == Some(true) {
            object.insert("x-mcp-idempotent-hint".into(), Value::Bool(true));
        }
        if is_search_like_hint(annotations) {
            object.insert("x-mcp-search-like-hint".into(), Value::Bool(true));
        }
    }
    if let Some(meta) = &tool.meta {
        if let Ok(value) = serde_json::to_value(meta) {
            object.insert("x-mcp-meta".into(), value);
        }
    }
    schema
}

fn is_search_like_hint(annotations: &crate::McpToolAnnotations) -> bool {
    annotations.read_only_hint == Some(true)
        && annotations.open_world_hint == Some(true)
        && annotations.destructive_hint != Some(true)
}
/// Represents mcp resource registration
#[derive(Clone, Debug, PartialEq)]
pub struct McpResourceRegistration {
    /// Stores the server name
    pub server_name: String,
    /// Stores the qualified name
    pub qualified_name: String,
    /// Stores the resource
    pub resource: McpResource,
}

impl McpResourceRegistration {
    /// Creates a new value
    #[must_use]
    pub fn new(server_name: &str, resource: McpResource) -> Self {
        Self {
            server_name: server_name.into(),
            qualified_name: build_mcp_resource_name(server_name, &resource.name),
            resource,
        }
    }
}

/// Discovers all [`DynamicMcpTool`] instances for every tool on every enabled server.
///
/// Each enabled server is contacted once: tools it advertises become individual
/// [`DynamicMcpTool`] entries with namespaced names (e.g. `mcp__github__create_issue`).
/// Servers that fail to connect or return an error are **silently skipped** — a single
/// bad server must not prevent the rest from loading.  Use `mcp status` to diagnose
/// unreachable servers.
///
/// [`DynamicMcpTool`]: crate::DynamicMcpTool
#[must_use]
pub fn discover_catalog_tools(config: &McpConfig) -> Vec<crate::DynamicMcpTool> {
    use crate::{DynamicMcpTool, McpClient};

    let pool = Arc::new(Mutex::new(McpSessionPool::new()));
    config
        .servers
        .iter()
        .filter(|server| server.enabled)
        .flat_map(|server| {
            match McpClient::discover_server(server, &config.client, &config.protocol_version) {
                Ok((_, catalog)) => catalog
                    .tools
                    .iter()
                    .map(|registration| {
                        DynamicMcpTool::from_registration_with_pool(registration, Arc::clone(&pool))
                    })
                    .collect::<Vec<_>>(),
                // Skip unreachable or misconfigured servers.
                Err(_) => Vec::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wonder_of_u_core::{FeatureFlag, ToolKind, ToolSource};

    use super::*;
    use crate::McpToolAnnotations;

    #[test]
    fn catalog_tool_spec_requires_tools_and_mcp_feature_flags() {
        let registration = McpToolRegistration::new(
            "my-server",
            McpTool {
                name: "do-thing".into(),
                description: Some("Do a thing".into()),
                input_schema: json!({"type": "object"}),
                output_schema: None,
                annotations: None,
                meta: None,
            },
        );

        let spec = registration.tool_spec();

        assert!(
            spec.required_features.contains(&FeatureFlag::Tools),
            "catalog tool spec must require FeatureFlag::Tools"
        );
        assert!(
            spec.required_features.contains(&FeatureFlag::Mcp),
            "catalog tool spec must require FeatureFlag::Mcp"
        );
    }

    #[test]
    fn catalog_exposes_namespaced_tool_specs() {
        let catalog = McpCatalog::from_server(
            "GitHub Tools",
            vec![McpTool {
                name: "Create Issue".into(),
                description: Some("Create a GitHub issue".into()),
                input_schema: json!({"type": "object"}),
                output_schema: None,
                annotations: None,
                meta: None,
            }],
            vec![McpResource {
                uri: "file:///issues".into(),
                name: "Issue List".into(),
                description: None,
                mime_type: None,
            }],
        );

        let spec = catalog.tool_specs().pop().expect("tool spec");
        assert_eq!(spec.name, "mcp__github_tools__create_issue");
        assert_eq!(spec.kind, ToolKind::Mcp);
        assert_eq!(spec.source, ToolSource::Mcp);
        assert_eq!(
            catalog.resources[0].qualified_name,
            "mcp__github_tools__resource__issue_list"
        );
    }

    #[test]
    fn mcp_tool_name_is_namespaced_with_server_prefix() {
        let registration = McpToolRegistration::new(
            "my-server",
            McpTool {
                name: "search".into(),
                description: Some("Search remotely".into()),
                input_schema: json!({"type": "object"}),
                output_schema: None,
                annotations: None,
                meta: None,
            },
        );

        assert_eq!(registration.qualified_name, "mcp__my_server__search");
        assert_ne!(registration.qualified_name, "search");
    }

    #[test]
    fn mcp_tool_cannot_shadow_native_tool() {
        let registration = McpToolRegistration::new(
            "my-server",
            McpTool {
                name: "bash".into(),
                description: Some("Remote bash".into()),
                input_schema: json!({"type": "object"}),
                output_schema: None,
                annotations: None,
                meta: None,
            },
        );
        let spec = registration.tool_spec();

        assert_eq!(spec.name, "mcp__my_server__bash");
        assert_ne!(spec.name, "bash");
        assert_eq!(spec.kind, ToolKind::Mcp);
        assert_eq!(spec.source, ToolSource::Mcp);
    }

    #[test]
    fn mcp_tool_annotations_promote_permission_hints_and_schema_metadata() {
        let registration = McpToolRegistration::new(
            "my-server",
            McpTool {
                name: "search".into(),
                description: Some("Search remotely".into()),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"}
                    }
                }),
                output_schema: Some(json!({
                    "type": "object",
                    "properties": {
                        "results": {"type": "array"}
                    }
                })),
                annotations: Some(McpToolAnnotations {
                    title: Some("Remote Search".into()),
                    read_only_hint: Some(true),
                    destructive_hint: Some(false),
                    idempotent_hint: Some(true),
                    open_world_hint: Some(true),
                }),
                meta: Some(
                    [("origin".to_string(), json!("catalog"))]
                        .into_iter()
                        .collect(),
                ),
            },
        );

        let spec = registration.tool_spec();

        assert!(spec.read_only);
        assert!(!spec.destructive);
        assert_eq!(spec.input_schema["title"], json!("Remote Search"));
        assert_eq!(spec.input_schema["x-mcp-open-world-hint"], json!(true));
        assert_eq!(spec.input_schema["x-mcp-idempotent-hint"], json!(true));
        assert_eq!(spec.input_schema["x-mcp-search-like-hint"], json!(true));
        assert_eq!(spec.input_schema["x-mcp-meta"]["origin"], json!("catalog"));
        assert_eq!(
            spec.input_schema["x-mcp-output-schema"]["properties"]["results"]["type"],
            json!("array")
        );
        assert_eq!(
            spec.input_schema["x-mcp-tool-annotations"]["readOnlyHint"],
            json!(true)
        );
    }

    #[test]
    fn destructive_hint_wins_over_read_only_hint() {
        let registration = McpToolRegistration::new(
            "my-server",
            McpTool {
                name: "delete".into(),
                description: Some("Delete remotely".into()),
                input_schema: json!({"type": "object"}),
                output_schema: None,
                annotations: Some(McpToolAnnotations {
                    title: None,
                    read_only_hint: Some(true),
                    destructive_hint: Some(true),
                    idempotent_hint: None,
                    open_world_hint: None,
                }),
                meta: None,
            },
        );

        let spec = registration.tool_spec();

        assert!(spec.destructive);
        assert!(!spec.read_only);
    }
}
