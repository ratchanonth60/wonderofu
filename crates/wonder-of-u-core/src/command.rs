use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    AdditionalWorkingDirectory, FeatureFlag, FeatureSet, PermissionMode, Result, SessionId,
    WonderError,
};
/// Enumerates command kind
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    /// Represents prompt
    Prompt,
    /// Represents local
    Local,
    /// Represents tui
    Tui,
    /// Represents non interactive
    NonInteractive,
    /// Represents resume entrypoint
    ResumeEntrypoint,
}
/// Enumerates command source
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandSource {
    /// Represents built in
    BuiltIn,
    /// Represents skill
    Skill,
    /// Represents plugin
    Plugin,
    /// Represents workflow
    Workflow,
    /// Represents mcp
    Mcp,
    /// Represents dynamic
    Dynamic,
}
/// Represents command spec
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandSpec {
    /// Stores the name
    pub name: String,
    /// Stores the aliases
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Stores the description
    pub description: String,
    /// Stores the kind
    pub kind: CommandKind,
    /// Stores the source
    pub source: CommandSource,
    /// Stores the required features
    #[serde(default)]
    pub required_features: BTreeSet<FeatureFlag>,
    /// Stores the hidden
    #[serde(default)]
    pub hidden: bool,
    /// Stores the requires auth
    #[serde(default)]
    pub requires_auth: bool,
    /// Stores the interactive only
    #[serde(default)]
    pub interactive_only: bool,
    /// Optional argument hint shown next to the command name in slash-command
    /// autocomplete (e.g. `[on|off]` or `<color|default>`).
    ///
    /// `None` means no hint is rendered; the field is omitted from serialised
    /// output when absent so existing stored specs remain valid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
}

impl CommandSpec {
    /// Creates a new value
    #[must_use]
    pub fn new(name: impl Into<String>, description: impl Into<String>, kind: CommandKind) -> Self {
        Self {
            name: name.into(),
            aliases: Vec::new(),
            description: description.into(),
            kind,
            source: CommandSource::BuiltIn,
            required_features: BTreeSet::new(),
            hidden: false,
            requires_auth: false,
            interactive_only: false,
            argument_hint: None,
        }
    }

    /// Sets the argument hint shown next to the command name in slash-command
    /// autocomplete (e.g. `[on|off]` or `<color|default>`).
    ///
    /// # Examples
    ///
    /// ```
    /// use wonder_of_u_core::{CommandSpec, CommandKind};
    ///
    /// let spec = CommandSpec::new("fast", "Toggle fast mode", CommandKind::Local)
    ///     .with_argument_hint("[on|off]");
    /// assert_eq!(spec.argument_hint.as_deref(), Some("[on|off]"));
    /// ```
    #[must_use]
    pub fn with_argument_hint(mut self, hint: impl Into<String>) -> Self {
        self.argument_hint = Some(hint.into());
        self
    }

    /// Validates the value
    pub fn validate(&self) -> Result<()> {
        let canonical = normalize_name(&self.name)?;
        let mut seen = BTreeSet::from([canonical]);
        for alias in &self.aliases {
            let alias = normalize_name(alias)?;
            if !seen.insert(alias.clone()) {
                return Err(WonderError::validation(format!(
                    "duplicate command alias: {alias}"
                )));
            }
        }
        Ok(())
    }
    /// Returns whether available
    #[must_use]
    pub fn is_available(
        &self,
        features: &FeatureSet,
        authenticated: bool,
        interactive: bool,
    ) -> bool {
        features.contains_all(&self.required_features)
            && (!self.requires_auth || authenticated)
            && (!self.interactive_only || interactive)
    }
    /// Returns whether visible
    #[must_use]
    pub fn is_visible(&self, query: &CommandQuery) -> bool {
        (query.include_hidden || !self.hidden)
            && self.is_available(&query.features, query.authenticated, query.interactive)
    }
}
/// Represents command query
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandQuery {
    /// Stores the features
    pub features: FeatureSet,
    /// Stores the authenticated
    pub authenticated: bool,
    /// Stores the interactive
    pub interactive: bool,
    /// Stores the include hidden
    pub include_hidden: bool,
}

