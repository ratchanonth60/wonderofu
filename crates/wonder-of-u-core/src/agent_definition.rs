//! Upstream-parity agent definition types and in-memory catalog.
//!
//! An [`AgentDefinition`] is the canonical, source-agnostic representation of a
//! sub-agent configuration.  Definitions are collected into an [`AgentCatalog`]
//! which applies precedence rules when definitions from multiple sources share
//! the same id.
//!
//! # Sources and precedence
//!
//! Definitions are loaded from three tiers, in ascending priority order:
//!
//! 1. **Built-in** — compiled into the binary; always available.
//! 2. **Project `.claude/agents/`** — overrides built-ins for the current project.
//! 3. **Project `agents/`** — highest project-level precedence.
//!
//! Within the same source tier the first definition loaded (alphabetically
//! by filename) wins when ids collide.  This makes precedence fully deterministic.
//!
//! # Parsing
//!
//! Definitions are parsed from two formats:
//!
//! * **Markdown with YAML frontmatter** (`.md` files):
//!   ```text
//!   ---
//!   name: my-agent
//!   description: Does something useful
//!   ---
//!
//!   You are a specialized agent…
//!   ```
//! * **JSON** (`.json` files): a flat object with the same field names.
//!
//! Both formats share [`RawDefinitionFields`] for (de)serialisation.
//!
//! # Security
//!
//! `hooks` and `mcpServers` fields are accepted in the raw parse but are
//! **stripped and noted** — never executed.  File-size and file-count caps
//! live in [`crate::agent_loader`]; this module is pure in-memory logic.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{Result, WonderError, fingerprint_str, fleet_roles::FleetRoleCatalog};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum byte length for an agent definition id.
pub const MAX_AGENT_ID_LEN: usize = 64;

/// Maximum byte length for a system prompt.
pub const MAX_SYSTEM_PROMPT_LEN: usize = 64 * 1024;

// ── Source enum ───────────────────────────────────────────────────────────────

/// Where an [`AgentDefinition`] originated.
///
/// The ordering reflects precedence: a higher discriminant wins when two
/// definitions share the same id.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDefinitionSource {
    /// Compiled into the binary; always available.
    Builtin,
    /// Loaded from the project's `.claude/agents/` directory.
    ProjectDotClaude,
    /// Loaded from the project's `agents/` directory.
    Project,
}

impl fmt::Display for AgentDefinitionSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builtin => f.write_str("builtin"),
            Self::ProjectDotClaude => f.write_str("project .claude/agents"),
            Self::Project => f.write_str("project agents"),
        }
    }
}

// ── Snapshot ──────────────────────────────────────────────────────────────────

/// Immutable snapshot of the effective fields from an [`AgentDefinition`],
/// captured at queue time and stored on [`crate::FleetMemberRequest`].
///
/// Because custom definitions can be edited or deleted after they are queued,
/// snapshotting ensures the dispatcher always has the original configuration
/// available even if the source file changes.
///
/// # Back-compatibility
///
/// All fields use `#[serde(default, skip_serializing_if)]` so older queue
/// files that pre-date this field are silently up-cast.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentDefinitionSnapshot {
    /// Canonical kebab-case id of the resolved definition.
    pub definition_id: String,
    /// Source tier where the definition was resolved from.
    pub source: Option<AgentDefinitionSource>,
    /// SHA-256 hex digest of the raw file content (absent for built-ins).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// System prompt / prompt preamble captured at queue time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Model override captured at queue time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Allowed tool names captured at queue time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    /// Disallowed tool names captured at queue time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disallowed_tools: Vec<String>,
    /// Permission mode string captured at queue time (e.g. `"default"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// Maximum conversation turns captured at queue time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
}

// ── Raw frontmatter / JSON DTO ────────────────────────────────────────────────

