use std::{
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    sync::Arc,
};

use async_trait::async_trait;
use clap::{Args, Parser, Subcommand, ValueEnum};
use wonder_of_u_agent::{ProviderResolver, require_storage_dir};
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandQuery,
    CommandSpec, FeatureFlag, ProviderReadiness, Result, WonderError,
};
use wonder_of_u_plugins::{
    PluginCatalog, PluginCatalogEntry, PluginCommandRegistration, PluginConfig, PluginConfigStore,
    PluginReadiness, PluginTrustDecision, normalize_plugin_id,
};
use wonder_of_u_skills::SkillCatalog;

use super::parse_command_args;

pub struct PluginCommand {
    storage_dir: Option<PathBuf>,
}

pub struct ReloadPluginsCommand {
    storage_dir: Option<PathBuf>,
}

struct PluginManifestCommand {
    storage_dir: Option<PathBuf>,
    plugin_id: String,
    command_name: String,
    spec: CommandSpec,
}

impl PluginCommand {
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "plugin",
            "Inspect discovered plugin manifests and trust status",
            CommandKind::Local,
        );
        spec.required_features.insert(FeatureFlag::Plugins);
        spec
    }
}

impl PluginManifestCommand {
    fn new(
        storage_dir: Option<PathBuf>,
        plugin_id: String,
        command_name: String,
        spec: CommandSpec,
    ) -> Self {
        Self {
            storage_dir,
            plugin_id,
            command_name,
            spec,
        }
    }
}

impl ReloadPluginsCommand {
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "reload-plugins",
            "Reload plugin and skill metadata catalogs from disk",
            CommandKind::NonInteractive,
        );
        spec.required_features.insert(FeatureFlag::Plugins);
        spec
    }
}

#[derive(Debug, Parser)]
struct PluginArgs {
    #[command(subcommand)]
    command: Option<PluginSubcommand>,
}

#[derive(Debug, Subcommand)]
enum PluginSubcommand {
    List,
    Status,
    Trust(PluginTrustArgs),
    Run(PluginRunArgs),
}

#[derive(Debug, Args)]
struct PluginTrustArgs {
    #[arg()]
    plugin: String,
    #[arg(long, value_enum)]
    state: PluginTrustStateArg,
}

#[derive(Debug, Args)]
struct PluginRunArgs {
    #[arg()]
    plugin: String,
    #[arg()]
    command: String,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum PluginTrustStateArg {
    Trusted,
    Untrusted,
    Blocked,
}

impl PluginTrustStateArg {
    const fn into_decision(self) -> PluginTrustDecision {
        match self {
            Self::Trusted => PluginTrustDecision::Trusted,
            Self::Untrusted => PluginTrustDecision::Untrusted,
            Self::Blocked => PluginTrustDecision::Blocked,
        }
    }
}

#[async_trait]
impl Command for PluginCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<PluginArgs>("plugin", &invocation)?;
        let output = match args.command.unwrap_or(PluginSubcommand::List) {
            PluginSubcommand::List => {
                let (_, plugins, _) = load_catalogs(&context.cwd, self.storage_dir.as_deref())?;
                render_plugin_list(&plugins)
            }
            PluginSubcommand::Status => {
                let (_, plugins, skills) =
                    load_catalogs(&context.cwd, self.storage_dir.as_deref())?;
                render_plugin_status(&plugins, &skills)
            }
            PluginSubcommand::Trust(args) => self.set_trust(&context.cwd, &args)?,
            PluginSubcommand::Run(args) => self.run_plugin(&context, &args)?,
        };
        Ok(CommandOutput::Text(output))
    }
}

#[async_trait]
impl Command for ReloadPluginsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let (_, plugins, skills) = load_catalogs(&context.cwd, self.storage_dir.as_deref())?;
        Ok(CommandOutput::Text(render_reload_status(&plugins, &skills)))
    }
}