impl CommandQuery {
    /// Creates a new value
    #[must_use]
    pub fn new(features: FeatureSet) -> Self {
        Self {
            features,
            authenticated: false,
            interactive: false,
            include_hidden: false,
        }
    }
    /// Handles including hidden
    #[must_use]
    pub fn including_hidden(mut self) -> Self {
        self.include_hidden = true;
        self
    }
    /// Returns whether the query allows the item
    #[must_use]
    pub fn allows(&self, spec: &CommandSpec) -> bool {
        spec.is_available(&self.features, self.authenticated, self.interactive)
    }
}

impl From<&CommandContext> for CommandQuery {
    fn from(context: &CommandContext) -> Self {
        Self {
            features: context.features.clone(),
            authenticated: context.authenticated,
            interactive: context.interactive,
            include_hidden: false,
        }
    }
}
/// Represents command invocation
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandInvocation {
    /// Stores the name
    pub name: String,
    /// Stores the args
    pub args: String,
    /// Stores the raw
    pub raw: String,
}
/// Enumerates command output
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CommandOutput {
    /// Represents text
    Text(String),
    /// Represents enqueue prompt
    EnqueuePrompt(String),
    /// Represents open ui
    OpenUi(String),
    /// Represents exit requested
    ExitRequested,
    /// Represents noop
    Noop,
}
/// Represents command context
#[derive(Clone, Debug)]
pub struct CommandContext {
    /// Stores the session identifier
    pub session_id: SessionId,
    /// Stores the cwd
    pub cwd: PathBuf,
    /// Stores the features
    pub features: FeatureSet,
    /// Stores the authenticated
    pub authenticated: bool,
    /// Stores the interactive
    pub interactive: bool,
    /// Stores the permission mode
    pub permission_mode: PermissionMode,
    /// Stores the theme
    pub theme: Option<String>,
    /// Stores the session color
    pub session_color: Option<String>,
    /// Stores the effort level
    pub effort_level: Option<String>,
    /// Stores the brief mode
    pub brief_mode: bool,
    /// Stores the fast mode
    pub fast_mode: bool,
    /// Stores whether token-optimisation mode is enabled for the session.
    pub optimize_token_mode: bool,
    /// Stores the session tags
    pub session_tags: Vec<String>,
    /// Stores the additional working directories
    pub additional_working_directories: Vec<AdditionalWorkingDirectory>,
}
/// Defines command behavior
#[async_trait]
pub trait Command: Send + Sync {
    /// Returns the item specification
    fn spec(&self) -> CommandSpec;

    /// Executes the operation
    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput>;
}

#[derive(Clone)]
struct CommandEntry {
    spec: CommandSpec,
    command: Arc<dyn Command>,
}
/// Stores command registry
#[derive(Default)]
pub struct CommandRegistry {
    commands: BTreeMap<String, CommandEntry>,
    aliases: BTreeMap<String, String>,
    order: Vec<String>,
}

impl CommandRegistry {
    /// Creates a new value
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles register
    pub fn register(&mut self, command: Arc<dyn Command>) -> Result<()> {
        let spec = command.spec();
        spec.validate()?;
        let canonical = normalize_name(&spec.name)?;
        if self.commands.contains_key(&canonical) || self.aliases.contains_key(&canonical) {
            return Err(WonderError::validation(format!(
                "duplicate command: {canonical}"
            )));
        }

        for alias in &spec.aliases {
            let alias = normalize_name(alias)?;
            if self.commands.contains_key(&alias) || self.aliases.contains_key(&alias) {
                return Err(WonderError::validation(format!(
                    "duplicate command alias: {alias}"
                )));
            }
        }

        for alias in &spec.aliases {
            self.aliases
                .insert(normalize_name(alias)?, canonical.clone());
        }
        self.order.push(canonical.clone());
        self.commands
            .insert(canonical, CommandEntry { spec, command });
        Ok(())
    }
    /// Handles resolve
    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<Arc<dyn Command>> {
        self.resolve_entry(name)
            .map(|entry| Arc::clone(&entry.command))
    }
    /// Resolves enabled
    #[must_use]
    pub fn resolve_enabled(&self, name: &str, query: &CommandQuery) -> Option<Arc<dyn Command>> {
        let entry = self.resolve_entry(name)?;
        query
            .allows(&entry.spec)
            .then(|| Arc::clone(&entry.command))
    }
    /// Resolves spec
    #[must_use]
    pub fn resolve_spec(&self, name: &str) -> Option<CommandSpec> {
        self.resolve_entry(name).map(|entry| entry.spec.clone())
    }

