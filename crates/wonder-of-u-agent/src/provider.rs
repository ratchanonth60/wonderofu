use std::{collections::BTreeMap, fmt, path::Path};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use wonder_of_u_core::{
    AuthMaterialKind, AuthSource, AuthState, ProviderReadiness, Result, WonderError,
};

use crate::{
    auth::{
        AuthMaterial, AwsCredentials, DEFAULT_COPILOT_API_BASE, StoredCredentials,
        parse_aws_credentials_file,
    },
    config::{AgentSettings, CredentialStore, SettingsStore},
    protocol::WireProtocol,
};
/// Represents model descriptor
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// Stores the id
    pub id: String,
    /// Stores the display name
    pub display_name: String,
}

impl ModelDescriptor {
    /// Creates a new value
    #[must_use]
    pub fn new(id: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
        }
    }
}
/// Represents provider descriptor
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    /// Stores the id
    pub id: String,
    /// Stores the display name
    pub display_name: String,
    /// Stores the auth kind
    pub auth_kind: AuthMaterialKind,
    /// Stores the default model
    pub default_model: String,
    /// Stores the models
    pub models: Vec<ModelDescriptor>,
    /// Stores the api base
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
    /// Stores the api key env
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// The HTTP wire-protocol shape this provider uses.
    ///
    /// Defaults to [`WireProtocol::OpenAiCompat`] when deserialising older
    /// persisted descriptors that predate this field.
    #[serde(default)]
    pub wire_protocol: WireProtocol,
    /// When `true`, the resolver validates that the requested model id exists
    /// in [`Self::models`] and rejects unknown ids.  When `false`, any
    /// non-empty model id string is accepted, enabling pass-through to
    /// providers whose model catalogues are not enumerated here (e.g.
    /// dynamically-routed or custom deployments).
    #[serde(default)]
    pub strict_model_validation: bool,
}

impl ProviderDescriptor {
    /// Handles model
    #[must_use]
    pub fn model(&self, model_id: &str) -> Option<&ModelDescriptor> {
        self.models.iter().find(|model| model.id == model_id)
    }
    /// Handles preferred fast model
    #[must_use]
    pub fn preferred_fast_model(&self) -> Option<&ModelDescriptor> {
        self.models.iter().find(|model| is_fast_model_id(&model.id))
    }
    /// Returns whether fast mode
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

/// Default AWS Bedrock API base URL (us-east-1).
pub const DEFAULT_BEDROCK_API_BASE: &str = "https://bedrock-runtime.us-east-1.amazonaws.com";

/// Constructs a [`ProviderDescriptor`] for an OpenAI-compatible gateway provider.
///
/// All gateway providers share the same wire protocol ([`WireProtocol::OpenAiCompat`])
/// and auth kind ([`AuthMaterialKind::ApiKey`]).  Model validation is intentionally
/// non-strict so that callers can use any model string the upstream gateway
/// accepts without needing a locally-maintained catalogue.
///
/// # Examples
///
/// ```
/// let desc = wonder_of_u_agent::provider::openai_compat_gateway(
///     "groq", "Groq", "https://api.groq.com/openai/v1",
///     "GROQ_API_KEY", "llama-3.3-70b-versatile",
/// );
/// assert_eq!(desc.id, "groq");
/// ```
#[must_use]
pub fn openai_compat_gateway(
    id: &str,
    display_name: &str,
    api_base: &str,
    api_key_env: &str,
    default_model: &str,
) -> ProviderDescriptor {
    ProviderDescriptor {
        id: id.into(),
        display_name: display_name.into(),
        auth_kind: AuthMaterialKind::ApiKey,
        default_model: default_model.into(),
        // Non-strict providers keep an empty catalogue; any non-empty model id is accepted.
        models: vec![],
        api_base: Some(api_base.into()),
        api_key_env: Some(api_key_env.into()),
        wire_protocol: WireProtocol::OpenAiCompat,
        strict_model_validation: false,
    }
}
/// Represents provider selection
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProviderSelection {
    /// Stores the provider
    pub provider: Option<String>,
    /// Stores the model
    pub model: Option<String>,
}

impl ProviderSelection {
    /// Creates a new value
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
    AwsSigV4 {
        credentials: AwsCredentials,
        source: AuthSource,
    },
    /// Short-lived bearer token sourced from `AWS_BEARER_TOKEN_BEDROCK`.
    AwsBearer {
        token: String,
        region: String,
        source: AuthSource,
    },
    /// AWS credentials resolved from a named profile (`~/.aws/credentials`).
    AwsProfile {
        credentials: AwsCredentials,
        source: AuthSource,
    },
    /// GCP Vertex AI readiness (project + location + credentials path).
    GcpOAuth2 {
        project: String,
        location: String,
        credentials_source: String,
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
            Self::AwsSigV4 {
                source,
                credentials,
            } => formatter
                .debug_struct("ResolvedAuthMaterial::AwsSigV4")
                .field("source", source)
                .field("region", &credentials.region)
                .field("access_key_id", &"[redacted]")
                .field("has_session_token", &credentials.session_token.is_some())
                .finish(),
            Self::AwsBearer { source, region, .. } => formatter
                .debug_struct("ResolvedAuthMaterial::AwsBearer")
                .field("source", source)
                .field("region", region)
                .field("token", &"[redacted]")
                .finish(),
            Self::AwsProfile {
                source,
                credentials,
            } => formatter
                .debug_struct("ResolvedAuthMaterial::AwsProfile")
                .field("source", source)
                .field("region", &credentials.region)
                .field("access_key_id", &"[redacted]")
                .field("has_session_token", &credentials.session_token.is_some())
                .finish(),
            Self::GcpOAuth2 {
                source,
                project,
                location,
                ..
            } => formatter
                .debug_struct("ResolvedAuthMaterial::GcpOAuth2")
                .field("source", source)
                .field("project", project)
                .field("location", location)
                .field("credentials_source", &"[redacted]")
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
            Self::AwsSigV4 { source, .. } => AuthState::ready(AuthMaterialKind::AwsSigV4, *source),
            Self::AwsBearer { source, .. } => {
                AuthState::ready(AuthMaterialKind::AwsBearer, *source)
            }
            Self::AwsProfile { source, .. } => {
                AuthState::ready(AuthMaterialKind::AwsProfile, *source)
            }
            Self::GcpOAuth2 { source, .. } => {
                AuthState::ready(AuthMaterialKind::GcpOAuth2, *source)
            }
        }
    }

