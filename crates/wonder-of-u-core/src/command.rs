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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    Prompt,
    Local,
    Tui,
    NonInteractive,
    ResumeEntrypoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandSource {
    BuiltIn,
    Skill,
    Plugin,
    Workflow,
    Mcp,
    Dynamic,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandSpec {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub description: String,
    pub kind: CommandKind,
    pub source: CommandSource,
    #[serde(default)]
    pub required_features: BTreeSet<FeatureFlag>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub requires_auth: bool,
    #[serde(default)]
    pub interactive_only: bool,
}

impl CommandSpec {
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
        }
    }

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

    #[must_use]
    pub fn is_visible(&self, query: &CommandQuery) -> bool {
        (query.include_hidden || !self.hidden)
            && self.is_available(&query.features, query.authenticated, query.interactive)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandQuery {
    pub features: FeatureSet,
    pub authenticated: bool,
    pub interactive: bool,
    pub include_hidden: bool,
}

impl CommandQuery {
    #[must_use]
    pub fn new(features: FeatureSet) -> Self {
        Self {
            features,
            authenticated: false,
            interactive: false,
            include_hidden: false,
        }
    }

    #[must_use]
    pub fn including_hidden(mut self) -> Self {
        self.include_hidden = true;
        self
    }

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandInvocation {
    pub name: String,
    pub args: String,
    pub raw: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CommandOutput {
    Text(String),
    EnqueuePrompt(String),
    OpenUi(String),
    ExitRequested,
    Noop,
}

#[derive(Clone, Debug)]
pub struct CommandContext {
    pub session_id: SessionId,
    pub cwd: PathBuf,
    pub features: FeatureSet,
    pub authenticated: bool,
    pub interactive: bool,
    pub permission_mode: PermissionMode,
    pub theme: Option<String>,
    pub session_color: Option<String>,
    pub effort_level: Option<String>,
    pub brief_mode: bool,
    pub session_tags: Vec<String>,
    pub additional_working_directories: Vec<AdditionalWorkingDirectory>,
}

#[async_trait]
pub trait Command: Send + Sync {
    fn spec(&self) -> CommandSpec;

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

#[derive(Default)]
pub struct CommandRegistry {
    commands: BTreeMap<String, CommandEntry>,
    aliases: BTreeMap<String, String>,
    order: Vec<String>,
}

impl CommandRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

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

    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<Arc<dyn Command>> {
        self.resolve_entry(name)
            .map(|entry| Arc::clone(&entry.command))
    }

    #[must_use]
    pub fn resolve_enabled(&self, name: &str, query: &CommandQuery) -> Option<Arc<dyn Command>> {
        let entry = self.resolve_entry(name)?;
        query
            .allows(&entry.spec)
            .then(|| Arc::clone(&entry.command))
    }

    #[must_use]
    pub fn resolve_spec(&self, name: &str) -> Option<CommandSpec> {
        self.resolve_entry(name).map(|entry| entry.spec.clone())
    }

    pub fn all_specs(&self) -> Vec<CommandSpec> {
        self.order
            .iter()
            .filter_map(|canonical| self.commands.get(canonical))
            .map(|entry| entry.spec.clone())
            .collect()
    }

    pub fn available_specs(&self, query: &CommandQuery) -> Vec<CommandSpec> {
        self.all_specs()
            .into_iter()
            .filter(|spec| query.allows(spec))
            .collect()
    }

    pub fn visible_specs(&self, query: &CommandQuery) -> Vec<CommandSpec> {
        self.all_specs()
            .into_iter()
            .filter(|spec| spec.is_visible(query))
            .collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.commands.len()
    }

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
}
