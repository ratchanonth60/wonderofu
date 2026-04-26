use std::{
    collections::BTreeSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use wonder_of_u_storage::StoragePaths;

use crate::{
    PluginCommandRegistration, PluginConfig, PluginManifest, PluginSkillSource,
    PluginTrustDecision, normalize_plugin_id,
};

const MANIFEST_FILE_NAMES: [&str; 2] = ["plugin.json", "wonder-plugin.json"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginSource {
    Project,
    User,
    Configured,
    Bundled,
}

impl PluginSource {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::User => "user",
            Self::Configured => "configured",
            Self::Bundled => "bundled",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginTrustLevel {
    Bundled,
    Trusted,
    Untrusted,
    Blocked,
}

impl PluginTrustLevel {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Bundled => "bundled",
            Self::Trusted => "trusted",
            Self::Untrusted => "untrusted",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginReadiness {
    Ready,
    NeedsTrust,
    Blocked,
    Invalid,
}

impl PluginReadiness {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NeedsTrust => "needs_trust",
            Self::Blocked => "blocked",
            Self::Invalid => "invalid",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginDiscoveryRoot {
    pub source: PluginSource,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginCatalogEntry {
    pub id: String,
    pub source: PluginSource,
    pub root_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub trust: PluginTrustLevel,
    pub readiness: PluginReadiness,
    pub manifest: Option<PluginManifest>,
    pub command_registrations: Vec<PluginCommandRegistration>,
    pub skill_sources: Vec<PluginSkillSource>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginCatalogError {
    pub source: PluginSource,
    pub path: PathBuf,
    pub plugin_id: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PluginCatalog {
    roots: Vec<PluginDiscoveryRoot>,
    entries: Vec<PluginCatalogEntry>,
    errors: Vec<PluginCatalogError>,
}

impl PluginCatalog {
    #[must_use]
    pub fn load(cwd: &Path, storage_dir: Option<&Path>, config: &PluginConfig) -> Self {
        let roots = discovery_roots(cwd, storage_dir, config);
        let mut entries = Vec::new();
        let mut errors = Vec::new();
        let mut seen_ids = BTreeSet::new();

        for root in &roots {
            match discover_plugin_dirs(&root.path) {
                Ok(plugin_dirs) => {
                    for plugin_dir in plugin_dirs {
                        let Some(manifest_path) = manifest_path(&plugin_dir) else {
                            continue;
                        };
                        let mut entry =
                            load_entry(root.source, &plugin_dir, &manifest_path, config);
                        if entry.readiness != PluginReadiness::Invalid {
                            if let Some(manifest) = &entry.manifest {
                                if let Ok(id) = manifest.plugin_id() {
                                    if !seen_ids.insert(id.clone()) {
                                        entry.readiness = PluginReadiness::Invalid;
                                        entry.notes.push(format!(
                                            "duplicate plugin id discovered in a higher-priority root: {id}"
                                        ));
                                        errors.push(PluginCatalogError {
                                            source: root.source,
                                            path: plugin_dir.clone(),
                                            plugin_id: Some(id),
                                            message: "duplicate plugin id in catalog".into(),
                                        });
                                    }
                                }
                            }
                        } else {
                            errors.push(PluginCatalogError {
                                source: root.source,
                                path: manifest_path.clone(),
                                plugin_id: Some(entry.id.clone()),
                                message: entry.notes.join("; "),
                            });
                        }
                        entries.push(entry);
                    }
                }
                Err(error) => errors.push(PluginCatalogError {
                    source: root.source,
                    path: root.path.clone(),
                    plugin_id: None,
                    message: error,
                }),
            }
        }

        Self {
            roots,
            entries,
            errors,
        }
    }

    #[must_use]
    pub fn roots(&self) -> &[PluginDiscoveryRoot] {
        &self.roots
    }

    #[must_use]
    pub fn entries(&self) -> &[PluginCatalogEntry] {
        &self.entries
    }

    #[must_use]
    pub fn errors(&self) -> &[PluginCatalogError] {
        &self.errors
    }

    pub fn ready_entries(&self) -> impl Iterator<Item = &PluginCatalogEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.readiness == PluginReadiness::Ready)
    }

    #[must_use]
    pub fn ready_count(&self) -> usize {
        self.ready_entries().count()
    }

    #[must_use]
    pub fn needs_trust_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.readiness == PluginReadiness::NeedsTrust)
            .count()
    }

    #[must_use]
    pub fn invalid_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.readiness == PluginReadiness::Invalid)
            .count()
    }

    #[must_use]
    pub fn blocked_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.readiness == PluginReadiness::Blocked)
            .count()
    }

    #[must_use]
    pub fn find(&self, plugin_id: &str) -> Option<&PluginCatalogEntry> {
        let normalized = normalize_plugin_id(plugin_id).ok()?;
        self.entries.iter().find(|entry| entry.id == normalized)
    }
}

