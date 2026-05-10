use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};

use async_trait::async_trait;
use clap::{Args, Parser, Subcommand};
use serde_json::json;
use wonder_of_u_agent::{
    CredentialStore, ProviderResolver, SettingsStore, poll_copilot_access_token,
    request_copilot_device_code, require_storage_dir,
};
use wonder_of_u_core::{
    AuthMaterialKind, Command, CommandContext, CommandInvocation, CommandKind, CommandOutput,
    CommandSpec, FeatureFlag, Result, WonderError,
};
use wonder_of_u_storage::{StoragePaths, SyncStatusReport};

use super::parse_command_args;

/// Represents login command
pub struct LoginCommand {
    storage_dir: Option<PathBuf>,
}

impl LoginCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "login",
            "Store provider credentials or run interactive provider login",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::ModelProvider]);
        spec
    }
}

#[derive(Debug, Parser)]
struct LoginArgs {
    #[arg(long)]
    provider: String,
    #[arg(long)]
    api_key: Option<String>,
    #[arg(long, default_value_t = false)]
    no_browser: bool,
}

#[async_trait]
impl Command for LoginCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<LoginArgs>("login", &invocation)?;
        let storage_dir = require_storage_dir(self.storage_dir.clone())?;

        let resolver = ProviderResolver::builtin();
        let provider_config = resolver.registry().get(&args.provider).ok_or_else(|| {
            WonderError::validation(format!("unknown provider: {}", args.provider))
        })?;
        match provider_config.auth_kind {
            AuthMaterialKind::ApiKey => {
                let api_key = args.api_key.as_deref().ok_or_else(|| {
                    WonderError::validation("login requires --api-key for api-key providers")
                })?;
                if api_key.trim().is_empty() {
                    return Err(WonderError::validation("api key cannot be empty"));
                }
                CredentialStore::new(&storage_dir).set_api_key(&args.provider, api_key)?;
            }
            AuthMaterialKind::OAuth => {
                if args.provider != "copilot" {
                    return Err(WonderError::validation(format!(
                        "provider `{}` does not support interactive oauth login in this slice",
                        args.provider
                    )));
                }
                if args.api_key.is_some() {
                    return Err(WonderError::validation(
                        "--api-key is not used for oauth providers; omit it for `copilot` login",
                    ));
                }
                let device_code = request_copilot_device_code()?;
                eprintln!(
                    "Authorize GitHub Copilot at {}\nEnter code: {}\nWaiting for approval...",
                    device_code.verification_uri, device_code.user_code
                );
                if !args.no_browser {
                    super::open_browser(device_code.verification_uri.as_str());
                }
                let token = poll_copilot_access_token(
                    device_code.device_code.as_str(),
                    device_code.interval,
                    Duration::from_secs(device_code.expires_in.max(1)),
                )?;
                CredentialStore::new(&storage_dir).set_oauth_token(
                    &args.provider,
                    token.access_token,
                    token.refresh_token,
                    token.expires_at,
                )?;
            }
            AuthMaterialKind::AwsSigV4 => {
                return Err(WonderError::validation(format!(
                    "provider `{}` uses AWS SigV4 auth; set AWS_BEARER_TOKEN_BEDROCK, or AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY, or AWS_PROFILE",
                    args.provider
                )));
            }
            AuthMaterialKind::AwsBearer => {
                return Err(WonderError::validation(format!(
                    "provider `{}` uses AWS bearer-token auth; set AWS_BEARER_TOKEN_BEDROCK",
                    args.provider
                )));
            }
            AuthMaterialKind::AwsProfile => {
                return Err(WonderError::validation(format!(
                    "provider `{}` uses AWS profile auth; set AWS_PROFILE and configure ~/.aws/credentials",
                    args.provider
                )));
            }
            AuthMaterialKind::GcpOAuth2 => {
                return Err(WonderError::validation(format!(
                    "provider `{}` uses GCP OAuth2; set VERTEXAI_PROJECT, VERTEXAI_LOCATION, and GOOGLE_APPLICATION_CREDENTIALS",
                    args.provider
                )));
            }
            AuthMaterialKind::None => {
                return Err(WonderError::validation(format!(
                    "provider `{}` does not require login",
                    args.provider
                )));
            }
        }
        let settings_store = SettingsStore::new(&storage_dir);
        let mut settings = settings_store.read()?;
        if settings.selected_provider.is_none() {
            settings.selected_provider = Some(args.provider.clone());
        }
        if settings.selected_model.is_none() {
            settings.selected_model = Some(provider_config.default_model.clone());
        }
        settings_store.write(&settings)?;

        let report = resolver.load_report(Some(storage_dir.as_path()))?;
        Ok(CommandOutput::Text(format!(
            concat!(
                "stored_credentials_for={}\n",
                "provider_selection={}\n",
                "provider_readiness={}"
            ),
            args.provider,
            selection_label(&report),
            report.readiness.label(),
        )))
    }
}