/// Flat serde DTO shared by markdown-frontmatter and JSON definitions.
///
/// All fields are optional at the parse layer; semantic validation is applied
/// when building an [`AgentDefinition`].
///
/// The `hooks` and `mcpServers` fields are explicitly typed as
/// `Option<serde_json::Value>` so they are accepted during deserialization
/// (for round-trip fidelity) but **never used** — they are rejected during
/// validation and stripped from the resulting definition.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RawDefinitionFields {
    /// Stable kebab-case id.  Derived from `name` if absent.
    #[serde(default)]
    pub id: Option<String>,
    /// Human-readable display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Agent type label (upstream alias for `name`).
    #[serde(rename = "agentType", default)]
    pub agent_type: Option<String>,
    /// Short description / `whenToUse` hint.
    #[serde(default)]
    pub description: Option<String>,
    /// Upstream `whenToUse` alias for `description`.
    #[serde(rename = "whenToUse", default)]
    pub when_to_use: Option<String>,
    /// System prompt / initial prompt (body text for `.md` files).
    #[serde(default)]
    pub prompt: Option<String>,
    /// Upstream alias for `prompt`.
    #[serde(rename = "systemPrompt", default)]
    pub system_prompt: Option<String>,
    /// Allowed tool names.
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    /// Disallowed tool names.
    #[serde(rename = "disallowedTools", default)]
    pub disallowed_tools: Option<Vec<String>>,
    /// Model override string.
    #[serde(default)]
    pub model: Option<String>,
    /// Permission mode (stored but not executed; passed to snapshot).
    #[serde(rename = "permissionMode", default)]
    pub permission_mode: Option<String>,
    /// Max conversation turns.
    #[serde(rename = "maxTurns", default)]
    pub max_turns: Option<u32>,
    /// UI color hint (stored for completeness, not executed).
    #[serde(default)]
    pub color: Option<String>,
    // Dangerous fields: accepted during parse, rejected during build.
    /// Hooks configuration — accepted but never executed.
    #[serde(default)]
    pub hooks: Option<serde_json::Value>,
    /// MCP server definitions — accepted but never executed.
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: Option<serde_json::Value>,
}

// ── AgentDefinition ───────────────────────────────────────────────────────────

/// The canonical in-memory representation of a sub-agent configuration.
///
/// Instances are created via [`AgentDefinition::from_raw`] (for parsed
/// definitions) or [`AgentDefinition::from_fleet_role`] (for compatibility
/// built-ins synthesised from [`crate::fleet_roles::FleetAgentRole`]).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentDefinition {
    /// Stable kebab-case identifier, e.g. `"rust-engineer"`.
    pub id: String,
    /// Human-readable display name, e.g. `"Rust Engineer"`.
    pub name: String,
    /// Short description shown in UIs and error messages.
    pub description: String,
    /// System prompt prepended to every task prompt at launch.
    pub system_prompt: String,
    /// Allowed tool names (`[]` = all tools allowed).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// Explicitly disallowed tool names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disallowed_tools: Vec<String>,
    /// Optional preferred model override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Maximum conversation turns (`None` = runtime default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    /// Permission mode string (advisory; dispatched to snapshot).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// UI color hint (informational only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Where this definition originated.
    pub source: AgentDefinitionSource,
    /// SHA-256 hex digest of the raw file content (absent for built-ins).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

impl AgentDefinition {
    /// Builds an [`AgentDefinition`] from parsed raw fields.
    ///
    /// # Alias resolution
    ///
    /// * `id` falls back to a kebab-case slug derived from `name` / `agentType`.
    /// * `description` falls back to `whenToUse`.
    /// * `system_prompt` falls back to `systemPrompt`.
    ///
    /// # Dangerous field handling
    ///
    /// `hooks` and `mcpServers` are stripped without error (they are silently
    /// ignored).  Callers that need to surface a warning should inspect
    /// `raw.hooks.is_some()` / `raw.mcp_servers.is_some()` before calling.
    ///
    /// # Errors
    ///
    /// Returns [`WonderError::Validation`] when required fields are missing or
    /// fail validation rules.
    pub fn from_raw(
        raw: RawDefinitionFields,
        source: AgentDefinitionSource,
        raw_content: &str,
    ) -> Result<Self> {
        // Resolve the display name — prefer explicit name over agentType.
        let name = raw
            .name
            .or(raw.agent_type)
            .filter(|n| !n.trim().is_empty())
            .ok_or_else(|| WonderError::validation("agent definition requires a `name` field"))?;

        // Derive a stable id from the explicit id or the display name.
        let id = raw
            .id
            .filter(|i| !i.trim().is_empty())
            .unwrap_or_else(|| name_to_id(&name));

        validate_agent_id(&id)?;

        let description = raw
            .description
            .or(raw.when_to_use)
            .filter(|d| !d.trim().is_empty())
            .ok_or_else(|| {
                WonderError::validation(format!(
                    "agent definition `{id}` requires a `description` field"
                ))
            })?;

        // For markdown files the body IS the system prompt; for JSON it can be
        // `prompt` or `systemPrompt`.
        let system_prompt = raw
            .prompt
            .or(raw.system_prompt)
            .filter(|p| !p.trim().is_empty())
            .ok_or_else(|| {
                WonderError::validation(format!(
                    "agent definition `{id}` requires a `prompt` / `systemPrompt` field"
                ))
            })?;

        if system_prompt.len() > MAX_SYSTEM_PROMPT_LEN {
            return Err(WonderError::validation(format!(
                "agent definition `{id}` system prompt exceeds maximum length ({} bytes)",
                MAX_SYSTEM_PROMPT_LEN
            )));
        }

        let content_hash = match source {
            AgentDefinitionSource::Builtin => None,
            _ => Some(fingerprint_str(raw_content)),
        };

        Ok(Self {
            id,
            name,
            description,
            system_prompt,
            tools: raw.tools.unwrap_or_default(),
            disallowed_tools: raw.disallowed_tools.unwrap_or_default(),
            model: raw.model,
            max_turns: raw.max_turns,
            permission_mode: raw.permission_mode,
            color: raw.color,
            source,
            content_hash,
        })
    }

