//! Shared hook inventory, fingerprinting, and trust-state helpers.

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{Result, WonderError, fingerprint_json};
use wonder_of_u_storage::StoragePaths;

const HOOK_TRUST_SCHEMA_VERSION: u16 = 1;

/// Parsed top-level `hooks.json` config.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct HooksConfig {
    /// Global kill switch for every hook.
    #[serde(default)]
    pub disable_all_hooks: bool,
    /// When set, only hook actions marked `managed: true` are eligible to run.
    #[serde(default)]
    pub allow_managed_hooks_only: bool,
    /// Hook matchers keyed by event name.
    #[serde(default)]
    pub hooks: BTreeMap<String, Vec<HookMatcherConfig>>,
}

/// Parsed hook matcher config.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct HookMatcherConfig {
    /// Tool, source, or lifecycle matcher. Empty/missing means all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    /// Marks every action under this matcher as centrally managed.
    #[serde(default)]
    pub managed: bool,
    /// Raw action configs. Keeping the raw JSON preserves future fields in the
    /// trust fingerprint even while v1 only executes command actions.
    #[serde(default)]
    pub hooks: Vec<Value>,
}

/// Persisted trust decision for one hook location.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct HookTrustRecord {
    /// Fingerprint approved by the user. A mismatch means the hook changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trusted_fingerprint: Option<String>,
    /// Disabled hooks never run, even when their fingerprint is trusted.
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
}

/// Persisted hook trust ledger stored beside `hooks.json`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct HookTrustState {
    /// State-file schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    /// Decisions keyed by generated hook ID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hooks: BTreeMap<String, HookTrustRecord>,
}

impl Default for HookTrustState {
    fn default() -> Self {
        Self {
            schema_version: HOOK_TRUST_SCHEMA_VERSION,
            hooks: BTreeMap::new(),
        }
    }
}

/// Primary trust status for a configured hook.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HookTrustStatus {
    /// Current fingerprint has been trusted and the hook is enabled.
    Trusted,
    /// No trust decision exists for this hook location.
    Untrusted,
    /// The hook location is explicitly disabled.
    Disabled,
    /// A previous fingerprint was trusted, but the config has since changed.
    Changed,
}

impl HookTrustStatus {
    /// User-facing stable status label.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Untrusted => "untrusted",
            Self::Disabled => "disabled",
            Self::Changed => "changed",
        }
    }
}

/// One hook action with trust metadata and safe display fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HookEntry {
    /// Stable location ID used by `/hooks trust <id>`.
    pub id: String,
    /// Event name such as `PreToolUse`.
    pub event: String,
    /// Display matcher, normalized to `*` when absent.
    pub matcher: String,
    /// SHA-256 fingerprint of event, matcher, and raw action config.
    pub fingerprint: String,
    /// Action kind from the raw config.
    pub kind: String,
    /// Safe display target for the action.
    pub target: String,
    /// Optional `if` condition from the raw action config.
    pub condition: Option<String>,
    /// Whether this hook is marked as managed.
    pub managed: bool,
    /// Whether this runtime can execute the hook kind.
    pub supported: bool,
    /// Primary trust status.
    pub status: HookTrustStatus,
}

impl HookEntry {
    /// Returns true when the current hook may run if its event/matcher matches.
    pub(crate) const fn is_trusted_and_enabled(&self) -> bool {
        matches!(self.status, HookTrustStatus::Trusted)
    }

    /// Returns compact display tags including orthogonal state.
    pub(crate) fn status_tags(&self) -> Vec<&'static str> {
        let mut tags = vec![self.status.label()];
        if self.managed {
            tags.push("managed");
        }
        if !self.supported {
            tags.push("unsupported");
        }
        tags
    }
}

/// Complete hook inventory for the current storage directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HooksInventory {
    /// Resolved `hooks.json` path.
    pub config_path: PathBuf,
    /// Resolved `hooks-state.json` path.
    pub state_path: PathBuf,
    /// Number of configured events.
    pub event_count: usize,
    /// Number of configured matchers.
    pub matcher_count: usize,
    /// Global hook kill switch.
    pub disable_all_hooks: bool,
    /// Managed-only switch.
    pub allow_managed_hooks_only: bool,
    /// Flattened hook entries.
    pub entries: Vec<HookEntry>,
}

/// Resolves the hooks config file path.
pub(crate) fn resolve_hooks_path(storage_dir: Option<&Path>) -> PathBuf {
    config_dir(storage_dir).join("hooks.json")
}

/// Resolves the hooks trust-state file path.
pub(crate) fn resolve_hooks_state_path(storage_dir: Option<&Path>) -> PathBuf {
    config_dir(storage_dir).join("hooks-state.json")
}

