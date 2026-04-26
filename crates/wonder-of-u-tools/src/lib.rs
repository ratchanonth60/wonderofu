mod bash;
mod files;
mod search;

use std::{path::Path, sync::Arc};

use serde::de::DeserializeOwned;
use serde_json::Value;
use wonder_of_u_core::{FeatureFlag, Result, Tool, ToolKind, ToolRegistry, ToolSpec, WonderError};

pub use bash::{BashInput, BashTool};
pub use files::{
    FileEditInput, FileEditTool, FileReadInput, FileReadTool, FileWriteInput, FileWriteMode,
    FileWriteTool,
};
pub use search::{GlobEntryType, GlobInput, GlobTool, GrepInput, GrepTool};

#[must_use]
pub fn builtin_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(BashTool),
        Arc::new(FileReadTool),
        Arc::new(FileWriteTool),
        Arc::new(FileEditTool),
        Arc::new(GlobTool),
        Arc::new(GrepTool),
    ]
}

pub fn builtin_registry() -> Result<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    for tool in builtin_tools() {
        registry.register(tool)?;
    }
    Ok(registry)
}

fn base_spec(name: &str, description: &str, kind: ToolKind) -> ToolSpec {
    let mut spec = ToolSpec::new(name, description, kind);
    spec.required_features.insert(FeatureFlag::Tools);
    spec
}

fn parse_input<T>(tool_name: &str, input: &Value) -> Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_value(input.clone())
        .map_err(|error| WonderError::validation(format!("invalid {tool_name} input: {error}")))
}

fn require_non_empty_path(tool_name: &str, field: &str, path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Err(WonderError::validation(format!(
            "{tool_name} requires a non-empty `{field}`"
        )));
    }
    Ok(())
}

fn require_non_empty_text(tool_name: &str, field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(WonderError::validation(format!(
            "{tool_name} requires a non-empty `{field}`"
        )));
    }
    Ok(())
}

fn display_path(path: &Path, cwd: &Path) -> String {
    path.strip_prefix(cwd)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_registers_expected_order() {
        let registry = builtin_registry().expect("registry");
        let names = registry
            .all_specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "bash",
                "file_read",
                "file_write",
                "file_edit",
                "glob",
                "grep"
            ]
        );
    }
}
