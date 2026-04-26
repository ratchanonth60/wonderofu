use std::{collections::BTreeMap, fmt, path::Path};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use wonder_of_u_core::{
    AuthMaterialKind, AuthSource, AuthState, ProviderReadiness, Result, WonderError,
};

use crate::{
    auth::{AuthMaterial, DEFAULT_COPILOT_API_BASE, StoredCredentials},
    config::{AgentSettings, CredentialStore, SettingsStore},
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    pub id: String,
    pub display_name: String,
}

impl ModelDescriptor {
    #[must_use]
    pub fn new(id: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    pub id: String,
    pub display_name: String,
    pub auth_kind: AuthMaterialKind,
    pub default_model: String,
    pub models: Vec<ModelDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
}

impl ProviderDescriptor {
    #[must_use]
    pub fn model(&self, model_id: &str) -> Option<&ModelDescriptor> {
        self.models.iter().find(|model| model.id == model_id)
    }

    #[must_use]
    pub fn preferred_fast_model(&self) -> Option<&ModelDescriptor> {
        self.models.iter().find(|model| is_fast_model_id(&model.id))
    }

    #[must_use]
    pub fn supports_fast_mode(&self) -> bool {
        self.preferred_fast_model().is_some()
    }
}

fn auth_material_label(material: &AuthMaterial) -> &'static str {
    match material {
        AuthMaterial::None => "none",
        AuthMaterial::ApiKey { .. } => "api_key",
        AuthMaterial::OAuth { .. } => "oauth",
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProviderSelection {
    pub provider: Option<String>,
    pub model: Option<String>,
}

impl ProviderSelection {
    #[must_use]
    pub fn new(provider: Option<String>, model: Option<String>) -> Self {
        Self { provider, model }
    }
}

#[derive(Clone, Eq, PartialEq)]
enum ResolvedAuthMaterial {
    None,
    ApiKey {
        key: String,
        source: AuthSource,
    },
    OAuth {
        access_token: Option<String>,
        refresh_token: Option<String>,
        expires_at: Option<time::OffsetDateTime>,
        source: AuthSource,
    },
}

impl fmt::Debug for ResolvedAuthMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("ResolvedAuthMaterial::None"),
            Self::ApiKey { source, .. } => formatter
                .debug_struct("ResolvedAuthMaterial::ApiKey")
                .field("source", source)
                .field("key", &"[redacted]")
                .finish(),
            Self::OAuth {
                source,
                access_token,
                refresh_token,
                expires_at,
            } => formatter
                .debug_struct("ResolvedAuthMaterial::OAuth")
                .field("source", source)
                .field("has_access_token", &access_token.is_some())
                .field("has_refresh_token", &refresh_token.is_some())
                .field("has_expiration", &expires_at.is_some())
                .finish(),
        }
    }
}

impl ResolvedAuthMaterial {
    fn auth_state(&self) -> AuthState {
        match self {
            Self::None => AuthState::not_required(),
            Self::ApiKey { source, .. } => AuthState::ready(AuthMaterialKind::ApiKey, *source),
            Self::OAuth { source, .. } => AuthState::ready(AuthMaterialKind::OAuth, *source),
        }
    }

    fn source(&self) -> Option<AuthSource> {
        match self {
            Self::None => None,
            Self::ApiKey { source, .. } | Self::OAuth { source, .. } => Some(*source),
        }
    }

    fn api_key(&self) -> Option<&str> {
        match self {
            Self::ApiKey { key, .. } => Some(key.as_str()),
            _ => None,
        }
    }

    fn oauth_access_token(&self) -> Option<&str> {
        match self {
            Self::OAuth { access_token, .. } => access_token.as_deref(),
            _ => None,
        }
    }

    fn oauth_refresh_token(&self) -> Option<&str> {
        match self {
            Self::OAuth { refresh_token, .. } => refresh_token.as_deref(),
            _ => None,
        }
    }