    /// Synthesises an [`AgentDefinition`] from an existing [`FleetAgentRole`].
    ///
    /// Used to populate the builtin catalog without duplicating role data.
    /// The role's `prompt_preamble` becomes the `system_prompt`.
    #[must_use]
    pub fn from_fleet_role(role: &crate::fleet_roles::FleetAgentRole) -> Self {
        Self {
            id: role.id.clone(),
            name: role.name.clone(),
            description: role.description.clone(),
            system_prompt: role.prompt_preamble.clone(),
            tools: role.allowed_tools.clone(),
            disallowed_tools: Vec::new(),
            model: role.default_model.clone(),
            max_turns: None,
            permission_mode: None,
            color: None,
            source: AgentDefinitionSource::Builtin,
            content_hash: None,
        }
    }

    /// Returns an [`AgentDefinitionSnapshot`] capturing the effective fields.
    ///
    /// This is stored on [`crate::FleetMemberRequest`] at queue time so the
    /// dispatcher never needs to re-resolve the definition from disk.
    #[must_use]
    pub fn snapshot(&self) -> AgentDefinitionSnapshot {
        AgentDefinitionSnapshot {
            definition_id: self.id.clone(),
            source: Some(self.source),
            content_hash: self.content_hash.clone(),
            system_prompt: Some(self.system_prompt.clone()),
            model: self.model.clone(),
            allowed_tools: self.tools.clone(),
            disallowed_tools: self.disallowed_tools.clone(),
            permission_mode: self.permission_mode.clone(),
            max_turns: self.max_turns,
        }
    }
}

// ── Catalog ───────────────────────────────────────────────────────────────────

/// An ordered, precedence-aware collection of [`AgentDefinition`] values.
///
/// # Precedence
///
/// When [`AgentCatalog::insert`] is called for a definition whose id already
/// exists, the new definition **replaces** the existing one only if its source
/// has equal or higher precedence (see [`AgentDefinitionSource`] ordering).
/// Within the same source tier the first insertion wins.
///
/// [`AgentCatalog::builtin`] always produces the full set of built-in
/// definitions including all existing [`FleetRoleCatalog`] roles plus
/// `general-purpose` and `explore`.
#[derive(Clone, Debug, Default)]
pub struct AgentCatalog {
    // BTreeMap for deterministic iteration and fast id lookup.
    defs: std::collections::BTreeMap<String, AgentDefinition>,
}

impl AgentCatalog {
    /// Creates an empty catalog.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Returns the built-in catalog.
    ///
    /// Contains all nine [`FleetRoleCatalog`] roles plus `general-purpose` and
    /// `explore` for upstream parity.  The [`FleetRoleCatalog`] roles are
    /// synthesised via [`AgentDefinition::from_fleet_role`] to avoid drift.
    #[must_use]
    pub fn builtin() -> Self {
        let mut catalog = Self::empty();

        // Synthesise all existing fleet roles so they remain accessible without
        // duplicating their definitions.
        for role in FleetRoleCatalog::builtin().list() {
            catalog.insert(AgentDefinition::from_fleet_role(role));
        }

        // Upstream-parity built-ins not covered by fleet roles.
        for def in extra_builtin_definitions() {
            catalog.insert(def);
        }

        catalog
    }

    /// Inserts a definition into the catalog, respecting precedence rules.
    ///
    /// The insertion wins when:
    /// * The id does not exist yet, **or**
    /// * The new definition's source has **strictly higher** precedence than
    ///   the existing one.
    ///
    /// Within the same source tier the first insertion is preserved.
    pub fn insert(&mut self, def: AgentDefinition) {
        match self.defs.get(&def.id) {
            None => {
                self.defs.insert(def.id.clone(), def);
            }
            Some(existing) if def.source > existing.source => {
                self.defs.insert(def.id.clone(), def);
            }
            // Same or lower precedence: first insertion wins.
            _ => {}
        }
    }

