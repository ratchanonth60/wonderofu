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
            "Interactive provider login — run bare to see all auth modes, or pass --provider to get specific setup guidance",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::ModelProvider]);
        spec
    }
}

#[derive(Debug, Parser)]
struct LoginArgs {
    /// Provider to log in to (e.g. `anthropic`, `openai`, `bedrock`, `vertex`,
    /// `local`). Omit to see a guide listing all auth modes.
    #[arg(long)]
    provider: Option<String>,
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

        // Bare `/login` with no --provider shows the auth-mode guide.
        let provider_id = match args.provider {
            None => return Ok(CommandOutput::Text(render_login_guide())),
            Some(ref p) => p.clone(),
        };

        let resolver = ProviderResolver::builtin();
        let provider_config = resolver
            .registry()
            .get(&provider_id)
            .ok_or_else(|| WonderError::validation(format!("unknown provider: {provider_id}")))?;

        match provider_config.auth_kind {
            AuthMaterialKind::ApiKey => {
                let api_key = args.api_key.as_deref().ok_or_else(|| {
                    WonderError::validation("login requires --api-key for api-key providers")
                })?;
                if api_key.trim().is_empty() {
                    return Err(WonderError::validation("api key cannot be empty"));
                }
                CredentialStore::new(&storage_dir).set_api_key(&provider_id, api_key)?;
            }
            AuthMaterialKind::OAuth => {
                if provider_id != "copilot" {
                    return Err(WonderError::validation(format!(
                        "provider `{provider_id}` does not support interactive oauth login in this slice",
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
                    &provider_id,
                    token.access_token,
                    token.refresh_token,
                    token.expires_at,
                )?;
            }
            // Env-based auth modes: return Ok guidance instead of a hard error so
            // callers can display actionable text to the user (parity with upstream).
            AuthMaterialKind::AwsSigV4 => {
                return Ok(CommandOutput::Text(format!(
                    "auth_mode=env\nprovider={provider_id}\n\
                     guidance=AWS SigV4 auth — set one of:\n\
                     \x20 AWS_BEARER_TOKEN_BEDROCK  (preferred for Bedrock),\n\
                     \x20 AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY  (static credentials), or\n\
                     \x20 AWS_PROFILE  (named profile in ~/.aws/credentials).\n\
                     No credentials are stored locally for this auth mode."
                )));
            }
            AuthMaterialKind::AwsBearer => {
                return Ok(CommandOutput::Text(format!(
                    "auth_mode=env\nprovider={provider_id}\n\
                     guidance=AWS bearer-token auth — set AWS_BEARER_TOKEN_BEDROCK.\n\
                     No credentials are stored locally for this auth mode."
                )));
            }
            AuthMaterialKind::AwsProfile => {
                return Ok(CommandOutput::Text(format!(
                    "auth_mode=env\nprovider={provider_id}\n\
                     guidance=AWS profile auth — set AWS_PROFILE and configure ~/.aws/credentials.\n\
                     No credentials are stored locally for this auth mode."
                )));
            }
            AuthMaterialKind::GcpOAuth2 => {
                return Ok(CommandOutput::Text(format!(
                    "auth_mode=env\nprovider={provider_id}\n\
                     guidance=GCP OAuth2 (Vertex AI) auth — set:\n\
                     \x20 VERTEXAI_PROJECT   (GCP project ID),\n\
                     \x20 VERTEXAI_LOCATION  (e.g. us-central1), and\n\
                     \x20 GOOGLE_APPLICATION_CREDENTIALS  (path to service-account JSON key), or\n\
                     \x20 GOOGLE_BEARER_TOKEN  (pre-obtained OAuth2 token).\n\
                     No credentials are stored locally for this auth mode."
                )));
            }
            // No-auth providers do not need login; guide the user to /model set.
            AuthMaterialKind::None => {
                return Ok(CommandOutput::Text(format!(
                    "auth_mode=none\nprovider={provider_id}\n\
                     guidance=This provider does not require authentication.\n\
                     Use `/model set {provider_id}` to switch to it, or `/model set {provider_id}:<model>` \
                     to pick a specific model."
                )));
            }
        }

        let settings_store = SettingsStore::new(&storage_dir);
        let mut settings = settings_store.read()?;
        if settings.selected_provider.is_none() {
            settings.selected_provider = Some(provider_id.clone());
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
            provider_id,
            selection_label(&report),
            report.readiness.label(),
        )))
    }
}