    /// Handles all specs
    pub fn all_specs(&self) -> Vec<CommandSpec> {
        self.order
            .iter()
            .filter_map(|canonical| self.commands.get(canonical))
            .map(|entry| entry.spec.clone())
            .collect()
    }

    /// Handles available specs
    pub fn available_specs(&self, query: &CommandQuery) -> Vec<CommandSpec> {
        self.all_specs()
            .into_iter()
            .filter(|spec| query.allows(spec))
            .collect()
    }

    /// Handles visible specs
    pub fn visible_specs(&self, query: &CommandQuery) -> Vec<CommandSpec> {
        self.all_specs()
            .into_iter()
            .filter(|spec| spec.is_visible(query))
            .collect()
    }
    /// Handles len
    #[must_use]
    pub fn len(&self) -> usize {
        self.commands.len()
    }
    /// Returns whether empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    fn resolve_entry(&self, name: &str) -> Option<&CommandEntry> {
        let normalized = normalize_lookup(name)?;
        self.commands.get(&normalized).or_else(|| {
            self.aliases
                .get(&normalized)
                .and_then(|canonical| self.commands.get(canonical))
        })
    }
}
/// Parses slash command
#[must_use]
pub fn parse_slash_command(input: &str) -> Option<CommandInvocation> {
    let rest = input.strip_prefix('/')?.trim_start();
    if rest.is_empty() {
        return None;
    }

    let mut parts = rest.splitn(2, char::is_whitespace);
    let name = parts.next()?.to_ascii_lowercase();
    let args = parts.next().unwrap_or_default().trim().to_string();

    Some(CommandInvocation {
        name,
        args,
        raw: input.to_string(),
    })
}

fn normalize_name(name: &str) -> Result<String> {
    let normalized = normalize_lookup(name)
        .ok_or_else(|| WonderError::validation("command name cannot be empty"))?;
    if normalized.chars().any(char::is_whitespace) {
        return Err(WonderError::validation(format!(
            "command name contains whitespace: {name}"
        )));
    }
    Ok(normalized)
}

