//! Runtime permissions loader for wonder-of-u.
//!
//! [`PermissionsLoader`] bridges the on-disk settings hierarchy with the
//! in-process [`ToolPermissionContext`].  It loads rules from the three-layer
//! hierarchy (policy → user → project) and exposes helpers to persist or
//! remove individual rules without losing unrelated settings.
//!
//! # Usage
//!
//! ```rust,ignore
//! let loader = PermissionsLoader::new(storage_dir, project_dir);
//! let (settings, rules) = loader.load()?;
//! // inject `rules` into ToolPermissionContext
//! ```

use std::{
    fs,
    path::{Path, PathBuf},
};

use wonder_of_u_core::{
    Result,
    permission::{PermissionRule, PermissionRuleBehavior, PermissionRuleSource},
};

use crate::config::{AgentSettings, SettingsHierarchy};

// ── PermissionsLoader ────────────────────────────────────────────────────────

/// Loads the merged permission ruleset from the three-layer settings hierarchy.
pub struct PermissionsLoader {
    policy_dir: Option<PathBuf>,
    user_dir: PathBuf,
    project_dir: Option<PathBuf>,
}

impl PermissionsLoader {
    /// Creates a loader.
    ///
    /// * `user_dir` — the user-level config directory (e.g. `~/.wonder-of-u/config`)
    /// * `project_dir` — optional project-level config directory (e.g. `.claude/`)
    /// * `policy_dir` — optional managed policy directory (set by MDM / org policy)
    pub fn new(
        user_dir: impl Into<PathBuf>,
        project_dir: Option<impl Into<PathBuf>>,
        policy_dir: Option<impl Into<PathBuf>>,
    ) -> Self {
        Self {
            policy_dir: policy_dir.map(Into::into),
            user_dir: user_dir.into(),
            project_dir: project_dir.map(Into::into),
        }
    }

    /// Loads settings and returns `(merged_settings, permission_rules)`.
    ///
    /// Permission rules are derived from `allow_tools` / `deny_tools` in each
    /// settings layer, combined with any `managed_permission_rules` from the
    /// policy layer.
    pub fn load(&self) -> Result<(AgentSettings, Vec<PermissionRule>)> {
        let hierarchy = SettingsHierarchy::load(
            self.policy_dir.as_deref(),
            &self.user_dir,
            self.project_dir.as_deref(),
        )?;
        Ok((hierarchy.merged, hierarchy.permission_rules))
    }

    /// Returns just the merged `PermissionRule`s.
    pub fn load_rules(&self) -> Result<Vec<PermissionRule>> {
        Ok(self.load()?.1)
    }
}

// ── Persistence helpers ──────────────────────────────────────────────────────

/// Appends a single [`PermissionRule`] to the appropriate `settings.json`.
///
/// The target file is chosen by `source`:
/// - `User` → `user_dir/settings.json`
/// - `Project` → `project_dir/settings.json`
/// - Other sources are read-only in the context of user-initiated edits.
///
/// If the rule already exists (matching tool + behavior) it is not duplicated.
pub fn persist_permission_rule(
    rule: &PermissionRule,
    user_dir: &Path,
    project_dir: Option<&Path>,
) -> Result<()> {
    let dir = settings_dir_for_source(rule.source, user_dir, project_dir)?;
    let path = dir.join("settings.json");

    let mut settings = load_settings_mutable(&path)?;

    let list = if rule.behavior == PermissionRuleBehavior::Allow {
        &mut settings.allow_tools
    } else {
        &mut settings.deny_tools
    };

    if !list.contains(&rule.tool) {
        list.push(rule.tool.clone());
    }

    write_settings_atomically(&path, &settings)
}

/// Removes a rule from `settings.json` for the given `source`.
///
/// A no-op if the rule does not exist in the file.
pub fn remove_permission_rule(
    tool: &str,
    behavior: PermissionRuleBehavior,
    source: PermissionRuleSource,
    user_dir: &Path,
    project_dir: Option<&Path>,
) -> Result<()> {
    let dir = settings_dir_for_source(source, user_dir, project_dir)?;
    let path = dir.join("settings.json");
    if !path.exists() {
        return Ok(());
    }

    let mut settings = load_settings_mutable(&path)?;

    let list = if behavior == PermissionRuleBehavior::Allow {
        &mut settings.allow_tools
    } else {
        &mut settings.deny_tools
    };

    list.retain(|t| t != tool);

    write_settings_atomically(&path, &settings)
}

// ── Private helpers ──────────────────────────────────────────────────────────

fn settings_dir_for_source(
    source: PermissionRuleSource,
    user_dir: &Path,
    project_dir: Option<&Path>,
) -> Result<PathBuf> {
    match source {
        PermissionRuleSource::User => Ok(user_dir.to_owned()),
        PermissionRuleSource::Project | PermissionRuleSource::Local => {
            project_dir.map(Path::to_owned).ok_or_else(|| {
                wonder_of_u_core::WonderError::validation(
                    "project_dir is required for project-level permission rules",
                )
            })
        }
        other => Err(wonder_of_u_core::WonderError::validation(format!(
            "cannot persist permission rules for source {other:?}"
        ))),
    }
}

