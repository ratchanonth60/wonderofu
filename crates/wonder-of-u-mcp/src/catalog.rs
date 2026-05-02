use wonder_of_u_core::{ToolKind, ToolSource, ToolSpec};

use crate::{McpResource, McpTool, build_mcp_resource_name, build_mcp_tool_name};
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
        spec.input_schema = self.tool.input_schema.clone();
        spec
    }
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

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wonder_of_u_core::{ToolKind, ToolSource};

    use super::*;

    #[test]
    fn catalog_exposes_namespaced_tool_specs() {
        let catalog = McpCatalog::from_server(
            "GitHub Tools",
            vec![McpTool {
                name: "Create Issue".into(),
                description: Some("Create a GitHub issue".into()),
                input_schema: json!({"type": "object"}),
                output_schema: None,
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
            },
        );
        let spec = registration.tool_spec();

        assert_eq!(spec.name, "mcp__my_server__bash");
        assert_ne!(spec.name, "bash");
        assert_eq!(spec.kind, ToolKind::Mcp);
        assert_eq!(spec.source, ToolSource::Mcp);
    }
}