/// Represents logout command
pub struct LogoutCommand {
    storage_dir: Option<PathBuf>,
}

impl LogoutCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "logout",
            "Remove stored provider credentials",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::ModelProvider]);
        spec
    }
}

#[derive(Debug, Parser)]
struct LogoutArgs {
    #[arg(long)]
    provider: Option<String>,
}

#[async_trait]
impl Command for LogoutCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<LogoutArgs>("logout", &invocation)?;
        let storage_dir = require_storage_dir(self.storage_dir.clone())?;
        let settings = SettingsStore::new(&storage_dir).read()?;
        let provider = args
            .provider
            .or(settings.selected_provider)
            .ok_or_else(|| {
                WonderError::validation("logout requires --provider or an active selection")
            })?;
        let removed = CredentialStore::new(&storage_dir).remove(&provider)?;

        Ok(CommandOutput::Text(format!(
            "provider={provider}\ncredentials_removed={removed}"
        )))
    }
}

/// Represents model command
pub struct ModelCommand {
    storage_dir: Option<PathBuf>,
}

impl ModelCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "model",
            "Inspect or change the active provider/model selection",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::ModelProvider]);
        spec
    }
}

#[derive(Debug, Parser)]
struct ModelArgs {
    #[command(subcommand)]
    command: Option<ModelSubcommand>,
}

#[derive(Debug, Subcommand)]
enum ModelSubcommand {
    Show,
    Set(ModelSetArgs),
}

#[derive(Debug, Args)]
struct ModelSetArgs {
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg()]
    selection: Option<String>,
}

#[async_trait]
impl Command for ModelCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let normalized_invocation = normalize_model_invocation(&invocation);
        let args = parse_command_args::<ModelArgs>("model", &normalized_invocation)?;
        match args.command.unwrap_or(ModelSubcommand::Show) {
            ModelSubcommand::Show => {
                let report =
                    ProviderResolver::builtin().load_report(self.storage_dir.as_deref())?;
                Ok(CommandOutput::Text(
                    if context.interactive && invocation.args.trim().is_empty() {
                        render_model_picker_output(&report)
                    } else {
                        render_model_status(&report)
                    },
                ))
            }
            ModelSubcommand::Set(args) => Ok(CommandOutput::Text(set_model_selection(
                self.storage_dir.as_deref(),
                args.provider,
                args.model,
                args.selection,
            )?)),
        }
    }
}

fn normalize_model_invocation(invocation: &CommandInvocation) -> CommandInvocation {
    let trimmed = invocation.args.trim();
    if trimmed.is_empty()
        || trimmed == "show"
        || trimmed.starts_with("show ")
        || trimmed == "set"
        || trimmed.starts_with("set ")
    {
        return invocation.clone();
    }
    CommandInvocation {
        name: invocation.name.clone(),
        args: format!("set {}", invocation.args),
        raw: invocation.raw.clone(),
    }
}