/// Loads `hooks.json`, returning defaults when the file is absent.
pub(crate) fn load_hooks_config(storage_dir: Option<&Path>) -> Result<HooksConfig> {
    let path = resolve_hooks_path(storage_dir);
    if !path.exists() {
        return Ok(HooksConfig::default());
    }
    let content = fs::read_to_string(&path)?;
    serde_json::from_str(&content).map_err(|error| {
        WonderError::validation(format!(
            "invalid hooks config `{}`: {error}",
            path.display()
        ))
    })
}

/// Loads persisted hook trust state, returning defaults when absent.
pub(crate) fn load_hook_trust_state(storage_dir: Option<&Path>) -> Result<HookTrustState> {
    let path = resolve_hooks_state_path(storage_dir);
    if !path.exists() {
        return Ok(HookTrustState::default());
    }
    let content = fs::read_to_string(&path)?;
    serde_json::from_str(&content).map_err(|error| {
        WonderError::validation(format!("invalid hooks state `{}`: {error}", path.display()))
    })
}

/// Builds the display/runtime inventory from config and trust state.
pub(crate) fn load_hooks_inventory(storage_dir: Option<&Path>) -> Result<HooksInventory> {
    let config = load_hooks_config(storage_dir)?;
    let state = load_hook_trust_state(storage_dir)?;
    let event_count = config.hooks.len();
    let matcher_count = config.hooks.values().map(Vec::len).sum::<usize>();
    let mut entries = Vec::new();

    for (event, matchers) in &config.hooks {
        for (matcher_index, matcher) in matchers.iter().enumerate() {
            for (hook_index, action) in matcher.hooks.iter().enumerate() {
                entries.push(build_hook_entry(
                    event,
                    matcher_index,
                    hook_index,
                    matcher,
                    action,
                    &state,
                ));
            }
        }
    }

    Ok(HooksInventory {
        config_path: resolve_hooks_path(storage_dir),
        state_path: resolve_hooks_state_path(storage_dir),
        event_count,
        matcher_count,
        disable_all_hooks: config.disable_all_hooks,
        allow_managed_hooks_only: config.allow_managed_hooks_only,
        entries,
    })
}

/// Builds a single hook entry. Runtime code uses this to apply the same trust
/// decision as `/hooks list`.
pub(crate) fn build_hook_entry(
    event: &str,
    matcher_index: usize,
    hook_index: usize,
    matcher: &HookMatcherConfig,
    action: &Value,
    state: &HookTrustState,
) -> HookEntry {
    let id = hook_id(event, matcher_index, hook_index);
    let fingerprint = hook_fingerprint(event, matcher, action);
    let record = state.hooks.get(&id);
    let status = hook_status(record, &fingerprint);
    let (kind, target) = hook_kind_target(action);
    let condition = action.get("if").and_then(Value::as_str).map(str::to_string);
    let managed = matcher.managed
        || action
            .get("managed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let supported = kind == "command";

    HookEntry {
        id,
        event: event.to_string(),
        matcher: matcher_label(matcher.matcher.as_deref()),
        fingerprint,
        kind,
        target,
        condition,
        managed,
        supported,
        status,
    }
}

/// Trusts the current fingerprint for a hook ID and enables it.
pub(crate) fn trust_hook(storage_dir: Option<&Path>, id: &str) -> Result<HookEntry> {
    let inventory = load_hooks_inventory(storage_dir)?;
    let entry = find_entry(&inventory, id)?.clone();
    let mut state = load_hook_trust_state(storage_dir)?;
    state.schema_version = HOOK_TRUST_SCHEMA_VERSION;
    let record = state.hooks.entry(entry.id.clone()).or_default();
    record.trusted_fingerprint = Some(entry.fingerprint.clone());
    record.disabled = false;
    write_hook_trust_state(&inventory.state_path, &state)?;

    let mut trusted = entry;
    trusted.status = HookTrustStatus::Trusted;
    Ok(trusted)
}

/// Enables or disables a configured hook ID.
pub(crate) fn set_hook_disabled(
    storage_dir: Option<&Path>,
    id: &str,
    disabled: bool,
) -> Result<HookEntry> {
    let inventory = load_hooks_inventory(storage_dir)?;
    let entry = find_entry(&inventory, id)?.clone();
    let mut state = load_hook_trust_state(storage_dir)?;
    state.schema_version = HOOK_TRUST_SCHEMA_VERSION;

    if disabled {
        state.hooks.entry(entry.id.clone()).or_default().disabled = true;
    } else if let Some(record) = state.hooks.get_mut(&entry.id) {
        record.disabled = false;
        if record.trusted_fingerprint.is_none() {
            state.hooks.remove(&entry.id);
        }
    }

    write_hook_trust_state(&inventory.state_path, &state)?;
    let updated = load_hooks_inventory(storage_dir)?
        .entries
        .into_iter()
        .find(|candidate| candidate.id == entry.id)
        .expect("hook existed before state update");
    Ok(updated)
}

fn config_dir(storage_dir: Option<&Path>) -> PathBuf {
    storage_dir
        .map(StoragePaths::new)
        .map(|paths| paths.config_dir())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".wonder-of-u")
                .join("config")
        })
}

