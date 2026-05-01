use std::{
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{
    FeatureFlag, FeatureSet, Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec,
    ToolUseId, WonderError, resolve_path,
};
use wonder_of_u_plugins::{PluginCatalog, PluginConfig, PluginConfigStore};
use wonder_of_u_skills::{SkillCatalog, SkillRegistration};
use wonder_of_u_storage::StoragePaths;

use crate::{
    app_root, base_spec, builtin_registry, parse_input, provider_tool_specs,
    require_non_empty_path, require_non_empty_text, schema_with_aliases,
};

const SOURCE_LSP_RUNTIME_UNAVAILABLE: &str = "LSP requests are not implemented in wonder-of-u-tools; only environment discovery and source-compatible validation are available";
const SOURCE_BRIEF_RUNTIME_UNAVAILABLE: &str = "SendUserMessage delivery is not implemented in wonder-of-u-tools; use normal assistant output or session brief mode instead";
const SOURCE_CONFIG_WRITE_UNAVAILABLE: &str =
    "config writes are not implemented in wonder-of-u-tools";
const SOURCE_CONFIG_READ_UNAVAILABLE: &str = "that setting is not backed by the Rust runtime yet";

const KNOWN_SOURCE_SETTINGS: &[&str] = &[
    "editorMode",
    "verbose",
    "preferredNotifChannel",
    "autoCompactEnabled",
    "autoMemoryEnabled",
    "autoDreamEnabled",
    "fileCheckpointingEnabled",
    "showTurnDuration",
    "terminalProgressBarEnabled",
    "todoFeatureEnabled",
    "alwaysThinkingEnabled",
    "language",
    "teammateMode",
    "classifierPermissionsEnabled",
    "voiceEnabled",
    "remoteControlAtStartup",
    "taskCompleteNotifEnabled",
    "inputNeededNotifEnabled",
    "agentPushNotifEnabled",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LspOperation {
    GoToDefinition,
    FindReferences,
    Hover,
    DocumentSymbol,
    WorkspaceSymbol,
    GoToImplementation,
    PrepareCallHierarchy,
    IncomingCalls,
    OutgoingCalls,
}

impl LspOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::GoToDefinition => "goToDefinition",
            Self::FindReferences => "findReferences",
            Self::Hover => "hover",
            Self::DocumentSymbol => "documentSymbol",
            Self::WorkspaceSymbol => "workspaceSymbol",
            Self::GoToImplementation => "goToImplementation",
            Self::PrepareCallHierarchy => "prepareCallHierarchy",
            Self::IncomingCalls => "incomingCalls",
            Self::OutgoingCalls => "outgoingCalls",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LspInput {
    pub operation: LspOperation,
    #[serde(rename = "file_path", alias = "filePath")]
    pub file_path: PathBuf,
    pub line: usize,
    pub character: usize,
}

impl LspInput {
    fn validate_shape(&self) -> Result<()> {
        require_non_empty_path("lsp", "file_path", &self.file_path)?;
        if self.line == 0 {
            return Err(WonderError::validation(
                "lsp requires `line` to be greater than zero",
            ));
        }
        if self.character == 0 {
            return Err(WonderError::validation(
                "lsp requires `character` to be greater than zero",
            ));
        }
        Ok(())
    }

    fn resolve_file(&self, context: &ToolContext) -> Result<PathBuf> {
        self.validate_shape()?;
        let resolved = resolve_path(&self.file_path, &context.cwd);
        let metadata = fs::metadata(&resolved).map_err(|error| match error.kind() {
            ErrorKind::NotFound => WonderError::validation(format!(
                "lsp file does not exist: {}",
                self.file_path.display()
            )),
            _ => WonderError::validation(format!(
                "unable to access lsp file {}: {error}",
                self.file_path.display()
            )),
        })?;
        if !metadata.is_file() {
            return Err(WonderError::validation(format!(
                "lsp path is not a file: {}",
                self.file_path.display()
            )));
        }
        Ok(resolved)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigInput {
    pub setting: String,
    #[serde(default)]
    pub value: Option<Value>,
}

impl ConfigInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("config", "setting", &self.setting)?;
        if let Some(value) = &self.value
            && !(value.is_string() || value.is_boolean() || value.is_number())
        {
            return Err(WonderError::validation(
                "config `value` must be a string, boolean, or number",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BriefStatus {
    Normal,
    Proactive,
}

impl BriefStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Proactive => "proactive",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BriefInput {
    pub message: String,
    #[serde(default)]
    pub attachments: Vec<PathBuf>,
    pub status: BriefStatus,
}

impl BriefInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("send_user_message", "message", &self.message)?;
        for attachment in &self.attachments {
            require_non_empty_path("send_user_message", "attachments", attachment)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillInput {
    pub skill: String,
    #[serde(default)]
    pub args: Option<String>,
}

impl SkillInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("skill", "skill", &self.skill)
    }

    fn normalized_skill(&self) -> &str {
        self.skill.trim().trim_start_matches('/')
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSearchInput {
    pub query: String,
    #[serde(default)]
    pub max_results: Option<usize>,
}

impl ToolSearchInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("tool_search", "query", &self.query)?;
        if self.max_results == Some(0) {
            return Err(WonderError::validation(
                "tool_search max_results must be greater than zero",
            ));
        }
        Ok(())
    }

    fn max_results(&self) -> usize {
        self.max_results.unwrap_or(5)
    }
}

#[derive(Debug, Default)]
pub struct LspTool;

#[derive(Debug, Default)]
pub struct ConfigTool;

#[derive(Debug, Default)]
pub struct BriefTool;

#[derive(Debug, Default)]
pub struct SkillTool;

#[derive(Debug, Default)]
pub struct ToolSearchTool;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct BriefAttachment {
    path: String,
    size: u64,
    is_image: bool,
}

#[derive(Clone, Copy)]
enum ConfigReadMode {
    SelectedModel,
    PermissionMode,
    Unsupported(&'static str),
}

struct ConfigSetting<'a> {
    key: &'a str,
    description: &'static str,
    read_mode: ConfigReadMode,
}

#[async_trait]
impl Tool for LspTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "lsp",
            "Inspect source-compatible LSP requests and local diagnostics availability",
            ToolKind::Search,
        )
        .with_input_schema(
            ToolSchema::object()
                .property(
                    "operation",
                    ToolSchema::enumeration(
                        "The LSP operation to perform",
                        [
                            "goToDefinition",
                            "findReferences",
                            "hover",
                            "documentSymbol",
                            "workspaceSymbol",
                            "goToImplementation",
                            "prepareCallHierarchy",
                            "incomingCalls",
                            "outgoingCalls",
                        ],
                    ),
                )
                .property(
                    "file_path",
                    schema_with_aliases(
                        ToolSchema::string(
                            "source file path; accepts the source-compatible `filePath` alias",
                        ),
                        &["filePath"],
                    ),
                )
                .property("line", ToolSchema::integer("1-based source line number"))
                .property(
                    "character",
                    ToolSchema::integer("1-based character offset within the line"),
                )
                .required("operation")
                .required("file_path")
                .required("line")
                .required("character"),
        );
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<LspInput>("lsp", input)?.validate_shape()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<LspInput>("lsp", &input)?;
        let resolved = input.resolve_file(&context)?;
        let mut result = ToolResult::failure(
            use_id,
            format!("lsp is unavailable: {SOURCE_LSP_RUNTIME_UNAVAILABLE}"),
        );
        result.metadata = json!({
            "supported": false,
            "tool": "lsp",
            "operation": input.operation.as_str(),
            "file_path": resolved.display().to_string(),
            "line": input.line,
            "character": input.character,
            "diagnostics": lsp_environment_summary(&context.cwd),
        });
        Ok(result)
    }
}

#[async_trait]
impl Tool for ConfigTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "config",
            "Read a safe source-compatible subset of runtime configuration state",
            ToolKind::Interaction,
        )
        .with_input_schema(ToolSchema::object());
        spec.input_schema = json!({
            "type": "object",
            "properties": {
                "setting": {
                    "type": "string",
                    "description": "configuration key to inspect, such as `model` or `permissions.defaultMode`"
                },
                "value": {
                    "description": "source-compatible write value; set requests are currently unsupported",
                    "oneOf": [
                        { "type": "string" },
                        { "type": "boolean" },
                        { "type": "number" }
                    ]
                }
            },
            "required": ["setting"],
            "additionalProperties": false
        });
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<ConfigInput>("config", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<ConfigInput>("config", &input)?;
        input.validate()?;

        if input.value.is_some() {
            let mut result = ToolResult::failure(
                use_id,
                format!("config set is unavailable: {SOURCE_CONFIG_WRITE_UNAVAILABLE}"),
            );
            result.metadata = json!({
                "supported": false,
                "tool": "config",
                "operation": "set",
                "setting": input.setting,
            });
            return Ok(result);
        }

        let Some(setting) = config_setting(&input.setting) else {
            let mut result =
                ToolResult::failure(use_id, format!("Unknown setting: \"{}\"", input.setting));
            result.metadata = json!({
                "supported": false,
                "tool": "config",
                "operation": "get",
                "setting": input.setting,
                "reason": "unknown_setting",
            });
            return Ok(result);
        };

        let storage_root = app_root().ok();
        match setting.read_mode {
            ConfigReadMode::SelectedModel => {
                let value = read_selected_model(storage_root.as_deref())?;
                let mut result =
                    ToolResult::success(use_id, format!("setting={}\nvalue={value}", setting.key));
                result.metadata = json!({
                    "success": true,
                    "operation": "get",
                    "setting": setting.key,
                    "value": value,
                });
                Ok(result)
            }
            ConfigReadMode::PermissionMode => {
                let value = serde_json::to_value(context.permission_mode)
                    .expect("permission mode is serializable");
                let mut result = ToolResult::success(
                    use_id,
                    format!(
                        "setting={}\nvalue={}",
                        setting.key,
                        value.as_str().unwrap_or("default")
                    ),
                );
                result.metadata = json!({
                    "success": true,
                    "operation": "get",
                    "setting": setting.key,
                    "value": value,
                });
                Ok(result)
            }
            ConfigReadMode::Unsupported(reason) => {
                let mut result = ToolResult::failure(
                    use_id,
                    format!("config get for `{}` is unavailable: {reason}", setting.key),
                );
                result.metadata = json!({
                    "supported": false,
                    "tool": "config",
                    "operation": "get",
                    "setting": setting.key,
                    "reason": SOURCE_CONFIG_READ_UNAVAILABLE,
                    "description": setting.description,
                });
                Ok(result)
            }
        }
    }
}

#[async_trait]
impl Tool for BriefTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "send_user_message",
            "Inspect a source-compatible SendUserMessage payload",
            ToolKind::Interaction,
        )
        .with_input_schema(
            ToolSchema::object()
                .property(
                    "message",
                    ToolSchema::string("message intended for the user"),
                )
                .property(
                    "attachments",
                    json!({
                        "type": "array",
                        "description": "optional paths to files the user should receive",
                        "items": { "type": "string" }
                    }),
                )
                .property(
                    "status",
                    ToolSchema::enumeration("message intent", ["normal", "proactive"]),
                )
                .required("message")
                .required("status"),
        );
        spec.aliases.push("SendUserMessage".into());
        spec.aliases.push("Brief".into());
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<BriefInput>("send_user_message", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<BriefInput>("send_user_message", &input)?;
        input.validate()?;
        let attachments = resolve_attachments(&context.cwd, &input.attachments)?;
        let mut result = ToolResult::failure(
            use_id,
            format!("send_user_message is unavailable: {SOURCE_BRIEF_RUNTIME_UNAVAILABLE}"),
        );
        result.metadata = json!({
            "supported": false,
            "tool": "send_user_message",
            "status": input.status.as_str(),
            "message": input.message,
            "attachment_count": attachments.len(),
            "attachments": attachments,
        });
        Ok(result)
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "skill",
            "Inspect bundled, local, and plugin-provided skill metadata",
            ToolKind::Skill,
        )
        .with_input_schema(
            ToolSchema::object()
                .property(
                    "skill",
                    ToolSchema::string(
                        "skill name or slash command, with or without a leading slash",
                    ),
                )
                .property(
                    "args",
                    ToolSchema::string("optional source-compatible skill arguments"),
                )
                .required("skill"),
        );
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec.required_features.insert(FeatureFlag::Skills);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<SkillInput>("skill", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<SkillInput>("skill", &input)?;
        input.validate()?;

        let storage_root = app_root().ok();
        let catalog = load_skill_catalog(&context.cwd, storage_root.as_deref(), &context.features)?;
        let Some(skill) = find_skill(&catalog, input.normalized_skill()) else {
            let mut result = ToolResult::failure(
                use_id,
                format!("Unknown skill: {}", input.normalized_skill()),
            );
            result.metadata = json!({
                "supported": false,
                "tool": "skill",
                "skill": input.normalized_skill(),
                "reason": "unknown_skill",
            });
            return Ok(result);
        };

        let prompt_excerpt = skill
            .prompt
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("-")
            .chars()
            .take(120)
            .collect::<String>();
        let allowed_tools = if skill.manifest.allowed_tools.is_empty() {
            "-".to_string()
        } else {
            skill.manifest.allowed_tools.join(",")
        };
        let slash_command = skill
            .command_spec
            .as_ref()
            .map(|command| command.name.as_str())
            .unwrap_or("-");

        let mut result = ToolResult::success(
            use_id,
            [
                "status=metadata_only".into(),
                format!("skill={}", skill.manifest.name),
                format!("source={}", skill.source.label()),
                format!("trust={}", skill.trust.label()),
                format!("allowed_tools={allowed_tools}"),
                format!("slash_command={slash_command}"),
                format!("prompt_excerpt={prompt_excerpt}"),
                "note=skill execution is intentionally unsupported in wonder-of-u-tools; this result only exposes catalog metadata".into(),
            ]
            .join("\n"),
        );
        result.metadata = json!({
            "success": true,
            "tool": "skill",
            "mode": "metadata_only",
            "execution_supported": false,
            "skill": skill.manifest.name.clone(),
            "args": input.args,
            "source": skill.source.label(),
            "trust": skill.trust.label(),
            "allowed_tools": skill.manifest.allowed_tools.clone(),
            "slash_command": skill.command_spec.as_ref().map(|command| command.name.clone()),
        });
        Ok(result)
    }
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec(
            "tool_search",
            "Search enabled tool schemas by exact selection or keyword",
            ToolKind::Search,
        )
        .with_input_schema(
            ToolSchema::object()
                .property(
                    "query",
                    ToolSchema::string("tool search query or `select:ToolName` selector"),
                )
                .property(
                    "max_results",
                    ToolSchema::integer("maximum number of matches to return"),
                )
                .required("query"),
        );
        spec.aliases.push("ToolSearch".into());
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<ToolSearchInput>("tool_search", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<ToolSearchInput>("tool_search", &input)?;
        input.validate()?;

        let registry = builtin_registry()?;
        let specs = provider_tool_specs(&registry, &context, None)
            .into_iter()
            .filter(|spec| spec.name != "tool_search")
            .collect::<Vec<_>>();
        let matches = if let Some(selection) = input.query.trim().strip_prefix("select:") {
            select_tools(&specs, selection)
        } else {
            keyword_tools(&specs, &input.query, input.max_results())
        };

        let mut result =
            ToolResult::success(use_id, render_tool_search_matches(&input.query, &matches));
        result.metadata = json!({
            "query": input.query,
            "matches": matches.iter().map(|spec| spec.name.clone()).collect::<Vec<_>>(),
            "total_tools": specs.len(),
        });
        Ok(result)
    }
}