pub(crate) fn set_model_selection(
    storage_dir: Option<&Path>,
    provider: Option<String>,
    model: Option<String>,
    selection: Option<String>,
) -> Result<String> {
    let storage_dir = require_storage_dir(storage_dir.map(Path::to_path_buf))?;
    let resolver = ProviderResolver::builtin();
    let settings_store = SettingsStore::new(&storage_dir);
    let settings = settings_store.read()?;
    let (selection_provider, selection_model) = parse_model_selection(selection)?;
    let provider = provider
        .or(selection_provider)
        .or_else(|| settings.selected_provider.clone())
        .ok_or_else(|| {
            WonderError::validation(
                "model set requires --provider, a provider:model selection, or an existing selected provider",
            )
        })?;
    let provider_config = resolver
        .registry()
        .get(&provider)
        .ok_or_else(|| WonderError::validation(format!("unknown provider: {provider}")))?;
    let model = model
        .or(selection_model)
        .unwrap_or_else(|| provider_config.default_model.clone());
    provider_config.model(&model).ok_or_else(|| {
        WonderError::validation(format!("unknown model `{model}` for provider `{provider}`"))
    })?;

    let mut settings = settings;
    settings.selected_provider = Some(provider);
    settings.selected_model = Some(model);
    settings_store.write(&settings)?;

    let report = resolver.load_report(Some(storage_dir.as_path()))?;
    Ok(format!(
        concat!(
            "provider_selection={}\n",
            "provider_readiness={}\n",
            "auth_status={}"
        ),
        selection_label(&report),
        report.readiness.label(),
        report.auth.status_label(),
    ))
}

fn render_model_status(report: &wonder_of_u_agent::ProviderStatusReport) -> String {
    let mut lines = vec![
        format!("provider_selection={}", selection_label(report)),
        format!("provider_readiness={}", report.readiness.label()),
    ];
    for provider in &report.available_providers {
        let models = provider
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        lines.push(format!(
            "provider[{}]={};auth={};default_model={};models={}",
            provider.id,
            provider.display_name,
            auth_kind_label(provider.auth_kind),
            provider.default_model,
            models
        ));
    }
    lines.join("\n")
}

fn render_model_picker_output(report: &wonder_of_u_agent::ProviderStatusReport) -> String {
    let mut lines = vec![
        "model_picker=true".into(),
        format!("provider_selection={}", selection_label(report)),
        format!("provider_readiness={}", report.readiness.label()),
    ];
    for provider in &report.available_providers {
        for model in &provider.models {
            lines.push(format!(
                "model_option={}",
                json!({
                    "provider": provider.id,
                    "provider_display": provider.display_name,
                    "model": model.id,
                    "model_display": model.display_name,
                    "default": provider.default_model == model.id,
                    "selected": report.provider.as_deref() == Some(provider.id.as_str())
                        && report.model.as_deref() == Some(model.id.as_str()),
                    "auth": auth_kind_label(provider.auth_kind),
                })
            ));
        }
    }
    lines.join("\n")
}

/// Represents config command
pub struct ConfigCommand {
    storage_dir: Option<PathBuf>,
}

impl ConfigCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "config",
            "Inspect persisted provider settings and overrides",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::ModelProvider]);
        spec
    }
}

#[derive(Debug, Parser)]
struct ConfigArgs {
    #[command(subcommand)]
    command: Option<ConfigSubcommand>,
}

#[derive(Debug, Subcommand)]
enum ConfigSubcommand {
    Show,
    SetApiBase(ConfigApiBaseArgs),
    ClearApiBase(ConfigProviderArgs),
}

#[derive(Debug, Args)]
struct ConfigApiBaseArgs {
    #[arg(long)]
    provider: String,
    #[arg(long)]
    api_base: String,
}

#[derive(Debug, Args)]
struct ConfigProviderArgs {
    #[arg(long)]
    provider: String,
}