    fn source(&self) -> Option<AuthSource> {
        match self {
            Self::None => None,
            Self::ApiKey { source, .. }
            | Self::OAuth { source, .. }
            | Self::AwsSigV4 { source, .. }
            | Self::AwsBearer { source, .. }
            | Self::AwsProfile { source, .. }
            | Self::GcpOAuth2 { source, .. } => Some(*source),
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

    /// Returns the underlying [`AwsCredentials`] for both `AwsSigV4` and
    /// `AwsProfile` variants, which share the same credential shape.
    fn aws_credentials(&self) -> Option<&AwsCredentials> {
        match self {
            Self::AwsSigV4 { credentials, .. } | Self::AwsProfile { credentials, .. } => {
                Some(credentials)
            }
            _ => None,
        }
    }
}
/// Represents resolved provider execution
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
    /// Handles provider
    #[must_use]
    pub fn provider(&self) -> &ProviderDescriptor {
        &self.provider
    }
    /// Handles provider id
    #[must_use]
    pub fn provider_id(&self) -> &str {
        &self.provider.id
    }
    /// Handles model
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }
    /// Handles api base
    #[must_use]
    pub fn api_base(&self) -> &str {
        &self.api_base
    }
    /// Handles auth state
    #[must_use]
    pub fn auth_state(&self) -> AuthState {
        self.auth.auth_state()
    }
    /// Handles auth source
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

    pub(crate) fn aws_credentials(&self) -> Result<&AwsCredentials> {
        self.auth.aws_credentials().ok_or_else(|| {
            WonderError::validation(format!(
                "provider `{}` is not configured with AWS SigV4 or profile auth",
                self.provider.id
            ))
        })
    }

    /// Returns `(token, region)` when the provider resolved via an AWS bearer
    /// token.  Returns an error for callers that expect this credential kind
    /// but encounter a different (or absent) material.
    ///
    /// Intentionally exposed for the Bedrock runtime stage; unused in v1.
    #[allow(dead_code)]
    pub(crate) fn aws_bearer_token(&self) -> Result<(&str, &str)> {
        match &self.auth {
            ResolvedAuthMaterial::AwsBearer { token, region, .. } => {
                Ok((token.as_str(), region.as_str()))
            }
            _ => Err(WonderError::validation(format!(
                "provider `{}` is not configured with AWS bearer-token auth",
                self.provider.id
            ))),
        }
    }
}
/// Stores provider registry
#[derive(Clone, Debug, Default)]
pub struct ProviderRegistry {
    providers: BTreeMap<String, ProviderDescriptor>,
}