/// Renders a guide listing all supported auth modes and their providers.
///
/// Shown when the user runs `/login` with no `--provider` argument.
fn render_login_guide() -> String {
    let resolver = ProviderResolver::builtin();
    let mut api_key_providers = Vec::new();
    let mut oauth_providers = Vec::new();
    let mut env_providers = Vec::new();
    let mut none_providers = Vec::new();

    for config in resolver.registry().providers() {
        match config.auth_kind {
            AuthMaterialKind::ApiKey => api_key_providers.push(config.id.clone()),
            AuthMaterialKind::OAuth => oauth_providers.push(config.id.clone()),
            AuthMaterialKind::AwsSigV4
            | AuthMaterialKind::AwsBearer
            | AuthMaterialKind::AwsProfile
            | AuthMaterialKind::GcpOAuth2 => env_providers.push(config.id.clone()),
            AuthMaterialKind::None => none_providers.push(config.id.clone()),
        }
    }

    api_key_providers.sort();
    oauth_providers.sort();
    env_providers.sort();
    none_providers.sort();

    let mut lines = vec![
        "## /login".into(),
        String::new(),
        "Pass `--provider <id>` to log in or see setup guidance for a specific provider.".into(),
        String::new(),
        "### api-key providers  (`/login --provider <id> --api-key <key>`)".into(),
    ];
    for p in &api_key_providers {
        lines.push(format!(
            "  api-key  — /login --provider {p} --api-key <your-key>"
        ));
    }
    lines.push(String::new());
    lines.push("### oauth providers  (interactive browser flow)".into());
    for p in &oauth_providers {
        lines.push(format!("  oauth    — /login --provider {p}"));
    }
    lines.push(String::new());
    lines.push("### env-variable providers  (no credentials stored locally)".into());
    for p in &env_providers {
        lines.push(format!(
            "  env      — /login --provider {p}  (shows env-var setup guide)"
        ));
    }
    lines.push(String::new());
    lines.push("### none / no-auth providers  (no login needed)".into());
    for p in &none_providers {
        lines.push(format!(
            "  none     — /login --provider {p}  (shows /model set guidance)"
        ));
    }
    lines.push(String::new());
    lines.push("Run `/model` to see the active provider and model.".into());

    lines.join("\n")
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
    // Strict providers maintain a curated catalogue; reject unknown ids.
    // Non-strict providers (gateways, local, native-dynamic) accept any
    // non-empty model string—we just guard against blank input.
    if provider_config.strict_model_validation {
        provider_config.model(&model).ok_or_else(|| {
            WonderError::validation(format!(
                "unknown model `{model}` for provider `{provider}` (strict catalogue)"
            ))
        })?;
    } else if model.trim().is_empty() {
        return Err(WonderError::validation("model id cannot be empty"));
    }

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
        // Non-strict providers accept any non-empty model id; display "<any>"
        // rather than an empty catalogue to signal pass-through semantics.
        let models = if provider.strict_model_validation {
            provider
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>()
                .join(",")
        } else {
            "<any>".to_string()
        };
        let mut meta = format!(
            "provider[{}]={};auth={};default_model={};models={}",
            provider.id,
            provider.display_name,
            auth_kind_label(provider.auth_kind),
            provider.default_model,
            models,
        );
        // Surface the env var names so users know what to set without digging
        // through docs.  Never print the value—only the variable name.
        if let Some(env) = &provider.api_key_env {
            meta.push_str(&format!(";api_key_env={env}"));
        }
        if let Some(env) = &provider.endpoint_env {
            meta.push_str(&format!(";endpoint_env={env}"));
        }
        lines.push(meta);
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
        if provider.strict_model_validation {
            // Strict providers: enumerate the curated catalogue.
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
        } else {
            // Non-strict providers (gateways, local, native-dynamic): emit the
            // default model as a representative picker entry.  Any non-empty
            // model string is valid; the label signals pass-through semantics.
            let model_id = &provider.default_model;
            let active_model = report.model.as_deref().unwrap_or(model_id);
            let is_active_provider = report.provider.as_deref() == Some(provider.id.as_str());
            // Show either the currently-selected model (if this is the active
            // provider) or the provider's default as the representative entry.
            let display_model = if is_active_provider {
                active_model
            } else {
                model_id.as_str()
            };
            lines.push(format!(
                "model_option={}",
                json!({
                    "provider": provider.id,
                    "provider_display": provider.display_name,
                    "model": display_model,
                    "model_display": format!("{display_model} (any model accepted)"),
                    "default": true,
                    "selected": is_active_provider,
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
            lines.push(format!(
                "provider[{}].strict_models={}",
                provider.id, provider.strict_model_validation
            ));
            if let Some(api_base) = provider.api_base {
                lines.push(format!("provider[{}].api_base={api_base}", provider.id));
            }
            // Expose env var names (never their values) for discoverability.
            if let Some(env) = &provider.api_key_env {
                lines.push(format!("provider[{}].api_key_env={env}", provider.id));
            }
            if let Some(env) = &provider.endpoint_env {
                lines.push(format!("provider[{}].endpoint_env={env}", provider.id));
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
    use super::{render_model_picker_output, render_model_status};
    use wonder_of_u_agent::{AgentSettings, AuthMaterial, ProviderResolver, StoredCredentials};
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

    // ── render_model_status ──────────────────────────────────────────────────

    /// Non-strict providers must show `models=<any>` rather than an empty
    /// string, signalling pass-through semantics to callers parsing the output.
    #[test]
    fn render_model_status_shows_any_for_non_strict_providers() {
        // Gemini is non-strict with an empty model catalogue.
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("gemini".into()),
            ..AgentSettings::default()
        };
        let credentials = StoredCredentials {
            providers: [(
                "gemini".into(),
                AuthMaterial::ApiKey {
                    key: "g-key".into(),
                },
            )]
            .into(),
        };
        let report = resolver
            .resolve_with_env(
                &settings,
                &credentials,
                std::iter::empty::<(&str, String)>(),
            )
            .expect("gemini resolves with stored api key");

        let output = render_model_status(&report);

        assert!(
            output.contains("models=<any>"),
            "gemini (non-strict) must show models=<any>; got:\n{output}"
        );
        assert!(
            output.contains("api_key_env=GEMINI_API_KEY"),
            "must include env var hint; got:\n{output}"
        );
    }

    /// Strict providers must still list their enumerated catalogue.
    #[test]
    fn render_model_status_shows_catalogue_for_strict_providers() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("openai".into()),
            ..AgentSettings::default()
        };
        let credentials = StoredCredentials {
            providers: [(
                "openai".into(),
                AuthMaterial::ApiKey {
                    key: "sk-key".into(),
                },
            )]
            .into(),
        };
        let report = resolver
            .resolve_with_env(
                &settings,
                &credentials,
                std::iter::empty::<(&str, String)>(),
            )
            .expect("openai resolves with stored api key");

        let output = render_model_status(&report);

        // Find the provider[openai]=… line specifically (other providers may
        // be present when env vars are set in the test environment).
        let openai_line = output
            .lines()
            .find(|l| l.starts_with("provider[openai]="))
            .expect("must have a provider[openai] line in render_model_status output");
        assert!(
            openai_line.contains("gpt-4.1"),
            "openai (strict) must list known models; got line: {openai_line}"
        );
        assert!(
            !openai_line.contains("models=<any>"),
            "strict provider line must not contain models=<any>; got: {openai_line}"
        );
    }

    // ── render_model_picker_output ───────────────────────────────────────────

    /// Non-strict providers must appear in the model picker with their default
    /// model even though their `models` list is empty.
    #[test]
    fn render_model_picker_output_includes_non_strict_provider() {
        // Groq is a gateway provider: strict=false, models=[].
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("groq".into()),
            ..AgentSettings::default()
        };
        let credentials = StoredCredentials {
            providers: [(
                "groq".into(),
                AuthMaterial::ApiKey {
                    key: "gq-key".into(),
                },
            )]
            .into(),
        };
        let report = resolver
            .resolve_with_env(
                &settings,
                &credentials,
                std::iter::empty::<(&str, String)>(),
            )
            .expect("groq resolves with stored api key");

        let output = render_model_picker_output(&report);

        assert!(
            output.contains("model_picker=true"),
            "must start with picker sentinel; got:\n{output}"
        );
        assert!(
            output.contains("model_option="),
            "non-strict groq must still emit a model_option line; got:\n{output}"
        );
        // The label should mention "any model accepted".
        assert!(
            output.contains("any model accepted"),
            "non-strict option must communicate arbitrary-model semantics; got:\n{output}"
        );
    }

    /// HF_TOKEN provider (huggingface) should appear in the picker and surface
    /// the correct env var name as the auth label.
    #[test]
    fn render_model_picker_output_huggingface_env_auth_label() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("huggingface".into()),
            ..AgentSettings::default()
        };
        let credentials = StoredCredentials {
            providers: [(
                "huggingface".into(),
                AuthMaterial::ApiKey {
                    key: "hf-secret".into(),
                },
            )]
            .into(),
        };
        let report = resolver
            .resolve_with_env(
                &settings,
                &credentials,
                std::iter::empty::<(&str, String)>(),
            )
            .expect("huggingface resolves with stored HF_TOKEN credential");

        let output = render_model_picker_output(&report);

        assert!(
            output.contains("huggingface"),
            "huggingface must be present in picker; got:\n{output}"
        );
        assert!(
            output.contains("model_option="),
            "must have at least one model_option; got:\n{output}"
        );
    }

    /// Azure OpenAI: non-strict, endpoint from env, api-key from env.
    /// The status output must mention the endpoint env var.
    #[test]
    fn render_model_status_azure_shows_endpoint_env_hint() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("azure".into()),
            ..AgentSettings::default()
        };
        let credentials = StoredCredentials {
            providers: [(
                "azure".into(),
                AuthMaterial::ApiKey {
                    key: "az-key".into(),
                },
            )]
            .into(),
        };
        // Azure needs endpoint env to resolve; provide a dummy.
        let report = resolver
            .resolve_with_env(
                &settings,
                &credentials,
                [(
                    "AZURE_OPENAI_API_ENDPOINT",
                    "https://my.openai.azure.com".to_string(),
                )],
            )
            .expect("azure resolves with api key + endpoint env");

        let output = render_model_status(&report);

        assert!(
            output.contains("models=<any>"),
            "azure (non-strict) must show models=<any>; got:\n{output}"
        );
        assert!(
            output.contains("endpoint_env=AZURE_OPENAI_API_ENDPOINT"),
            "must include endpoint env hint; got:\n{output}"
        );
        assert!(
            output.contains("api_key_env=AZURE_OPENAI_API_KEY"),
            "must include api key env hint; got:\n{output}"
        );
    }

    /// Local provider: no auth needed, accepts arbitrary model ids in picker.
    #[test]
    fn render_model_picker_output_local_provider_appears() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("local".into()),
            selected_model: Some("phi3:mini".into()),
            ..AgentSettings::default()
        };
        let report = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
            )
            .expect("local provider needs no auth");

        let output = render_model_picker_output(&report);

        assert!(
            output.contains("model_picker=true"),
            "local picker output must include sentinel; got:\n{output}"
        );
        assert!(
            output.contains("local"),
            "local provider must appear in picker output; got:\n{output}"
        );
        // The active model should be the custom one we set, not the default.
        assert!(
            output.contains("phi3:mini"),
            "active custom model must appear; got:\n{output}"
        );
    }

    /// Non-strict model selection must accept arbitrary model ids without
    /// returning a validation error.
    #[test]
    fn set_model_selection_accepts_arbitrary_model_for_non_strict_provider() {
        let dir = tempfile::tempdir().expect("temp dir");
        // Seed the storage dir with groq credentials.
        use wonder_of_u_agent::{CredentialStore, SettingsStore};
        CredentialStore::new(dir.path())
            .set_api_key("groq", "groq-secret")
            .expect("write groq credential");
        let s = wonder_of_u_agent::AgentSettings {
            selected_provider: Some("groq".into()),
            selected_model: Some("mixtral-8x7b-32768".into()),
            ..wonder_of_u_agent::AgentSettings::default()
        };
        SettingsStore::new(dir.path())
            .write(&s)
            .expect("write settings");

        let result = super::set_model_selection(
            Some(dir.path()),
            Some("groq".into()),
            Some("mixtral-8x7b-32768".into()),
            None,
        );

        assert!(
            result.is_ok(),
            "non-strict provider should accept arbitrary model id; got: {:?}",
            result
        );
        let output = result.unwrap();
        assert!(
            output.contains("provider_selection=groq:mixtral-8x7b-32768"),
            "output must confirm selection; got:\n{output}"
        );
    }

    /// Strict provider must still reject unknown model ids.
    #[test]
    fn set_model_selection_rejects_unknown_model_for_strict_provider() {
        let dir = tempfile::tempdir().expect("temp dir");
        use wonder_of_u_agent::{CredentialStore, SettingsStore};
        CredentialStore::new(dir.path())
            .set_api_key("openai", "sk-secret")
            .expect("write openai credential");
        let s = wonder_of_u_agent::AgentSettings {
            selected_provider: Some("openai".into()),
            ..wonder_of_u_agent::AgentSettings::default()
        };
        SettingsStore::new(dir.path())
            .write(&s)
            .expect("write settings");

        let result = super::set_model_selection(
            Some(dir.path()),
            Some("openai".into()),
            Some("nonexistent-model-xyz".into()),
            None,
        );

        assert!(
            result.is_err(),
            "strict provider must reject unknown model; got ok"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("strict catalogue"),
            "error must mention strict catalogue; got: {msg}"
        );
    }

    // ── /login parity tests ──────────────────────────────────────────────────

    // Bring LoginCommand in scope once for all login tests below.
    use super::LoginCommand;

    fn make_login_ctx(dir: &std::path::Path) -> wonder_of_u_core::CommandContext {
        use wonder_of_u_core::{CommandContext, FeatureSet, PermissionMode, SessionId};
        CommandContext {
            session_id: SessionId::new(),
            cwd: dir.to_path_buf(),
            features: FeatureSet::first_release(),
            authenticated: false,
            interactive: false,
            permission_mode: PermissionMode::Default,
            theme: None,
            session_color: None,
            effort_level: None,
            brief_mode: false,
            fast_mode: false,
            optimize_token_mode: false,
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        }
    }

    /// Bare `/login` (no --provider) must return Ok guidance, not a parse error.
    #[test]
    fn login_bare_returns_guide_listing_auth_modes() {
        use futures::executor::block_on;
        use wonder_of_u_core::{Command, CommandInvocation, CommandOutput};

        let dir = tempfile::tempdir().expect("temp dir");
        let cmd = LoginCommand::new(Some(dir.path().to_path_buf()));
        let result = block_on(cmd.execute(
            make_login_ctx(dir.path()),
            CommandInvocation {
                name: "login".into(),
                args: String::new(),
                raw: "/login".into(),
            },
        ));
        let output = result.expect("bare /login must succeed");
        let CommandOutput::Text(text) = output else {
            panic!("expected Text output");
        };
        assert!(
            text.contains("## /login"),
            "guide must have a heading; got:\n{text}"
        );
        assert!(
            text.contains("api-key"),
            "guide must mention api-key auth; got:\n{text}"
        );
        assert!(
            text.contains("oauth"),
            "guide must mention oauth; got:\n{text}"
        );
        assert!(
            text.contains("none"),
            "guide must mention no-auth providers; got:\n{text}"
        );
    }

    /// `/login --provider bedrock` (AWS SigV4) must return Ok guidance — not an error.
    #[test]
    fn login_aws_sigv4_provider_returns_guidance_not_error() {
        use futures::executor::block_on;
        use wonder_of_u_core::{Command, CommandInvocation, CommandOutput};

        let dir = tempfile::tempdir().expect("temp dir");
        let cmd = LoginCommand::new(Some(dir.path().to_path_buf()));
        let result = block_on(cmd.execute(
            make_login_ctx(dir.path()),
            CommandInvocation {
                name: "login".into(),
                args: "--provider bedrock".into(),
                raw: "/login --provider bedrock".into(),
            },
        ));
        let output = result.expect("AWS SigV4 provider must return Ok guidance");
        let CommandOutput::Text(text) = output else {
            panic!("expected Text output");
        };
        assert!(
            text.contains("auth_mode=env"),
            "must mark auth_mode=env; got:\n{text}"
        );
        assert!(
            text.contains("AWS"),
            "must contain AWS guidance; got:\n{text}"
        );
    }

    /// `/login --provider local` (no-auth) must return Ok with `/model set` hint.
    #[test]
    fn login_no_auth_provider_returns_guidance_not_error() {
        use futures::executor::block_on;
        use wonder_of_u_core::{Command, CommandInvocation, CommandOutput};

        let dir = tempfile::tempdir().expect("temp dir");
        let cmd = LoginCommand::new(Some(dir.path().to_path_buf()));
        let result = block_on(cmd.execute(
            make_login_ctx(dir.path()),
            CommandInvocation {
                name: "login".into(),
                args: "--provider local".into(),
                raw: "/login --provider local".into(),
            },
        ));
        let output = result.expect("no-auth provider must return Ok guidance");
        let CommandOutput::Text(text) = output else {
            panic!("expected Text output");
        };
        assert!(
            text.contains("auth_mode=none"),
            "must mark auth_mode=none; got:\n{text}"
        );
        assert!(
            text.contains("/model set"),
            "must suggest /model set; got:\n{text}"
        );
    }

    /// `/login --provider vertex` (GCP OAuth2) must return Ok guidance — not an error.
    #[test]
    fn login_gcp_provider_returns_guidance_not_error() {
        use futures::executor::block_on;
        use wonder_of_u_core::{Command, CommandInvocation, CommandOutput};

        let dir = tempfile::tempdir().expect("temp dir");
        let cmd = LoginCommand::new(Some(dir.path().to_path_buf()));
        let result = block_on(cmd.execute(
            make_login_ctx(dir.path()),
            CommandInvocation {
                name: "login".into(),
                args: "--provider vertex".into(),
                raw: "/login --provider vertex".into(),
            },
        ));
        let output = result.expect("GCP provider must return Ok guidance");
        let CommandOutput::Text(text) = output else {
            panic!("expected Text output");
        };
        assert!(
            text.contains("auth_mode=env"),
            "must mark auth_mode=env; got:\n{text}"
        );
        assert!(text.contains("GCP"), "must mention GCP; got:\n{text}");
    }

    /// The command spec description must convey the interactive-login concept.
    #[test]
    fn login_command_spec_description_conveys_interactive_flow() {
        let spec = LoginCommand::command_spec();
        assert!(
            spec.description.contains("interactive") || spec.description.contains("login"),
            "description should convey interactive login; got: {}",
            spec.description
        );
    }
}
