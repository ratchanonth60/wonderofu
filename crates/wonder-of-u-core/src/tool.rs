use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    FeatureFlag, FeatureSet, PermissionDecision, PermissionMode, Result, SessionId, ToolUseId,
    WonderError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Shell,
    FileRead,
    FileWrite,
    Search,
    Web,
    Planning,
    Task,
    Agent,
    Interaction,
    Skill,
    Mcp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    Native,
    Mcp,
    Plugin,
    Skill,
    AgentGenerated,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub description: String,
    pub kind: ToolKind,
    pub source: ToolSource,
    #[serde(default)]
    pub required_features: BTreeSet<FeatureFlag>,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub destructive: bool,
    #[serde(default)]
    pub concurrency_safe: bool,
}

impl ToolSpec {
    #[must_use]
    pub fn new(name: impl Into<String>, description: impl Into<String>, kind: ToolKind) -> Self {
        Self {
            name: name.into(),
            aliases: Vec::new(),
            description: description.into(),
            kind,
            source: ToolSource::Native,
            required_features: BTreeSet::new(),
            input_schema: Value::Null,
            read_only: false,
            destructive: false,
            concurrency_safe: false,
        }
    }

    #[must_use]
    pub fn is_enabled(&self, features: &FeatureSet) -> bool {
        features.contains_all(&self.required_features)
    }
}

#[derive(Clone, Debug)]
pub struct ToolContext {
    pub session_id: SessionId,
    pub cwd: PathBuf,
    pub permission_mode: PermissionMode,
    pub features: FeatureSet,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub use_id: ToolUseId,
    pub success: bool,
    pub content: String,
    #[serde(default)]
    pub metadata: Value,
}

impl ToolResult {
    #[must_use]
    pub fn success(use_id: ToolUseId, content: impl Into<String>) -> Self {
        Self {
            use_id,
            success: true,
            content: content.into(),
            metadata: Value::Null,
        }
    }

    #[must_use]
    pub fn failure(use_id: ToolUseId, content: impl Into<String>) -> Self {
        Self {
            use_id,
            success: false,
            content: content.into(),
            metadata: Value::Null,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolProgress {
    pub use_id: ToolUseId,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<f32>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    fn validate_input(&self, _input: &Value) -> Result<()> {
        Ok(())
    }

    fn permission_decision(&self, context: &ToolContext, _input: &Value) -> PermissionDecision {
        let spec = self.spec();
        context
            .permission_mode
            .default_decision(spec.read_only, spec.destructive)
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult>;
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
    aliases: BTreeMap<String, String>,
}

impl ToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<()> {
        let spec = tool.spec();
        let canonical = normalize_name(&spec.name)?;
        if self.tools.contains_key(&canonical) || self.aliases.contains_key(&canonical) {
            return Err(WonderError::validation(format!(
                "duplicate tool: {canonical}"
            )));
        }

        for alias in &spec.aliases {
            let alias = normalize_name(alias)?;
            if self.tools.contains_key(&alias) || self.aliases.contains_key(&alias) {
                return Err(WonderError::validation(format!(
                    "duplicate tool alias: {alias}"
                )));
            }
        }

        for alias in &spec.aliases {
            self.aliases
                .insert(normalize_name(alias)?, canonical.clone());
        }
        self.tools.insert(canonical, tool);
        Ok(())
    }

    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let normalized = name.trim().to_ascii_lowercase();
        self.tools.get(&normalized).cloned().or_else(|| {
            self.aliases
                .get(&normalized)
                .and_then(|canonical| self.tools.get(canonical))
                .cloned()
        })
    }

    pub fn enabled_specs(&self, features: &FeatureSet) -> Vec<ToolSpec> {
        self.tools
            .values()
            .map(|tool| tool.spec())
            .filter(|spec| spec.is_enabled(features))
            .collect()
    }
}

fn normalize_name(name: &str) -> Result<String> {
    let normalized = name.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Err(WonderError::validation("tool name cannot be empty"));
    }
    if normalized.chars().any(char::is_whitespace) {
        return Err(WonderError::validation(format!(
            "tool name contains whitespace: {name}"
        )));
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ReadTool;

    #[async_trait]
    impl Tool for ReadTool {
        fn spec(&self) -> ToolSpec {
            let mut spec = ToolSpec::new("file_read", "read a file", ToolKind::FileRead);
            spec.aliases.push("read".into());
            spec.read_only = true;
            spec
        }

        async fn execute(
            &self,
            _context: ToolContext,
            use_id: ToolUseId,
            _input: Value,
        ) -> Result<ToolResult> {
            Ok(ToolResult::success(use_id, "ok"))
        }
    }

    #[test]
    fn registry_resolves_tool_aliases() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(ReadTool)).expect("register");

        assert!(registry.resolve("file_read").is_some());
        assert!(registry.resolve("read").is_some());
    }

    #[test]
    fn default_permission_allows_read_only_tools() {
        let tool = ReadTool;
        let context = ToolContext {
            session_id: SessionId::new(),
            cwd: PathBuf::from("/workspace"),
            permission_mode: PermissionMode::Default,
            features: FeatureSet::first_release(),
        };

        assert!(
            tool.permission_decision(&context, &Value::Null)
                .is_allowed()
        );
    }
}
