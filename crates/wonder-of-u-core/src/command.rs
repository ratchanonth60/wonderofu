use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{FeatureFlag, FeatureSet, Result, SessionId, WonderError};

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
    pub features: FeatureSet,
    pub authenticated: bool,
    pub interactive: bool,
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

#[derive(Default)]
pub struct CommandRegistry {
    commands: BTreeMap<String, Arc<dyn Command>>,
    aliases: BTreeMap<String, String>,
}

impl CommandRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, command: Arc<dyn Command>) -> Result<()> {
        let spec = command.spec();
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
        self.commands.insert(canonical, command);
        Ok(())
    }

    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<Arc<dyn Command>> {
        let normalized = name.trim().to_ascii_lowercase();
        self.commands.get(&normalized).cloned().or_else(|| {
            self.aliases
                .get(&normalized)
                .and_then(|canonical| self.commands.get(canonical))
                .cloned()
        })
    }

    pub fn visible_specs(
        &self,
        features: &FeatureSet,
        authenticated: bool,
        interactive: bool,
    ) -> Vec<CommandSpec> {
        self.commands
            .values()
            .map(|command| command.spec())
            .filter(|spec| !spec.hidden && spec.is_available(features, authenticated, interactive))
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
    let normalized = name.trim().trim_start_matches('/').to_ascii_lowercase();
    if normalized.is_empty() {
        return Err(WonderError::validation("command name cannot be empty"));
    }
    if normalized.chars().any(char::is_whitespace) {
        return Err(WonderError::validation(format!(
            "command name contains whitespace: {name}"
        )));
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct HelpCommand;

    #[async_trait]
    impl Command for HelpCommand {
        fn spec(&self) -> CommandSpec {
            let mut spec = CommandSpec::new("help", "show help", CommandKind::Local);
            spec.aliases.push("h".into());
            spec
        }

        async fn execute(
            &self,
            _context: CommandContext,
            _invocation: CommandInvocation,
        ) -> Result<CommandOutput> {
            Ok(CommandOutput::Text("help".into()))
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
        registry.register(Arc::new(HelpCommand)).expect("register");

        assert!(registry.resolve("help").is_some());
        assert!(registry.resolve("h").is_some());
    }
}