impl ProviderRegistry {
    /// Handles builtin
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
                wire_protocol: WireProtocol::Copilot,
                strict_model_validation: true,
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
                wire_protocol: WireProtocol::OpenAiCompat,
                strict_model_validation: true,
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
                wire_protocol: WireProtocol::AnthropicCompat,
                strict_model_validation: true,
            })
            .expect("builtin provider");
        registry
            .register(ProviderDescriptor {
                id: "bedrock".into(),
                display_name: "Amazon Bedrock".into(),
                auth_kind: AuthMaterialKind::AwsSigV4,
                default_model: "anthropic.claude-3-7-sonnet-20250219-v1:0".into(),
                models: vec![
                    ModelDescriptor::new(
                        "anthropic.claude-3-7-sonnet-20250219-v1:0",
                        "Claude 3.7 Sonnet (Bedrock)",
                    ),
                    ModelDescriptor::new(
                        "anthropic.claude-3-5-haiku-20241022-v1:0",
                        "Claude 3.5 Haiku (Bedrock)",
                    ),
                ],
                api_base: Some(DEFAULT_BEDROCK_API_BASE.into()),
                api_key_env: None,
                wire_protocol: WireProtocol::BedrockAnthropic,
                strict_model_validation: true,
            })
            .expect("builtin provider");

        // ── OpenAI-compatible gateway providers ──────────────────────────────
        // Each gateway speaks the standard OpenAI chat-completions wire protocol
        // and authenticates with a single API key env var.  Model validation is
        // non-strict so any model string accepted by the upstream gateway passes
        // through without needing a locally-maintained catalogue.
        let gateways: &[(&str, &str, &str, &str, &str)] = &[
            (
                "hyper",
                "Hyper AI",
                "https://api.hyper.ai/v1",
                "HYPER_API_KEY",
                "llama-3.3-70b",
            ),
            (
                "vercel",
                "Vercel AI",
                "https://api.v0.dev/v1",
                "VERCEL_API_KEY",
                "gpt-4o-mini",
            ),
            (
                "zai",
                "Z.ai",
                "https://api.z.ai/api/v1",
                "ZAI_API_KEY",
                "auto",
            ),
            (
                "minimax",
                "MiniMax",
                "https://api.minimax.io/v1",
                "MINIMAX_API_KEY",
                "MiniMax-Text-01",
            ),
            (
                "huggingface",
                "Hugging Face",
                "https://api-inference.huggingface.co/v1",
                "HF_TOKEN",
                "Qwen/Qwen2.5-72B-Instruct",
            ),
            (
                "cerebras",
                "Cerebras",
                "https://api.cerebras.ai/v1",
                "CEREBRAS_API_KEY",
                "llama-3.3-70b",
            ),
            (
                "openrouter",
                "OpenRouter",
                "https://openrouter.ai/api/v1",
                "OPENROUTER_API_KEY",
                "openai/gpt-4o-mini",
            ),
            (
                "ionet",
                "IO Intelligence",
                "https://api.intelligence.io.solutions/api/v1",
                "IONET_API_KEY",
                "meta-llama/Llama-3.3-70B-Instruct",
            ),
            (
                "groq",
                "Groq",
                "https://api.groq.com/openai/v1",
                "GROQ_API_KEY",
                "llama-3.3-70b-versatile",
            ),
            (
                "avian",
                "Avian",
                "https://app.avian.io/api/v1",
                "AVIAN_API_KEY",
                "gpt-4o-mini",
            ),
            (
                "opencode",
                "OpenCode",
                "https://opencode.ai/api/v1",
                "OPENCODE_API_KEY",
                "gpt-4o",
            ),
        ];
        for &(id, display_name, api_base, api_key_env, default_model) in gateways {
            registry
                .register(openai_compat_gateway(
                    id,
                    display_name,
                    api_base,
                    api_key_env,
                    default_model,
                ))
                .expect("builtin gateway provider");
        }

        registry
    }

    /// Handles register
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
    /// Handles get
    #[must_use]
    pub fn get(&self, provider: &str) -> Option<&ProviderDescriptor> {
        self.providers.get(provider)
    }

    /// Handles providers
    pub fn providers(&self) -> impl Iterator<Item = &ProviderDescriptor> {
        self.providers.values()
    }
}
/// Represents provider status report
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderStatusReport {
    /// Stores the provider
    pub provider: Option<String>,
    /// Stores the model
    pub model: Option<String>,
    /// Stores the auth
    pub auth: AuthState,
    /// Stores the readiness
    pub readiness: ProviderReadiness,
    /// Stores the available providers
    pub available_providers: Vec<ProviderDescriptor>,
}

