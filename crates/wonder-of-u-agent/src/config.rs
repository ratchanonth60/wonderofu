use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsStr,
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use wonder_of_u_core::{
    Result, WonderError,
    permission::{PermissionRule, PermissionRuleBehavior, PermissionRuleSource},
};
use wonder_of_u_storage::StoragePaths;

use crate::auth::{AuthMaterial, StoredCredentials};

fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let next_extension = match path.extension().and_then(OsStr::to_str) {
        Some(extension) => format!("{extension}.next"),
        None => "next".into(),
    };
    let pending_path = path.with_extension(next_extension);

    let file = File::create(&pending_path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    fs::rename(pending_path, path)?;
    Ok(())
}

/// Reads and deserialises `settings.json` from `dir`, returning a default if
/// the file is absent. Propagates IO or JSON errors.
fn load_settings_from_dir(dir: &Path) -> Result<AgentSettings> {
    let path = dir.join("settings.json");
    if !path.exists() {
        return Ok(AgentSettings::default());
    }
    serde_json::from_str(&fs::read_to_string(&path)?).map_err(Into::into)
}

/// Represents agent settings
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentSettings {
    /// Schema version – increment when breaking changes are made to this struct.
    #[serde(default)]
    pub schema_version: u16,
    /// Stores the selected provider
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_provider: Option<String>,
    /// Stores the selected model
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_model: Option<String>,
    /// Stores the selected TUI theme
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Stores the selected assistant output rendering style
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_style: Option<String>,
    /// Stores whether vim keybindings are enabled in the TUI prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vim_mode: Option<bool>,
    /// Stores the fast mode
    #[serde(default)]
    pub fast_mode: bool,
    /// Stores the effort level
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_level: Option<String>,
    /// Stores the providers
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub providers: BTreeMap<String, ProviderOverride>,
    /// Stores persisted environment variables injected into agent context.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env_vars: HashMap<String, String>,
    /// Tool names that this layer explicitly allows. Converted to
    /// `PermissionRule` during hierarchy merge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_tools: Vec<String>,
    /// Tool names that this layer explicitly denies. Converted to
    /// `PermissionRule` during hierarchy merge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_tools: Vec<String>,
}
/// Represents provider override
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProviderOverride {
    /// Stores the model
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Stores the api base
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
}

/// A single layer in the settings hierarchy: a parsed `AgentSettings` together
/// with the permission source it should be attributed to.
#[derive(Clone, Debug)]
pub struct SettingsLayer {
    /// The permission source this layer is attributed to (Policy / User / Project).
    pub source: PermissionRuleSource,
    /// The directory from which this layer was loaded.
    pub path: PathBuf,
    /// The parsed settings for this layer.
    pub settings: AgentSettings,
}

/// The merged result of loading settings from policy, user, and project
/// directories.
///
/// Layers are ordered highest-precedence first: `[Policy, User, Project]`.
/// `merged` is the unified `AgentSettings` and `permission_rules` is the
/// flattened list of `PermissionRule` values derived from `allow_tools` /
/// `deny_tools` across all layers.
#[derive(Clone, Debug)]
pub struct SettingsHierarchy {
    /// All resolved layers, highest precedence first.
    pub layers: Vec<SettingsLayer>,
    /// The merged `AgentSettings` (highest-precedence field wins).
    pub merged: AgentSettings,
    /// Permission rules derived from `allow_tools` / `deny_tools` in all layers.
    pub permission_rules: Vec<PermissionRule>,
}

impl SettingsHierarchy {
    /// Load settings from `policy_dir` (MDM/policy), `user_dir`, and
    /// `project_dir`. Any directory may be `None` or absent on disk — that
    /// layer is skipped (no error). Settings file name within each directory
    /// is `settings.json`.
    ///
    /// Precedence order (highest first): Policy > User > Project.
    pub fn load(
        policy_dir: Option<&Path>,
        user_dir: &Path,
        project_dir: Option<&Path>,
    ) -> Result<Self> {
        let mut layers: Vec<SettingsLayer> = Vec::with_capacity(3);

        if let Some(dir) = policy_dir {
            layers.push(SettingsLayer {
                source: PermissionRuleSource::Policy,
                path: dir.to_path_buf(),
                settings: load_settings_from_dir(dir)?,
            });
        }

        layers.push(SettingsLayer {
            source: PermissionRuleSource::User,
            path: user_dir.to_path_buf(),
            settings: load_settings_from_dir(user_dir)?,
        });

        if let Some(dir) = project_dir {
            layers.push(SettingsLayer {
                source: PermissionRuleSource::Project,
                path: dir.to_path_buf(),
                settings: load_settings_from_dir(dir)?,
            });
        }

        let (merged, permission_rules) = Self::merge(&layers);
        Ok(Self {
            layers,
            merged,
            permission_rules,
        })
    }