fn config_setting(key: &str) -> Option<ConfigSetting<'_>> {
    Some(match key {
        "model" => ConfigSetting {
            key,
            description: "Override the default model",
            read_mode: ConfigReadMode::SelectedModel,
        },
        "permissions.defaultMode" => ConfigSetting {
            key,
            description: "Default permission mode for tool usage",
            read_mode: ConfigReadMode::PermissionMode,
        },
        "theme" => ConfigSetting {
            key,
            description: "Color theme for the UI",
            read_mode: ConfigReadMode::Unsupported(
                "theme is session/UI state and is not persisted in the current Rust runtime",
            ),
        },
        _ if KNOWN_SOURCE_SETTINGS.contains(&key) => ConfigSetting {
            key,
            description: "Source-compatible setting key",
            read_mode: ConfigReadMode::Unsupported(SOURCE_CONFIG_READ_UNAVAILABLE),
        },
        _ => return None,
    })
}

fn read_selected_model(storage_root: Option<&Path>) -> Result<String> {
    let Some(storage_root) = storage_root else {
        return Ok("default".into());
    };
    let path = StoragePaths::new(storage_root).settings_path();
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok("default".into()),
        Err(error) => return Err(error.into()),
    };
    let value: Value = serde_json::from_str(&content)?;
    Ok(value
        .get("selected_model")
        .and_then(Value::as_str)
        .filter(|model| !model.trim().is_empty())
        .unwrap_or("default")
        .to_string())
}

