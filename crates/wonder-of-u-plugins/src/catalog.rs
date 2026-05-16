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

/// Manifest filenames accepted during plugin discovery.
pub const PLUGIN_MANIFEST_FILE_NAMES: [&str; 3] = [
    "plugin.json",
    "wonder-plugin.json",
    "wonder-of-u-plugin.json",
];
/// Enumerates plugin source
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginSource {
    /// Represents project
    Project,
    /// Represents user
    User,
    /// Represents configured
    Configured,
    /// Represents bundled
    Bundled,
}

impl PluginSource {
    /// Constant fn
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
/// Enumerates plugin trust level
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginTrustLevel {
    /// Represents bundled
    Bundled,
    /// Represents trusted
    Trusted,
    /// Represents untrusted
    Untrusted,
    /// Represents blocked
    Blocked,
}

impl PluginTrustLevel {
    /// Constant fn
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
/// Enumerates plugin readiness
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginReadiness {
    /// Represents ready
    Ready,
    /// Represents needs trust
    NeedsTrust,
    /// Represents blocked
    Blocked,
    /// Represents invalid
    Invalid,
}

impl PluginReadiness {
    /// Constant fn
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
/// Represents plugin discovery root
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginDiscoveryRoot {
    /// Stores the source
    pub source: PluginSource,
    /// Stores the path
    pub path: PathBuf,
}
/// Represents plugin catalog entry
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginCatalogEntry {
    /// Stores the id
    pub id: String,
    /// Stores the source
    pub source: PluginSource,
    /// Stores the root directory
    pub root_dir: PathBuf,
    /// Stores the manifest path
    pub manifest_path: PathBuf,
    /// Stores the trust
    pub trust: PluginTrustLevel,
    /// Stores the readiness
    pub readiness: PluginReadiness,
    /// Stores the manifest
    pub manifest: Option<PluginManifest>,
    /// Stores the command registrations
    pub command_registrations: Vec<PluginCommandRegistration>,
    /// Stores the skill sources
    pub skill_sources: Vec<PluginSkillSource>,
    /// Stores the notes
    pub notes: Vec<String>,
}
/// Describes plugin catalog error
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginCatalogError {
    /// Stores the source
    pub source: PluginSource,
    /// Stores the path
    pub path: PathBuf,
    /// Stores the plugin identifier
    pub plugin_id: Option<String>,
    /// Stores the message
    pub message: String,
}
/// Stores plugin catalog
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PluginCatalog {
    roots: Vec<PluginDiscoveryRoot>,
    entries: Vec<PluginCatalogEntry>,
    errors: Vec<PluginCatalogError>,
}

impl PluginCatalog {
    /// Handles load
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
                        let Some(manifest_path) = plugin_manifest_path(&plugin_dir) else {
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
    /// Handles roots
    #[must_use]
    pub fn roots(&self) -> &[PluginDiscoveryRoot] {
        &self.roots
    }
    /// Handles entries
    #[must_use]
    pub fn entries(&self) -> &[PluginCatalogEntry] {
        &self.entries
    }
    /// Handles errors
    #[must_use]
    pub fn errors(&self) -> &[PluginCatalogError] {
        &self.errors
    }

    /// Handles ready entries
    pub fn ready_entries(&self) -> impl Iterator<Item = &PluginCatalogEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.readiness == PluginReadiness::Ready)
    }
    /// Handles ready count
    #[must_use]
    pub fn ready_count(&self) -> usize {
        self.ready_entries().count()
    }
    /// Handles needs trust count
    #[must_use]
    pub fn needs_trust_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.readiness == PluginReadiness::NeedsTrust)
            .count()
    }
    /// Handles invalid count
    #[must_use]
    pub fn invalid_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.readiness == PluginReadiness::Invalid)
            .count()
    }
    /// Handles blocked count
    #[must_use]
    pub fn blocked_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.readiness == PluginReadiness::Blocked)
            .count()
    }
    /// Handles find
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
    if plugin_manifest_path(root).is_some() {
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
        .filter(|path| plugin_manifest_path(path).is_some())
        .collect::<Vec<_>>();
    plugin_dirs.sort();
    Ok(plugin_dirs)
}