    /// Merge layers (highest-precedence first) into a unified `AgentSettings`
    /// and a flat `Vec<PermissionRule>`.
    ///
    /// Merge semantics:
    /// - `Option<T>` scalar fields: first `Some` wins.
    /// - `bool` scalar fields: first layer that sets it `true` wins.
    /// - `providers` map: first definition per key wins.
    /// - `allow_tools` / `deny_tools`: collected from all layers and converted
    ///   to `PermissionRule` with the layer's source.
    fn merge(layers: &[SettingsLayer]) -> (AgentSettings, Vec<PermissionRule>) {
        let mut merged = AgentSettings::default();
        let mut rules: Vec<PermissionRule> = Vec::new();

        for layer in layers {
            let s = &layer.settings;

            if merged.selected_provider.is_none() {
                merged.selected_provider = s.selected_provider.clone();
            }
            if merged.selected_model.is_none() {
                merged.selected_model = s.selected_model.clone();
            }
            if merged.theme.is_none() {
                merged.theme = s.theme.clone();
            }
            if merged.output_style.is_none() {
                merged.output_style = s.output_style.clone();
            }
            if merged.vim_mode.is_none() {
                merged.vim_mode = s.vim_mode;
            }
            // fast_mode: first layer that enables it wins.
            if !merged.fast_mode && s.fast_mode {
                merged.fast_mode = true;
            }
            if merged.effort_level.is_none() {
                merged.effort_level = s.effort_level.clone();
            }
            // Providers: first definition per key wins.
            for (k, v) in &s.providers {
                merged
                    .providers
                    .entry(k.clone())
                    .or_insert_with(|| v.clone());
            }
            for (k, v) in &s.env_vars {
                merged
                    .env_vars
                    .entry(k.clone())
                    .or_insert_with(|| v.clone());
            }

            // Convert allow_tools entries to Allow rules.
            for tool in &s.allow_tools {
                rules.push(PermissionRule::new(
                    tool.clone(),
                    PermissionRuleBehavior::Allow,
                    layer.source,
                ));
            }
            // Convert deny_tools entries to Deny rules.
            for tool in &s.deny_tools {
                rules.push(PermissionRule::new(
                    tool.clone(),
                    PermissionRuleBehavior::Deny,
                    layer.source,
                ));
            }
        }

        (merged, rules)
    }
}

/// Returns indices of `(shadowed_allow_idx, masking_deny_idx)` pairs where an
/// Allow rule in `rules` is masked by a higher-precedence Deny rule that covers
/// the same tool pattern.
///
/// "Higher precedence" means a strictly lower `PermissionRuleSource::precedence()`
/// value (e.g. `Policy` = 0 outranks `User` = 6).
pub fn detect_shadowed_rules(rules: &[PermissionRule]) -> Vec<(usize, usize)> {
    let mut shadowed: Vec<(usize, usize)> = Vec::new();

    for (allow_idx, allow_rule) in rules.iter().enumerate() {
        if allow_rule.behavior != PermissionRuleBehavior::Allow {
            continue;
        }
        for (deny_idx, deny_rule) in rules.iter().enumerate() {
            if deny_rule.behavior != PermissionRuleBehavior::Deny {
                continue;
            }
            // The deny must have strictly higher precedence (lower numeric value).
            if deny_rule.source.precedence() >= allow_rule.source.precedence() {
                continue;
            }
            // The deny covers the same tool when its pattern is "*" (wildcard)
            // or an exact match on the tool name.
            let deny_tool = deny_rule.tool.trim();
            if deny_tool == "*" || deny_tool == allow_rule.tool.trim() {
                shadowed.push((allow_idx, deny_idx));
            }
        }
    }

    shadowed
}

/// Stores settings store
#[derive(Clone, Debug)]
pub struct SettingsStore {
    paths: StoragePaths,
}

