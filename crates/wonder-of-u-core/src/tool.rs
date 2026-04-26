use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    AdditionalWorkingDirectory, FeatureFlag, FeatureSet, PermissionDecision, PermissionMode,
    PermissionRequest, PermissionRule, Result, SessionId, ToolPermissionContext, ToolUseId,
    WonderError, evaluate_permission,
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

    pub fn validate(&self) -> Result<()> {
        let canonical = normalize_name(&self.name)?;
        let mut seen = BTreeSet::from([canonical]);
        for alias in &self.aliases {
            let alias = normalize_name(alias)?;
            if !seen.insert(alias.clone()) {
                return Err(WonderError::validation(format!(
                    "duplicate tool alias: {alias}"
                )));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn with_input_schema(mut self, schema: ToolSchema) -> Self {
        self.input_schema = schema.into();
        self
    }

    #[must_use]
    pub fn is_enabled(&self, features: &FeatureSet) -> bool {
        features.contains_all(&self.required_features)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolSchema {
    value: Value,
}

impl ToolSchema {
    #[must_use]
    pub fn any() -> Self {
        Self { value: Value::Null }
    }

    #[must_use]
    pub fn object() -> Self {
        Self {
            value: json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false,
            }),
        }
    }

    #[must_use]
    pub fn property(mut self, name: impl Into<String>, schema: Value) -> Self {
        if let Some(properties) = self.properties_mut() {
            properties.insert(name.into(), schema);
        }
        self
    }

    #[must_use]
    pub fn required(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        if let Some(required) = self.required_mut() {
            if !required
                .iter()
                .any(|value| value.as_str() == Some(name.as_str()))
            {
                required.push(Value::String(name));
            }
        }
        self
    }

    #[must_use]
    pub fn additional_properties(mut self, allowed: bool) -> Self {
        if let Some(object) = self.value.as_object_mut() {
            object.insert("additionalProperties".into(), Value::Bool(allowed));
        }
        self
    }

    #[must_use]
    pub fn string(description: impl Into<String>) -> Value {
        json!({
            "type": "string",
            "description": description.into(),
        })
    }

    #[must_use]
    pub fn boolean(description: impl Into<String>) -> Value {
        json!({
            "type": "boolean",
            "description": description.into(),
        })
    }

    #[must_use]
    pub fn integer(description: impl Into<String>) -> Value {
        json!({
            "type": "integer",
            "description": description.into(),
        })
    }

    #[must_use]
    pub fn enumeration(
        description: impl Into<String>,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Value {
        json!({
            "type": "string",
            "description": description.into(),
            "enum": values.into_iter().map(Into::into).collect::<Vec<_>>(),
        })
    }

    #[must_use]
    pub fn into_value(self) -> Value {
        self.value
    }

    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.value
    }

    fn properties_mut(&mut self) -> Option<&mut Map<String, Value>> {
        self.value
            .as_object_mut()?
            .get_mut("properties")?
            .as_object_mut()
    }

    fn required_mut(&mut self) -> Option<&mut Vec<Value>> {
        self.value
            .as_object_mut()?
            .get_mut("required")?
            .as_array_mut()
    }
}

impl From<ToolSchema> for Value {
    fn from(schema: ToolSchema) -> Self {
        schema.into_value()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolQuery {
    pub features: FeatureSet,
}

impl ToolQuery {
    #[must_use]
    pub fn new(features: FeatureSet) -> Self {
        Self { features }
    }

    #[must_use]
    pub fn allows(&self, spec: &ToolSpec) -> bool {
        spec.is_enabled(&self.features)
    }
}

impl From<&ToolContext> for ToolQuery {
    fn from(context: &ToolContext) -> Self {
        Self {
            features: context.features.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ToolContext {
    pub session_id: SessionId,
    pub cwd: PathBuf,
    pub permission_mode: PermissionMode,
    pub additional_working_directories: Vec<AdditionalWorkingDirectory>,
    pub permission_rules: Vec<PermissionRule>,
    pub features: FeatureSet,
}

impl ToolContext {
    #[must_use]
    pub fn permission_context(&self) -> ToolPermissionContext {
        ToolPermissionContext {
            cwd: self.cwd.clone(),
            mode: self.permission_mode,
            additional_working_directories: self.additional_working_directories.clone(),
            rules: self.permission_rules.clone(),
        }
    }
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

    fn permission_decision(&self, context: &ToolContext, input: &Value) -> PermissionDecision {
        let spec = self.spec();
        let request = permission_request_for_spec(&spec, input);
        evaluate_permission(&context.permission_context(), &request)
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult>;
}

#[derive(Clone)]
struct ToolEntry {
    spec: ToolSpec,
    tool: Arc<dyn Tool>,
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, ToolEntry>,
    aliases: BTreeMap<String, String>,
    order: Vec<String>,
}

impl ToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<()> {
        let spec = tool.spec();
        spec.validate()?;
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
        self.order.push(canonical.clone());
        self.tools.insert(canonical, ToolEntry { spec, tool });
        Ok(())
    }

    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.resolve_entry(name)
            .map(|entry| Arc::clone(&entry.tool))
    }

    #[must_use]
    pub fn resolve_enabled(&self, name: &str, query: &ToolQuery) -> Option<Arc<dyn Tool>> {
        let entry = self.resolve_entry(name)?;
        query.allows(&entry.spec).then(|| Arc::clone(&entry.tool))
    }

    pub fn all_specs(&self) -> Vec<ToolSpec> {
        self.order
            .iter()
            .filter_map(|canonical| self.tools.get(canonical))
            .map(|entry| entry.spec.clone())
            .collect()
    }

    pub fn enabled_specs(&self, features: &FeatureSet) -> Vec<ToolSpec> {
        self.enabled_specs_for(&ToolQuery::new(features.clone()))
    }

    pub fn enabled_specs_for(&self, query: &ToolQuery) -> Vec<ToolSpec> {
        self.all_specs()
            .into_iter()
            .filter(|spec| query.allows(spec))
            .collect()
    }

    fn resolve_entry(&self, name: &str) -> Option<&ToolEntry> {
        let normalized = normalize_lookup(name)?;
        self.tools.get(&normalized).or_else(|| {
            self.aliases
                .get(&normalized)
                .and_then(|canonical| self.tools.get(canonical))
        })
    }
}

fn normalize_name(name: &str) -> Result<String> {
    let normalized = normalize_lookup(name)
        .ok_or_else(|| WonderError::validation("tool name cannot be empty"))?;
    if normalized.chars().any(char::is_whitespace) {
        return Err(WonderError::validation(format!(
            "tool name contains whitespace: {name}"
        )));
    }
    Ok(normalized)
}

fn normalize_lookup(name: &str) -> Option<String> {
    let normalized = name.trim().to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn permission_request_for_spec(spec: &ToolSpec, input: &Value) -> PermissionRequest {
    let mut request = PermissionRequest::new(spec.name.clone())
        .with_aliases(spec.aliases.clone())
        .read_only(spec.read_only)
        .destructive(spec.destructive);

    if let Some(command) = infer_shell_command(spec, input) {
        request = request.with_shell_command(command);
    }

    let inferred_paths = infer_paths(input);
    if !inferred_paths.is_empty() {
        request = request.with_paths(inferred_paths);
    }

    request
}

fn infer_shell_command(spec: &ToolSpec, input: &Value) -> Option<String> {
    if spec.kind != ToolKind::Shell {
        return None;
    }

    let object = input.as_object()?;
    ["command", "cmd", "script"]
        .into_iter()
        .find_map(|field| object.get(field).and_then(Value::as_str))
        .map(str::to_string)
}

fn infer_paths(input: &Value) -> Vec<PathBuf> {
    let Some(object) = input.as_object() else {
        return Vec::new();
    };

    const PATH_FIELDS: [&str; 8] = [
        "path",
        "paths",
        "file_path",
        "file_paths",
        "directory",
        "directories",
        "cwd",
        "target",
    ];

    PATH_FIELDS
        .into_iter()
        .filter_map(|field| object.get(field))
        .flat_map(value_to_paths)
        .collect()
}

fn value_to_paths(value: &Value) -> Vec<PathBuf> {
    match value {
        Value::String(path) => vec![PathBuf::from(path)],
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(PathBuf::from)
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ReadTool;
    struct WebTool;
    struct BashTool;

    #[async_trait]
    impl Tool for ReadTool {
        fn spec(&self) -> ToolSpec {
            let mut spec = ToolSpec::new("file_read", "read a file", ToolKind::FileRead)
                .with_input_schema(
                    ToolSchema::object()
                        .property("path", ToolSchema::string("absolute file path"))
                        .required("path"),
                );
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

    #[async_trait]
    impl Tool for WebTool {
        fn spec(&self) -> ToolSpec {
            let mut spec = ToolSpec::new("web_fetch", "fetch a page", ToolKind::Web);
            spec.required_features.insert(FeatureFlag::WebTools);
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

    #[async_trait]
    impl Tool for BashTool {
        fn spec(&self) -> ToolSpec {
            let mut spec = ToolSpec::new("bash", "run a shell command", ToolKind::Shell)
                .with_input_schema(
                    ToolSchema::object()
                        .property("command", ToolSchema::string("shell command to run"))
                        .required("command"),
                );
            spec.destructive = true;
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

    fn tool_context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd: PathBuf::from("/workspace"),
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
        }
    }

    #[test]
    fn registry_resolves_tool_aliases() {
        let mut registry = ToolRegistry::new();
        let tool = Arc::new(ReadTool);
        registry.register(tool).expect("register");

        let mut alias_registry = ToolRegistry::new();
        let mut spec = ToolSpec::new("file_read", "read a file", ToolKind::FileRead);
        spec.aliases.push("read".into());

        struct AliasTool(ToolSpec);
        #[async_trait]
        impl Tool for AliasTool {
            fn spec(&self) -> ToolSpec {
                self.0.clone()
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

        alias_registry
            .register(Arc::new(AliasTool(spec)))
            .expect("register alias");

        assert!(alias_registry.resolve("file_read").is_some());
        assert!(alias_registry.resolve("read").is_some());
    }

    #[test]
    fn default_permission_allows_read_only_tools() {
        let tool = ReadTool;
        let context = tool_context();

        assert!(
            tool.permission_decision(&context, &json!({ "path": "src/lib.rs" }))
                .is_allowed()
        );
    }

    #[test]
    fn file_tools_require_review_for_paths_outside_scope() {
        let tool = ReadTool;
        let context = tool_context();

        let decision = tool.permission_decision(&context, &json!({ "path": "../secret.txt" }));

        assert!(matches!(decision, PermissionDecision::Ask { .. }));
    }

    #[test]
    fn allow_rule_grants_access_to_extra_path_prefix() {
        let tool = ReadTool;
        let mut context = tool_context();
        context.permission_rules.push(
            PermissionRule::new(
                "file_read",
                crate::PermissionRuleBehavior::Allow,
                crate::PermissionRuleSource::CliArg,
            )
            .for_path_prefix("../shared"),
        );

        let decision =
            tool.permission_decision(&context, &json!({ "path": "../shared/readme.md" }));

        assert!(matches!(decision, PermissionDecision::Allow { .. }));
    }

    #[test]
    fn shell_tools_apply_safety_checks_from_input() {
        let tool = BashTool;
        let context = tool_context();

        let decision = tool.permission_decision(&context, &json!({ "command": "echo ${cmd@P}" }));

        assert!(matches!(decision, PermissionDecision::Deny { .. }));
    }

    #[test]
    fn registry_preserves_order_and_filters_disabled_tools() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(ReadTool)).expect("read");
        registry.register(Arc::new(WebTool)).expect("web");

        let disabled_query = ToolQuery::new(FeatureSet::empty());
        let enabled_query = ToolQuery::new(FeatureSet::first_release());

        assert_eq!(
            registry
                .all_specs()
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>(),
            vec!["file_read", "web_fetch"]
        );
        assert!(
            registry
                .resolve_enabled("web_fetch", &disabled_query)
                .is_none()
        );
        assert!(
            registry
                .resolve_enabled("web_fetch", &enabled_query)
                .is_some()
        );
        assert_eq!(
            registry
                .enabled_specs_for(&disabled_query)
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>(),
            vec!["file_read"]
        );
    }

    #[test]
    fn schema_builder_tracks_properties_and_required_fields() {
        let schema = ToolSchema::object()
            .property("query", ToolSchema::string("search term"))
            .property("limit", ToolSchema::integer("maximum result count"))
            .required("query");

        assert_eq!(
            schema.into_value(),
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "search term"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "maximum result count"
                    }
                },
                "required": ["query"],
                "additionalProperties": false,
            })
        );
    }

    #[test]
    fn tool_spec_validation_rejects_duplicate_aliases() {
        let mut spec = ToolSpec::new("demo_tool", "demo tool", ToolKind::Skill);
        spec.aliases = vec!["run".into(), "run".into()];

        let error = spec.validate().expect_err("duplicate alias");
        assert!(error.to_string().contains("duplicate tool alias"));
    }
}
