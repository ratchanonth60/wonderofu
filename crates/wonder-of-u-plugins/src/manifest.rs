use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use wonder_of_u_core::{CommandKind, CommandSource, CommandSpec, FeatureFlag, Result, WonderError};

/// Schema version for plugin manifest
pub const PLUGIN_MANIFEST_SCHEMA_VERSION: u16 = 1;

fn default_schema_version() -> u16 {
    PLUGIN_MANIFEST_SCHEMA_VERSION
}

const fn default_command_kind() -> CommandKind {
    CommandKind::Local
}
/// Represents plugin manifest
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Stores the schema version
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    /// Stores the name
    pub name: String,
    /// Stores the version
    pub version: String,
    /// Stores the description
    pub description: String,
    /// Stores the commands
    #[serde(default)]
    pub commands: Vec<PluginCommandDefinition>,
    /// Stores the skills
    #[serde(default)]
    pub skills: Vec<PluginSkillDefinition>,
    /// Sidebar content sections injected by this plugin.
    #[serde(default)]
    pub sidebar_sections: Vec<PluginSidebarSection>,
}

/// A sidebar section registered by a plugin manifest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PluginSidebarSection {
    /// Section title, rendered as `"─ {title} ─"`.
    pub title: String,
    /// Body lines shown under the header.
    #[serde(default)]
    pub lines: Vec<String>,
}

impl PluginManifest {
    /// Reads from path
    pub fn read_from_path(path: &Path) -> Result<Self> {
        let manifest: Self = serde_json::from_str(&fs::read_to_string(path)?)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validates the value
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != PLUGIN_MANIFEST_SCHEMA_VERSION {
            return Err(WonderError::validation(format!(
                "unsupported plugin manifest schema version: {}",
                self.schema_version
            )));
        }
        if self.name.trim().is_empty() {
            return Err(WonderError::validation("plugin name cannot be empty"));
        }
        if self.version.trim().is_empty() {
            return Err(WonderError::validation(format!(
                "plugin `{}` version cannot be empty",
                self.name
            )));
        }
        if self.description.trim().is_empty() {
            return Err(WonderError::validation(format!(
                "plugin `{}` description cannot be empty",
                self.name
            )));
        }

        let _ = self.plugin_id()?;

        let mut command_names = BTreeSet::new();
        for command in &self.commands {
            command.validate(&self.name)?;
            let spec = command.command_spec();
            let mut names = Vec::with_capacity(1 + spec.aliases.len());
            names.push(normalize_registry_name(&spec.name, "command")?);
            names.extend(
                spec.aliases
                    .iter()
                    .map(|alias| normalize_registry_name(alias, "command"))
                    .collect::<Result<Vec<_>>>()?,
            );
            for name in names {
                if !command_names.insert(name.clone()) {
                    return Err(WonderError::validation(format!(
                        "plugin `{}` declares duplicate command or alias: {name}",
                        self.name
                    )));
                }
            }
        }

        let mut skill_paths = BTreeSet::new();
        for skill in &self.skills {
            skill.validate(&self.name)?;
            if !skill_paths.insert(skill.path.clone()) {
                return Err(WonderError::validation(format!(
                    "plugin `{}` declares duplicate skill path: {}",
                    self.name,
                    skill.path.display()
                )));
            }
        }

        Ok(())
    }

    /// Handles plugin id
    pub fn plugin_id(&self) -> Result<String> {
        normalize_plugin_id(&self.name)
    }

    /// Handles command registrations
    pub fn command_registrations(
        &self,
        plugin_id: &str,
        root_dir: &Path,
    ) -> Result<Vec<PluginCommandRegistration>> {
        self.commands
            .iter()
            .map(|command| command.registration(plugin_id, root_dir))
            .collect()
    }

    /// Handles skill sources
    pub fn skill_sources(
        &self,
        plugin_id: &str,
        root_dir: &Path,
    ) -> Result<Vec<PluginSkillSource>> {
        self.skills
            .iter()
            .map(|skill| skill.source(plugin_id, root_dir))
            .collect()
    }
}
/// Represents plugin command definition
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PluginCommandDefinition {
    /// Stores the name
    pub name: String,
    /// Stores the aliases
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Stores the description
    pub description: String,
    /// Stores the kind
    #[serde(default = "default_command_kind")]
    pub kind: CommandKind,
    /// Stores the path
    pub path: PathBuf,
    /// Stores the allowed tools
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Stores the hidden
    #[serde(default)]
    pub hidden: bool,
    /// Stores the requires auth
    #[serde(default)]
    pub requires_auth: bool,
    /// Stores the interactive only
    #[serde(default)]
    pub interactive_only: bool,
}