    fn oauth_expires_at(&self) -> Option<time::OffsetDateTime> {
        match self {
            Self::OAuth { expires_at, .. } => *expires_at,
            _ => None,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ResolvedProviderExecution {
    provider: ProviderDescriptor,
    model: String,
    api_base: String,
    auth: ResolvedAuthMaterial,
}

impl fmt::Debug for ResolvedProviderExecution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedProviderExecution")
            .field("provider", &self.provider.id)
            .field("model", &self.model)
            .field("api_base", &self.api_base)
            .field("auth", &self.auth)
            .finish()
    }
}

impl ResolvedProviderExecution {
    #[must_use]
    pub fn provider(&self) -> &ProviderDescriptor {
        &self.provider
    }

    #[must_use]
    pub fn provider_id(&self) -> &str {
        &self.provider.id
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    #[must_use]
    pub fn api_base(&self) -> &str {
        &self.api_base
    }

    #[must_use]
    pub fn auth_state(&self) -> AuthState {
        self.auth.auth_state()
    }

    #[must_use]
    pub fn auth_source(&self) -> Option<AuthSource> {
        self.auth.source()
    }

    pub(crate) fn api_key(&self) -> Result<&str> {
        self.auth.api_key().ok_or_else(|| {
            WonderError::validation(format!(
                "provider `{}` is not configured with api-key auth",
                self.provider.id
            ))
        })
    }

    pub(crate) fn oauth_access_token(&self) -> Result<&str> {
        self.auth.oauth_access_token().ok_or_else(|| {
            WonderError::validation(format!(
                "provider `{}` is not configured with oauth access-token auth",
                self.provider.id
            ))
        })
    }

    pub(crate) fn oauth_refresh_token(&self) -> Option<&str> {
        self.auth.oauth_refresh_token()
    }

    pub(crate) fn oauth_expires_at(&self) -> Option<time::OffsetDateTime> {
        self.auth.oauth_expires_at()
    }
}

#[derive(Clone, Debug, Default)]
pub struct ProviderRegistry {
    providers: BTreeMap<String, ProviderDescriptor>,
}

impl ProviderRegistry {
    #[must_use]
    pub fn builtin() -> Self {
        let mut registry = Self::default();
        registry
            .register(ProviderDescriptor {
                id: "copilot".into(),
                display_name: "GitHub Copilot".into(),
                auth_kind: AuthMaterialKind::OAuth,
                default_model: "gpt-4.1".into(),
                models: vec![
                    ModelDescriptor::new("gpt-4.1", "GPT-4.1"),
                    ModelDescriptor::new("claude-sonnet-4", "Claude Sonnet 4"),
                ],
                api_base: Some(DEFAULT_COPILOT_API_BASE.into()),
                api_key_env: None,
            })
            .expect("builtin provider");
        registry
            .register(ProviderDescriptor {
                id: "openai".into(),
                display_name: "OpenAI".into(),
                auth_kind: AuthMaterialKind::ApiKey,
                default_model: "gpt-4.1".into(),
                models: vec![
                    ModelDescriptor::new("gpt-4.1", "GPT-4.1"),
                    ModelDescriptor::new("gpt-4o-mini", "GPT-4o mini"),
                ],
                api_base: Some("https://api.openai.com/v1".into()),
                api_key_env: Some("OPENAI_API_KEY".into()),
            })
            .expect("builtin provider");
        registry
            .register(ProviderDescriptor {
                id: "anthropic".into(),
                display_name: "Anthropic".into(),
                auth_kind: AuthMaterialKind::ApiKey,
                default_model: "claude-3-7-sonnet-latest".into(),
                models: vec![
                    ModelDescriptor::new("claude-3-7-sonnet-latest", "Claude 3.7 Sonnet Latest"),
                    ModelDescriptor::new("claude-3-5-haiku-latest", "Claude 3.5 Haiku Latest"),
                ],
                api_base: Some("https://api.anthropic.com".into()),
                api_key_env: Some("ANTHROPIC_API_KEY".into()),
            })
            .expect("builtin provider");
        registry
    }

    pub fn register(&mut self, provider: ProviderDescriptor) -> Result<()> {
        if self.providers.contains_key(&provider.id) {
            return Err(WonderError::validation(format!(
                "duplicate provider id: {}",
                provider.id
            )));
        }
        self.providers.insert(provider.id.clone(), provider);
        Ok(())
    }

