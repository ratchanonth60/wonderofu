use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandQuery,
    CommandSpec, Result, WonderError,
};

/// Represents help command
pub struct HelpCommand {
    specs: Arc<[CommandSpec]>,
    storage_dir: Option<PathBuf>,
}

impl HelpCommand {
    /// Creates a new value
    pub fn new(specs: Arc<[CommandSpec]>, storage_dir: Option<PathBuf>) -> Self {
        Self { specs, storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "help",
            "Show available command help",
            CommandKind::NonInteractive,
        );
        spec.aliases.push("h".into());
        spec
    }

    fn runtime_specs(&self, cwd: &Path) -> Vec<CommandSpec> {
        let mut specs = self.specs.iter().cloned().collect::<Vec<_>>();
        let mut seen = specs
            .iter()
            .flat_map(|spec| {
                std::iter::once(spec.name.as_str()).chain(spec.aliases.iter().map(String::as_str))
            })
            .filter_map(normalize)
            .collect::<BTreeSet<_>>();
        if let Ok((_, plugins, _)) = super::plugin::load_catalogs(cwd, self.storage_dir.as_deref())
        {
            for plugin in plugins.entries() {
                for command in &plugin.command_registrations {
                    let names = std::iter::once(command.spec.name.as_str())
                        .chain(command.spec.aliases.iter().map(String::as_str))
                        .filter_map(normalize)
                        .collect::<Vec<_>>();
                    if names.iter().any(|name| seen.contains(name)) {
                        continue;
                    }
                    seen.extend(names);
                    specs.push(command.spec.clone());
                }
            }
        }
        specs
    }

    fn render_catalog(&self, specs: &[CommandSpec], query: &CommandQuery) -> String {
        let mut lines = vec!["Available commands:".to_string()];
        for spec in specs.iter().filter(|spec| spec.is_visible(query)) {
            let aliases = if spec.aliases.is_empty() {
                String::new()
            } else {
                format!(" [aliases: {}]", spec.aliases.join(", "))
            };
            lines.push(format!(
                "  {:<12} {}{}",
                spec.name, spec.description, aliases
            ));
        }
        lines.push(String::new());
        lines.push("Use `wonder-of-u slash /<command> ...` to exercise slash-command parsing from the CLI.".into());
        lines.join("\n")
    }

    fn render_command_help(
        &self,
        specs: &[CommandSpec],
        name: &str,
        query: &CommandQuery,
    ) -> Result<String> {
        let normalized = normalize(name)
            .ok_or_else(|| WonderError::not_found("command", name.trim().to_string()))?;
        let spec = specs
            .iter()
            .find(|spec| query.allows(spec) && matches_name(spec, &normalized))
            .ok_or_else(|| WonderError::not_found("command", normalized.clone()))?;

        let mut lines = vec![
            spec.name.clone(),
            format!("  Description: {}", spec.description),
        ];
        if !spec.aliases.is_empty() {
            lines.push(format!("  Aliases: {}", spec.aliases.join(", ")));
        }
        lines.push(format!("  Kind: {:?}", spec.kind));
        if spec.hidden {
            lines.push("  Visibility: hidden from help listings".into());
        }
        if !spec.required_features.is_empty() {
            lines.push(format!(
                "  Required features: {}",
                spec.required_features
                    .iter()
                    .map(|flag| format!("{flag:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(lines.join("\n"))
    }
}

#[async_trait]
impl Command for HelpCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let query = CommandQuery::from(&context);
        let specs = self.runtime_specs(&context.cwd);
        let output = if invocation.args.is_empty() {
            self.render_catalog(&specs, &query)
        } else {
            self.render_command_help(&specs, &invocation.args, &query)?
        };
        Ok(CommandOutput::Text(output))
    }
}

fn matches_name(spec: &CommandSpec, normalized: &str) -> bool {
    normalize(&spec.name).as_deref() == Some(normalized)
        || spec
            .aliases
            .iter()
            .any(|alias| normalize(alias).as_deref() == Some(normalized))
}

fn normalize(name: &str) -> Option<String> {
    let normalized = name.trim().trim_start_matches('/').to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}