fn discovery_roots(
    cwd: &Path,
    storage_dir: Option<&Path>,
    config: &PluginConfig,
) -> Vec<PluginDiscoveryRoot> {
    let mut roots = Vec::new();
    let mut seen = BTreeSet::new();

    let project_root = cwd.join(".wonder").join("plugins");
    if seen.insert(project_root.clone()) {
        roots.push(PluginDiscoveryRoot {
            source: PluginSource::Project,
            path: project_root,
        });
    }

    if let Some(storage_dir) = storage_dir {
        let user_root = StoragePaths::new(storage_dir).plugins_dir();
        if seen.insert(user_root.clone()) {
            roots.push(PluginDiscoveryRoot {
                source: PluginSource::User,
                path: user_root,
            });
        }
    }

    for dir in &config.additional_plugin_dirs {
        let configured_root = resolve_configured_path(cwd, dir);
        if seen.insert(configured_root.clone()) {
            roots.push(PluginDiscoveryRoot {
                source: PluginSource::Configured,
                path: configured_root,
            });
        }
    }

    roots
}

fn resolve_configured_path(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn discover_plugin_dirs(root: &Path) -> Result<Vec<PathBuf>, String> {
    if manifest_path(root).is_some() {
        return Ok(vec![root.to_path_buf()]);
    }

    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };

    let mut plugin_dirs = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_dir())
                .map(|_| entry.path())
        })
        .filter(|path| manifest_path(path).is_some())
        .collect::<Vec<_>>();
    plugin_dirs.sort();
    Ok(plugin_dirs)
}

fn manifest_path(root: &Path) -> Option<PathBuf> {
    MANIFEST_FILE_NAMES
        .into_iter()
        .map(|name| root.join(name))
        .find(|path| path.is_file())
}

fn load_entry(
    source: PluginSource,
    root_dir: &Path,
    manifest_path: &Path,
    config: &PluginConfig,
) -> PluginCatalogEntry {
    let fallback_id = root_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("plugin")
        .to_ascii_lowercase();

    match PluginManifest::read_from_path(manifest_path) {
        Ok(manifest) => {
            let id = manifest.plugin_id().unwrap_or_else(|_| fallback_id.clone());
            let trust = effective_trust(source, config, &id);
            let readiness = readiness_for_trust(trust);
            match (
                manifest.command_registrations(&id, root_dir),
                manifest.skill_sources(&id, root_dir),
            ) {
                (Ok(command_registrations), Ok(skill_sources)) => PluginCatalogEntry {
                    id,
                    source,
                    root_dir: root_dir.to_path_buf(),
                    manifest_path: manifest_path.to_path_buf(),
                    trust,
                    readiness,
                    manifest: Some(manifest),
                    command_registrations,
                    skill_sources,
                    notes: Vec::new(),
                },
                (command_result, skill_result) => {
                    let mut notes = Vec::new();
                    if let Err(error) = command_result {
                        notes.push(error.to_string());
                    }
                    if let Err(error) = skill_result {
                        notes.push(error.to_string());
                    }
                    PluginCatalogEntry {
                        id,
                        source,
                        root_dir: root_dir.to_path_buf(),
                        manifest_path: manifest_path.to_path_buf(),
                        trust,
                        readiness: PluginReadiness::Invalid,
                        manifest: Some(manifest),
                        command_registrations: Vec::new(),
                        skill_sources: Vec::new(),
                        notes,
                    }
                }
            }
        }
        Err(error) => PluginCatalogEntry {
            id: fallback_id.clone(),
            source,
            root_dir: root_dir.to_path_buf(),
            manifest_path: manifest_path.to_path_buf(),
            trust: effective_trust(source, config, &fallback_id),
            readiness: PluginReadiness::Invalid,
            manifest: None,
            command_registrations: Vec::new(),
            skill_sources: Vec::new(),
            notes: vec![error.to_string()],
        },
    }
}