    /// Looks up a definition by its exact kebab-case id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&AgentDefinition> {
        self.defs.get(id)
    }

    /// Returns all definitions in deterministic (alphabetical id) order.
    #[must_use]
    pub fn list(&self) -> Vec<&AgentDefinition> {
        self.defs.values().collect()
    }

    /// Returns the stable ids of all definitions in alphabetical order.
    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        self.defs.keys().map(String::as_str).collect()
    }

    /// Returns the number of definitions in the catalog.
    #[must_use]
    pub fn len(&self) -> usize {
        self.defs.len()
    }

    /// Returns `true` when the catalog contains no definitions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// Formats a comma-separated list of known definition ids for error messages.
    #[must_use]
    pub fn known_ids_display(&self) -> String {
        self.ids().join(", ")
    }

    /// Resolves a possibly-friendly label to a canonical definition.
    ///
    /// Accepts, in order:
    ///
    /// 1. Exact id match (`"rust-engineer"`).
    /// 2. Case-insensitive display-name match (`"Rust Engineer"`).
    /// 3. Well-known backward-compatible aliases (`"code-review"` → `"code-reviewer"`).
    ///
    /// Returns `None` when no definition matches.
    #[must_use]
    pub fn resolve_alias(&self, label: &str) -> Option<&AgentDefinition> {
        // 1. Exact id.
        if let Some(def) = self.defs.get(label) {
            return Some(def);
        }

        // 2. Case-insensitive name match.
        let lower = label.to_lowercase();
        for def in self.defs.values() {
            if def.name.to_lowercase() == lower {
                return Some(def);
            }
        }

        // 3. Backward-compatible aliases inherited from FleetRoleCatalog.
        let alias_id = Self::well_known_alias(label)?;
        self.defs.get(alias_id)
    }

    /// Maps a well-known label to a canonical id, or returns `None`.
    fn well_known_alias(label: &str) -> Option<&'static str> {
        match label {
            "code-review" | "CodeReview" | "code_review" => Some("code-reviewer"),
            "Rust Engineer" | "rust_engineer" => Some("rust-engineer"),
            "Rust Tester" | "rust_tester" => Some("rust-tester"),
            "Rust Architect" | "rust_architect" => Some("rust-architect"),
            "Rust Refactor" | "rust_refactor" => Some("rust-refactor"),
            "Rust Optimizer" | "rust_optimizer" => Some("rust-optimizer"),
            "Rust Documenter" | "rust_documenter" => Some("rust-documenter"),
            "TUI Designer" | "tui_designer" | "tui-design" => Some("tui-designer"),
            "Rubber Duck" | "rubber_duck" | "rubber-duck-debug" => Some("rubber-duck"),
            "general" | "General Purpose" | "GeneralPurpose" | "general_purpose" => {
                Some("general-purpose")
            }
            "Explore" | "Explorer" | "explore-agent" => Some("explore"),
            _ => None,
        }
    }
}

// ── Extra built-in definitions ────────────────────────────────────────────────

/// Returns upstream-parity built-in definitions not covered by fleet roles.
fn extra_builtin_definitions() -> Vec<AgentDefinition> {
    vec![
        AgentDefinition {
            id: "general-purpose".into(),
            name: "General Purpose".into(),
            description: "A general-purpose sub-agent for tasks that do not require a specific \
                          specialised role."
                .into(),
            system_prompt: "You are a capable, general-purpose AI assistant. Complete the assigned \
                            task accurately and concisely. When in doubt, prefer safety and \
                            correctness over speed."
                .into(),
            tools: Vec::new(), // unrestricted
            disallowed_tools: Vec::new(),
            model: None,
            max_turns: None,
            permission_mode: None,
            color: None,
            source: AgentDefinitionSource::Builtin,
            content_hash: None,
        },
        AgentDefinition {
            id: "explore".into(),
            name: "Explore".into(),
            description: "An exploration-focused sub-agent that researches codebases, APIs, and \
                          documentation without writing code."
                .into(),
            system_prompt: "You are an exploration specialist. Your job is to research, read, and \
                            summarise — not to write code or modify files. Investigate the \
                            requested topic thoroughly and produce a clear, structured report of \
                            your findings including relevant file paths, function names, and \
                            dependencies."
                .into(),
            tools: vec!["bash".into(), "file_read".into(), "glob".into(), "grep".into()],
            disallowed_tools: vec!["file_write".into(), "file_edit".into()],
            model: None,
            max_turns: None,
            permission_mode: Some("default".into()),
            color: None,
            source: AgentDefinitionSource::Builtin,
            content_hash: None,
        },
    ]
}