fn resolve_attachments(cwd: &Path, attachments: &[PathBuf]) -> Result<Vec<BriefAttachment>> {
    attachments
        .iter()
        .map(|attachment| {
            let resolved = resolve_path(attachment, cwd);
            let metadata = fs::metadata(&resolved).map_err(|error| match error.kind() {
                ErrorKind::NotFound => WonderError::validation(format!(
                    "attachment does not exist: {}",
                    attachment.display()
                )),
                _ => WonderError::validation(format!(
                    "unable to access attachment {}: {error}",
                    attachment.display()
                )),
            })?;
            if !metadata.is_file() {
                return Err(WonderError::validation(format!(
                    "attachment is not a file: {}",
                    attachment.display()
                )));
            }
            Ok(BriefAttachment {
                path: resolved.display().to_string(),
                size: metadata.len(),
                is_image: resolved
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        matches!(
                            extension.to_ascii_lowercase().as_str(),
                            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg"
                        )
                    }),
            })
        })
        .collect()
}

fn load_skill_catalog(
    cwd: &Path,
    storage_root: Option<&Path>,
    features: &FeatureSet,
) -> Result<SkillCatalog> {
    let config = match storage_root {
        Some(storage_root) => PluginConfigStore::new(storage_root).read()?,
        None => PluginConfig::default(),
    };
    let plugins = if features.contains(FeatureFlag::Plugins) {
        PluginCatalog::load(cwd, storage_root, &config)
    } else {
        PluginCatalog::default()
    };
    Ok(SkillCatalog::load(cwd, storage_root, &config, &plugins))
}