#[async_trait]
impl Command for PluginManifestCommand {
    fn spec(&self) -> CommandSpec {
        self.spec.clone()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let (_, plugins, _) = load_catalogs(&context.cwd, self.storage_dir.as_deref())?;
        let plugin = plugins
            .find(&self.plugin_id)
            .ok_or_else(|| WonderError::not_found("plugin", self.plugin_id.clone()))?;
        ensure_plugin_runnable(plugin)?;
        let command = find_plugin_command(plugin, &self.command_name).ok_or_else(|| {
            WonderError::not_found(
                "plugin command",
                format!("{}:{}", self.plugin_id, self.command_name),
            )
        })?;
        ensure_plugin_command_runnable(
            command,
            ProviderResolver::builtin().load_report(self.storage_dir.as_deref())?,
        )?;
        let args = if invocation.args.trim().is_empty() {
            Vec::new()
        } else {
            shell_words::split(&invocation.args).map_err(|error| {
                WonderError::validation(format!("invalid /{} arguments: {error}", invocation.name))
            })?
        };
        Ok(CommandOutput::Text(execute_plugin_registration(
            plugin,
            command,
            &context.cwd,
            &args,
        )?))
    }
}

impl PluginCommand {
    fn run_plugin(&self, context: &CommandContext, args: &PluginRunArgs) -> Result<String> {
        let (_, plugins, _) = load_catalogs(&context.cwd, self.storage_dir.as_deref())?;
        let plugin_id = normalize_plugin_id(&args.plugin)?;
        let plugin = plugins
            .find(&plugin_id)
            .ok_or_else(|| WonderError::not_found("plugin", plugin_id.clone()))?;
        ensure_plugin_runnable(plugin)?;
        let command = find_plugin_command(plugin, &args.command).ok_or_else(|| {
            WonderError::not_found("plugin command", format!("{plugin_id}:{}", args.command))
        })?;
        ensure_plugin_command_runnable(
            command,
            ProviderResolver::builtin().load_report(self.storage_dir.as_deref())?,
        )?;
        execute_plugin_registration(plugin, command, &context.cwd, &args.args)
    }

    fn set_trust(&self, cwd: &Path, args: &PluginTrustArgs) -> Result<String> {
        let storage_dir = require_storage_dir(self.storage_dir.clone())?;
        let store = PluginConfigStore::new(&storage_dir);
        store.set_trust(&args.plugin, args.state.into_decision())?;
        let (_, plugins, _) = load_catalogs(cwd, Some(storage_dir.as_path()))?;
        let plugin_id = wonder_of_u_plugins::normalize_plugin_id(&args.plugin)?;
        let plugin = plugins
            .find(&plugin_id)
            .ok_or_else(|| WonderError::not_found("plugin", plugin_id.clone()))?;

        Ok(format!(
            concat!(
                "config_path={}\n",
                "plugin={}\n",
                "trust={}\n",
                "readiness={}"
            ),
            store.paths().plugin_settings_path().display(),
            plugin.id,
            plugin.trust.label(),
            plugin.readiness.label(),
        ))
    }
}

pub(crate) fn load_plugin_config(storage_dir: Option<&Path>) -> Result<PluginConfig> {
    match storage_dir {
        Some(storage_dir) => PluginConfigStore::new(storage_dir).read(),
        None => Ok(PluginConfig::default()),
    }
}

pub(crate) fn load_catalogs(
    cwd: &Path,
    storage_dir: Option<&Path>,
) -> Result<(PluginConfig, PluginCatalog, SkillCatalog)> {
    let config = load_plugin_config(storage_dir)?;
    let plugins = PluginCatalog::load(cwd, storage_dir, &config);
    let skills = SkillCatalog::load(cwd, storage_dir, &config, &plugins);
    Ok((config, plugins, skills))
}