// ── Parsing helpers ───────────────────────────────────────────────────────────

/// Error type for definition parse failures.
#[derive(Debug, thiserror::Error)]
pub enum DefinitionParseError {
    /// The file content could not be decoded as UTF-8.
    #[error("file is not valid UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    /// The YAML frontmatter block failed to parse.
    #[error("YAML frontmatter parse error: {0}")]
    YamlFrontmatter(String),
    /// The JSON body failed to parse.
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    /// A required field is missing or invalid after parsing.
    #[error("definition validation error: {0}")]
    Validation(String),
}

impl From<DefinitionParseError> for WonderError {
    fn from(e: DefinitionParseError) -> Self {
        WonderError::validation(e.to_string())
    }
}

/// Parses a markdown file with YAML frontmatter into [`RawDefinitionFields`].
///
/// The frontmatter block is delimited by leading and trailing `---` lines.
/// The body text after the closing `---` becomes the `prompt` field (trimmed).
/// A body with only whitespace is treated as absent.
///
/// # Errors
///
/// Returns [`DefinitionParseError::YamlFrontmatter`] when the YAML block
/// fails to deserialise into [`RawDefinitionFields`].
pub fn parse_markdown_frontmatter(
    content: &str,
) -> std::result::Result<RawDefinitionFields, DefinitionParseError> {
    // Fast path: no frontmatter marker.
    if !content.trim_start().starts_with("---") {
        // Treat the whole file as a plain prompt with no metadata fields.
        let prompt = content.trim();
        return Ok(RawDefinitionFields {
            prompt: if prompt.is_empty() {
                None
            } else {
                Some(prompt.to_owned())
            },
            ..Default::default()
        });
    }

    // Find the opening --- (could have leading whitespace stripped already).
    let after_first = content.trim_start().strip_prefix("---").unwrap_or(content);
    // Find the closing ---.
    let Some(close_pos) = after_first.find("\n---") else {
        // No closing fence — treat whole file as prompt.
        let prompt = content.trim();
        return Ok(RawDefinitionFields {
            prompt: if prompt.is_empty() {
                None
            } else {
                Some(prompt.to_owned())
            },
            ..Default::default()
        });
    };

    let yaml_block = &after_first[..close_pos];
    let body_start = close_pos + "\n---".len();
    // Skip the optional newline immediately after the closing fence.
    let body = after_first[body_start..].trim_start_matches('\n').trim();

    let mut fields: RawDefinitionFields = serde_yaml::from_str(yaml_block).map_err(|e| {
        DefinitionParseError::YamlFrontmatter(e.to_string())
    })?;

    // Body text is the system prompt if no explicit prompt field was set.
    if fields.prompt.is_none() && fields.system_prompt.is_none() && !body.is_empty() {
        fields.prompt = Some(body.to_owned());
    }

    Ok(fields)
}

/// Parses a JSON byte slice into [`RawDefinitionFields`].
///
/// # Errors
///
/// Returns [`DefinitionParseError::Json`] on deserialization failure.
pub fn parse_json_definition(
    content: &str,
) -> std::result::Result<RawDefinitionFields, DefinitionParseError> {
    Ok(serde_json::from_str(content)?)
}

// ── Validation helpers ────────────────────────────────────────────────────────

/// Validates a kebab-case agent id: `[a-z0-9][a-z0-9-]*`, no leading/trailing/consecutive dashes.
fn validate_agent_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(WonderError::validation("agent definition id must not be empty"));
    }
    if id.len() > MAX_AGENT_ID_LEN {
        return Err(WonderError::validation(format!(
            "agent definition id `{id}` exceeds maximum length ({MAX_AGENT_ID_LEN} chars)"
        )));
    }

    let valid = id
        .chars()
        .enumerate()
        .all(|(i, c)| match c {
            'a'..='z' | '0'..='9' => true,
            '-' if i > 0 => true,
            _ => false,
        })
        && !id.ends_with('-')
        && !id.contains("--");

    if valid {
        Ok(())
    } else {
        Err(WonderError::validation(format!(
            "agent definition id `{id}` is invalid: must match [a-z0-9][a-z0-9-]* \
             with no leading, trailing, or consecutive dashes"
        )))
    }
}