#[async_trait]
impl Command for ConfigCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<ConfigArgs>("config", &invocation)?;
        match args.command.unwrap_or(ConfigSubcommand::Show) {
            ConfigSubcommand::Show => self.show(),
            ConfigSubcommand::SetApiBase(args) => self.set_api_base(args),
            ConfigSubcommand::ClearApiBase(args) => self.clear_api_base(args),
        }
    }
}

impl ConfigCommand {
    fn show(&self) -> Result<CommandOutput> {
        let resolver = ProviderResolver::builtin();
        let report = resolver.load_report(self.storage_dir.as_deref())?;
        let mut lines = vec![
            format!("provider_selection={}", selection_label(&report)),
            format!("provider_readiness={}", report.readiness.label()),
            format!("auth_status={}", report.auth.status_label()),
        ];

        match &self.storage_dir {
            Some(storage_dir) => {
                let settings_store = SettingsStore::new(storage_dir);
                let settings = settings_store.read()?;
                let credentials = CredentialStore::new(storage_dir).read()?;
                let paths = StoragePaths::new(storage_dir);
                lines.push(format!("storage_dir={}", storage_dir.display()));
                lines.push(format!("settings_path={}", paths.settings_path().display()));
                lines.push(format!(
                    "credentials_path={}",
                    paths.credentials_path().display()
                ));
                lines.push(format!(
                    "selected_provider={}",
                    settings.selected_provider.as_deref().unwrap_or("none")
                ));
                lines.push(format!(
                    "selected_model={}",
                    settings.selected_model.as_deref().unwrap_or("none")
                ));
                let sync_status = SyncStatusReport::inspect(&paths);
                lines.push(format!(
                    "settings_sync={}",
                    sync_status.settings_sync.status.label()
                ));
                lines.push(format!(
                    "settings_sync_cloud={}",
                    sync_status.settings_sync.cloud_status.label()
                ));
                lines.push(format!(
                    "settings_sync_settings_exists={}",
                    sync_status.settings_sync.settings_exists
                ));
                lines.push(format!(
                    "settings_sync_user_memory_exists={}",
                    sync_status.settings_sync.user_memory_exists
                ));
                lines.push(format!(
                    "settings_sync_user_memory_path={}",
                    sync_status.settings_sync.user_memory_path.display()
                ));
                lines.push(format!(
                    "settings_sync_cloud_attempted={}",
                    sync_status.settings_sync.cloud_attempted
                ));
                lines.push(format!(
                    "remote_managed_settings={}",
                    sync_status.remote_managed_settings.status.label()
                ));
                lines.push(format!(
                    "remote_managed_settings_cloud_attempted={}",
                    sync_status.remote_managed_settings.cloud_attempted
                ));
                lines.push(format!(
                    "team_memory_sync={}",
                    sync_status.team_memory_sync.status.label()
                ));
                lines.push(format!(
                    "team_memory_sync_cloud_attempted={}",
                    sync_status.team_memory_sync.cloud_attempted
                ));
                for provider in credentials.providers.keys() {
                    lines.push(format!("stored_credential={provider}"));
                }
                for (provider, override_config) in settings.providers {
                    if let Some(model) = override_config.model {
                        lines.push(format!("provider_override[{provider}].model={model}"));
                    }
                    if let Some(api_base) = override_config.api_base {
                        lines.push(format!("provider_override[{provider}].api_base={api_base}"));
                    }
                }
            }
            None => lines.push("storage_dir=disabled".into()),
        }

        for provider in report.available_providers {
            lines.push(format!(
                "provider[{}].default_model={}",
                provider.id, provider.default_model
            ));
            lines.push(format!(
                "provider[{}].auth={}",
                provider.id,
                auth_kind_label(provider.auth_kind)
            ));
            if let Some(api_base) = provider.api_base {
                lines.push(format!("provider[{}].api_base={api_base}", provider.id));
            }
        }

        Ok(CommandOutput::Text(lines.join("\n")))
    }