impl SettingsStore {
    /// Creates a new value
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
        }
    }
    /// Handles paths
    #[must_use]
    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    /// Handles read
    pub fn read(&self) -> Result<AgentSettings> {
        let path = self.paths.settings_path();
        if !path.exists() {
            return Ok(AgentSettings::default());
        }

        serde_json::from_str(&fs::read_to_string(path)?).map_err(Into::into)
    }

    /// Handles write
    pub fn write(&self, settings: &AgentSettings) -> Result<()> {
        fs::create_dir_all(self.paths.config_dir())?;
        write_json_atomically(&self.paths.settings_path(), settings)
    }
}
/// Stores credential store
#[derive(Clone, Debug)]
pub struct CredentialStore {
    paths: StoragePaths,
}

impl CredentialStore {
    /// Creates a new value
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: StoragePaths::new(base_dir),
        }
    }

    /// Handles read
    pub fn read(&self) -> Result<StoredCredentials> {
        let path = self.paths.credentials_path();
        if !path.exists() {
            return Ok(StoredCredentials::default());
        }

        serde_json::from_str(&fs::read_to_string(path)?).map_err(Into::into)
    }

    /// Handles write
    pub fn write(&self, credentials: &StoredCredentials) -> Result<()> {
        fs::create_dir_all(self.paths.config_dir())?;
        write_json_atomically(&self.paths.credentials_path(), credentials)
    }

    /// Handles set api key
    pub fn set_api_key(&self, provider: &str, api_key: impl Into<String>) -> Result<()> {
        let mut credentials = self.read()?;
        credentials.providers.insert(
            provider.to_string(),
            AuthMaterial::ApiKey {
                key: api_key.into(),
            },
        );
        self.write(&credentials)
    }

    /// Handles set oauth token
    pub fn set_oauth_token(
        &self,
        provider: &str,
        access_token: impl Into<String>,
        refresh_token: Option<String>,
        expires_at: Option<time::OffsetDateTime>,
    ) -> Result<()> {
        let mut credentials = self.read()?;
        credentials.providers.insert(
            provider.to_string(),
            AuthMaterial::OAuth {
                access_token: Some(access_token.into()),
                refresh_token,
                expires_at,
            },
        );
        self.write(&credentials)
    }

    /// Handles remove
    pub fn remove(&self, provider: &str) -> Result<bool> {
        let mut credentials = self.read()?;
        let removed = credentials.providers.remove(provider).is_some();
        self.write(&credentials)?;
        Ok(removed)
    }
}

/// Handles require storage dir
pub fn require_storage_dir(storage_dir: Option<PathBuf>) -> Result<PathBuf> {
    storage_dir.ok_or_else(|| {
        WonderError::validation(
            "this command requires --storage-dir so provider settings can be persisted",
        )
    })
}

#[cfg(test)]
mod tests {
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    // ---------------------------------------------------------------------------
    // SettingsStore / CredentialStore (pre-existing tests)
    // ---------------------------------------------------------------------------

    #[test]
    fn settings_store_round_trips_selection() {
        let dir = unique_test_dir("agent-settings");
        let store = SettingsStore::new(&dir);
        let settings = AgentSettings {
            selected_provider: Some("openai".into()),
            selected_model: Some("gpt-4.1".into()),
            theme: Some("midnight".into()),
            output_style: Some("plain".into()),
            ..AgentSettings::default()
        };

        store.write(&settings).expect("write settings");

        assert_eq!(store.read().expect("read settings"), settings);
    }

    #[test]
    fn settings_store_round_trips_env_vars() {
        let dir = unique_test_dir("agent-settings-env-vars");
        let store = SettingsStore::new(&dir);
        let settings = AgentSettings {
            env_vars: HashMap::from([
                ("ANTHROPIC_API_KEY".into(), "secret".into()),
                ("WONDER_MODEL".into(), "sonnet".into()),
            ]),
            ..AgentSettings::default()
        };

        store.write(&settings).expect("write settings");

        assert_eq!(store.read().expect("read settings"), settings);
    }

    #[test]
    fn credential_store_round_trips_api_keys() {
        let dir = unique_test_dir("agent-credentials");
        let store = CredentialStore::new(&dir);

        store.set_api_key("openai", "secret").expect("store key");

        let loaded = store.read().expect("read credentials");
        assert!(matches!(
            loaded.providers.get("openai"),
            Some(AuthMaterial::ApiKey { key }) if key == "secret"
        ));
    }