fn hook_id(event: &str, matcher_index: usize, hook_index: usize) -> String {
    format!("{}.{}.{}", slug_event(event), matcher_index, hook_index)
}

fn slug_event(event: &str) -> String {
    let slug = event
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if slug.is_empty() { "hook".into() } else { slug }
}

fn hook_fingerprint(event: &str, matcher: &HookMatcherConfig, action: &Value) -> String {
    let material = json!({
        "event": event,
        "matcher": matcher.matcher,
        "matcher_managed": matcher.managed,
        "action": action,
    });
    fingerprint_json(&material).expect("hook fingerprint material is serializable JSON")
}

fn hook_status(record: Option<&HookTrustRecord>, fingerprint: &str) -> HookTrustStatus {
    let Some(record) = record else {
        return HookTrustStatus::Untrusted;
    };
    if record.disabled {
        return HookTrustStatus::Disabled;
    }
    match record.trusted_fingerprint.as_deref() {
        Some(trusted) if trusted == fingerprint => HookTrustStatus::Trusted,
        Some(_) => HookTrustStatus::Changed,
        None => HookTrustStatus::Untrusted,
    }
}

fn matcher_label(matcher: Option<&str>) -> String {
    matcher
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("*")
        .to_string()
}

fn hook_kind_target(action: &Value) -> (String, String) {
    match action.get("type").and_then(Value::as_str) {
        Some("command") => (
            "command".into(),
            action
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        ),
        Some("prompt") => (
            "prompt".into(),
            action
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        ),
        Some("agent") => (
            "agent".into(),
            action
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        ),
        Some("http") => (
            "http".into(),
            action
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        ),
        Some(other) => (other.to_string(), String::new()),
        None => ("unknown".into(), String::new()),
    }
}

fn find_entry<'a>(inventory: &'a HooksInventory, id: &str) -> Result<&'a HookEntry> {
    inventory
        .entries
        .iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| WonderError::not_found("hook", id))
}

fn write_hook_trust_state(path: &Path, state: &HookTrustState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_json_atomically(path, state)
}

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

const fn default_schema_version() -> u16 {
    HOOK_TRUST_SCHEMA_VERSION
}

const fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(pattern: &str) -> HookMatcherConfig {
        HookMatcherConfig {
            matcher: Some(pattern.into()),
            managed: false,
            hooks: Vec::new(),
        }
    }

    #[test]
    fn hook_fingerprint_is_stable_for_json_key_order() {
        let left = json!({"type": "command", "command": "echo hi", "if": "true"});
        let right = json!({"if": "true", "command": "echo hi", "type": "command"});

        assert_eq!(
            hook_fingerprint("PreToolUse", &matcher("bash"), &left),
            hook_fingerprint("PreToolUse", &matcher("bash"), &right)
        );
    }

    #[test]
    fn hook_fingerprint_changes_when_command_changes() {
        let left = json!({"type": "command", "command": "echo hi"});
        let right = json!({"type": "command", "command": "echo bye"});

        assert_ne!(
            hook_fingerprint("PreToolUse", &matcher("bash"), &left),
            hook_fingerprint("PreToolUse", &matcher("bash"), &right)
        );
    }

    #[test]
    fn hook_fingerprint_changes_when_matcher_changes() {
        let action = json!({"type": "command", "command": "echo hi"});

        assert_ne!(
            hook_fingerprint("PreToolUse", &matcher("bash"), &action),
            hook_fingerprint("PreToolUse", &matcher("file_read"), &action)
        );
    }

    #[test]
    fn trusted_fingerprint_mismatch_marks_changed() {
        let matcher = matcher("bash");
        let action = json!({"type": "command", "command": "echo hi"});
        let original = hook_fingerprint("PreToolUse", &matcher, &action);
        let state = HookTrustState {
            hooks: BTreeMap::from([(
                "pretooluse.0.0".into(),
                HookTrustRecord {
                    trusted_fingerprint: Some(original),
                    disabled: false,
                },
            )]),
            ..HookTrustState::default()
        };
        let changed = json!({"type": "command", "command": "echo bye"});

        let entry = build_hook_entry("PreToolUse", 0, 0, &matcher, &changed, &state);

        assert_eq!(entry.status, HookTrustStatus::Changed);
    }
}