    #[must_use]
    pub fn get(&self, provider: &str) -> Option<&ProviderDescriptor> {
        self.providers.get(provider)
    }

    pub fn providers(&self) -> impl Iterator<Item = &ProviderDescriptor> {
        self.providers.values()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderStatusReport {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub auth: AuthState,
    pub readiness: ProviderReadiness,
    pub available_providers: Vec<ProviderDescriptor>,
}

impl ProviderStatusReport {
    #[must_use]
    pub fn selection_label(&self) -> Option<String> {
        match (&self.provider, &self.model) {
            (Some(provider), Some(model)) => Some(format!("{provider}:{model}")),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProviderResolver {
    registry: ProviderRegistry,
}

impl Default for ProviderResolver {
    fn default() -> Self {
        Self::builtin()
    }
}

impl ProviderResolver {
    #[must_use]
    pub fn builtin() -> Self {
        Self {
            registry: ProviderRegistry::builtin(),
        }
    }

    #[must_use]
    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    pub fn load_report(&self, storage_dir: Option<&Path>) -> Result<ProviderStatusReport> {
        let settings = match storage_dir {
            Some(path) => SettingsStore::new(path).read()?,
            None => AgentSettings::default(),
        };
        let credentials = match storage_dir {
            Some(path) => CredentialStore::new(path).read()?,
            None => StoredCredentials::default(),
        };

        self.resolve_with_env(&settings, &credentials, std::env::vars())
    }

    pub fn load_execution(
        &self,
        storage_dir: Option<&Path>,
        selection: ProviderSelection,
    ) -> Result<ResolvedProviderExecution> {
        let settings = match storage_dir {
            Some(path) => SettingsStore::new(path).read()?,
            None => AgentSettings::default(),
        };
        let credentials = match storage_dir {
            Some(path) => CredentialStore::new(path).read()?,
            None => StoredCredentials::default(),
        };

        self.resolve_execution_with_env(&settings, &credentials, std::env::vars(), &selection)
    }

    pub fn resolve_with_env<I, K, V>(
        &self,
        settings: &AgentSettings,
        credentials: &StoredCredentials,
        env: I,
    ) -> Result<ProviderStatusReport>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        let env = env
            .into_iter()
            .map(|(key, value)| (key.as_ref().to_string(), value.into()))
            .collect::<BTreeMap<_, _>>();

        let provider = self.select_provider(settings, credentials, &env, None)?;
        let available_providers = self.registry.providers().cloned().collect::<Vec<_>>();

        let Some(provider_id) = provider.clone() else {
            return Ok(ProviderStatusReport {
                provider: None,
                model: None,
                auth: AuthState::default(),
                readiness: ProviderReadiness::Unconfigured,
                available_providers,
            });
        };

        let descriptor = self.registry.get(&provider_id).ok_or_else(|| {
            WonderError::validation(format!("unknown provider selection: {provider_id}"))
        })?;
        let model = self.resolve_model(descriptor, settings, None, None)?;
        let auth = self.resolve_auth(descriptor, credentials, &env)?;
        let readiness = if auth.is_ready() {
            ProviderReadiness::Ready
        } else {
            ProviderReadiness::MissingAuth
        };

        Ok(ProviderStatusReport {
            provider: Some(provider_id),
            model: Some(model),
            auth,
            readiness,
            available_providers,
        })
    }

    pub fn resolve_execution_with_env<I, K, V>(
        &self,
        settings: &AgentSettings,
        credentials: &StoredCredentials,
        env: I,
        selection: &ProviderSelection,
    ) -> Result<ResolvedProviderExecution>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        let env = env
            .into_iter()
            .map(|(key, value)| (key.as_ref().to_string(), value.into()))
            .collect::<BTreeMap<_, _>>();

        let provider_id = self
            .select_provider(settings, credentials, &env, selection.provider.as_deref())?
            .ok_or_else(|| {
                WonderError::validation(
                    "no provider configured; select a provider/model or supply exactly one ready provider auth source",
                )
            })?;
        let descriptor = self.registry.get(&provider_id).ok_or_else(|| {
            WonderError::validation(format!("unknown provider selection: {provider_id}"))
        })?;
        let model = self.resolve_model(
            descriptor,
            settings,
            selection.provider.as_deref(),
            selection.model.as_deref(),
        )?;
        let api_base = self.resolve_api_base(descriptor, settings)?;
        let auth = self
            .resolve_auth_material(descriptor, credentials, &env)?
            .ok_or_else(|| self.missing_auth_error(descriptor))?;

        Ok(ResolvedProviderExecution {
            provider: descriptor.clone(),
            model,
            api_base,
            auth,
        })
    }

    fn select_provider(
        &self,
        settings: &AgentSettings,
        credentials: &StoredCredentials,
        env: &BTreeMap<String, String>,
        explicit_provider: Option<&str>,
    ) -> Result<Option<String>> {
        if let Some(provider) = explicit_provider {
            if self.registry.get(provider).is_none() {
                return Err(WonderError::validation(format!(
                    "unknown provider selection: {provider}"
                )));
            }
            return Ok(Some(provider.to_string()));
        }

        if let Some(provider) = settings.selected_provider.as_deref() {
            if self.registry.get(provider).is_none() {
                return Err(WonderError::validation(format!(
                    "unknown provider in settings: {provider}"
                )));
            }
            return Ok(Some(provider.to_string()));
        }

        let ready = self
            .registry
            .providers()
            .filter_map(|provider| {
                self.resolve_auth(provider, credentials, env)
                    .ok()
                    .filter(|auth| auth.is_ready())
                    .map(|_| provider.id.as_str())
            })
            .collect::<Vec<_>>();

        if ready.len() == 1 {
            Ok(ready.first().map(|provider| (*provider).to_string()))
        } else {
            Ok(None)
        }
    }

    fn resolve_model(
        &self,
        provider: &ProviderDescriptor,
        settings: &AgentSettings,
        explicit_provider: Option<&str>,
        explicit_model: Option<&str>,
    ) -> Result<String> {
        let stored_model = if explicit_provider.is_some()
            && settings.selected_provider.as_deref() != explicit_provider
        {
            None
        } else {
            settings.selected_model.clone()
        };
        let mut configured = explicit_model
            .map(ToString::to_string)
            .or(stored_model)
            .or_else(|| {
                settings
                    .providers
                    .get(&provider.id)
                    .and_then(|config| config.model.clone())
            })
            .unwrap_or_else(|| provider.default_model.clone());

        if explicit_model.is_none() && settings.fast_mode {
            configured = if is_fast_model_id(&configured) {
                configured
            } else if let Some(fast_model) = provider.preferred_fast_model() {
                fast_model.id.clone()
            } else {
                configured
            };
        }

        provider.model(&configured).ok_or_else(|| {
            WonderError::validation(format!(
                "unknown model `{configured}` for provider `{}`",
                provider.id
            ))
        })?;

        Ok(configured)
    }

    fn resolve_api_base(
        &self,
        provider: &ProviderDescriptor,
        settings: &AgentSettings,
    ) -> Result<String> {
        settings
            .providers
            .get(&provider.id)
            .and_then(|config| config.api_base.clone())
            .or_else(|| provider.api_base.clone())
            .filter(|api_base| !api_base.trim().is_empty())
            .ok_or_else(|| {
                WonderError::validation(format!(
                    "provider `{}` does not declare an API base URL",
                    provider.id
                ))
            })
    }

    fn resolve_auth(
        &self,
        provider: &ProviderDescriptor,
        credentials: &StoredCredentials,
        env: &BTreeMap<String, String>,
    ) -> Result<AuthState> {
        if provider.auth_kind == AuthMaterialKind::OAuth
            && let Some(AuthMaterial::OAuth {
                access_token,
                refresh_token,
                expires_at,
            }) = credentials.providers.get(&provider.id)
            && access_token
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
            && oauth_access_token_expired(*expires_at)
            && refresh_token
                .as_deref()
                .map(str::trim)
                .is_none_or(|value| value.is_empty())
        {
            return Ok(AuthState::pending(
                AuthMaterialKind::OAuth,
                Some(AuthSource::CredentialsFile),
                "stored oauth access token expired; run `wonder-of-u login --provider copilot` to refresh credentials",
            ));
        }
        Ok(
            match self.resolve_auth_material(provider, credentials, env)? {
                Some(material) => material.auth_state(),
                None => match provider.auth_kind {
                    AuthMaterialKind::None => AuthState::not_required(),
                    AuthMaterialKind::ApiKey => AuthState::missing(AuthMaterialKind::ApiKey),
                    AuthMaterialKind::OAuth => match credentials.providers.get(&provider.id) {
                        Some(AuthMaterial::OAuth {
                            access_token,
                            refresh_token,
                            expires_at,
                        }) => {
                            let message = if access_token
                                .as_deref()
                                .map(str::trim)
                                .is_some_and(|value| !value.is_empty())
                                && oauth_access_token_expired(*expires_at)
                                && refresh_token
                                    .as_deref()
                                    .map(str::trim)
                                    .is_none_or(|value| value.is_empty())
                            {
                                "stored oauth access token expired; run `wonder-of-u login --provider copilot` to refresh credentials"
                            } else {
                                "oauth credential placeholder stored without an access token"
                            };
                            AuthState::pending(
                                AuthMaterialKind::OAuth,
                                Some(AuthSource::CredentialsFile),
                                message,
                            )
                        }
                        Some(other) => {
                            return Err(WonderError::validation(format!(
                                "provider `{}` requires oauth auth, found incompatible credential kind: {}",
                                provider.id,
                                auth_material_label(other)
                            )));
                        }
                        None => AuthState::pending(
                            AuthMaterialKind::OAuth,
                            Some(AuthSource::Interactive),
                            "run `wonder-of-u login --provider copilot` to authorize",
                        ),
                    },
                },
            },
        )
    }

    fn resolve_auth_material(
        &self,
        provider: &ProviderDescriptor,
        credentials: &StoredCredentials,
        env: &BTreeMap<String, String>,
    ) -> Result<Option<ResolvedAuthMaterial>> {
        match provider.auth_kind {
            AuthMaterialKind::None => Ok(Some(ResolvedAuthMaterial::None)),
            AuthMaterialKind::ApiKey => {
                if let Some(env_var) = &provider.api_key_env {
                    if let Some(value) = env
                        .get(env_var)
                        .map(|value| value.trim())
                        .filter(|value| !value.is_empty())
                    {
                        return Ok(Some(ResolvedAuthMaterial::ApiKey {
                            key: value.to_string(),
                            source: AuthSource::Environment,
                        }));
                    }
                }

                match credentials.providers.get(&provider.id) {
                    Some(AuthMaterial::ApiKey { key }) if !key.trim().is_empty() => {
                        Ok(Some(ResolvedAuthMaterial::ApiKey {
                            key: key.clone(),
                            source: AuthSource::CredentialsFile,
                        }))
                    }
                    Some(AuthMaterial::ApiKey { .. }) | None => Ok(None),
                    Some(other) => Err(WonderError::validation(format!(
                        "provider `{}` requires api_key auth, found incompatible credential kind: {}",
                        provider.id,
                        auth_material_label(other)
                    ))),
                }
            }
            AuthMaterialKind::OAuth => match credentials.providers.get(&provider.id) {
                Some(AuthMaterial::OAuth {
                    access_token,
                    refresh_token,
                    expires_at,
                }) if access_token
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|value| !value.is_empty()) =>
                {
                    Ok(Some(ResolvedAuthMaterial::OAuth {
                        access_token: access_token.clone(),
                        refresh_token: refresh_token.clone(),
                        expires_at: *expires_at,
                        source: AuthSource::CredentialsFile,
                    }))
                }
                Some(AuthMaterial::OAuth { .. }) | None => Ok(None),
                Some(other) => Err(WonderError::validation(format!(
                    "provider `{}` requires oauth auth, found incompatible credential kind: {}",
                    provider.id,
                    auth_material_label(other)
                ))),
            },
        }
    }

    fn missing_auth_error(&self, provider: &ProviderDescriptor) -> WonderError {
        match provider.auth_kind {
            AuthMaterialKind::None => WonderError::validation(format!(
                "provider `{}` does not require authentication",
                provider.id
            )),
            AuthMaterialKind::ApiKey => {
                let env_hint = provider
                    .api_key_env
                    .as_deref()
                    .unwrap_or("<provider api key env var>");
                WonderError::validation(format!(
                    "provider `{}` is missing api-key auth; set {env_hint} or run `wonder-of-u login --provider {} --api-key ...`",
                    provider.id, provider.id
                ))
            }
            AuthMaterialKind::OAuth => WonderError::validation(format!(
                "provider `{}` is missing oauth auth; run `wonder-of-u login --provider {}`",
                provider.id, provider.id
            )),
        }
    }
}

fn is_fast_model_id(model_id: &str) -> bool {
    let normalized = model_id.to_ascii_lowercase();
    normalized.contains("mini") || normalized.contains("haiku")
}

fn oauth_access_token_expired(expires_at: Option<OffsetDateTime>) -> bool {
    expires_at.is_some_and(|value| value <= OffsetDateTime::now_utc())
}

#[cfg(test)]
mod tests {
    use wonder_of_u_core::{AuthStatus, ProviderReadiness};

    use super::*;

    #[test]
    fn resolver_uses_stored_selection_and_credentials() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("openai".into()),
            selected_model: Some("gpt-4.1".into()),
            ..AgentSettings::default()
        };
        let credentials = StoredCredentials {
            providers: BTreeMap::from([(
                "openai".into(),
                AuthMaterial::ApiKey {
                    key: "secret".into(),
                },
            )]),
        };

        let report = resolver
            .resolve_with_env(
                &settings,
                &credentials,
                std::iter::empty::<(&str, String)>(),
            )
            .expect("resolve provider");

        assert_eq!(report.selection_label().as_deref(), Some("openai:gpt-4.1"));
        assert_eq!(report.auth.status, AuthStatus::Ready);
        assert_eq!(report.readiness, ProviderReadiness::Ready);
    }