impl PluginCommandDefinition {
    fn validate(&self, plugin_name: &str) -> Result<()> {
        if self.description.trim().is_empty() {
            return Err(WonderError::validation(format!(
                "plugin `{plugin_name}` command `{}` description cannot be empty",
                self.name
            )));
        }
        let mut allowed_tools = BTreeSet::new();
        for tool in &self.allowed_tools {
            let normalized = normalize_registry_name(tool, "tool")?;
            if !allowed_tools.insert(normalized.clone()) {
                return Err(WonderError::validation(format!(
                    "plugin `{plugin_name}` command `{}` declares duplicate allowed tool: {normalized}",
                    self.name
                )));
            }
        }
        validate_relative_path(
            &self.path,
            &format!("plugin `{plugin_name}` command `{}` path", self.name),
        )?;
        self.command_spec().validate()
    }
    /// Handles command spec
    #[must_use]
    pub fn command_spec(&self) -> CommandSpec {
        let mut spec = CommandSpec::new(&self.name, &self.description, self.kind);
        spec.aliases = self.aliases.clone();
        spec.source = CommandSource::Plugin;
        spec.required_features.insert(FeatureFlag::Plugins);
        spec.hidden = self.hidden;
        spec.requires_auth = self.requires_auth;
        spec.interactive_only = self.interactive_only;
        spec
    }

    fn registration(&self, plugin_id: &str, root_dir: &Path) -> Result<PluginCommandRegistration> {
        let entry_path = resolve_path(root_dir, &self.path, true, "plugin command entry")?;
        Ok(PluginCommandRegistration {
            plugin_id: plugin_id.to_string(),
            entry_path,
            allowed_tools: self
                .allowed_tools
                .iter()
                .map(|tool| normalize_registry_name(tool, "tool"))
                .collect::<Result<BTreeSet<_>>>()?,
            spec: self.command_spec(),
        })
    }
}
/// Represents plugin skill definition
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PluginSkillDefinition {
    /// Stores the path
    pub path: PathBuf,
}

impl PluginSkillDefinition {
    fn validate(&self, plugin_name: &str) -> Result<()> {
        validate_relative_path(&self.path, &format!("plugin `{plugin_name}` skill path"))
    }

    fn source(&self, plugin_id: &str, root_dir: &Path) -> Result<PluginSkillSource> {
        let path = resolve_path(root_dir, &self.path, false, "plugin skill path")?;
        Ok(PluginSkillSource {
            plugin_id: plugin_id.to_string(),
            path,
        })
    }
}
/// Represents plugin command registration
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginCommandRegistration {
    /// Stores the plugin identifier
    pub plugin_id: String,
    /// Stores the entry path
    pub entry_path: PathBuf,
    /// Stores the allowed tools
    pub allowed_tools: BTreeSet<String>,
    /// Stores the spec
    pub spec: CommandSpec,
}

impl PluginCommandRegistration {
    /// Returns whether this plugin command explicitly allows a tool name.
    #[must_use]
    pub fn allows_tool(&self, tool_name: &str) -> bool {
        normalize_registry_name(tool_name, "tool")
            .map(|normalized| self.allowed_tools.contains(&normalized))
            .unwrap_or(false)
    }
}
/// Represents plugin skill source
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginSkillSource {
    /// Stores the plugin identifier
    pub plugin_id: String,
    /// Stores the path
    pub path: PathBuf,
}

/// Normalizes plugin id
pub fn normalize_plugin_id(name: &str) -> Result<String> {
    let mut normalized = String::new();
    let mut last_dash = false;

    for ch in name.trim().chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if ch == '-' || ch == '_' || ch.is_whitespace() {
            Some('-')
        } else {
            None
        };

        let Some(mapped) = mapped else {
            continue;
        };

        if mapped == '-' {
            if !last_dash && !normalized.is_empty() {
                normalized.push('-');
            }
            last_dash = true;
        } else {
            normalized.push(mapped);
            last_dash = false;
        }
    }

    let normalized = normalized.trim_matches('-').to_string();
    if normalized.is_empty() {
        return Err(WonderError::validation(format!(
            "plugin name cannot be normalized into a stable id: {name}"
        )));
    }
    Ok(normalized)
}

fn normalize_registry_name(name: &str, resource: &str) -> Result<String> {
    let normalized = name.trim().trim_start_matches('/').to_ascii_lowercase();
    if normalized.is_empty() {
        return Err(WonderError::validation(format!(
            "{resource} name cannot be empty"
        )));
    }
    if normalized.chars().any(char::is_whitespace) {
        return Err(WonderError::validation(format!(
            "{resource} name contains whitespace: {name}"
        )));
    }
    Ok(normalized)
}