fn load_settings_mutable(path: &Path) -> Result<AgentSettings> {
    if !path.exists() {
        return Ok(AgentSettings::default());
    }
    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

fn write_settings_atomically(path: &Path, settings: &AgentSettings) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let next = path.with_extension("json.next");
    let json = serde_json::to_string_pretty(settings)?;
    fs::write(&next, format!("{json}\n"))?;
    fs::rename(&next, path)?;
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;
    use wonder_of_u_core::permission::PermissionRuleBehavior;

    fn user_dir(tmp: &TempDir) -> PathBuf {
        tmp.path().join("user")
    }

    fn project_dir(tmp: &TempDir) -> PathBuf {
        tmp.path().join("project")
    }

    fn write_settings(dir: &Path, settings: &AgentSettings) {
        fs::create_dir_all(dir).unwrap();
        let json = serde_json::to_string_pretty(settings).unwrap();
        fs::write(dir.join("settings.json"), format!("{json}\n")).unwrap();
    }

    #[test]
    fn load_empty_dirs_returns_no_rules() {
        let tmp = TempDir::new().unwrap();
        let loader = PermissionsLoader::new(user_dir(&tmp), None::<PathBuf>, None::<PathBuf>);
        let rules = loader.load_rules().unwrap();
        assert!(rules.is_empty(), "expected no rules, got {rules:?}");
    }

    #[test]
    fn load_user_allow_tools_produces_allow_rules() {
        let tmp = TempDir::new().unwrap();
        let u = user_dir(&tmp);
        write_settings(
            &u,
            &AgentSettings {
                allow_tools: vec!["bash".into(), "file_read".into()],
                ..AgentSettings::default()
            },
        );
        let loader = PermissionsLoader::new(u, None::<PathBuf>, None::<PathBuf>);
        let rules = loader.load_rules().unwrap();
        assert!(
            rules
                .iter()
                .any(|r| r.tool == "bash" && r.behavior == PermissionRuleBehavior::Allow)
        );
        assert!(
            rules
                .iter()
                .any(|r| r.tool == "file_read" && r.behavior == PermissionRuleBehavior::Allow)
        );
    }

    #[test]
    fn load_project_deny_tools_produces_deny_rules() {
        let tmp = TempDir::new().unwrap();
        let p = project_dir(&tmp);
        write_settings(
            &p,
            &AgentSettings {
                deny_tools: vec!["web_search".into()],
                ..AgentSettings::default()
            },
        );
        let loader = PermissionsLoader::new(user_dir(&tmp), Some(p), None::<PathBuf>);
        let rules = loader.load_rules().unwrap();
        assert!(
            rules
                .iter()
                .any(|r| r.tool == "web_search" && r.behavior == PermissionRuleBehavior::Deny)
        );
    }

    #[test]
    fn persist_adds_allow_tool_to_settings() {
        let tmp = TempDir::new().unwrap();
        let u = user_dir(&tmp);
        fs::create_dir_all(&u).unwrap();
        let rule = PermissionRule::new(
            "bash",
            PermissionRuleBehavior::Allow,
            PermissionRuleSource::User,
        );
        persist_permission_rule(&rule, &u, None).unwrap();
        let s: AgentSettings =
            serde_json::from_str(&fs::read_to_string(u.join("settings.json")).unwrap()).unwrap();
        assert!(s.allow_tools.contains(&"bash".to_string()));
    }

    #[test]
    fn persist_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let u = user_dir(&tmp);
        fs::create_dir_all(&u).unwrap();
        let rule = PermissionRule::new(
            "bash",
            PermissionRuleBehavior::Allow,
            PermissionRuleSource::User,
        );
        persist_permission_rule(&rule, &u, None).unwrap();
        persist_permission_rule(&rule, &u, None).unwrap();
        let s: AgentSettings =
            serde_json::from_str(&fs::read_to_string(u.join("settings.json")).unwrap()).unwrap();
        assert_eq!(s.allow_tools.iter().filter(|t| *t == "bash").count(), 1);
    }

    #[test]
    fn remove_deletes_rule_from_settings() {
        let tmp = TempDir::new().unwrap();
        let u = user_dir(&tmp);
        write_settings(
            &u,
            &AgentSettings {
                allow_tools: vec!["bash".into(), "file_read".into()],
                ..AgentSettings::default()
            },
        );
        remove_permission_rule(
            "bash",
            PermissionRuleBehavior::Allow,
            PermissionRuleSource::User,
            &u,
            None,
        )
        .unwrap();
        let s: AgentSettings =
            serde_json::from_str(&fs::read_to_string(u.join("settings.json")).unwrap()).unwrap();
        assert!(!s.allow_tools.contains(&"bash".to_string()));
        assert!(s.allow_tools.contains(&"file_read".to_string()));
    }

    #[test]
    fn remove_is_noop_when_rule_absent() {
        let tmp = TempDir::new().unwrap();
        let u = user_dir(&tmp);
        // No settings.json written – should not error.
        remove_permission_rule(
            "bash",
            PermissionRuleBehavior::Allow,
            PermissionRuleSource::User,
            &u,
            None,
        )
        .unwrap();
    }

    #[test]
    fn persist_to_project_requires_project_dir() {
        let tmp = TempDir::new().unwrap();
        let u = user_dir(&tmp);
        let rule = PermissionRule::new(
            "bash",
            PermissionRuleBehavior::Deny,
            PermissionRuleSource::Project,
        );
        let result = persist_permission_rule(&rule, &u, None);
        assert!(result.is_err());
    }
}
