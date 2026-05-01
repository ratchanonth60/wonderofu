use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use wonder_of_u_core::{
    CommandKind, CommandSource, CommandSpec, FeatureFlag, Result, ToolKind, ToolSource, ToolSpec,
    WonderError,
};

pub const SKILL_MANIFEST_SCHEMA_VERSION: u16 = 1;

fn default_schema_version() -> u16 {
    SKILL_MANIFEST_SCHEMA_VERSION
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SkillManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slash_command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slash_aliases: Vec<String>,
}

impl SkillManifest {
    pub fn read_from_path(path: &Path) -> Result<Self> {
        let manifest: Self = serde_json::from_str(&fs::read_to_string(path)?)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SKILL_MANIFEST_SCHEMA_VERSION {
            return Err(WonderError::validation(format!(
                "unsupported skill manifest schema version: {}",
                self.schema_version
            )));
        }
        if self.name.trim().is_empty() {
            return Err(WonderError::validation("skill name cannot be empty"));
        }
        if self.description.trim().is_empty() {
            return Err(WonderError::validation(format!(
                "skill `{}` description cannot be empty",
                self.name
            )));
        }
        if let Some(prompt) = &self.prompt {
            if prompt.trim().is_empty() {
                return Err(WonderError::validation(format!(
                    "skill `{}` inline prompt cannot be empty",
                    self.name
                )));
            }
        }
        if let Some(prompt_path) = &self.prompt_path {
            validate_relative_path(prompt_path, "skill prompt path")?;
        }
        self.tool_spec().validate()?;
        if let Some(command_spec) = self.command_spec()? {
            command_spec.validate()?;
        }
        Ok(())
    }

    pub fn prompt_body(&self, root_dir: &Path) -> Result<String> {
        if let Some(prompt) = &self.prompt {
            return Ok(prompt.clone());
        }

        let prompt_path = self
            .prompt_path
            .clone()
            .unwrap_or_else(|| PathBuf::from("prompt.md"));
        let resolved = resolve_path(root_dir, &prompt_path)?;
        let prompt = fs::read_to_string(&resolved)?;
        if prompt.trim().is_empty() {
            return Err(WonderError::validation(format!(
                "skill `{}` prompt file is empty: {}",
                self.name,
                resolved.display()
            )));
        }
        Ok(prompt)
    }

    pub fn tool_spec(&self) -> ToolSpec {
        let mut spec = ToolSpec::new(&self.name, &self.description, ToolKind::Skill);
        spec.source = ToolSource::Skill;
        spec.required_features.insert(FeatureFlag::Skills);
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec
    }

    pub fn command_spec(&self) -> Result<Option<CommandSpec>> {
        let Some(name) = &self.slash_command else {
            return Ok(None);
        };
        let mut spec = CommandSpec::new(name, &self.description, CommandKind::Prompt);
        spec.aliases = self.slash_aliases.clone();
        spec.source = CommandSource::Skill;
        spec.required_features.insert(FeatureFlag::Skills);
        Ok(Some(spec))
    }
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
            "{label} cannot escape the skill root: {}",
            path.display()
        )));
    }
    Ok(())
}

fn resolve_path(root_dir: &Path, relative: &Path) -> Result<PathBuf> {
    validate_relative_path(relative, "skill prompt path")?;
    let resolved = root_dir.join(relative);
    if !resolved.is_file() {
        return Err(WonderError::validation(format!(
            "skill prompt file does not exist: {}",
            resolved.display()
        )));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use wonder_of_u_core::{CommandSource, ToolKind, ToolSource};
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    #[test]
    fn manifest_builds_tool_and_command_specs() {
        let manifest: SkillManifest = serde_json::from_value(json!({
            "schema_version": 1,
            "name": "lint-skill",
            "description": "Review lint output",
            "prompt": "Use cargo check results.",
            "allowed_tools": ["grep", "file_read"],
            "slash_command": "lint-skill",
            "slash_aliases": ["lint-review"]
        }))
        .expect("manifest");

        manifest.validate().expect("valid manifest");
        let tool_spec = manifest.tool_spec();
        let command_spec = manifest
            .command_spec()
            .expect("command spec")
            .expect("skill command");

        assert_eq!(tool_spec.kind, ToolKind::Skill);
        assert_eq!(tool_spec.source, ToolSource::Skill);
        assert_eq!(command_spec.source, CommandSource::Skill);
    }

    #[test]
    fn prompt_body_falls_back_to_prompt_file() {
        let dir = unique_test_dir("skill-prompt");
        fs::write(dir.join("prompt.md"), "Prompt body").expect("prompt file");
        let manifest = SkillManifest {
            schema_version: 1,
            name: "prompt-skill".into(),
            description: "prompt skill".into(),
            prompt: None,
            prompt_path: None,
            allowed_tools: Vec::new(),
            slash_command: None,
            slash_aliases: Vec::new(),
        };

        assert_eq!(
            manifest.prompt_body(&dir).expect("prompt body"),
            "Prompt body"
        );
    }
}
