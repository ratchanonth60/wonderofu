/// Normalizes mcp name
pub fn normalize_mcp_name(value: &str) -> String {
    let mut normalized = String::new();
    let mut previous_was_separator = false;

    for ch in value.trim().chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            previous_was_separator = false;
            ch.to_ascii_lowercase()
        } else {
            if previous_was_separator || normalized.is_empty() {
                continue;
            }
            previous_was_separator = true;
            '_'
        };
        normalized.push(mapped);
    }

    let normalized = normalized.trim_matches('_');
    if normalized.is_empty() {
        "unnamed".into()
    } else {
        normalized.to_string()
    }
}

/// Builds mcp tool name
pub fn build_mcp_tool_name(server_name: &str, tool_name: &str) -> String {
    format!(
        "mcp__{}__{}",
        normalize_mcp_name(server_name),
        normalize_mcp_name(tool_name)
    )
}

/// Builds mcp resource name
pub fn build_mcp_resource_name(server_name: &str, resource_name: &str) -> String {
    format!(
        "mcp__{}__resource__{}",
        normalize_mcp_name(server_name),
        normalize_mcp_name(resource_name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_segments_for_qualified_names() {
        assert_eq!(normalize_mcp_name(" GitHub Tools "), "github_tools");
        assert_eq!(normalize_mcp_name("***"), "unnamed");
    }

    #[test]
    fn builds_tool_and_resource_names() {
        assert_eq!(
            build_mcp_tool_name("GitHub Tools", "Create-Issue"),
            "mcp__github_tools__create_issue"
        );
        assert_eq!(
            build_mcp_resource_name("GitHub Tools", "Issue List"),
            "mcp__github_tools__resource__issue_list"
        );
    }
}