fn validate_relative_path(path: &Path, label: &str) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Err(WonderError::validation(format!("{label} cannot be empty")));
    }
    if path.is_absolute() {
        return Err(WonderError::validation(format!(
            "{label} must be relative: {}",
            path.display()
        )));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    }) {
        return Err(WonderError::validation(format!(
            "{label} cannot escape the plugin root: {}",
            path.display()
        )));
    }
    Ok(())
}

fn resolve_path(
    root_dir: &Path,
    relative: &Path,
    expect_file: bool,
    label: &str,
) -> Result<PathBuf> {
    validate_relative_path(relative, label)?;
    let resolved = root_dir.join(relative);
    if !resolved.exists() {
        return Err(WonderError::validation(format!(
            "{label} does not exist: {}",
            resolved.display()
        )));
    }
    if expect_file && !resolved.is_file() {
        return Err(WonderError::validation(format!(
            "{label} must point to a file: {}",
            resolved.display()
        )));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use wonder_of_u_core::CommandSource;
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    #[test]
    fn manifest_validation_rejects_duplicate_command_aliases() {
        let manifest: PluginManifest = serde_json::from_value(json!({
            "schema_version": 1,
            "name": "demo plugin",
            "version": "0.1.0",
            "description": "demo",
            "commands": [
                {
                    "name": "demo",
                    "aliases": ["run", "run"],
                    "description": "Demo command",
                    "path": "commands/demo.txt"
                }
            ]
        }))
        .expect("manifest");

        let error = manifest
            .validate()
            .expect_err("duplicate alias should fail");
        assert!(error.to_string().contains("duplicate command alias"));
    }

    #[test]
    fn manifest_validation_rejects_escaping_skill_paths() {
        let manifest: PluginManifest = serde_json::from_value(json!({
            "schema_version": 1,
            "name": "demo",
            "version": "0.1.0",
            "description": "demo",
            "skills": [{ "path": "../outside" }]
        }))
        .expect("manifest");

        let error = manifest.validate().expect_err("escaping path");
        assert!(error.to_string().contains("cannot escape the plugin root"));
    }

    #[test]
    fn manifest_builds_registration_metadata() {
        let dir = unique_test_dir("plugin-manifest");
        fs::create_dir_all(dir.join("commands")).expect("commands dir");
        fs::create_dir_all(dir.join("skills/demo")).expect("skills dir");
        fs::write(dir.join("commands/demo.txt"), "echo demo").expect("command file");

        let manifest = PluginManifest {
            schema_version: 1,
            name: "Demo Plugin".into(),
            version: "0.1.0".into(),
            description: "demo".into(),
            commands: vec![PluginCommandDefinition {
                name: "demo".into(),
                aliases: vec!["run-demo".into()],
                description: "Run demo".into(),
                kind: CommandKind::Local,
                path: PathBuf::from("commands/demo.txt"),
                allowed_tools: Vec::new(),
                hidden: false,
                requires_auth: false,
                interactive_only: false,
            }],
            skills: vec![PluginSkillDefinition {
                path: PathBuf::from("skills/demo"),
            }],
            sidebar_sections: Vec::new(),
        };

        let commands = manifest
            .command_registrations(&manifest.plugin_id().expect("plugin id"), &dir)
            .expect("commands");
        let skills = manifest
            .skill_sources(&manifest.plugin_id().expect("plugin id"), &dir)
            .expect("skills");

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].spec.source, CommandSource::Plugin);
        assert_eq!(skills.len(), 1);
        assert!(skills[0].path.ends_with("skills/demo"));
    }

    #[test]
    fn plugin_commands_are_permission_scoped() {
        let dir = unique_test_dir("plugin-manifest-allowed-tools");
        fs::create_dir_all(dir.join("commands")).expect("commands dir");
        fs::write(dir.join("commands/demo.txt"), "echo demo").expect("command file");

        let manifest = PluginManifest {
            schema_version: 1,
            name: "Scoped Plugin".into(),
            version: "0.1.0".into(),
            description: "demo".into(),
            commands: vec![PluginCommandDefinition {
                name: "demo".into(),
                aliases: Vec::new(),
                description: "Run demo".into(),
                kind: CommandKind::Local,
                path: PathBuf::from("commands/demo.txt"),
                allowed_tools: vec!["file_read".into()],
                hidden: false,
                requires_auth: false,
                interactive_only: false,
            }],
            skills: Vec::new(),
            sidebar_sections: Vec::new(),
        };

        let registration = manifest
            .command_registrations(&manifest.plugin_id().expect("plugin id"), &dir)
            .expect("commands")
            .pop()
            .expect("registration");

        assert!(registration.allows_tool("file_read"));
        assert!(registration.allows_tool("/FILE_READ"));
        assert!(!registration.allows_tool("bash"));
    }
}