fn find_skill<'a>(catalog: &'a SkillCatalog, query: &str) -> Option<&'a SkillRegistration> {
    let normalized = normalize_lookup(query)?;
    catalog.skills().iter().find(|skill| {
        normalize_lookup(&skill.manifest.name).as_deref() == Some(normalized.as_str())
            || skill.command_spec.as_ref().is_some_and(|command| {
                normalize_lookup(&command.name).as_deref() == Some(normalized.as_str())
                    || command.aliases.iter().any(|alias| {
                        normalize_lookup(alias).as_deref() == Some(normalized.as_str())
                    })
            })
    })
}

fn normalize_lookup(name: &str) -> Option<String> {
    let normalized = name.trim().trim_start_matches('/').to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn select_tools(specs: &[ToolSpec], selection: &str) -> Vec<ToolSpec> {
    selection
        .split(',')
        .filter_map(|name| {
            let normalized = normalize_lookup(name)?;
            specs.iter().find(|spec| {
                normalize_lookup(&spec.name).as_deref() == Some(normalized.as_str())
                    || spec.aliases.iter().any(|alias| {
                        normalize_lookup(alias).as_deref() == Some(normalized.as_str())
                    })
            })
        })
        .fold(Vec::new(), |mut matches, spec| {
            if !matches.iter().any(|existing| existing.name == spec.name) {
                matches.push(spec.clone());
            }
            matches
        })
}

fn keyword_tools(specs: &[ToolSpec], query: &str, limit: usize) -> Vec<ToolSpec> {
    let query = query.trim().to_ascii_lowercase();
    let terms = query
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>();
    let (required, optional): (Vec<_>, Vec<_>) =
        terms.iter().partition(|term| term.starts_with('+'));
    let required = required
        .into_iter()
        .map(|term: &&str| term.trim_start_matches('+'))
        .filter(|term: &&str| !term.is_empty())
        .collect::<Vec<_>>();
    let scoring_terms = if required.is_empty() {
        terms
    } else {
        required
            .iter()
            .copied()
            .chain(optional.into_iter().copied())
            .collect::<Vec<_>>()
    };

    let mut scored = specs
        .iter()
        .filter_map(|spec| {
            let haystacks = std::iter::once(spec.name.to_ascii_lowercase())
                .chain(spec.aliases.iter().map(|alias| alias.to_ascii_lowercase()))
                .chain(std::iter::once(spec.description.to_ascii_lowercase()))
                .collect::<Vec<_>>();

            if !required.is_empty()
                && !required
                    .iter()
                    .all(|term| haystacks.iter().any(|value| value.contains(term)))
            {
                return None;
            }

            let score = scoring_terms.iter().fold(0usize, |score, term| {
                score
                    + haystacks.iter().fold(0usize, |score, haystack| {
                        score
                            + if haystack == term {
                                10
                            } else if haystack
                                .split(|ch: char| !ch.is_ascii_alphanumeric())
                                .any(|part| part == *term)
                            {
                                6
                            } else if haystack.contains(term) {
                                2
                            } else {
                                0
                            }
                    })
            });

            (score > 0).then(|| (score, spec.clone()))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.name.cmp(&right.1.name))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, spec)| spec)
        .collect()
}