pub(crate) fn resolve_dynamic_plugin_command(
    cwd: &Path,
    storage_dir: Option<&Path>,
    name: &str,
    query: &CommandQuery,
) -> Result<Option<Arc<dyn Command>>> {
    let (_, plugins, _) = load_catalogs(cwd, storage_dir)?;
    let Some((plugin, command)) = plugins
        .entries()
        .iter()
        .find_map(|plugin| find_plugin_command(plugin, name).map(|command| (plugin, command)))
    else {
        return Ok(None);
    };
    Ok(query.allows(&command.spec).then(|| {
        Arc::new(PluginManifestCommand::new(
            storage_dir.map(Path::to_path_buf),
            plugin.id.clone(),
            command.spec.name.clone(),
            command.spec.clone(),
        )) as Arc<dyn Command>
    }))
}

fn render_plugin_list(plugins: &PluginCatalog) -> String {
    let mut lines = vec![
        format!("roots={}", plugins.roots().len()),
        format!("plugins={}", plugins.entries().len()),
        format!("ready={}", plugins.ready_count()),
        format!("needs_trust={}", plugins.needs_trust_count()),
        format!("blocked={}", plugins.blocked_count()),
        format!("invalid={}", plugins.invalid_count()),
    ];
    if plugins.entries().is_empty() {
        lines.push("note=no plugins discovered".into());
    }
    for (index, plugin) in plugins.entries().iter().enumerate() {
        lines.push(format!(
            "plugin[{index}]={} source={} trust={} readiness={} commands={} skills={}",
            plugin.id,
            plugin.source.label(),
            plugin.trust.label(),
            plugin.readiness.label(),
            plugin.command_registrations.len(),
            plugin.skill_sources.len(),
        ));
    }
    lines.push(
        "note=trusted ready plugin commands can run with `plugin run` or dynamic slash-command lookup; sandboxing remains deferred".into(),
    );
    lines.join("\n")
}

fn render_plugin_status(plugins: &PluginCatalog, skills: &SkillCatalog) -> String {
    let mut lines = vec![
        format!("roots={}", plugins.roots().len()),
        format!("plugins={}", plugins.entries().len()),
        format!("ready={}", plugins.ready_count()),
        format!("needs_trust={}", plugins.needs_trust_count()),
        format!("blocked={}", plugins.blocked_count()),
        format!("invalid={}", plugins.invalid_count()),
        format!("catalog_errors={}", plugins.errors().len()),
        format!("plugin_skills_loaded={}", skills.plugin_count()),
    ];
    for (index, root) in plugins.roots().iter().enumerate() {
        lines.push(format!("root[{index}].source={}", root.source.label()));
        lines.push(format!("root[{index}].path={}", root.path.display()));
    }
    for (index, plugin) in plugins.entries().iter().enumerate() {
        lines.push(format!("plugin[{index}].id={}", plugin.id));
        lines.push(format!("plugin[{index}].source={}", plugin.source.label()));
        lines.push(format!(
            "plugin[{index}].manifest={}",
            plugin.manifest_path.display()
        ));
        lines.push(format!("plugin[{index}].trust={}", plugin.trust.label()));
        lines.push(format!(
            "plugin[{index}].readiness={}",
            plugin.readiness.label()
        ));
        lines.push(format!(
            "plugin[{index}].commands={}",
            plugin.command_registrations.len()
        ));
        lines.push(format!(
            "plugin[{index}].skills={}",
            plugin.skill_sources.len()
        ));
        if !plugin.notes.is_empty() {
            lines.push(format!(
                "plugin[{index}].notes={}",
                plugin.notes.join(" | ")
            ));
        }
    }
    for (index, error) in plugins.errors().iter().enumerate() {
        lines.push(format!("error[{index}].source={}", error.source.label()));
        lines.push(format!("error[{index}].path={}", error.path.display()));
        lines.push(format!("error[{index}].message={}", error.message));
    }
    lines.push(
        "note=trusted ready plugin commands support `plugin run` and dynamic slash-command lookup; sandboxing, prompts, and daemons remain deferred".into(),
    );
    lines.join("\n")
}