fn normalize_lookup(name: &str) -> Option<String> {
    let normalized = name.trim().trim_start_matches('/').to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NamedCommand {
        spec: CommandSpec,
    }

    impl NamedCommand {
        fn new(name: &str, description: &str) -> Self {
            Self {
                spec: CommandSpec::new(name, description, CommandKind::Local),
            }
        }
    }

    #[async_trait]
    impl Command for NamedCommand {
        fn spec(&self) -> CommandSpec {
            self.spec.clone()
        }

        async fn execute(
            &self,
            _context: CommandContext,
            _invocation: CommandInvocation,
        ) -> Result<CommandOutput> {
            Ok(CommandOutput::Text(self.spec.name.clone()))
        }
    }

    #[test]
    fn slash_command_parser_extracts_name_and_args() {
        let invocation = parse_slash_command("/model set sonnet").expect("slash command");
        assert_eq!(invocation.name, "model");
        assert_eq!(invocation.args, "set sonnet");
    }

    #[test]
    fn slash_command_parser_ignores_plain_text() {
        assert!(parse_slash_command("hello /help").is_none());
    }

    #[test]
    fn registry_resolves_aliases() {
        let mut registry = CommandRegistry::new();
        let mut command = NamedCommand::new("help", "show help");
        command.spec.aliases.push("h".into());
        registry.register(Arc::new(command)).expect("register");

        assert!(registry.resolve("help").is_some());
        assert!(registry.resolve("h").is_some());
        assert!(registry.resolve("/HELP").is_some());
    }

    #[test]
    fn visible_specs_preserve_registration_order_and_hide_internal_commands() {
        let mut registry = CommandRegistry::new();
        registry
            .register(Arc::new(NamedCommand::new("doctor", "diagnostics")))
            .expect("doctor");

        let mut hidden = NamedCommand::new("features", "feature flags");
        hidden.spec.hidden = true;
        registry.register(Arc::new(hidden)).expect("features");

        registry
            .register(Arc::new(NamedCommand::new("status", "runtime status")))
            .expect("status");

        let query = CommandQuery::new(FeatureSet::first_release());
        let available = registry.available_specs(&query);
        let visible = registry.visible_specs(&query);

        assert_eq!(
            available
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>(),
            vec!["doctor", "features", "status"]
        );
        assert_eq!(
            visible
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>(),
            vec!["doctor", "status"]
        );
        assert!(registry.resolve_enabled("features", &query).is_some());
    }

    #[test]
    fn resolve_enabled_respects_auth_and_interactive_requirements() {
        let mut registry = CommandRegistry::new();
        let mut command = NamedCommand::new("login", "authenticate");
        command.spec.requires_auth = true;
        command.spec.interactive_only = true;
        registry.register(Arc::new(command)).expect("register");

        let query = CommandQuery::new(FeatureSet::first_release());
        assert!(registry.resolve_enabled("login", &query).is_none());

        let query = CommandQuery {
            authenticated: true,
            interactive: true,
            ..CommandQuery::new(FeatureSet::first_release())
        };
        assert!(registry.resolve_enabled("login", &query).is_some());
    }

    #[test]
    fn command_spec_validation_rejects_duplicate_aliases() {
        let mut spec = CommandSpec::new("demo", "demo command", CommandKind::Local);
        spec.aliases = vec!["run".into(), "run".into()];

        let error = spec.validate().expect_err("duplicate alias");
        assert!(error.to_string().contains("duplicate command alias"));
    }

    #[test]
    fn argument_hint_defaults_to_none() {
        let spec = CommandSpec::new("help", "show help", CommandKind::Local);
        assert!(spec.argument_hint.is_none());
    }

    #[test]
    fn with_argument_hint_sets_the_hint() {
        let spec = CommandSpec::new("fast", "toggle fast mode", CommandKind::Local)
            .with_argument_hint("[on|off]");
        assert_eq!(spec.argument_hint.as_deref(), Some("[on|off]"));
    }

    #[test]
    fn command_spec_clone_preserves_argument_hint() {
        let spec = CommandSpec::new("color", "set color", CommandKind::Local)
            .with_argument_hint("<color|default>");
        let cloned = spec.clone();
        assert_eq!(cloned.argument_hint, spec.argument_hint);
    }

    #[test]
    fn argument_hint_round_trips_through_serde() {
        let spec = CommandSpec::new("effort", "set effort", CommandKind::Local)
            .with_argument_hint("[low|medium|high|max|auto]");
        let json = serde_json::to_string(&spec).expect("serialize");
        let decoded: CommandSpec = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            decoded.argument_hint.as_deref(),
            Some("[low|medium|high|max|auto]")
        );
    }

    #[test]
    fn argument_hint_absent_in_json_when_none() {
        let spec = CommandSpec::new("status", "show status", CommandKind::Local);
        let json = serde_json::to_string(&spec).expect("serialize");
        assert!(
            !json.contains("argument_hint"),
            "field should be omitted when None"
        );
    }

    #[test]
    fn argument_hint_none_deserializes_from_legacy_json_without_field() {
        // Ensures existing serialised CommandSpec payloads (without the new
        // field) continue to deserialise correctly with argument_hint = None.
        let json =
            r#"{"name":"status","description":"show status","kind":"local","source":"built_in"}"#;
        let spec: CommandSpec = serde_json::from_str(json).expect("deserialize legacy");
        assert!(spec.argument_hint.is_none());
    }
}
