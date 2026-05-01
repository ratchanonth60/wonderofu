use std::{
    collections::BTreeSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use wonder_of_u_core::{CommandSpec, ToolSpec};
use wonder_of_u_plugins::{PluginCatalog, PluginConfig};
use wonder_of_u_storage::StoragePaths;

use crate::{BundledSkill, SkillManifest, bundled_skills};

const SKILL_MANIFEST_FILE: &str = "skill.json";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SkillSource {
    Bundled,
    Project,
    User,
    Configured,
    Plugin { plugin_id: String },
}

impl SkillSource {
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Bundled => "bundled".into(),
            Self::Project => "project".into(),
            Self::User => "user".into(),
            Self::Configured => "configured".into(),
            Self::Plugin { plugin_id } => format!("plugin:{plugin_id}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillTrust {
    Bundled,
    Local,
    PluginTrusted,
}

impl SkillTrust {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Bundled => "bundled",
            Self::Local => "local",
            Self::PluginTrusted => "plugin_trusted",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkillRegistration {
    pub source: SkillSource,
    pub trust: SkillTrust,
    pub root_dir: PathBuf,
    pub manifest_path: Option<PathBuf>,
    pub manifest: SkillManifest,
    pub prompt: String,
    pub tool_spec: ToolSpec,
    pub command_spec: Option<CommandSpec>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillCatalogError {
    pub source: SkillSource,
    pub path: PathBuf,
    pub skill_name: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkillCatalog {
    skills: Vec<SkillRegistration>,
    errors: Vec<SkillCatalogError>,
}

impl SkillCatalog {
    #[must_use]
    pub fn load(
        cwd: &Path,
        storage_dir: Option<&Path>,
        config: &PluginConfig,
        plugins: &PluginCatalog,
    ) -> Self {
        let mut catalog = Self::default();
        let mut seen_tools = BTreeSet::new();
        let mut seen_commands = BTreeSet::new();

        for bundled in bundled_skills() {
            catalog.push(
                load_bundled_skill(&bundled),
                &mut seen_tools,
                &mut seen_commands,
            );
        }

        for (source, root) in discovery_roots(cwd, storage_dir, config) {
            match discover_skill_manifests(&root) {
                Ok(manifests) => {
                    for manifest_path in manifests {
                        catalog.push(
                            load_skill_from_manifest(
                                source.clone(),
                                SkillTrust::Local,
                                manifest_path,
                            ),
                            &mut seen_tools,
                            &mut seen_commands,
                        );
                    }
                }
                Err(error) => catalog.errors.push(SkillCatalogError {
                    source: source.clone(),
                    path: root,
                    skill_name: None,
                    message: error,
                }),
            }
        }

        for plugin in plugins.ready_entries() {
            for skill_source in &plugin.skill_sources {
                catalog.push(
                    load_skill_entry(
                        SkillSource::Plugin {
                            plugin_id: plugin.id.clone(),
                        },
                        SkillTrust::PluginTrusted,
                        skill_source.path.clone(),
                    ),
                    &mut seen_tools,
                    &mut seen_commands,
                );
            }
        }

        catalog
    }

    #[must_use]
    pub fn skills(&self) -> &[SkillRegistration] {
        &self.skills
    }

    #[must_use]
    pub fn errors(&self) -> &[SkillCatalogError] {
        &self.errors
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    #[must_use]
    pub fn command_count(&self) -> usize {
        self.skills
            .iter()
            .filter(|skill| skill.command_spec.is_some())
            .count()
    }

    #[must_use]
    pub fn bundled_count(&self) -> usize {
        self.skills
            .iter()
            .filter(|skill| skill.source == SkillSource::Bundled)
            .count()
    }

    #[must_use]
    pub fn plugin_count(&self) -> usize {
        self.skills
            .iter()
            .filter(|skill| matches!(skill.source, SkillSource::Plugin { .. }))
            .count()
    }

    #[must_use]
    pub fn find(&self, name: &str) -> Option<&SkillRegistration> {
        let normalized = normalize_lookup(name)?;
        self.skills.iter().find(|skill| {
            normalize_lookup(&skill.manifest.name).as_deref() == Some(normalized.as_str())
        })
    }

    fn push(
        &mut self,
        result: Result<SkillRegistration, SkillCatalogError>,
        seen_tools: &mut BTreeSet<String>,
        seen_commands: &mut BTreeSet<String>,
    ) {
        let skill = match result {
            Ok(skill) => skill,
            Err(error) => {
                self.errors.push(error);
                return;
            }
        };

        let tool_name = normalize_lookup(&skill.tool_spec.name).expect("validated tool name");
        if !seen_tools.insert(tool_name.clone()) {
            self.errors.push(SkillCatalogError {
                source: skill.source.clone(),
                path: skill.root_dir.clone(),
                skill_name: Some(skill.manifest.name.clone()),
                message: format!("duplicate skill tool name: {tool_name}"),
            });
            return;
        }

        let mut command_names = Vec::new();
        if let Some(command_spec) = &skill.command_spec {
            command_names
                .push(normalize_lookup(&command_spec.name).expect("validated command name"));
            command_names.extend(
                command_spec
                    .aliases
                    .iter()
                    .filter_map(|alias| normalize_lookup(alias)),
            );
        }
        if command_names
            .iter()
            .any(|name| seen_commands.contains(name))
        {
            self.errors.push(SkillCatalogError {
                source: skill.source.clone(),
                path: skill.root_dir.clone(),
                skill_name: Some(skill.manifest.name.clone()),
                message: format!("duplicate skill slash command: {}", command_names.join(",")),
            });
            return;
        }
        seen_commands.extend(command_names);

        self.skills.push(skill);
    }
}

fn discovery_roots(
    cwd: &Path,
    storage_dir: Option<&Path>,
    config: &PluginConfig,
) -> Vec<(SkillSource, PathBuf)> {
    let mut roots = Vec::new();
    let mut seen = BTreeSet::new();

    let project_root = cwd.join(".wonder").join("skills");
    if seen.insert(project_root.clone()) {
        roots.push((SkillSource::Project, project_root));
    }

    if let Some(storage_dir) = storage_dir {
        let user_root = StoragePaths::new(storage_dir).skills_dir();
        if seen.insert(user_root.clone()) {
            roots.push((SkillSource::User, user_root));
        }
    }

    for dir in &config.additional_skill_dirs {
        let configured_root = if dir.is_absolute() {
            dir.clone()
        } else {
            cwd.join(dir)
        };
        if seen.insert(configured_root.clone()) {
            roots.push((SkillSource::Configured, configured_root));
        }
    }

    roots
}

fn discover_skill_manifests(root: &Path) -> Result<Vec<PathBuf>, String> {
    if root.is_file() {
        return root
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value == SKILL_MANIFEST_FILE)
            .then(|| vec![root.to_path_buf()])
            .ok_or_else(|| format!("unsupported skill manifest path: {}", root.display()));
    }

    let root_manifest = root.join(SKILL_MANIFEST_FILE);
    if root_manifest.is_file() {
        return Ok(vec![root_manifest]);
    }

    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };

    let mut manifests = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_dir())
                .map(|_| entry.path().join(SKILL_MANIFEST_FILE))
        })
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    manifests.sort();
    Ok(manifests)
}

fn load_bundled_skill(bundled: &BundledSkill) -> Result<SkillRegistration, SkillCatalogError> {
    bundled
        .manifest
        .validate()
        .map_err(|error| SkillCatalogError {
            source: SkillSource::Bundled,
            path: PathBuf::from("bundled-inline"),
            skill_name: Some(bundled.manifest.name.clone()),
            message: error.to_string(),
        })?;

    Ok(SkillRegistration {
        source: SkillSource::Bundled,
        trust: SkillTrust::Bundled,
        root_dir: PathBuf::from("bundled-inline"),
        manifest_path: None,
        manifest: bundled.manifest.clone(),
        prompt: bundled.prompt.clone(),
        tool_spec: bundled.manifest.tool_spec(),
        command_spec: bundled
            .manifest
            .command_spec()
            .map_err(|error| SkillCatalogError {
                source: SkillSource::Bundled,
                path: PathBuf::from("bundled-inline"),
                skill_name: Some(bundled.manifest.name.clone()),
                message: error.to_string(),
            })?,
    })
}

fn load_skill_from_manifest(
    source: SkillSource,
    trust: SkillTrust,
    manifest_path: PathBuf,
) -> Result<SkillRegistration, SkillCatalogError> {
    let root_dir = manifest_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let manifest =
        SkillManifest::read_from_path(&manifest_path).map_err(|error| SkillCatalogError {
            source: source.clone(),
            path: manifest_path.clone(),
            skill_name: None,
            message: error.to_string(),
        })?;
    let prompt = manifest
        .prompt_body(&root_dir)
        .map_err(|error| SkillCatalogError {
            source: source.clone(),
            path: manifest_path.clone(),
            skill_name: Some(manifest.name.clone()),
            message: error.to_string(),
        })?;
    let command_spec = manifest.command_spec().map_err(|error| SkillCatalogError {
        source: source.clone(),
        path: manifest_path.clone(),
        skill_name: Some(manifest.name.clone()),
        message: error.to_string(),
    })?;

    Ok(SkillRegistration {
        source,
        trust,
        root_dir,
        manifest_path: Some(manifest_path),
        tool_spec: manifest.tool_spec(),
        command_spec,
        manifest,
        prompt,
    })
}

fn load_skill_entry(
    source: SkillSource,
    trust: SkillTrust,
    path: PathBuf,
) -> Result<SkillRegistration, SkillCatalogError> {
    if path.is_file() {
        return load_skill_from_manifest(source, trust, path);
    }
    let manifest_path = path.join(SKILL_MANIFEST_FILE);
    if !manifest_path.is_file() {
        return Err(SkillCatalogError {
            source,
            path,
            skill_name: None,
            message: format!("skill directory does not contain {SKILL_MANIFEST_FILE}"),
        });
    }
    load_skill_from_manifest(source, trust, manifest_path)
}

fn normalize_lookup(name: &str) -> Option<String> {
    let normalized = name.trim().trim_start_matches('/').to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use wonder_of_u_plugins::{PluginCatalog, PluginConfig, PluginTrustDecision};
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn write_skill(dir: &Path, name: &str, command: Option<&str>) {
        fs::create_dir_all(dir).expect("skill dir");
        fs::write(
            dir.join(SKILL_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "name": name,
                "description": format!("{name} skill"),
                "prompt_path": "prompt.md",
                "allowed_tools": ["grep", "file_read"],
                "slash_command": command,
            }))
            .expect("skill manifest"),
        )
        .expect("write skill manifest");
        fs::write(dir.join("prompt.md"), format!("Prompt for {name}")).expect("write prompt");
    }

    fn write_plugin(root: &Path) {
        fs::create_dir_all(root.join("commands")).expect("commands dir");
        write_skill(
            &root.join("skills/release-check"),
            "release-check",
            Some("release-check"),
        );
        fs::write(root.join("commands/run.txt"), "echo run").expect("command file");
        fs::write(
            root.join("plugin.json"),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "name": "demo-plugin",
                "version": "0.1.0",
                "description": "demo plugin",
                "commands": [{
                    "name": "demo-plugin-run",
                    "description": "Run plugin",
                    "path": "commands/run.txt"
                }],
                "skills": [{
                    "path": "skills/release-check"
                }]
            }))
            .expect("plugin manifest"),
        )
        .expect("write plugin manifest");
    }

    #[test]
    fn catalog_assembles_bundled_user_and_plugin_skills() {
        let cwd = unique_test_dir("skills-catalog-cwd");
        let storage = unique_test_dir("skills-catalog-storage");
        let configured_plugins = unique_test_dir("skills-catalog-plugins");

        write_skill(
            &storage.join("skills/lint-review"),
            "lint-review",
            Some("lint-review"),
        );
        let plugin_root = configured_plugins.join("demo-plugin");
        fs::create_dir_all(&plugin_root).expect("plugin root");
        write_plugin(&plugin_root);

        let mut config = PluginConfig {
            additional_plugin_dirs: vec![configured_plugins],
            ..PluginConfig::default()
        };
        config
            .set_trust("demo-plugin", PluginTrustDecision::Trusted)
            .expect("trust plugin");

        let plugins = PluginCatalog::load(&cwd, Some(storage.as_path()), &config);
        let catalog = SkillCatalog::load(&cwd, Some(storage.as_path()), &config, &plugins);

        assert!(catalog.find("workspace-audit").is_some());
        assert!(catalog.find("lint-review").is_some());
        assert!(catalog.find("release-check").is_some());
        assert_eq!(catalog.bundled_count(), 1);
        assert_eq!(catalog.plugin_count(), 1);
        assert_eq!(catalog.command_count(), 3);
    }

    #[test]
    fn untrusted_plugin_skills_are_not_loaded() {
        let cwd = unique_test_dir("skills-untrusted-cwd");
        let configured_plugins = unique_test_dir("skills-untrusted-plugins");
        let plugin_root = configured_plugins.join("demo-plugin");
        fs::create_dir_all(&plugin_root).expect("plugin root");
        write_plugin(&plugin_root);

        let config = PluginConfig {
            additional_plugin_dirs: vec![configured_plugins],
            ..PluginConfig::default()
        };
        let plugins = PluginCatalog::load(&cwd, None, &config);
        let catalog = SkillCatalog::load(&cwd, None, &config, &plugins);

        assert!(catalog.find("release-check").is_none());
        assert_eq!(catalog.plugin_count(), 0);
    }
}