/// Returns the first supported manifest filename found in a plugin directory.
#[must_use]
pub fn plugin_manifest_path(root: &Path) -> Option<PathBuf> {
    PLUGIN_MANIFEST_FILE_NAMES
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

    fn write_plugin_with_manifest_name(
        root: &Path,
        manifest_name: &str,
        name: &str,
        command_name: &str,
        skill_name: &str,
    ) {
        let plugin_dir = root.join(name);
        fs::create_dir_all(plugin_dir.join("commands")).expect("commands dir");
        fs::create_dir_all(plugin_dir.join("skills").join(skill_name)).expect("skills dir");
        fs::write(plugin_dir.join("commands/run.txt"), "echo run").expect("command file");
        fs::write(
            plugin_dir.join(manifest_name),
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

    #[test]
    fn catalog_discovers_legacy_manifest_aliases() {
        let cwd = unique_test_dir("plugin-legacy-manifest-cwd");
        let project_root = cwd.join(".wonder/plugins");
        write_plugin_with_manifest_name(
            &project_root,
            "wonder-of-u-plugin.json",
            "legacy-plugin",
            "legacy-run",
            "legacy-skill",
        );

        let catalog = PluginCatalog::load(&cwd, None, &PluginConfig::default());
        let plugin = catalog.find("legacy-plugin").expect("legacy plugin");

        assert_eq!(plugin.readiness, PluginReadiness::Ready);
        assert_eq!(
            plugin
                .manifest_path
                .file_name()
                .and_then(|name| name.to_str()),
            Some("wonder-of-u-plugin.json")
        );
    }

    #[test]
    fn plugin_broken_manifest_does_not_crash_registry() {
        let cwd = unique_test_dir("plugin-broken-manifest-cwd");
        let project_root = cwd.join(".wonder/plugins");
        write_plugin(&project_root, "valid-plugin", "valid-run", "valid-skill");

        let broken_dir = project_root.join("broken-plugin");
        fs::create_dir_all(&broken_dir).expect("broken plugin dir");
        fs::write(broken_dir.join("plugin.json"), "{ not valid json").expect("broken manifest");

        let catalog = PluginCatalog::load(&cwd, None, &PluginConfig::default());

        assert_eq!(catalog.entries().len(), 2);
        assert_eq!(catalog.ready_count(), 1);
        assert_eq!(catalog.invalid_count(), 1);
        assert_eq!(catalog.errors().len(), 1);
        assert_eq!(
            catalog
                .find("valid-plugin")
                .expect("valid plugin")
                .command_registrations
                .len(),
            1
        );
        let broken = catalog.find("broken-plugin").expect("broken plugin");
        assert_eq!(broken.readiness, PluginReadiness::Invalid);
        assert!(broken.command_registrations.is_empty());
    }

    #[test]
    fn plugin_reload_with_broken_plugin_preserves_valid_plugins() {
        let cwd = unique_test_dir("plugin-reload-cwd");
        let project_root = cwd.join(".wonder/plugins");
        write_plugin(&project_root, "valid-plugin", "valid-run", "valid-skill");

        let first_load = PluginCatalog::load(&cwd, None, &PluginConfig::default());
        assert_eq!(first_load.ready_count(), 1);
        assert_eq!(
            first_load
                .find("valid-plugin")
                .expect("valid plugin")
                .command_registrations
                .len(),
            1
        );

        let broken_dir = project_root.join("broken-plugin");
        fs::create_dir_all(&broken_dir).expect("broken plugin dir");
        fs::write(broken_dir.join("plugin.json"), "{ broken").expect("broken manifest");

        let reloaded = PluginCatalog::load(&cwd, None, &PluginConfig::default());

        assert_eq!(reloaded.ready_count(), 1);
        assert_eq!(reloaded.invalid_count(), 1);
        assert_eq!(reloaded.errors().len(), 1);
        let valid = reloaded
            .find("valid-plugin")
            .expect("valid plugin after reload");
        assert_eq!(valid.readiness, PluginReadiness::Ready);
        assert_eq!(valid.command_registrations.len(), 1);
        assert_eq!(valid.command_registrations[0].spec.name, "valid-run");
    }
}