    #[test]
    fn resolver_auto_selects_unique_ready_provider_from_env() {
        let resolver = ProviderResolver::builtin();

        let report = resolver
            .resolve_with_env(
                &AgentSettings::default(),
                &StoredCredentials::default(),
                [("OPENAI_API_KEY", "secret".to_string())],
            )
            .expect("resolve provider");

        assert_eq!(report.provider.as_deref(), Some("openai"));
        assert_eq!(report.model.as_deref(), Some("gpt-4.1"));
        assert_eq!(report.auth.source_label(), Some("environment"));
    }

    #[test]
    fn oauth_provider_reports_pending_until_login_runs() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("copilot".into()),
            ..AgentSettings::default()
        };

        let report = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
            )
            .expect("resolve provider");

        assert_eq!(report.auth.status, AuthStatus::Pending);
        assert_eq!(report.readiness, ProviderReadiness::MissingAuth);
    }

    #[test]
    fn oauth_provider_reports_ready_when_access_token_exists() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("copilot".into()),
            ..AgentSettings::default()
        };
        let credentials = StoredCredentials {
            providers: BTreeMap::from([(
                "copilot".into(),
                AuthMaterial::OAuth {
                    access_token: Some("oauth-secret".into()),
                    refresh_token: None,
                    expires_at: None,
                },
            )]),
        };

        let report = resolver
            .resolve_with_env(
                &settings,
                &credentials,
                std::iter::empty::<(&str, String)>(),
            )
            .expect("resolve provider");

        assert_eq!(report.auth.status, AuthStatus::Ready);
        assert_eq!(report.readiness, ProviderReadiness::Ready);
    }

    #[test]
    fn expired_oauth_provider_reports_pending_without_refresh_token() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("copilot".into()),
            ..AgentSettings::default()
        };
        let credentials = StoredCredentials {
            providers: BTreeMap::from([(
                "copilot".into(),
                AuthMaterial::OAuth {
                    access_token: Some("oauth-secret".into()),
                    refresh_token: None,
                    expires_at: Some(OffsetDateTime::now_utc() - time::Duration::minutes(5)),
                },
            )]),
        };

        let report = resolver
            .resolve_with_env(
                &settings,
                &credentials,
                std::iter::empty::<(&str, String)>(),
            )
            .expect("resolve provider");

        assert_eq!(report.auth.status, AuthStatus::Pending);
        assert_eq!(report.readiness, ProviderReadiness::MissingAuth);
    }

    #[test]
    fn resolver_rejects_unknown_models() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("openai".into()),
            selected_model: Some("bad-model".into()),
            ..AgentSettings::default()
        };

        let error = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
            )
            .expect_err("unknown model");

        assert!(error.to_string().contains("unknown model `bad-model`"));
    }

    #[test]
    fn load_execution_uses_api_base_override_and_env_precedence() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("openai".into()),
            providers: BTreeMap::from([(
                "openai".into(),
                crate::ProviderOverride {
                    model: None,
                    api_base: Some("http://localhost:4100/v1".into()),
                },
            )]),
            ..AgentSettings::default()
        };
        let credentials = StoredCredentials {
            providers: BTreeMap::from([(
                "openai".into(),
                AuthMaterial::ApiKey {
                    key: "stored-secret".into(),
                },
            )]),
        };

        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &credentials,
                [("OPENAI_API_KEY", "env-secret".to_string())],
                &ProviderSelection::default(),
            )
            .expect("resolve execution");

        assert_eq!(resolved.provider_id(), "openai");
        assert_eq!(resolved.model(), "gpt-4.1");
        assert_eq!(resolved.api_base(), "http://localhost:4100/v1");
        assert_eq!(resolved.auth_source(), Some(AuthSource::Environment));
    }

    #[test]
    fn load_execution_rejects_missing_api_key_auth() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("openai".into()),
            ..AgentSettings::default()
        };

        let error = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
                &ProviderSelection::default(),
            )
            .expect_err("missing auth");

        let text = error.to_string();
        assert!(text.contains("OPENAI_API_KEY"));
        assert!(text.contains("wonder-of-u login"));
    }

    #[test]
    fn explicit_provider_override_ignores_incompatible_stored_model() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("openai".into()),
            selected_model: Some("gpt-4.1".into()),
            ..AgentSettings::default()
        };
        let credentials = StoredCredentials {
            providers: BTreeMap::from([(
                "anthropic".into(),
                AuthMaterial::ApiKey {
                    key: "anthropic-secret".into(),
                },
            )]),
        };

        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &credentials,
                std::iter::empty::<(&str, String)>(),
                &ProviderSelection::new(Some("anthropic".into()), None),
            )
            .expect("resolve anthropic execution");

        assert_eq!(resolved.provider_id(), "anthropic");
        assert_eq!(resolved.model(), "claude-3-7-sonnet-latest");
    }

    #[test]
    fn fast_mode_prefers_provider_fast_model() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("openai".into()),
            selected_model: Some("gpt-4.1".into()),
            fast_mode: true,
            ..AgentSettings::default()
        };
        let credentials = StoredCredentials {
            providers: BTreeMap::from([(
                "openai".into(),
                AuthMaterial::ApiKey {
                    key: "openai-secret".into(),
                },
            )]),
        };

        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &credentials,
                std::iter::empty::<(&str, String)>(),
                &ProviderSelection::default(),
            )
            .expect("resolve openai execution");

        assert_eq!(resolved.model(), "gpt-4o-mini");
    }

    #[test]
    fn explicit_model_override_skips_fast_mode_remap() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("openai".into()),
            fast_mode: true,
            ..AgentSettings::default()
        };
        let credentials = StoredCredentials {
            providers: BTreeMap::from([(
                "openai".into(),
                AuthMaterial::ApiKey {
                    key: "openai-secret".into(),
                },
            )]),
        };

        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &credentials,
                std::iter::empty::<(&str, String)>(),
                &ProviderSelection::new(Some("openai".into()), Some("gpt-4.1".into())),
            )
            .expect("resolve openai execution");

        assert_eq!(resolved.model(), "gpt-4.1");
    }

    #[test]
    fn provider_descriptor_reports_fast_mode_support() {
        let registry = ProviderRegistry::builtin();
        assert!(
            registry
                .get("openai")
                .expect("openai provider")
                .supports_fast_mode()
        );
        assert!(
            registry
                .get("anthropic")
                .expect("anthropic provider")
                .supports_fast_mode()
        );
        assert!(
            !registry
                .get("copilot")
                .expect("copilot provider")
                .supports_fast_mode()
        );
    }
}