/// Converts a display name to a kebab-case id slug.
///
/// `"Rust Engineer"` → `"rust-engineer"`.
fn name_to_id(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        // Collapse consecutive dashes.
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── builtin catalog ───────────────────────────────────────────────────────

    #[test]
    fn builtin_catalog_contains_all_fleet_role_ids() {
        let catalog = AgentCatalog::builtin();
        // All nine FleetRoleCatalog built-in ids must be present.
        for id in [
            "rust-engineer",
            "rust-tester",
            "rust-architect",
            "rust-refactor",
            "rust-optimizer",
            "rust-documenter",
            "tui-designer",
            "rubber-duck",
            "code-reviewer",
        ] {
            assert!(
                catalog.get(id).is_some(),
                "built-in catalog must contain `{id}`"
            );
        }
    }

    #[test]
    fn builtin_catalog_contains_upstream_parity_roles() {
        let catalog = AgentCatalog::builtin();
        assert!(
            catalog.get("general-purpose").is_some(),
            "built-in catalog must contain `general-purpose`"
        );
        assert!(
            catalog.get("explore").is_some(),
            "built-in catalog must contain `explore`"
        );
    }

    #[test]
    fn builtin_catalog_has_eleven_or_more_entries() {
        let catalog = AgentCatalog::builtin();
        assert!(
            catalog.len() >= 11,
            "expected at least 11 built-in definitions (9 fleet roles + general-purpose + explore)"
        );
    }

    // ── alias resolution ──────────────────────────────────────────────────────

    #[test]
    fn resolve_alias_exact_id() {
        let catalog = AgentCatalog::builtin();
        assert_eq!(
            catalog.resolve_alias("rust-engineer").map(|d| d.id.as_str()),
            Some("rust-engineer")
        );
    }

    #[test]
    fn resolve_alias_display_name_case_insensitive() {
        let catalog = AgentCatalog::builtin();
        assert_eq!(
            catalog.resolve_alias("Rust Engineer").map(|d| d.id.as_str()),
            Some("rust-engineer")
        );
        assert_eq!(
            catalog.resolve_alias("RUST ENGINEER").map(|d| d.id.as_str()),
            Some("rust-engineer")
        );
    }

    #[test]
    fn resolve_alias_well_known_short_alias() {
        let catalog = AgentCatalog::builtin();
        assert_eq!(
            catalog.resolve_alias("code-review").map(|d| d.id.as_str()),
            Some("code-reviewer")
        );
        assert_eq!(
            catalog.resolve_alias("general").map(|d| d.id.as_str()),
            Some("general-purpose")
        );
        assert_eq!(
            catalog.resolve_alias("Explore").map(|d| d.id.as_str()),
            Some("explore")
        );
    }

    #[test]
    fn resolve_alias_unknown_returns_none() {
        let catalog = AgentCatalog::builtin();
        assert!(catalog.resolve_alias("completely-unknown-agent").is_none());
    }

    // ── precedence ────────────────────────────────────────────────────────────

    #[test]
    fn project_source_overrides_builtin() {
        let mut catalog = AgentCatalog::builtin();
        let custom = AgentDefinition {
            id: "rust-engineer".into(),
            name: "Custom Rust Engineer".into(),
            description: "Project-level override.".into(),
            system_prompt: "Custom prompt.".into(),
            tools: vec![],
            disallowed_tools: vec![],
            model: None,
            max_turns: None,
            permission_mode: None,
            color: None,
            source: AgentDefinitionSource::Project,
            content_hash: Some("abc123".into()),
        };
        catalog.insert(custom);
        assert_eq!(
            catalog.get("rust-engineer").map(|d| d.name.as_str()),
            Some("Custom Rust Engineer")
        );
    }

    #[test]
    fn builtin_does_not_override_project_source() {
        let mut catalog = AgentCatalog::empty();
        // Insert project-level definition first.
        catalog.insert(AgentDefinition {
            id: "my-agent".into(),
            name: "My Agent".into(),
            description: "Project custom.".into(),
            system_prompt: "Project prompt.".into(),
            tools: vec![],
            disallowed_tools: vec![],
            model: None,
            max_turns: None,
            permission_mode: None,
            color: None,
            source: AgentDefinitionSource::Project,
            content_hash: None,
        });
        // Attempt to insert a builtin with the same id.
        catalog.insert(AgentDefinition {
            id: "my-agent".into(),
            name: "Overwritten".into(),
            description: "Should not win.".into(),
            system_prompt: "Should not win.".into(),
            tools: vec![],
            disallowed_tools: vec![],
            model: None,
            max_turns: None,
            permission_mode: None,
            color: None,
            source: AgentDefinitionSource::Builtin,
            content_hash: None,
        });
        // Project source wins; builtin insertion should have been ignored.
        assert_eq!(
            catalog.get("my-agent").map(|d| d.name.as_str()),
            Some("My Agent")
        );
    }

    #[test]
    fn same_source_first_insertion_wins() {
        let mut catalog = AgentCatalog::empty();
        catalog.insert(AgentDefinition {
            id: "dup".into(),
            name: "First".into(),
            description: "First.".into(),
            system_prompt: "First.".into(),
            tools: vec![],
            disallowed_tools: vec![],
            model: None,
            max_turns: None,
            permission_mode: None,
            color: None,
            source: AgentDefinitionSource::ProjectDotClaude,
            content_hash: None,
        });
        catalog.insert(AgentDefinition {
            id: "dup".into(),
            name: "Second".into(),
            description: "Second.".into(),
            system_prompt: "Second.".into(),
            tools: vec![],
            disallowed_tools: vec![],
            model: None,
            max_turns: None,
            permission_mode: None,
            color: None,
            source: AgentDefinitionSource::ProjectDotClaude,
            content_hash: None,
        });
        assert_eq!(catalog.get("dup").map(|d| d.name.as_str()), Some("First"));
    }

    // ── markdown parsing ──────────────────────────────────────────────────────

    #[test]
    fn parse_markdown_with_yaml_frontmatter() {
        let md = "---\nname: My Agent\ndescription: Does stuff\n---\n\nYou are a helper.";
        let fields = parse_markdown_frontmatter(md).expect("parse");
        assert_eq!(fields.name.as_deref(), Some("My Agent"));
        assert_eq!(fields.description.as_deref(), Some("Does stuff"));
        assert_eq!(fields.prompt.as_deref(), Some("You are a helper."));
    }

    #[test]
    fn parse_markdown_body_becomes_prompt() {
        let md = "---\nname: Scout\ndescription: Explores\n---\n\nExplore the repo.";
        let fields = parse_markdown_frontmatter(md).expect("parse");
        assert_eq!(fields.prompt.as_deref(), Some("Explore the repo."));
    }

    #[test]
    fn parse_markdown_without_frontmatter_treats_body_as_prompt() {
        let md = "You are a simple agent.";
        let fields = parse_markdown_frontmatter(md).expect("parse");
        assert_eq!(fields.prompt.as_deref(), Some("You are a simple agent."));
        assert!(fields.name.is_none());
    }

    #[test]
    fn parse_markdown_aliases_when_to_use_and_agent_type() {
        let md = "---\nagentType: Scout\nwhenToUse: When exploring\n---\n\nExplore.";
        let fields = parse_markdown_frontmatter(md).expect("parse");
        assert_eq!(fields.agent_type.as_deref(), Some("Scout"));
        assert_eq!(fields.when_to_use.as_deref(), Some("When exploring"));
    }

    #[test]
    fn parse_markdown_with_tools_list() {
        let md = "---\nname: Bash Expert\ndescription: Runs scripts\ntools:\n  - bash\n  - file_read\n---\n\nRun things.";
        let fields = parse_markdown_frontmatter(md).expect("parse");
        assert_eq!(
            fields.tools.as_deref(),
            Some(&["bash".to_owned(), "file_read".to_owned()][..])
        );
    }

    #[test]
    fn parse_markdown_invalid_yaml_returns_error() {
        let md = "---\nname: [invalid yaml\n---\n\nBody.";
        assert!(parse_markdown_frontmatter(md).is_err());
    }

    // ── JSON parsing ──────────────────────────────────────────────────────────

    #[test]
    fn parse_json_definition_basic() {
        let json =
            r#"{"name":"My Agent","description":"Does stuff","prompt":"You are a helper."}"#;
        let fields = parse_json_definition(json).expect("parse");
        assert_eq!(fields.name.as_deref(), Some("My Agent"));
        assert_eq!(fields.description.as_deref(), Some("Does stuff"));
        assert_eq!(fields.prompt.as_deref(), Some("You are a helper."));
    }

    #[test]
    fn parse_json_accepts_system_prompt_alias() {
        let json = r#"{"name":"X","description":"Y","systemPrompt":"The prompt."}"#;
        let fields = parse_json_definition(json).expect("parse");
        assert_eq!(fields.system_prompt.as_deref(), Some("The prompt."));
    }

    #[test]
    fn parse_json_accepts_dangerous_fields_without_error() {
        // hooks and mcpServers must not cause a parse error; they are stripped later.
        let json = r#"{"name":"X","description":"Y","prompt":"P","hooks":{"postStart":"echo hi"},"mcpServers":{}}"#;
        let fields = parse_json_definition(json).expect("parse");
        assert!(fields.hooks.is_some(), "hooks accepted during parse");
        assert!(fields.mcp_servers.is_some(), "mcpServers accepted during parse");
    }

    // ── AgentDefinition::from_raw ─────────────────────────────────────────────

    #[test]
    fn from_raw_requires_name() {
        let raw = RawDefinitionFields {
            description: Some("desc".into()),
            prompt: Some("prompt".into()),
            ..Default::default()
        };
        assert!(AgentDefinition::from_raw(raw, AgentDefinitionSource::Project, "").is_err());
    }

    #[test]
    fn from_raw_derives_id_from_name() {
        let raw = RawDefinitionFields {
            name: Some("My Custom Agent".into()),
            description: Some("desc".into()),
            prompt: Some("prompt".into()),
            ..Default::default()
        };
        let def =
            AgentDefinition::from_raw(raw, AgentDefinitionSource::Project, "content").unwrap();
        assert_eq!(def.id, "my-custom-agent");
    }

    #[test]
    fn from_raw_uses_explicit_id() {
        let raw = RawDefinitionFields {
            id: Some("custom-id".into()),
            name: Some("My Agent".into()),
            description: Some("desc".into()),
            prompt: Some("prompt".into()),
            ..Default::default()
        };
        let def =
            AgentDefinition::from_raw(raw, AgentDefinitionSource::Project, "content").unwrap();
        assert_eq!(def.id, "custom-id");
    }

    #[test]
    fn from_raw_content_hash_absent_for_builtins() {
        let raw = RawDefinitionFields {
            name: Some("My Agent".into()),
            description: Some("desc".into()),
            prompt: Some("prompt".into()),
            ..Default::default()
        };
        let def =
            AgentDefinition::from_raw(raw, AgentDefinitionSource::Builtin, "content").unwrap();
        assert!(def.content_hash.is_none());
    }

    #[test]
    fn from_raw_content_hash_present_for_project() {
        let raw = RawDefinitionFields {
            name: Some("My Agent".into()),
            description: Some("desc".into()),
            prompt: Some("prompt".into()),
            ..Default::default()
        };
        let def =
            AgentDefinition::from_raw(raw, AgentDefinitionSource::Project, "raw content").unwrap();
        assert!(def.content_hash.is_some());
    }

    #[test]
    fn snapshot_captures_effective_fields() {
        let def = AgentDefinition {
            id: "test".into(),
            name: "Test".into(),
            description: "desc".into(),
            system_prompt: "prompt".into(),
            tools: vec!["bash".into()],
            disallowed_tools: vec!["file_write".into()],
            model: Some("claude-opus-4-5".into()),
            max_turns: Some(10),
            permission_mode: Some("default".into()),
            color: None,
            source: AgentDefinitionSource::Project,
            content_hash: Some("deadbeef".into()),
        };
        let snap = def.snapshot();
        assert_eq!(snap.definition_id, "test");
        assert_eq!(snap.source, Some(AgentDefinitionSource::Project));
        assert_eq!(snap.content_hash.as_deref(), Some("deadbeef"));
        assert_eq!(snap.model.as_deref(), Some("claude-opus-4-5"));
        assert_eq!(snap.max_turns, Some(10));
        assert_eq!(snap.allowed_tools, vec!["bash"]);
        assert_eq!(snap.disallowed_tools, vec!["file_write"]);
    }

    // ── name_to_id ────────────────────────────────────────────────────────────

    #[test]
    fn name_to_id_converts_spaces_to_dashes() {
        assert_eq!(name_to_id("My Custom Agent"), "my-custom-agent");
    }

    #[test]
    fn name_to_id_collapses_consecutive_separators() {
        assert_eq!(name_to_id("My  Agent"), "my-agent");
        assert_eq!(name_to_id("My--Agent"), "my-agent");
    }

    // ── validate_agent_id ─────────────────────────────────────────────────────

    #[test]
    fn validate_agent_id_accepts_valid_ids() {
        assert!(validate_agent_id("rust-engineer").is_ok());
        assert!(validate_agent_id("general-purpose").is_ok());
        assert!(validate_agent_id("a").is_ok());
        assert!(validate_agent_id("r2d2").is_ok());
    }

    #[test]
    fn validate_agent_id_rejects_invalid_ids() {
        assert!(validate_agent_id("").is_err());
        assert!(validate_agent_id("Rust-Engineer").is_err()); // uppercase
        assert!(validate_agent_id("-leading").is_err());
        assert!(validate_agent_id("trailing-").is_err());
        assert!(validate_agent_id("double--dash").is_err());
        assert!(validate_agent_id("has space").is_err());
    }
}