impl ProviderStatusReport {
    /// Handles selection label
    #[must_use]
    pub fn selection_label(&self) -> Option<String> {
        match (&self.provider, &self.model) {
            (Some(provider), Some(model)) => Some(format!("{provider}:{model}")),
            _ => None,
        }
    }
}
/// Represents provider resolver
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
    /// Handles builtin
    #[must_use]
    pub fn builtin() -> Self {
        Self {
            registry: ProviderRegistry::builtin(),
        }
    }
    /// Builds the registry
    #[must_use]
    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    /// Loads report
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

    /// Loads report for selection
    pub fn load_report_for_selection(
        &self,
        storage_dir: Option<&Path>,
        selection: &ProviderSelection,
    ) -> Result<ProviderStatusReport> {
        let settings = match storage_dir {
            Some(path) => SettingsStore::new(path).read()?,
            None => AgentSettings::default(),
        };
        let credentials = match storage_dir {
            Some(path) => CredentialStore::new(path).read()?,
            None => StoredCredentials::default(),
        };

        self.resolve_report_for_selection_with_env(
            &settings,
            &credentials,
            std::env::vars(),
            selection,
        )
    }

    /// Loads execution
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

    /// Resolves with env
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

    /// Resolves report for selection with env
    pub fn resolve_report_for_selection_with_env<I, K, V>(
        &self,
        settings: &AgentSettings,
        credentials: &StoredCredentials,
        env: I,
        selection: &ProviderSelection,
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

        let provider =
            self.select_provider(settings, credentials, &env, selection.provider.as_deref())?;
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
        let model = self.resolve_model(
            descriptor,
            settings,
            selection.provider.as_deref(),
            selection.model.as_deref(),
        )?;
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

    /// Resolves execution with env
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

        if provider.strict_model_validation {
            // Reject model ids not listed in the provider's model catalogue.
            provider.model(&configured).ok_or_else(|| {
                WonderError::validation(format!(
                    "unknown model `{configured}` for provider `{}`",
                    provider.id
                ))
            })?;
        } else if configured.trim().is_empty() {
            // Non-strict providers still must not receive an empty model id.
            return Err(WonderError::validation(format!(
                "model id for provider `{}` must not be empty",
                provider.id
            )));
        }

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
                    AuthMaterialKind::AwsSigV4 => AuthState::missing(AuthMaterialKind::AwsSigV4),
                    AuthMaterialKind::AwsBearer => AuthState::missing(AuthMaterialKind::AwsBearer),
                    AuthMaterialKind::AwsProfile => {
                        AuthState::missing(AuthMaterialKind::AwsProfile)
                    }
                    AuthMaterialKind::GcpOAuth2 => AuthState::missing(AuthMaterialKind::GcpOAuth2),
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
            AuthMaterialKind::AwsSigV4 => {
                // Bedrock resolution priority:
                //   1. AWS_BEARER_TOKEN_BEDROCK  → AwsBearer (no signing needed)
                //   2. AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY → AwsSigV4
                //   3. AWS_PROFILE + credentials file → AwsProfile
                let region = env
                    .get("AWS_REGION")
                    .or_else(|| env.get("AWS_DEFAULT_REGION"))
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "us-east-1".to_string());

                // Priority 1: bearer token
                if let Some(token) = env
                    .get("AWS_BEARER_TOKEN_BEDROCK")
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty())
                {
                    return Ok(Some(ResolvedAuthMaterial::AwsBearer {
                        token: token.to_string(),
                        region,
                        source: AuthSource::Environment,
                    }));
                }

                // Priority 2: static SigV4 env vars
                if let (Some(access_key_id), Some(secret_access_key)) = (
                    env.get("AWS_ACCESS_KEY_ID")
                        .map(|v| v.trim())
                        .filter(|v| !v.is_empty()),
                    env.get("AWS_SECRET_ACCESS_KEY")
                        .map(|v| v.trim())
                        .filter(|v| !v.is_empty()),
                ) {
                    let session_token = env
                        .get("AWS_SESSION_TOKEN")
                        .map(|v| v.trim())
                        .filter(|v| !v.is_empty())
                        .map(ToString::to_string);
                    return Ok(Some(ResolvedAuthMaterial::AwsSigV4 {
                        credentials: AwsCredentials {
                            access_key_id: access_key_id.to_string(),
                            secret_access_key: secret_access_key.to_string(),
                            session_token,
                            region,
                        },
                        source: AuthSource::Environment,
                    }));
                }

                // Priority 3: named profile from credentials file
                if let Some(resolved) = resolve_profile_from_env_map(env) {
                    return Ok(Some(ResolvedAuthMaterial::AwsProfile {
                        credentials: resolved,
                        source: AuthSource::CredentialsFile,
                    }));
                }

                Ok(None)
            }
            AuthMaterialKind::AwsBearer => {
                // Standalone bearer-token provider (not yet registered, but
                // the resolution path is available for future providers).
                let region = env
                    .get("AWS_REGION")
                    .or_else(|| env.get("AWS_DEFAULT_REGION"))
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "us-east-1".to_string());

                if let Some(token) = env
                    .get("AWS_BEARER_TOKEN_BEDROCK")
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty())
                {
                    Ok(Some(ResolvedAuthMaterial::AwsBearer {
                        token: token.to_string(),
                        region,
                        source: AuthSource::Environment,
                    }))
                } else {
                    Ok(None)
                }
            }
            AuthMaterialKind::AwsProfile => {
                // Standalone profile provider.
                if let Some(resolved) = resolve_profile_from_env_map(env) {
                    Ok(Some(ResolvedAuthMaterial::AwsProfile {
                        credentials: resolved,
                        source: AuthSource::CredentialsFile,
                    }))
                } else {
                    Ok(None)
                }
            }
            AuthMaterialKind::GcpOAuth2 => {
                // All three GCP vars must be present for readiness.
                let project = env
                    .get("VERTEXAI_PROJECT")
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty())
                    .map(ToString::to_string);
                let location = env
                    .get("VERTEXAI_LOCATION")
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty())
                    .map(ToString::to_string);
                let credentials_source = env
                    .get("GOOGLE_APPLICATION_CREDENTIALS")
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty())
                    .map(ToString::to_string);

                match (project, location, credentials_source) {
                    (Some(project), Some(location), Some(credentials_source)) => {
                        Ok(Some(ResolvedAuthMaterial::GcpOAuth2 {
                            project,
                            location,
                            credentials_source,
                            source: AuthSource::Environment,
                        }))
                    }
                    _ => Ok(None),
                }
            }
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
            AuthMaterialKind::AwsSigV4 => WonderError::validation(format!(
                "provider `{}` requires AWS auth; set AWS_BEARER_TOKEN_BEDROCK, or AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY, or AWS_PROFILE with a valid ~/.aws/credentials entry",
                provider.id
            )),
            AuthMaterialKind::AwsBearer => WonderError::validation(format!(
                "provider `{}` requires an AWS bearer token; set AWS_BEARER_TOKEN_BEDROCK",
                provider.id
            )),
            AuthMaterialKind::AwsProfile => WonderError::validation(format!(
                "provider `{}` requires a named AWS profile; set AWS_PROFILE and ensure ~/.aws/credentials (or AWS_SHARED_CREDENTIALS_FILE) contains that profile",
                provider.id
            )),
            AuthMaterialKind::GcpOAuth2 => WonderError::validation(format!(
                "provider `{}` requires GCP credentials; set VERTEXAI_PROJECT, VERTEXAI_LOCATION, and GOOGLE_APPLICATION_CREDENTIALS",
                provider.id
            )),
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