fn effective_trust(
    source: PluginSource,
    config: &PluginConfig,
    plugin_id: &str,
) -> PluginTrustLevel {
    match config.trust_for(plugin_id) {
        Some(PluginTrustDecision::Trusted) => PluginTrustLevel::Trusted,
        Some(PluginTrustDecision::Untrusted) => PluginTrustLevel::Untrusted,
        Some(PluginTrustDecision::Blocked) => PluginTrustLevel::Blocked,
        None => match source {
            PluginSource::Bundled => PluginTrustLevel::Bundled,
            PluginSource::Project | PluginSource::User => PluginTrustLevel::Trusted,
            PluginSource::Configured => PluginTrustLevel::Untrusted,
        },
    }
}

fn readiness_for_trust(trust: PluginTrustLevel) -> PluginReadiness {
    match trust {
        PluginTrustLevel::Bundled | PluginTrustLevel::Trusted => PluginReadiness::Ready,
        PluginTrustLevel::Untrusted => PluginReadiness::NeedsTrust,
        PluginTrustLevel::Blocked => PluginReadiness::Blocked,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;
    use crate::PluginConfig;

    fn write_plugin(root: &Path, name: &str, command_name: &str, skill_name: &str) {
        let plugin_dir = root.join(name);
        fs::create_dir_all(plugin_dir.join("commands")).expect("commands dir");
        fs::create_dir_all(plugin_dir.join("skills").join(skill_name)).expect("skills dir");
        fs::write(plugin_dir.join("commands/run.txt"), "echo run").expect("command file");
        fs::write(
            plugin_dir.join("plugin.json"),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "name": name,
                "version": "0.1.0",
                "description": format!("{name} plugin"),
                "commands": [{
                    "name": command_name,
                    "description": format!("{command_name} command"),
                    "path": "commands/run.txt"
                }],
                "skills": [{
                    "path": format!("skills/{skill_name}")
                }]
            }))
            .expect("manifest json"),
        )
        .expect("manifest file");
    }

    #[test]
    fn catalog_discovers_user_project_and_configured_roots() {
        let cwd = unique_test_dir("plugin-catalog-cwd");
        let storage = unique_test_dir("plugin-catalog-storage");
        let configured = unique_test_dir("plugin-catalog-configured");

        write_plugin(
            &cwd.join(".wonder/plugins"),
            "project-plugin",
            "project-run",
            "project-skill",
        );
        write_plugin(
            &storage.join("plugins"),
            "user-plugin",
            "user-run",
            "user-skill",
        );
        write_plugin(
            &configured,
            "configured-plugin",
            "configured-run",
            "configured-skill",
        );

        let config = PluginConfig {
            additional_plugin_dirs: vec![configured.clone()],
            ..PluginConfig::default()
        };
        let catalog = PluginCatalog::load(&cwd, Some(storage.as_path()), &config);

        assert_eq!(catalog.entries().len(), 3);
        assert_eq!(catalog.ready_count(), 2);
        assert_eq!(catalog.needs_trust_count(), 1);
        assert_eq!(
            catalog
                .find("configured-plugin")
                .expect("configured plugin")
                .readiness,
            PluginReadiness::NeedsTrust
        );
        assert_eq!(catalog.errors().len(), 0);
    }

    #[test]
    fn configured_trust_override_marks_plugin_ready() {
        let cwd = unique_test_dir("plugin-trust-cwd");
        let configured = unique_test_dir("plugin-trust-configured");
        write_plugin(
            &configured,
            "Configured Plugin",
            "configured-run",
            "configured-skill",
        );

        let mut config = PluginConfig {
            additional_plugin_dirs: vec![configured],
            ..PluginConfig::default()
        };
        config
            .set_trust("configured-plugin", PluginTrustDecision::Trusted)
            .expect("trust entry");

        let catalog = PluginCatalog::load(&cwd, None, &config);
        let plugin = catalog
            .find("configured-plugin")
            .expect("configured plugin");
        assert_eq!(plugin.readiness, PluginReadiness::Ready);
        assert_eq!(plugin.trust, PluginTrustLevel::Trusted);
    }
}