    #[test]
    fn credential_store_round_trips_oauth_tokens() {
        let dir = unique_test_dir("agent-oauth-credentials");
        let store = CredentialStore::new(&dir);

        store
            .set_oauth_token("copilot", "oauth-secret", None, None)
            .expect("store oauth token");

        let loaded = store.read().expect("read credentials");
        assert!(matches!(
            loaded.providers.get("copilot"),
            Some(AuthMaterial::OAuth {
                access_token: Some(token),
                refresh_token: None,
                ..
            }) if token == "oauth-secret"
        ));
    }

    #[test]
    fn require_storage_dir_rejects_missing_paths() {
        let error = require_storage_dir(None).expect_err("missing storage dir");
        assert!(error.to_string().contains("--storage-dir"));
    }

    // ---------------------------------------------------------------------------
    // SettingsHierarchy helpers
    // ---------------------------------------------------------------------------

    fn write_layer(dir: &Path, content: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("settings.json"), content).unwrap();
    }

    // ---------------------------------------------------------------------------
    // SettingsHierarchy – only user_dir present
    // ---------------------------------------------------------------------------

    #[test]
    fn hierarchy_load_user_only() {
        let base = unique_test_dir("hierarchy-user-only");
        let user_dir = base.join("user");
        write_layer(
            &user_dir,
            r#"{"selected_provider":"anthropic","allow_tools":["bash"]}"#,
        );

        let hier = SettingsHierarchy::load(None, &user_dir, None).expect("load hierarchy");

        assert_eq!(hier.layers.len(), 1);
        assert_eq!(hier.merged.selected_provider.as_deref(), Some("anthropic"));
        assert_eq!(hier.permission_rules.len(), 1);
        assert_eq!(
            hier.permission_rules[0].behavior,
            PermissionRuleBehavior::Allow
        );
        assert_eq!(hier.permission_rules[0].tool, "bash");
        assert_eq!(hier.permission_rules[0].source, PermissionRuleSource::User);
    }

    // ---------------------------------------------------------------------------
    // SettingsHierarchy – all three layers, verify merge precedence
    // ---------------------------------------------------------------------------

    #[test]
    fn hierarchy_merge_precedence() {
        let base = unique_test_dir("hierarchy-merge-precedence");

        // Policy: selected_provider = "anthropic", deny bash
        let policy_dir = base.join("policy");
        write_layer(
            &policy_dir,
            r#"{"selected_provider":"anthropic","deny_tools":["bash"]}"#,
        );

        // User: selected_provider = "openai" (should lose to policy), allow file_read
        let user_dir = base.join("user");
        write_layer(
            &user_dir,
            r#"{"selected_provider":"openai","allow_tools":["file_read"]}"#,
        );

        // Project: selected_model (no higher layer set it – should propagate)
        let project_dir = base.join("project");
        write_layer(
            &project_dir,
            r#"{"selected_model":"claude-3-7-sonnet-20250219","theme":"midnight","output_style":"plain"}"#,
        );

        let hier = SettingsHierarchy::load(Some(&policy_dir), &user_dir, Some(&project_dir))
            .expect("load hierarchy");

        assert_eq!(hier.layers.len(), 3);

        // Policy wins for selected_provider
        assert_eq!(
            hier.merged.selected_provider.as_deref(),
            Some("anthropic"),
            "policy provider should win"
        );

        // Project's model propagates since no higher layer set it
        assert_eq!(
            hier.merged.selected_model.as_deref(),
            Some("claude-3-7-sonnet-20250219"),
            "project model should propagate"
        );
        assert_eq!(
            hier.merged.theme.as_deref(),
            Some("midnight"),
            "project theme should propagate"
        );
        assert_eq!(
            hier.merged.output_style.as_deref(),
            Some("plain"),
            "project output style should propagate"
        );

        // Two permission rules: deny bash (policy) + allow file_read (user)
        assert_eq!(hier.permission_rules.len(), 2);

        let deny_rule = hier
            .permission_rules
            .iter()
            .find(|r| r.behavior == PermissionRuleBehavior::Deny)
            .expect("deny rule");
        assert_eq!(deny_rule.tool, "bash");
        assert_eq!(deny_rule.source, PermissionRuleSource::Policy);

        let allow_rule = hier
            .permission_rules
            .iter()
            .find(|r| r.behavior == PermissionRuleBehavior::Allow)
            .expect("allow rule");
        assert_eq!(allow_rule.tool, "file_read");
        assert_eq!(allow_rule.source, PermissionRuleSource::User);
    }

    #[test]
    fn hierarchy_merge_env_vars_respects_precedence() {
        let base = unique_test_dir("hierarchy-merge-env-vars");

        let policy_dir = base.join("policy");
        write_layer(
            &policy_dir,
            r#"{"env_vars":{"SHARED":"policy","POLICY_ONLY":"yes"}}"#,
        );

        let user_dir = base.join("user");
        write_layer(
            &user_dir,
            r#"{"env_vars":{"SHARED":"user","USER_ONLY":"yes"}}"#,
        );

        let project_dir = base.join("project");
        write_layer(
            &project_dir,
            r#"{"env_vars":{"SHARED":"project","PROJECT_ONLY":"yes"}}"#,
        );

        let hierarchy = SettingsHierarchy::load(Some(&policy_dir), &user_dir, Some(&project_dir))
            .expect("load hierarchy");

        assert_eq!(
            hierarchy.merged.env_vars.get("SHARED"),
            Some(&"policy".into())
        );
        assert_eq!(
            hierarchy.merged.env_vars.get("POLICY_ONLY"),
            Some(&"yes".into())
        );
        assert_eq!(
            hierarchy.merged.env_vars.get("USER_ONLY"),
            Some(&"yes".into())
        );
        assert_eq!(
            hierarchy.merged.env_vars.get("PROJECT_ONLY"),
            Some(&"yes".into())
        );
    }

    // ---------------------------------------------------------------------------
    // detect_shadowed_rules
    // ---------------------------------------------------------------------------

    #[test]
    fn detect_shadowed_rules_finds_masked_allow() {
        // Policy deny on "bash" shadows a User allow on "bash".
        let rules = vec![
            PermissionRule::new(
                "bash",
                PermissionRuleBehavior::Allow,
                PermissionRuleSource::User,
            ),
            PermissionRule::new(
                "bash",
                PermissionRuleBehavior::Deny,
                PermissionRuleSource::Policy,
            ),
        ];

        let shadowed = detect_shadowed_rules(&rules);
        // Allow at index 0 is shadowed by Deny at index 1.
        assert_eq!(shadowed, vec![(0, 1)]);
    }

    #[test]
    fn detect_shadowed_rules_no_shadow_when_allow_is_higher_precedence() {
        // Policy allow on "bash" (precedence 0) is NOT shadowed by a User deny
        // (precedence 6), because the deny has lower precedence than the allow.
        let rules = vec![
            PermissionRule::new(
                "bash",
                PermissionRuleBehavior::Allow,
                PermissionRuleSource::Policy,
            ),
            PermissionRule::new(
                "bash",
                PermissionRuleBehavior::Deny,
                PermissionRuleSource::User,
            ),
        ];

        assert!(detect_shadowed_rules(&rules).is_empty());
    }

    #[test]
    fn detect_shadowed_rules_wildcard_deny_shadows_allow() {
        // A wildcard policy deny shadows a user allow on any specific tool.
        let rules = vec![
            PermissionRule::new(
                "file_read",
                PermissionRuleBehavior::Allow,
                PermissionRuleSource::User,
            ),
            PermissionRule::new(
                "*",
                PermissionRuleBehavior::Deny,
                PermissionRuleSource::Policy,
            ),
        ];

        let shadowed = detect_shadowed_rules(&rules);
        assert_eq!(shadowed, vec![(0, 1)]);
    }

    #[test]
    fn detect_shadowed_rules_returns_empty_when_no_shadowing() {
        // Two allow rules from different sources – no shadowing.
        let rules = vec![
            PermissionRule::new(
                "bash",
                PermissionRuleBehavior::Allow,
                PermissionRuleSource::User,
            ),
            PermissionRule::new(
                "file_read",
                PermissionRuleBehavior::Allow,
                PermissionRuleSource::Project,
            ),
        ];

        assert!(detect_shadowed_rules(&rules).is_empty());
    }
}