    fn set_api_base(&self, args: ConfigApiBaseArgs) -> Result<CommandOutput> {
        let storage_dir = require_storage_dir(self.storage_dir.clone())?;
        if args.api_base.trim().is_empty() {
            return Err(WonderError::validation("api base cannot be empty"));
        }

        let resolver = ProviderResolver::builtin();
        resolver.registry().get(&args.provider).ok_or_else(|| {
            WonderError::validation(format!("unknown provider: {}", args.provider))
        })?;

        let store = SettingsStore::new(&storage_dir);
        let mut settings = store.read()?;
        settings
            .providers
            .entry(args.provider.clone())
            .or_default()
            .api_base = Some(args.api_base.clone());
        store.write(&settings)?;

        Ok(CommandOutput::Text(format!(
            "provider_override[{}].api_base={}",
            args.provider, args.api_base
        )))
    }

    fn clear_api_base(&self, args: ConfigProviderArgs) -> Result<CommandOutput> {
        let storage_dir = require_storage_dir(self.storage_dir.clone())?;
        let store = SettingsStore::new(&storage_dir);
        let mut settings = store.read()?;
        let mut removed = false;
        if let Some(override_config) = settings.providers.get_mut(&args.provider) {
            removed = override_config.api_base.take().is_some();
            if override_config.api_base.is_none() && override_config.model.is_none() {
                settings.providers.remove(&args.provider);
            }
        }
        store.write(&settings)?;

        Ok(CommandOutput::Text(format!(
            "provider_override[{}].api_base_cleared={removed}",
            args.provider
        )))
    }
}

fn parse_model_selection(selection: Option<String>) -> Result<(Option<String>, Option<String>)> {
    let Some(selection) = selection else {
        return Ok((None, None));
    };

    if let Some((provider, model)) = selection.split_once(':') {
        if provider.trim().is_empty() || model.trim().is_empty() {
            return Err(WonderError::validation(
                "model selection must be in the form provider:model",
            ));
        }
        return Ok((Some(provider.to_string()), Some(model.to_string())));
    }

    Ok((None, Some(selection)))
}

fn selection_label(report: &wonder_of_u_agent::ProviderStatusReport) -> String {
    report
        .selection_label()
        .unwrap_or_else(|| "unconfigured".into())
}

fn auth_kind_label(kind: AuthMaterialKind) -> &'static str {
    match kind {
        AuthMaterialKind::None => "none",
        AuthMaterialKind::ApiKey => "api_key",
        AuthMaterialKind::OAuth => "oauth",
        AuthMaterialKind::AwsSigV4 => "aws_sigv4",
        AuthMaterialKind::AwsBearer => "aws_bearer",
        AuthMaterialKind::AwsProfile => "aws_profile",
        AuthMaterialKind::GcpOAuth2 => "gcp_oauth2",
    }
}

#[cfg(test)]
mod tests {
    use crate::commands::browser_launch_disabled;

    use super::normalize_model_invocation;
    use wonder_of_u_core::CommandInvocation;

    #[test]
    fn model_shorthand_normalizes_to_set_subcommand() {
        let invocation = CommandInvocation {
            name: "model".into(),
            args: "openai:gpt-4.1".into(),
            raw: "/model openai:gpt-4.1".into(),
        };

        let normalized = normalize_model_invocation(&invocation);

        assert_eq!(normalized.args, "set openai:gpt-4.1");
    }

    #[test]
    fn model_show_and_set_invocations_are_preserved() {
        for args in ["", "show", "set --provider openai --model gpt-4.1"] {
            let invocation = CommandInvocation {
                name: "model".into(),
                args: args.into(),
                raw: format!("/model {args}").trim().to_string(),
            };

            let normalized = normalize_model_invocation(&invocation);

            assert_eq!(normalized.args, args);
        }
    }

    #[test]
    fn browser_launch_is_disabled_under_tests() {
        assert!(
            browser_launch_disabled(),
            "tests must never spawn a real system browser"
        );
    }
}