fn render_tool_search_matches(query: &str, matches: &[ToolSpec]) -> String {
    serde_json::to_string_pretty(&json!({
        "query": query,
        "matches": matches
            .iter()
            .map(|spec| {
                json!({
                    "name": spec.name,
                    "aliases": spec.aliases,
                    "description": spec.description,
                    "parameters": spec.input_schema,
                })
            })
            .collect::<Vec<_>>()
    }))
    .expect("tool search matches serialize")
}

fn lsp_environment_summary(cwd: &Path) -> Value {
    json!({
        "cwd": cwd.display().to_string(),
        "cargo_project": cwd.join("Cargo.toml").is_file(),
        "node_project": cwd.join("package.json").is_file(),
        "python_project": cwd.join("pyproject.toml").is_file(),
        "servers": {
            "rust-analyzer": executable_on_path("rust-analyzer"),
            "typescript-language-server": executable_on_path("typescript-language-server"),
            "pyright-langserver": executable_on_path("pyright-langserver"),
            "gopls": executable_on_path("gopls"),
            "clangd": executable_on_path("clangd"),
            "jdtls": executable_on_path("jdtls"),
        }
    })
}

fn executable_on_path(name: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|path| path.join(name).is_file())
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use serde_json::json;
    use wonder_of_u_core::{FeatureSet, PermissionMode, SessionId, ToolQuery};
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn tool_context(root: PathBuf) -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd: root,
            permission_mode: PermissionMode::AcceptEdits,
            additional_working_directories: Vec::new(),
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
        }
    }

    #[test]
    fn lsp_validation_accepts_source_file_path_alias() {
        let dir = unique_test_dir("tools-lsp-alias");
        let file = dir.join("src.rs");
        fs::write(&file, "fn main() {}\n").expect("source file");

        let input: LspInput = serde_json::from_value(json!({
            "operation": "hover",
            "filePath": file.display().to_string(),
            "line": 1,
            "character": 1,
        }))
        .expect("input");

        input.resolve_file(&tool_context(dir)).expect("valid");
    }

    #[test]
    fn lsp_execute_returns_explicit_unsupported_failure() {
        let dir = unique_test_dir("tools-lsp-unsupported");
        let file = dir.join("lib.rs");
        fs::write(&file, "pub fn demo() {}\n").expect("source file");
        let tool = LspTool;

        let result = block_on(tool.execute(
            tool_context(dir.clone()),
            ToolUseId::new(),
            json!({
                "operation": "documentSymbol",
                "filePath": file.display().to_string(),
                "line": 1,
                "character": 1,
            }),
        ))
        .expect("execute");

        assert!(!result.success);
        assert!(result.content.contains(SOURCE_LSP_RUNTIME_UNAVAILABLE));
        assert_eq!(result.metadata["supported"], false);
        assert_eq!(result.metadata["operation"], "documentSymbol");
        assert_eq!(
            result.metadata["diagnostics"]["cwd"],
            dir.display().to_string()
        );
    }

    #[test]
    fn config_reads_selected_model_and_permission_mode() {
        let dir = unique_test_dir("tools-config-model");
        let storage = dir.join(".config").join("wonder-of-u");
        fs::create_dir_all(storage.join("config")).expect("config dir");
        fs::write(
            storage.join("config/settings.json"),
            json!({ "selected_model": "opus" }).to_string(),
        )
        .expect("settings");

        let model = read_selected_model(Some(&storage)).expect("model");
        assert_eq!(model, "opus");

        let tool = ConfigTool;
        let result = block_on(tool.execute(
            tool_context(dir),
            ToolUseId::new(),
            json!({ "setting": "permissions.defaultMode" }),
        ))
        .expect("execute");

        assert!(result.success);
        assert!(result.content.contains("acceptEdits"));
        assert_eq!(result.metadata["value"], "acceptEdits");
    }

    #[test]
    fn config_reports_known_unbacked_reads_and_unavailable_writes() {
        let tool = ConfigTool;
        let context = tool_context(unique_test_dir("tools-config-unsupported"));

        let read = block_on(tool.execute(
            context.clone(),
            ToolUseId::new(),
            json!({ "setting": "theme" }),
        ))
        .expect("read");
        assert!(!read.success);
        assert_eq!(read.metadata["setting"], "theme");

        let write = block_on(tool.execute(
            context,
            ToolUseId::new(),
            json!({ "setting": "model", "value": "opus" }),
        ))
        .expect("write");
        assert!(!write.success);
        assert!(write.content.contains(SOURCE_CONFIG_WRITE_UNAVAILABLE));
        assert_eq!(write.metadata["operation"], "set");
    }

    #[test]
    fn brief_reports_attachment_metadata_without_claiming_delivery() {
        let dir = unique_test_dir("tools-brief-attachments");
        let screenshot = dir.join("shot.png");
        fs::write(&screenshot, [1_u8, 2, 3, 4]).expect("attachment");
        let tool = BriefTool;

        let result = block_on(tool.execute(
            tool_context(dir),
            ToolUseId::new(),
            json!({
                "message": "Done",
                "attachments": [screenshot.display().to_string()],
                "status": "normal",
            }),
        ))
        .expect("execute");

        assert!(!result.success);
        assert_eq!(result.metadata["attachment_count"], 1);
        assert_eq!(result.metadata["attachments"][0]["is_image"], true);
        assert!(result.content.contains(SOURCE_BRIEF_RUNTIME_UNAVAILABLE));
    }

    #[test]
    fn skill_returns_metadata_for_project_skills() {
        let dir = unique_test_dir("tools-skill-project");
        let skill_dir = dir.join(".wonder/skills/review");
        fs::create_dir_all(&skill_dir).expect("skill dir");
        fs::write(
            skill_dir.join("skill.json"),
            json!({
                "schema_version": 1,
                "name": "review",
                "description": "Review a patch",
                "prompt": "Inspect the diff carefully.",
                "allowed_tools": ["grep", "file_read"],
                "slash_command": "review",
                "slash_aliases": ["review-pr"]
            })
            .to_string(),
        )
        .expect("manifest");

        let tool = SkillTool;
        let result = block_on(tool.execute(
            tool_context(dir),
            ToolUseId::new(),
            json!({ "skill": "/review-pr", "args": "123" }),
        ))
        .expect("execute");

        assert!(result.success);
        assert!(result.content.contains("status=metadata_only"));
        assert!(result.content.contains("slash_command=review"));
        assert_eq!(result.metadata["execution_supported"], false);
        assert_eq!(result.metadata["skill"], "review");
    }

    #[test]
    fn skill_hides_plugin_backed_entries_when_plugins_feature_is_disabled() {
        let dir = unique_test_dir("tools-skill-plugin-gate");
        let plugin_dir = dir.join(".wonder/plugins/demo");
        let skill_dir = plugin_dir.join("skills/plugin-skill");
        fs::create_dir_all(&skill_dir).expect("skill dir");
        fs::write(
            plugin_dir.join("plugin.json"),
            json!({
                "schema_version": 1,
                "name": "Demo Plugin",
                "version": "1.0.0",
                "description": "Demo plugin",
                "skills": [{ "path": "skills/plugin-skill" }]
            })
            .to_string(),
        )
        .expect("plugin manifest");
        fs::write(
            skill_dir.join("skill.json"),
            json!({
                "schema_version": 1,
                "name": "plugin-skill",
                "description": "Plugin-provided skill",
                "prompt": "Inspect plugin state."
            })
            .to_string(),
        )
        .expect("skill manifest");

        let mut context = tool_context(dir);
        context.features.disable(FeatureFlag::Plugins);

        let tool = SkillTool;
        let result = block_on(tool.execute(
            context,
            ToolUseId::new(),
            json!({ "skill": "plugin-skill" }),
        ))
        .expect("execute");

        assert!(!result.success);
        assert_eq!(result.metadata["reason"], "unknown_skill");
    }

    #[test]
    fn tool_search_supports_select_and_keyword_queries() {
        let tool = ToolSearchTool;
        let context = tool_context(unique_test_dir("tools-tool-search"));

        let select = block_on(tool.execute(
            context.clone(),
            ToolUseId::new(),
            json!({ "query": "select:LSP,Config" }),
        ))
        .expect("select");
        assert!(select.success);
        assert!(select.content.contains("\"name\": \"lsp\""));
        assert!(select.content.contains("\"name\": \"config\""));

        let keyword = block_on(tool.execute(
            context,
            ToolUseId::new(),
            json!({ "query": "search the web", "max_results": 2 }),
        ))
        .expect("keyword");
        assert!(keyword.success);
        assert!(keyword.content.contains("\"name\": \"web_search\""));
    }

    #[test]
    fn feature_queries_hide_new_tools_when_required_flags_are_missing() {
        let registry = builtin_registry().expect("registry");
        let tools_only = FeatureSet::from_iter([FeatureFlag::Tools]);

        assert!(
            registry
                .resolve_enabled("config", &ToolQuery::new(tools_only))
                .is_some()
        );
        assert!(
            registry
                .resolve_enabled("skill", &ToolQuery::new(FeatureSet::empty()))
                .is_none()
        );
        assert!(
            registry
                .resolve_enabled("tool_search", &ToolQuery::new(FeatureSet::empty()))
                .is_none()
        );
    }
}