fn render_reload_status(plugins: &PluginCatalog, skills: &SkillCatalog) -> String {
    vec![
        "reloaded=true".into(),
        format!("plugins={}", plugins.entries().len()),
        format!("ready_plugins={}", plugins.ready_count()),
        format!("skills={}", skills.len()),
        format!("skill_commands={}", skills.command_count()),
        "note=reload refreshed plugin and skill catalogs; plugin run uses these registrations directly".into(),
    ]
    .join("\n")
}

fn ensure_plugin_runnable(plugin: &PluginCatalogEntry) -> Result<()> {
    match plugin.readiness {
        PluginReadiness::Ready => Ok(()),
        PluginReadiness::NeedsTrust => Err(WonderError::permission_denied(format!(
            "plugin `{}` is not trusted; run `wonder-of-u plugin trust {} --state trusted` first",
            plugin.id, plugin.id
        ))),
        PluginReadiness::Blocked => Err(WonderError::permission_denied(format!(
            "plugin `{}` is blocked and cannot be executed",
            plugin.id
        ))),
        PluginReadiness::Invalid => Err(WonderError::validation(format!(
            "plugin `{}` is invalid and cannot be executed: {}",
            plugin.id,
            plugin.notes.join(" | ")
        ))),
    }
}

fn find_plugin_command<'a>(
    plugin: &'a PluginCatalogEntry,
    command_name: &str,
) -> Option<&'a PluginCommandRegistration> {
    let normalized = normalize_command_lookup(command_name);
    plugin.command_registrations.iter().find(|command| {
        normalize_command_lookup(&command.spec.name) == normalized
            || command
                .spec
                .aliases
                .iter()
                .any(|alias| normalize_command_lookup(alias) == normalized)
    })
}

fn normalize_command_lookup(value: &str) -> String {
    value.trim().trim_start_matches('/').to_ascii_lowercase()
}

fn ensure_plugin_command_runnable(
    command: &PluginCommandRegistration,
    provider_report: wonder_of_u_agent::ProviderStatusReport,
) -> Result<()> {
    if command.spec.interactive_only {
        return Err(WonderError::validation(format!(
            "plugin command `{}` is interactive-only and cannot run in this non-interactive slice",
            command.spec.name
        )));
    }
    if command.spec.requires_auth && provider_report.readiness != ProviderReadiness::Ready {
        return Err(WonderError::permission_denied(format!(
            "plugin command `{}` requires provider auth, but provider readiness is {}",
            command.spec.name,
            provider_report.readiness.label()
        )));
    }
    Ok(())
}

fn execute_plugin_registration(
    plugin: &PluginCatalogEntry,
    command: &PluginCommandRegistration,
    cwd: &Path,
    args: &[String],
) -> Result<String> {
    let output = ProcessCommand::new(&command.entry_path)
        .args(args)
        .current_dir(cwd)
        .output()?;
    Ok(render_plugin_run_output(
        plugin, command, cwd, args, &output,
    ))
}

fn render_plugin_run_output(
    plugin: &PluginCatalogEntry,
    command: &PluginCommandRegistration,
    cwd: &Path,
    args: &[String],
    output: &std::process::Output,
) -> String {
    let status_code = output.status.code();
    let lines = vec![
        format!("plugin={}", plugin.id),
        format!("command={}", command.spec.name),
        format!("entry_path={}", command.entry_path.display()),
        format!("cwd={}", cwd.display()),
        format!("args={}", shell_words::join(args.iter().map(String::as_str))),
        format!("kind={:?}", command.spec.kind),
        format!("requires_auth={}", command.spec.requires_auth),
        format!("interactive_only={}", command.spec.interactive_only),
        format!("success={}", output.status.success()),
        format!(
            "exit_status={}",
            status_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".into())
        ),
        "stdout:".into(),
        render_stream(&output.stdout),
        "stderr:".into(),
        render_stream(&output.stderr),
        "note=plugin run and dynamic slash-command lookup execute the manifest entry directly; sandboxing and daemons remain deferred".into(),
    ];
    lines.join("\n")
}

fn render_stream(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.trim().is_empty() {
        "-".into()
    } else {
        text.trim_end().to_string()
    }
}