/// Resolves a named AWS profile from the environment variables available in
/// `env` (the test-injectable `BTreeMap` snapshot).
///
/// This mirrors [`crate::auth::resolve_aws_profile_from_env`] but operates on
/// the env-snapshot used by the provider resolver, enabling test injection.
/// When neither `AWS_SHARED_CREDENTIALS_FILE` nor `HOME` are present in the
/// snapshot, profile resolution is skipped (returns `None`).
fn resolve_profile_from_env_map(env: &BTreeMap<String, String>) -> Option<AwsCredentials> {
    let profile = env
        .get("AWS_PROFILE")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| "default".to_string());

    // Derive the credentials file path entirely from the injected env map so
    // that tests with an empty map never accidentally read $HOME/.aws/credentials
    // from the host filesystem.
    let creds_path = if let Some(explicit) = env
        .get("AWS_SHARED_CREDENTIALS_FILE")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        std::path::PathBuf::from(explicit)
    } else if let Some(home) = env.get("HOME").map(|v| v.trim()).filter(|v| !v.is_empty()) {
        std::path::Path::new(home).join(".aws").join("credentials")
    } else {
        // No HOME in the snapshot → skip profile resolution.
        return None;
    };

    let content = std::fs::read_to_string(&creds_path).ok()?;

    // Region from the env snapshot (injected map), with a default of us-east-1.
    // We pass this via a temporary std::env mutation only in test contexts where
    // --test-threads=1 is mandated; for production use the real env is fine.
    parse_aws_credentials_file(&content, &profile)
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

    #[test]
    fn bedrock_provider_registered_in_builtin_registry() {
        let registry = ProviderRegistry::builtin();
        let bedrock = registry.get("bedrock").expect("bedrock provider");
        assert_eq!(
            bedrock.auth_kind,
            wonder_of_u_core::AuthMaterialKind::AwsSigV4
        );
        assert_eq!(
            bedrock.default_model,
            "anthropic.claude-3-7-sonnet-20250219-v1:0"
        );
        assert!(bedrock.models.len() >= 2);
    }

    #[test]
    fn bedrock_provider_resolves_from_aws_env_vars() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("bedrock".into()),
            ..AgentSettings::default()
        };

        let report = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                [
                    ("AWS_ACCESS_KEY_ID", "AKIAIOSFODNN7EXAMPLE".to_string()),
                    (
                        "AWS_SECRET_ACCESS_KEY",
                        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
                    ),
                    ("AWS_REGION", "us-west-2".to_string()),
                ],
            )
            .expect("resolve bedrock provider");

        assert_eq!(report.provider.as_deref(), Some("bedrock"));
        assert_eq!(report.auth.status, wonder_of_u_core::AuthStatus::Ready);
        assert_eq!(
            report.auth.kind,
            wonder_of_u_core::AuthMaterialKind::AwsSigV4
        );
        assert_eq!(report.auth.source_label(), Some("environment"));
        assert_eq!(report.readiness, ProviderReadiness::Ready);
    }

    #[test]
    fn bedrock_provider_reports_missing_auth_without_env_vars() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("bedrock".into()),
            ..AgentSettings::default()
        };

        let report = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
            )
            .expect("resolve bedrock provider");

        assert_eq!(report.auth.status, wonder_of_u_core::AuthStatus::Missing);
        assert_eq!(report.readiness, ProviderReadiness::MissingAuth);
    }

    #[test]
    fn bedrock_provider_resolves_execution_with_sigv4_auth() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("bedrock".into()),
            ..AgentSettings::default()
        };

        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [
                    ("AWS_ACCESS_KEY_ID", "AKIAIOSFODNN7EXAMPLE".to_string()),
                    (
                        "AWS_SECRET_ACCESS_KEY",
                        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
                    ),
                    ("AWS_SESSION_TOKEN", "session-token-xyz".to_string()),
                    ("AWS_REGION", "eu-west-1".to_string()),
                ],
                &ProviderSelection::default(),
            )
            .expect("resolve bedrock execution");

        assert_eq!(resolved.provider_id(), "bedrock");
        let creds = resolved.aws_credentials().expect("aws credentials");
        assert_eq!(creds.region, "eu-west-1");
        assert!(creds.session_token.is_some());
        assert_eq!(resolved.auth_source(), Some(AuthSource::Environment));
    }

    #[test]
    fn bedrock_provider_uses_default_region_when_unset() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("bedrock".into()),
            ..AgentSettings::default()
        };

        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [
                    ("AWS_ACCESS_KEY_ID", "AKID".to_string()),
                    ("AWS_SECRET_ACCESS_KEY", "secret".to_string()),
                ],
                &ProviderSelection::default(),
            )
            .expect("resolve bedrock with default region");

        let creds = resolved.aws_credentials().expect("aws credentials");
        assert_eq!(creds.region, "us-east-1");
    }

    #[test]
    fn bedrock_provider_missing_auth_error_mentions_env_vars() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("bedrock".into()),
            ..AgentSettings::default()
        };

        let error = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
                &ProviderSelection::default(),
            )
            .expect_err("should fail without aws creds");

        let message = error.to_string();
        assert!(message.contains("AWS_ACCESS_KEY_ID"));
        assert!(message.contains("AWS_SECRET_ACCESS_KEY"));
    }

    // ── Stage-1: protocol metadata ─────────────────────────────────────────

    #[test]
    fn builtin_copilot_has_copilot_protocol() {
        let registry = ProviderRegistry::builtin();
        let provider = registry.get("copilot").expect("copilot provider");
        assert_eq!(provider.wire_protocol, WireProtocol::Copilot);
    }

    #[test]
    fn builtin_openai_has_openai_compat_protocol() {
        let registry = ProviderRegistry::builtin();
        let provider = registry.get("openai").expect("openai provider");
        assert_eq!(provider.wire_protocol, WireProtocol::OpenAiCompat);
    }

    #[test]
    fn builtin_anthropic_has_anthropic_compat_protocol() {
        let registry = ProviderRegistry::builtin();
        let provider = registry.get("anthropic").expect("anthropic provider");
        assert_eq!(provider.wire_protocol, WireProtocol::AnthropicCompat);
    }

    #[test]
    fn builtin_bedrock_has_bedrock_anthropic_protocol() {
        let registry = ProviderRegistry::builtin();
        let provider = registry.get("bedrock").expect("bedrock provider");
        assert_eq!(provider.wire_protocol, WireProtocol::BedrockAnthropic);
    }

    #[test]
    fn native_providers_have_strict_model_validation_enabled() {
        // Gateway providers are intentionally non-strict; only the four native
        // providers (copilot/openai/anthropic/bedrock) enforce the catalogue.
        let registry = ProviderRegistry::builtin();
        for id in ["copilot", "openai", "anthropic", "bedrock"] {
            let provider = registry.get(id).unwrap_or_else(|| panic!("{id} provider"));
            assert!(
                provider.strict_model_validation,
                "native provider `{id}` should have strict_model_validation=true"
            );
        }
    }

    #[test]
    fn gateway_providers_have_non_strict_model_validation() {
        let registry = ProviderRegistry::builtin();
        for id in [
            "hyper",
            "vercel",
            "zai",
            "minimax",
            "huggingface",
            "cerebras",
            "openrouter",
            "ionet",
            "groq",
            "avian",
            "opencode",
        ] {
            let provider = registry
                .get(id)
                .unwrap_or_else(|| panic!("gateway provider `{id}` not registered"));
            assert!(
                !provider.strict_model_validation,
                "gateway provider `{id}` must have strict_model_validation=false"
            );
        }
    }

    #[test]
    fn all_gateway_providers_registered_with_openai_compat_protocol() {
        let registry = ProviderRegistry::builtin();
        for id in [
            "hyper",
            "vercel",
            "zai",
            "minimax",
            "huggingface",
            "cerebras",
            "openrouter",
            "ionet",
            "groq",
            "avian",
            "opencode",
        ] {
            let provider = registry
                .get(id)
                .unwrap_or_else(|| panic!("gateway provider `{id}` not registered"));
            assert_eq!(
                provider.wire_protocol,
                WireProtocol::OpenAiCompat,
                "gateway provider `{id}` must use WireProtocol::OpenAiCompat"
            );
            assert_eq!(
                provider.auth_kind,
                AuthMaterialKind::ApiKey,
                "gateway provider `{id}` must use AuthMaterialKind::ApiKey"
            );
            assert!(
                provider.api_key_env.is_some(),
                "gateway provider `{id}` must declare an api_key_env"
            );
            assert!(
                provider.api_base.is_some(),
                "gateway provider `{id}` must declare an api_base"
            );
            assert!(
                !provider.default_model.trim().is_empty(),
                "gateway provider `{id}` must have a non-empty default_model"
            );
        }
    }

    #[test]
    fn groq_provider_has_expected_env_var_and_resolves_from_env() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("groq".into()),
            ..AgentSettings::default()
        };

        let report = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                [("GROQ_API_KEY", "test-groq-key".to_string())],
            )
            .expect("resolve groq provider");

        assert_eq!(report.provider.as_deref(), Some("groq"));
        assert_eq!(report.auth.status, wonder_of_u_core::AuthStatus::Ready);
        assert_eq!(report.auth.source_label(), Some("environment"));
        assert_eq!(report.readiness, ProviderReadiness::Ready);
    }

    #[test]
    fn openrouter_provider_has_expected_env_var_and_resolves_from_env() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("openrouter".into()),
            ..AgentSettings::default()
        };

        let report = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                [("OPENROUTER_API_KEY", "test-or-key".to_string())],
            )
            .expect("resolve openrouter provider");

        assert_eq!(report.provider.as_deref(), Some("openrouter"));
        assert_eq!(report.auth.status, wonder_of_u_core::AuthStatus::Ready);
        assert_eq!(report.auth.source_label(), Some("environment"));
        assert_eq!(report.readiness, ProviderReadiness::Ready);
    }

    #[test]
    fn huggingface_provider_has_expected_env_var_and_resolves_from_env() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("huggingface".into()),
            ..AgentSettings::default()
        };

        let report = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                [("HF_TOKEN", "test-hf-key".to_string())],
            )
            .expect("resolve huggingface provider");

        assert_eq!(report.provider.as_deref(), Some("huggingface"));
        assert_eq!(report.auth.status, wonder_of_u_core::AuthStatus::Ready);
        assert_eq!(report.auth.source_label(), Some("environment"));
        assert_eq!(report.readiness, ProviderReadiness::Ready);
    }

    #[test]
    fn gateway_provider_accepts_arbitrary_model_id() {
        // Non-strict providers must accept any non-empty model string; use groq as representative.
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("groq".into()),
            selected_model: Some("mixtral-8x7b-32768".into()),
            ..AgentSettings::default()
        };
        let credentials = StoredCredentials {
            providers: BTreeMap::from([(
                "groq".into(),
                AuthMaterial::ApiKey {
                    key: "groq-secret".into(),
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
            .expect("groq must accept arbitrary model id");

        assert_eq!(resolved.model(), "mixtral-8x7b-32768");
        assert_eq!(resolved.provider_id(), "groq");
    }

    #[test]
    fn gateway_providers_env_vars_do_not_collide_with_each_other() {
        // Each gateway must declare a unique api_key_env so auto-selection logic
        // (which activates when exactly one provider has a ready key) stays reliable.
        let registry = ProviderRegistry::builtin();
        let mut seen_env_vars: BTreeMap<String, &str> = BTreeMap::new();
        for provider in registry.providers() {
            if let Some(env_var) = &provider.api_key_env {
                if let Some(existing_id) = seen_env_vars.get(env_var.as_str()) {
                    panic!(
                        "env var `{env_var}` is shared by providers `{existing_id}` and `{}`",
                        provider.id
                    );
                }
                seen_env_vars.insert(env_var.clone(), &provider.id);
            }
        }
    }

    #[test]
    fn gateway_provider_missing_key_reports_missing_auth() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("cerebras".into()),
            ..AgentSettings::default()
        };

        let report = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
            )
            .expect("resolve cerebras provider");

        assert_eq!(report.auth.status, wonder_of_u_core::AuthStatus::Missing);
        assert_eq!(report.readiness, ProviderReadiness::MissingAuth);
    }

    // ── Stage-1: strict vs. lenient model validation ───────────────────────

    #[test]
    fn strict_provider_rejects_unknown_model() {
        // The existing builtin providers are all strict; use openai as a proxy.
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("openai".into()),
            selected_model: Some("gpt-99-turbo-fantasy".into()),
            ..AgentSettings::default()
        };

        let error = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
            )
            .expect_err("unknown model must be rejected by strict provider");

        assert!(
            error
                .to_string()
                .contains("unknown model `gpt-99-turbo-fantasy`"),
            "error message should identify the bad model; got: {error}"
        );
    }

    #[test]
    fn non_strict_provider_accepts_arbitrary_non_empty_model() {
        // Build a custom provider with strict_model_validation=false.
        let mut registry = ProviderRegistry::builtin();
        registry
            .register(ProviderDescriptor {
                id: "custom".into(),
                display_name: "Custom".into(),
                auth_kind: AuthMaterialKind::None,
                default_model: "any-model".into(),
                models: vec![],
                api_base: Some("https://custom.example.com/v1".into()),
                api_key_env: None,
                wire_protocol: WireProtocol::OpenAiCompat,
                strict_model_validation: false,
            })
            .expect("register custom provider");

        let resolver = ProviderResolver { registry };
        let settings = AgentSettings {
            selected_provider: Some("custom".into()),
            selected_model: Some("whatever-the-operator-wants".into()),
            ..AgentSettings::default()
        };

        let report = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
            )
            .expect("non-strict provider should accept arbitrary model id");

        assert_eq!(report.model.as_deref(), Some("whatever-the-operator-wants"));
    }

    #[test]
    fn non_strict_provider_rejects_empty_model_id() {
        let mut registry = ProviderRegistry::builtin();
        registry
            .register(ProviderDescriptor {
                id: "lenient".into(),
                display_name: "Lenient".into(),
                auth_kind: AuthMaterialKind::None,
                default_model: "   ".into(), // blank default — should trigger the guard
                models: vec![],
                api_base: Some("https://lenient.example.com/v1".into()),
                api_key_env: None,
                wire_protocol: WireProtocol::OpenAiCompat,
                strict_model_validation: false,
            })
            .expect("register lenient provider");

        let resolver = ProviderResolver { registry };
        let settings = AgentSettings {
            selected_provider: Some("lenient".into()),
            ..AgentSettings::default()
        };

        let error = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
            )
            .expect_err("blank model id must be rejected even in lenient mode");

        assert!(
            error.to_string().contains("must not be empty"),
            "error should mention empty model id; got: {error}"
        );
    }

    #[test]
    fn provider_descriptor_serde_back_compat_missing_new_fields() {
        // A JSON blob that predates wire_protocol / strict_model_validation
        // must still deserialise cleanly, picking up the serde defaults.
        let legacy_json = serde_json::json!({
            "id": "legacy",
            "display_name": "Legacy",
            "auth_kind": "api_key",
            "default_model": "model-x",
            "models": [],
            "api_base": "https://legacy.example.com"
        });

        let descriptor: ProviderDescriptor =
            serde_json::from_value(legacy_json).expect("deserialise legacy descriptor");

        assert_eq!(descriptor.wire_protocol, WireProtocol::OpenAiCompat);
        assert!(!descriptor.strict_model_validation);
    }

    // ── provider-auth-env: AwsBearer priority for Bedrock ─────────────────

    #[test]
    fn bedrock_bearer_token_takes_priority_over_sigv4_env_vars() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("bedrock".into()),
            ..AgentSettings::default()
        };

        // Both bearer token AND static keys are present; bearer wins.
        let report = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                [
                    ("AWS_BEARER_TOKEN_BEDROCK", "my-bearer-token".to_string()),
                    ("AWS_ACCESS_KEY_ID", "AKIAIOSFODNN7EXAMPLE".to_string()),
                    (
                        "AWS_SECRET_ACCESS_KEY",
                        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
                    ),
                    ("AWS_REGION", "us-west-2".to_string()),
                ],
            )
            .expect("resolve bedrock with bearer token");

        assert_eq!(report.auth.status, wonder_of_u_core::AuthStatus::Ready);
        assert_eq!(
            report.auth.kind,
            wonder_of_u_core::AuthMaterialKind::AwsBearer,
            "bearer token must take priority over SigV4 env vars"
        );
        assert_eq!(report.auth.source_label(), Some("environment"));
        assert_eq!(report.readiness, ProviderReadiness::Ready);
    }

    #[test]
    fn bedrock_bearer_token_execution_returns_token_accessor() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("bedrock".into()),
            ..AgentSettings::default()
        };

        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [
                    ("AWS_BEARER_TOKEN_BEDROCK", "test-bearer-xyz".to_string()),
                    ("AWS_REGION", "ap-northeast-1".to_string()),
                ],
                &ProviderSelection::default(),
            )
            .expect("resolve bedrock execution with bearer");

        let (token, region) = resolved.aws_bearer_token().expect("bearer token accessor");
        assert_eq!(token, "test-bearer-xyz");
        assert_eq!(region, "ap-northeast-1");
        // aws_credentials() should fail — bearer and SigV4/Profile are mutually exclusive.
        assert!(resolved.aws_credentials().is_err());
    }

    #[test]
    fn bedrock_bearer_token_blank_falls_through_to_sigv4() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("bedrock".into()),
            ..AgentSettings::default()
        };

        // A blank bearer token must NOT shadow the SigV4 path.
        let report = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                [
                    ("AWS_BEARER_TOKEN_BEDROCK", "   ".to_string()),
                    ("AWS_ACCESS_KEY_ID", "AKID".to_string()),
                    ("AWS_SECRET_ACCESS_KEY", "SECRET".to_string()),
                ],
            )
            .expect("resolve bedrock with blank bearer + sigv4");

        assert_eq!(
            report.auth.kind,
            wonder_of_u_core::AuthMaterialKind::AwsSigV4,
            "blank bearer must fall through to SigV4"
        );
        assert_eq!(report.auth.status, wonder_of_u_core::AuthStatus::Ready);
    }

    #[test]
    fn bedrock_sigv4_takes_priority_over_profile() {
        // When both SigV4 env vars AND a credentials file are present, SigV4 wins.
        let dir = std::env::temp_dir().join("wou_provider_test_sigv4_over_profile");
        std::fs::create_dir_all(&dir).ok();
        let creds_path = dir.join("credentials");
        std::fs::write(
            &creds_path,
            "[default]\naws_access_key_id = PROFILEKEY\naws_secret_access_key = PROFILESECRET\n",
        )
        .expect("write test creds");

        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("bedrock".into()),
            ..AgentSettings::default()
        };

        let report = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                [
                    ("AWS_ACCESS_KEY_ID", "ENVKEY".to_string()),
                    ("AWS_SECRET_ACCESS_KEY", "ENVSECRET".to_string()),
                    (
                        "AWS_SHARED_CREDENTIALS_FILE",
                        creds_path.to_str().unwrap().to_string(),
                    ),
                ],
            )
            .expect("resolve bedrock sigv4 over profile");

        assert_eq!(
            report.auth.kind,
            wonder_of_u_core::AuthMaterialKind::AwsSigV4,
            "SigV4 env vars must take priority over profile file"
        );
        std::fs::remove_file(&creds_path).ok();
    }

    #[test]
    fn bedrock_resolves_from_aws_profile_when_no_env_vars() {
        // Write a minimal credentials file and inject its path via the env map.
        let dir = std::env::temp_dir().join("wou_provider_test_profile_fallback");
        std::fs::create_dir_all(&dir).ok();
        let creds_path = dir.join("credentials");
        std::fs::write(
            &creds_path,
            "[default]\naws_access_key_id = PROFILEKEY\naws_secret_access_key = PROFILESECRET\nregion = eu-central-1\n",
        )
        .expect("write test creds");

        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("bedrock".into()),
            ..AgentSettings::default()
        };

        let report = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                [(
                    "AWS_SHARED_CREDENTIALS_FILE",
                    creds_path.to_str().unwrap().to_string(),
                )],
            )
            .expect("resolve bedrock via profile");

        assert_eq!(
            report.auth.kind,
            wonder_of_u_core::AuthMaterialKind::AwsProfile,
            "should resolve via AWS_PROFILE credentials file"
        );
        assert_eq!(report.auth.status, wonder_of_u_core::AuthStatus::Ready);
        assert_eq!(report.auth.source_label(), Some("credentials_file"));
        assert_eq!(report.readiness, ProviderReadiness::Ready);

        std::fs::remove_file(&creds_path).ok();
    }

    #[test]
    fn bedrock_missing_auth_error_mentions_all_three_options() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("bedrock".into()),
            ..AgentSettings::default()
        };

        let error = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
                &ProviderSelection::default(),
            )
            .expect_err("should fail without aws creds");

        let message = error.to_string();
        assert!(
            message.contains("AWS_BEARER_TOKEN_BEDROCK"),
            "error should mention bearer token option; got: {message}"
        );
        assert!(
            message.contains("AWS_ACCESS_KEY_ID"),
            "error should mention SigV4 option; got: {message}"
        );
        assert!(
            message.contains("AWS_PROFILE"),
            "error should mention profile option; got: {message}"
        );
    }
}
