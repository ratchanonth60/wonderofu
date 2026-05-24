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
    /// When set, the API base URL is read from this environment variable at
    /// resolution time.  Takes precedence over [`Self::api_base`] if both
    /// are provided.  Enables providers like Azure OpenAI whose endpoint URL
    /// is user/deployment-specific (e.g. `"AZURE_OPENAI_API_ENDPOINT"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_env: Option<String>,
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
/// Default local model API base URL (Ollama default endpoint).
///
/// Used as the last-resort fallback for the `local` provider when no
/// environment override is present.  Ollama listens on `11434` by default
/// and exposes an OpenAI-compatible `/v1` prefix.
pub const DEFAULT_LOCAL_API_BASE: &str = "http://localhost:11434/v1";

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
/// let desc = wonder_of_u_agent::openai_compat_gateway(
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
        endpoint_env: None,
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
    ///
    /// `access_token` is populated when `GOOGLE_BEARER_TOKEN` is set in the
    /// environment, allowing tests and CI to inject a pre-obtained token
    /// without a full service-account key-file exchange.
    GcpOAuth2 {
        project: String,
        location: String,
        /// Path to service-account key file (`GOOGLE_APPLICATION_CREDENTIALS`).
        /// May be empty when `access_token` is present.
        credentials_source: String,
        /// Pre-obtained OAuth2 bearer token (`GOOGLE_BEARER_TOKEN`), if set.
        access_token: Option<String>,
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

    /// Returns the API key if one is present, or `None` for auth-free
    /// providers (e.g. `local` with [`AuthMaterialKind::None`]).
    ///
    /// Callers that need to conditionally include an `Authorization` header
    /// should prefer this over [`Self::api_key`] so that local/Ollama
    /// deployments that do not require a key work out of the box.
    pub(crate) fn optional_api_key(&self) -> Option<&str> {
        self.auth.api_key()
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

    /// Returns the GCP project and location from `GcpOAuth2` auth material.
    ///
    /// Used by the Vertex AI runtime to construct the endpoint URL.
    pub(crate) fn gcp_project_location(&self) -> Result<(&str, &str)> {
        match &self.auth {
            ResolvedAuthMaterial::GcpOAuth2 {
                project, location, ..
            } => Ok((project.as_str(), location.as_str())),
            _ => Err(WonderError::validation(format!(
                "provider `{}` is not configured with GCP OAuth2 auth",
                self.provider.id
            ))),
        }
    }

    /// Returns the pre-obtained GCP OAuth2 access token, if present.
    ///
    /// Set by providing `GOOGLE_BEARER_TOKEN` in the environment; `None`
    /// when only a service-account key file was configured.
    pub(crate) fn gcp_access_token(&self) -> Option<&str> {
        match &self.auth {
            ResolvedAuthMaterial::GcpOAuth2 { access_token, .. } => access_token.as_deref(),
            _ => None,
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
                default_model: "gpt-5.5".into(),
                models: vec![
                    ModelDescriptor::new("gpt-5.5", "GPT-5.5"),
                    ModelDescriptor::new("gpt-5.4", "GPT-5.4"),
                    ModelDescriptor::new("gpt-5.4-mini", "GPT-5.4 mini"),
                    ModelDescriptor::new("gpt-5.4-nano", "GPT-5.4 nano"),
                    ModelDescriptor::new("gpt-5.3-codex", "GPT-5.3-Codex"),
                    ModelDescriptor::new("gpt-5.2", "GPT-5.2"),
                    ModelDescriptor::new("gpt-5.2-codex", "GPT-5.2-Codex"),
                    ModelDescriptor::new("gpt-5-mini", "GPT-5 mini"),
                    ModelDescriptor::new("gpt-4.1", "GPT-4.1"),
                    ModelDescriptor::new("claude-haiku-4.5", "Claude Haiku 4.5"),
                    ModelDescriptor::new("claude-opus-4.5", "Claude Opus 4.5"),
                    ModelDescriptor::new("claude-opus-4.6", "Claude Opus 4.6"),
                    ModelDescriptor::new("claude-opus-4.6-fast", "Claude Opus 4.6 (fast mode)"),
                    ModelDescriptor::new("claude-opus-4.7", "Claude Opus 4.7"),
                    ModelDescriptor::new("claude-sonnet-4.5", "Claude Sonnet 4.5"),
                    ModelDescriptor::new("claude-sonnet-4.6", "Claude Sonnet 4.6"),
                    ModelDescriptor::new("gemini-2.5-pro", "Gemini 2.5 Pro"),
                    ModelDescriptor::new("gemini-3-flash", "Gemini 3 Flash"),
                    ModelDescriptor::new("gemini-3.1-pro", "Gemini 3.1 Pro"),
                    ModelDescriptor::new("gemini-3.5-flash", "Gemini 3.5 Flash"),
                    ModelDescriptor::new("raptor-mini", "Raptor mini"),
                    ModelDescriptor::new("goldeneye", "Goldeneye"),
                ],
                api_base: Some(DEFAULT_COPILOT_API_BASE.into()),
                api_key_env: None,
                wire_protocol: WireProtocol::Copilot,
                strict_model_validation: true,
                endpoint_env: None,
            })
            .expect("builtin provider");
        registry
            .register(ProviderDescriptor {
                id: "openai".into(),
                display_name: "OpenAI".into(),
                auth_kind: AuthMaterialKind::ApiKey,
                default_model: "gpt-5.5".into(),
                models: vec![
                    ModelDescriptor::new("gpt-5.5", "GPT-5.5"),
                    ModelDescriptor::new("gpt-5.4", "GPT-5.4"),
                    ModelDescriptor::new("gpt-5.4-mini", "GPT-5.4 mini"),
                    ModelDescriptor::new("gpt-5.4-nano", "GPT-5.4 nano"),
                    ModelDescriptor::new("gpt-5-mini", "GPT-5 mini"),
                    ModelDescriptor::new("gpt-5-nano", "GPT-5 nano"),
                    ModelDescriptor::new("gpt-5", "GPT-5"),
                    ModelDescriptor::new("gpt-4.1", "GPT-4.1"),
                ],
                api_base: Some("https://api.openai.com/v1".into()),
                api_key_env: Some("OPENAI_API_KEY".into()),
                wire_protocol: WireProtocol::OpenAiCompat,
                strict_model_validation: true,
                endpoint_env: None,
            })
            .expect("builtin provider");
        registry
            .register(ProviderDescriptor {
                id: "anthropic".into(),
                display_name: "Anthropic".into(),
                auth_kind: AuthMaterialKind::ApiKey,
                default_model: "claude-opus-4-7".into(),
                models: vec![
                    ModelDescriptor::new("claude-opus-4-7", "Claude Opus 4.7"),
                    ModelDescriptor::new("claude-sonnet-4-6", "Claude Sonnet 4.6"),
                    ModelDescriptor::new("claude-haiku-4-5-20251001", "Claude Haiku 4.5"),
                    ModelDescriptor::new("claude-haiku-4-5", "Claude Haiku 4.5 Alias"),
                ],
                api_base: Some("https://api.anthropic.com".into()),
                api_key_env: Some("ANTHROPIC_API_KEY".into()),
                wire_protocol: WireProtocol::AnthropicCompat,
                strict_model_validation: true,
                endpoint_env: None,
            })
            .expect("builtin provider");
        registry
            .register(ProviderDescriptor {
                id: "bedrock".into(),
                display_name: "Amazon Bedrock".into(),
                auth_kind: AuthMaterialKind::AwsSigV4,
                default_model: "anthropic.claude-opus-4-7".into(),
                models: vec![
                    ModelDescriptor::new("anthropic.claude-opus-4-7", "Claude Opus 4.7 (Bedrock)"),
                    ModelDescriptor::new(
                        "anthropic.claude-sonnet-4-6",
                        "Claude Sonnet 4.6 (Bedrock)",
                    ),
                    ModelDescriptor::new(
                        "anthropic.claude-haiku-4-5-20251001-v1:0",
                        "Claude Haiku 4.5 (Bedrock)",
                    ),
                ],
                api_base: Some(DEFAULT_BEDROCK_API_BASE.into()),
                api_key_env: None,
                wire_protocol: WireProtocol::BedrockAnthropic,
                strict_model_validation: true,
                endpoint_env: None,
            })
            .expect("builtin provider");
        registry
            .register(ProviderDescriptor {
                id: "gemini".into(),
                display_name: "Google Gemini".into(),
                auth_kind: AuthMaterialKind::ApiKey,
                default_model: "gemini-2.0-flash".into(),
                models: vec![],
                api_base: Some("https://generativelanguage.googleapis.com".into()),
                api_key_env: Some("GEMINI_API_KEY".into()),
                wire_protocol: WireProtocol::GeminiNative,
                strict_model_validation: false,
                endpoint_env: None,
            })
            .expect("builtin provider");
        registry
            .register(ProviderDescriptor {
                id: "vertex".into(),
                display_name: "Google Vertex AI".into(),
                auth_kind: AuthMaterialKind::GcpOAuth2,
                default_model: "gemini-2.0-flash".into(),
                models: vec![],
                // Vertex endpoints are location-specific; the runtime computes
                // the real URL from project + location in the resolved auth.
                // This placeholder satisfies the api_base requirement.
                api_base: Some("https://aiplatform.googleapis.com".into()),
                api_key_env: None,
                wire_protocol: WireProtocol::VertexGemini,
                strict_model_validation: false,
                endpoint_env: None,
            })
            .expect("builtin provider");
        registry
            .register(ProviderDescriptor {
                id: "azure".into(),
                display_name: "Azure OpenAI".into(),
                auth_kind: AuthMaterialKind::ApiKey,
                default_model: "gpt-4o".into(),
                models: vec![],
                // api_base is resolved from AZURE_OPENAI_API_ENDPOINT at runtime.
                api_base: None,
                api_key_env: Some("AZURE_OPENAI_API_KEY".into()),
                wire_protocol: WireProtocol::AzureOpenAi,
                strict_model_validation: false,
                endpoint_env: Some("AZURE_OPENAI_API_ENDPOINT".into()),
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
            // Synthetic AI – OpenAI-compatible gateway; base URL is approximate.
            // Authenticates with SYNTHETIC_API_KEY.
            (
                "synthetic",
                "Synthetic AI",
                "https://api.synthetic.ai/v1",
                "SYNTHETIC_API_KEY",
                "synthetic-1",
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
            .register(ProviderDescriptor {
                id: "local".into(),
                display_name: "Local Models".into(),
                auth_kind: AuthMaterialKind::None,
                default_model: "llama3.2".into(),
                // No enumerated catalogue — any non-empty model id is accepted.
                models: vec![],
                api_base: Some(DEFAULT_LOCAL_API_BASE.into()),
                api_key_env: None,
                wire_protocol: WireProtocol::OpenAiCompat,
                strict_model_validation: false,
                endpoint_env: None,
            })
            .expect("builtin provider");
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
    /// Stores every provider registered in the resolver registry.
    pub available_providers: Vec<ProviderDescriptor>,
    /// Stores providers with explicit settings, credentials, or env configuration.
    pub configured_providers: Vec<ProviderDescriptor>,
    /// Stores providers whose authentication material is ready.
    pub authenticated_providers: Vec<ProviderDescriptor>,
    /// Stores providers that can execute immediately.
    pub ready_providers: Vec<ProviderDescriptor>,
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
        let inventory = self.collect_provider_inventory(settings, credentials, &env)?;

        let Some(provider_id) = provider.clone() else {
            return Ok(ProviderStatusReport {
                provider: None,
                model: None,
                auth: AuthState::default(),
                readiness: ProviderReadiness::Unconfigured,
                available_providers: inventory.registered_providers,
                configured_providers: inventory.configured_providers,
                authenticated_providers: inventory.authenticated_providers,
                ready_providers: inventory.ready_providers,
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
            available_providers: inventory.registered_providers,
            configured_providers: inventory.configured_providers,
            authenticated_providers: inventory.authenticated_providers,
            ready_providers: inventory.ready_providers,
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
        let inventory = self.collect_provider_inventory(settings, credentials, &env)?;

        let Some(provider_id) = provider.clone() else {
            return Ok(ProviderStatusReport {
                provider: None,
                model: None,
                auth: AuthState::default(),
                readiness: ProviderReadiness::Unconfigured,
                available_providers: inventory.registered_providers,
                configured_providers: inventory.configured_providers,
                authenticated_providers: inventory.authenticated_providers,
                ready_providers: inventory.ready_providers,
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
            available_providers: inventory.registered_providers,
            configured_providers: inventory.configured_providers,
            authenticated_providers: inventory.authenticated_providers,
            ready_providers: inventory.ready_providers,
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
        let api_base = self.resolve_api_base(descriptor, settings, &env)?;
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

        // Separate auth-requiring providers from auth-free ones (e.g. local).
        // Auth-free providers are only auto-selected as a last resort so that
        // a configured API key always takes precedence over the local fallback.
        let (auth_free, auth_required): (Vec<_>, Vec<_>) = self
            .registry
            .providers()
            .partition(|p| p.auth_kind == AuthMaterialKind::None);

        let ready_auth = auth_required
            .iter()
            .filter_map(|provider| {
                self.resolve_auth(provider, credentials, env)
                    .ok()
                    .filter(|auth| auth.is_ready())
                    .map(|_| provider.id.as_str())
            })
            .collect::<Vec<_>>();

        if ready_auth.len() == 1 {
            return Ok(ready_auth.first().map(|id| (*id).to_string()));
        }

        // No auth-requiring provider is uniquely ready; fall back to any
        // auth-free provider that is available (should be exactly one: local).
        if ready_auth.is_empty() {
            let ready_free = auth_free
                .iter()
                .filter_map(|provider| {
                    self.resolve_auth(provider, credentials, env)
                        .ok()
                        .filter(|auth| auth.is_ready())
                        .map(|_| provider.id.as_str())
                })
                .collect::<Vec<_>>();

            if ready_free.len() == 1 {
                return Ok(ready_free.first().map(|id| (*id).to_string()));
            }
        }

        Ok(None)
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
        env: &BTreeMap<String, String>,
    ) -> Result<String> {
        // Priority 1: per-provider settings override always wins regardless of
        // provider kind.
        if let Some(base) = settings
            .providers
            .get(&provider.id)
            .and_then(|config| config.api_base.clone())
            .filter(|api_base| !api_base.trim().is_empty())
        {
            return Ok(base);
        }

        // Priority 2–4: environment-variable chain for the `local` provider.
        // Resolution order matches the documented priority in the scope:
        //   WONDER_OF_U_LOCAL_API_BASE → OLLAMA_HOST → LMSTUDIO_API_BASE → fallback
        if provider.id == "local" {
            if let Some(base) = env
                .get("WONDER_OF_U_LOCAL_API_BASE")
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
            {
                return Ok(base.to_string());
            }

            if let Some(host) = env
                .get("OLLAMA_HOST")
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
            {
                return Ok(ollama_host_to_openai_base(host));
            }

            if let Some(base) = env
                .get("LMSTUDIO_API_BASE")
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
            {
                return Ok(base.to_string());
            }
        }

        // Priority 5: the descriptor's own api_base (hardcoded fallback).
        if let Some(base) = provider
            .api_base
            .clone()
            .filter(|api_base| !api_base.trim().is_empty())
        {
            return Ok(base);
        }

        // Priority 6: providers with deployment-specific endpoints can name the
        // environment variable that supplies their API base (for example Azure).
        if let Some(base) = provider.endpoint_env.as_deref().and_then(|var| {
            env.get(var)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        }) {
            return Ok(base);
        }

        let hint = provider
            .endpoint_env
            .as_deref()
            .map(|var| {
                format!(
                    "; set the `{var}` environment variable or configure `api_base` in settings"
                )
            })
            .unwrap_or_default();
        Err(WonderError::validation(format!(
            "provider `{}` does not declare an API base URL{hint}",
            provider.id,
        )))
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
                // All three GCP vars must be present for readiness,
                // OR: VERTEXAI_PROJECT + VERTEXAI_LOCATION + GOOGLE_BEARER_TOKEN
                // (pre-obtained token, no key file required).
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

                // Pre-obtained bearer token takes priority over key-file path.
                let access_token = env
                    .get("GOOGLE_BEARER_TOKEN")
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty())
                    .map(ToString::to_string);
                let credentials_source = env
                    .get("GOOGLE_APPLICATION_CREDENTIALS")
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty())
                    .map(ToString::to_string);

                match (project, location) {
                    (Some(project), Some(location))
                        if access_token.is_some() || credentials_source.is_some() =>
                    {
                        Ok(Some(ResolvedAuthMaterial::GcpOAuth2 {
                            project,
                            location,
                            credentials_source: credentials_source.unwrap_or_default(),
                            access_token,
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

    fn collect_provider_inventory(
        &self,
        settings: &AgentSettings,
        credentials: &StoredCredentials,
        env: &BTreeMap<String, String>,
    ) -> Result<ProviderInventory> {
        let mut inventory = ProviderInventory::default();

        for provider in self.registry.providers() {
            let provider = provider.clone();
            let auth = self.resolve_auth(&provider, credentials, env)?;
            let model_ready = self
                .resolve_model(&provider, settings, Some(provider.id.as_str()), None)
                .is_ok();
            let api_base_ready = self.resolve_api_base(&provider, settings, env).is_ok();

            if self.provider_has_explicit_config(&provider, settings, credentials, env) {
                inventory.configured_providers.push(provider.clone());
            }
            if provider.auth_kind != AuthMaterialKind::None && auth.is_ready() {
                inventory.authenticated_providers.push(provider.clone());
            }
            if auth.is_ready() && model_ready && api_base_ready {
                inventory.ready_providers.push(provider.clone());
            }
            inventory.registered_providers.push(provider);
        }

        Ok(inventory)
    }

    fn provider_has_explicit_config(
        &self,
        provider: &ProviderDescriptor,
        settings: &AgentSettings,
        credentials: &StoredCredentials,
        env: &BTreeMap<String, String>,
    ) -> bool {
        if settings.selected_provider.as_deref() == Some(provider.id.as_str()) {
            return true;
        }

        if settings.providers.get(&provider.id).is_some_and(|config| {
            config
                .model
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
                || config
                    .api_base
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
        }) {
            return true;
        }

        if credentials.providers.contains_key(&provider.id) {
            return true;
        }

        if provider.id == "local"
            && [
                "WONDER_OF_U_LOCAL_API_BASE",
                "OLLAMA_HOST",
                "LMSTUDIO_API_BASE",
            ]
            .into_iter()
            .any(|var| env_var_is_set(env, var))
        {
            return true;
        }

        if provider
            .api_key_env
            .as_deref()
            .is_some_and(|var| env_var_is_set(env, var))
        {
            return true;
        }

        if provider
            .endpoint_env
            .as_deref()
            .is_some_and(|var| env_var_is_set(env, var))
        {
            return true;
        }

        match provider.auth_kind {
            AuthMaterialKind::None | AuthMaterialKind::ApiKey | AuthMaterialKind::OAuth => false,
            AuthMaterialKind::AwsSigV4 => {
                env_var_is_set(env, "AWS_BEARER_TOKEN_BEDROCK")
                    || (env_var_is_set(env, "AWS_ACCESS_KEY_ID")
                        && env_var_is_set(env, "AWS_SECRET_ACCESS_KEY"))
                    || env_var_is_set(env, "AWS_PROFILE")
            }
            AuthMaterialKind::AwsBearer => env_var_is_set(env, "AWS_BEARER_TOKEN_BEDROCK"),
            AuthMaterialKind::AwsProfile => env_var_is_set(env, "AWS_PROFILE"),
            AuthMaterialKind::GcpOAuth2 => [
                "VERTEXAI_PROJECT",
                "VERTEXAI_LOCATION",
                "GOOGLE_BEARER_TOKEN",
                "GOOGLE_APPLICATION_CREDENTIALS",
            ]
            .into_iter()
            .any(|var| env_var_is_set(env, var)),
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

#[derive(Default)]
struct ProviderInventory {
    registered_providers: Vec<ProviderDescriptor>,
    configured_providers: Vec<ProviderDescriptor>,
    authenticated_providers: Vec<ProviderDescriptor>,
    ready_providers: Vec<ProviderDescriptor>,
}

fn is_fast_model_id(model_id: &str) -> bool {
    let normalized = model_id.to_ascii_lowercase();
    normalized.contains("mini") || normalized.contains("haiku")
}

fn oauth_access_token_expired(expires_at: Option<OffsetDateTime>) -> bool {
    expires_at.is_some_and(|value| value <= OffsetDateTime::now_utc())
}

fn env_var_is_set(env: &BTreeMap<String, String>, key: &str) -> bool {
    env.get(key)
        .map(String::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

/// Converts an `OLLAMA_HOST` value to an OpenAI-compatible base URL by
/// appending `/v1` when the host does not already end with that path segment.
///
/// Handles trailing slashes and existing `/v1` suffixes gracefully.
///
/// # Examples
///
/// ```text
/// "http://localhost:11434"       → "http://localhost:11434/v1"
/// "http://localhost:11434/"      → "http://localhost:11434/v1"
/// "http://localhost:11434/v1"    → "http://localhost:11434/v1"
/// "http://localhost:11434/v1/"   → "http://localhost:11434/v1"
/// ```
fn ollama_host_to_openai_base(host: &str) -> String {
    let trimmed = host.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
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

    fn provider_ids(providers: &[ProviderDescriptor]) -> Vec<&str> {
        providers
            .iter()
            .map(|provider| provider.id.as_str())
            .collect()
    }

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
        assert_eq!(report.model.as_deref(), Some("gpt-5.5"));
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
        assert_eq!(resolved.model(), "gpt-5.5");
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
        assert_eq!(resolved.model(), "claude-opus-4-7");
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

        assert_eq!(resolved.model(), "gpt-5.4-mini");
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
            registry
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
        assert_eq!(bedrock.default_model, "anthropic.claude-opus-4-7");
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
        // Providers with a curated model catalogue enforce strict validation.
        // Gateway/local/native-dynamic providers are intentionally non-strict.
        let registry = ProviderRegistry::builtin();
        for id in ["copilot", "openai", "anthropic", "bedrock"] {
            let provider = registry.get(id).unwrap_or_else(|| panic!("{id} provider"));
            assert!(
                provider.strict_model_validation,
                "native provider `{id}` should have strict_model_validation=true"
            );
        }
        let open_providers = ["gemini", "vertex", "azure", "local"];
        for id in open_providers {
            let provider = registry
                .get(id)
                .unwrap_or_else(|| panic!("missing builtin: {id}"));
            assert!(
                !provider.strict_model_validation,
                "open-catalog provider `{}` should have strict_model_validation=false",
                provider.id
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
                endpoint_env: None,
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
                endpoint_env: None,
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

    // ── provider-local-models ──────────────────────────────────────────────

    #[test]
    fn local_provider_registered_in_builtin_registry() {
        let registry = ProviderRegistry::builtin();
        let provider = registry.get("local").expect("local provider must exist");
        assert_eq!(provider.display_name, "Local Models");
        assert_eq!(provider.auth_kind, AuthMaterialKind::None);
        assert_eq!(provider.wire_protocol, WireProtocol::OpenAiCompat);
        assert!(!provider.strict_model_validation);
        assert_eq!(provider.default_model, "llama3.2");
        assert_eq!(provider.api_base.as_deref(), Some(DEFAULT_LOCAL_API_BASE),);
    }

    #[test]
    fn local_provider_auth_state_is_not_required_with_no_credentials() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("local".into()),
            ..AgentSettings::default()
        };

        let report = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
            )
            .expect("local provider should always resolve");

        assert_eq!(
            report.auth.status,
            wonder_of_u_core::AuthStatus::NotRequired,
            "local provider must not require auth"
        );
        assert_eq!(
            report.readiness,
            ProviderReadiness::Ready,
            "local provider must be ready without any credentials"
        );
    }

    #[test]
    fn local_provider_accepts_arbitrary_model_id() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("local".into()),
            selected_model: Some("mistral-nemo:12b-instruct-2407-q4_K_M".into()),
            ..AgentSettings::default()
        };

        let report = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
            )
            .expect("arbitrary model id must be accepted by local provider");

        assert_eq!(
            report.model.as_deref(),
            Some("mistral-nemo:12b-instruct-2407-q4_K_M")
        );
    }

    #[test]
    fn local_provider_uses_default_api_base_without_env() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("local".into()),
            ..AgentSettings::default()
        };

        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
                &ProviderSelection::default(),
            )
            .expect("local provider should resolve without env");

        assert_eq!(resolved.api_base(), DEFAULT_LOCAL_API_BASE);
    }

    #[test]
    fn local_provider_env_api_base_overrides_default() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("local".into()),
            ..AgentSettings::default()
        };

        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [(
                    "WONDER_OF_U_LOCAL_API_BASE",
                    "http://myhost:8080/v1".to_string(),
                )],
                &ProviderSelection::default(),
            )
            .expect("local provider should resolve with WONDER_OF_U_LOCAL_API_BASE");

        assert_eq!(resolved.api_base(), "http://myhost:8080/v1");
    }

    #[test]
    fn local_provider_ollama_host_converted_to_v1_base() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("local".into()),
            ..AgentSettings::default()
        };

        // Without trailing /v1 — must be appended.
        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [("OLLAMA_HOST", "http://gpu-box:11434".to_string())],
                &ProviderSelection::default(),
            )
            .expect("local provider should resolve via OLLAMA_HOST");

        assert_eq!(resolved.api_base(), "http://gpu-box:11434/v1");
    }

    #[test]
    fn local_provider_ollama_host_already_has_v1_suffix() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("local".into()),
            ..AgentSettings::default()
        };

        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [("OLLAMA_HOST", "http://gpu-box:11434/v1".to_string())],
                &ProviderSelection::default(),
            )
            .expect("local provider should not duplicate /v1");

        assert_eq!(resolved.api_base(), "http://gpu-box:11434/v1");
    }

    #[test]
    fn local_provider_wonder_env_takes_priority_over_ollama_host() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("local".into()),
            ..AgentSettings::default()
        };

        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [
                    (
                        "WONDER_OF_U_LOCAL_API_BASE",
                        "http://primary:9999/v1".to_string(),
                    ),
                    ("OLLAMA_HOST", "http://secondary:11434".to_string()),
                ],
                &ProviderSelection::default(),
            )
            .expect("WONDER_OF_U_LOCAL_API_BASE must beat OLLAMA_HOST");

        assert_eq!(resolved.api_base(), "http://primary:9999/v1");
    }

    #[test]
    fn local_provider_lmstudio_api_base_used_when_ollama_absent() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("local".into()),
            ..AgentSettings::default()
        };

        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [("LMSTUDIO_API_BASE", "http://localhost:1234/v1".to_string())],
                &ProviderSelection::default(),
            )
            .expect("local provider should resolve via LMSTUDIO_API_BASE");

        assert_eq!(resolved.api_base(), "http://localhost:1234/v1");
    }

    #[test]
    fn local_provider_settings_api_base_beats_all_env_vars() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("local".into()),
            providers: BTreeMap::from([(
                "local".into(),
                crate::ProviderOverride {
                    model: None,
                    api_base: Some("http://settings-override:4242/v1".into()),
                },
            )]),
            ..AgentSettings::default()
        };

        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [
                    (
                        "WONDER_OF_U_LOCAL_API_BASE",
                        "http://env:9999/v1".to_string(),
                    ),
                    ("OLLAMA_HOST", "http://ollama:11434".to_string()),
                ],
                &ProviderSelection::default(),
            )
            .expect("settings api_base must beat all env vars");

        assert_eq!(resolved.api_base(), "http://settings-override:4242/v1");
    }

    #[test]
    fn local_provider_auto_selected_when_no_other_auth_configured() {
        // With no API keys or OAuth tokens configured, the local provider
        // should be auto-selected as the sole ready provider.
        let resolver = ProviderResolver::builtin();

        let report = resolver
            .resolve_with_env(
                &AgentSettings::default(),
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
            )
            .expect("local provider auto-selection should succeed");

        assert_eq!(
            report.provider.as_deref(),
            Some("local"),
            "local should be auto-selected when nothing else is configured"
        );
        assert_eq!(report.readiness, ProviderReadiness::Ready);
    }

    #[test]
    fn status_report_separates_registered_configured_authenticated_and_ready_without_env() {
        let resolver = ProviderResolver::builtin();

        let report = resolver
            .resolve_with_env(
                &AgentSettings::default(),
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
            )
            .expect("status report without env");

        assert!(provider_ids(&report.available_providers).contains(&"openai"));
        assert_eq!(
            provider_ids(&report.configured_providers),
            Vec::<&str>::new()
        );
        assert_eq!(
            provider_ids(&report.authenticated_providers),
            Vec::<&str>::new()
        );
        assert_eq!(provider_ids(&report.ready_providers), vec!["local"]);
    }

    #[test]
    fn status_report_marks_api_key_provider_authenticated_and_ready_with_env() {
        let resolver = ProviderResolver::builtin();

        let report = resolver
            .resolve_with_env(
                &AgentSettings::default(),
                &StoredCredentials::default(),
                [("OPENAI_API_KEY", "sk-test-key".to_string())],
            )
            .expect("status report with openai env");

        assert!(provider_ids(&report.configured_providers).contains(&"openai"));
        assert_eq!(
            provider_ids(&report.authenticated_providers),
            vec!["openai"]
        );
        assert!(provider_ids(&report.ready_providers).contains(&"openai"));
        assert!(provider_ids(&report.ready_providers).contains(&"local"));
    }

    #[test]
    fn status_report_tracks_authenticated_but_not_ready_provider_when_endpoint_missing() {
        let resolver = ProviderResolver::builtin();

        let report = resolver
            .resolve_with_env(
                &AgentSettings::default(),
                &StoredCredentials::default(),
                [("AZURE_OPENAI_API_KEY", "azure-test-key".to_string())],
            )
            .expect("status report with partial azure env");

        assert!(provider_ids(&report.configured_providers).contains(&"azure"));
        assert!(provider_ids(&report.authenticated_providers).contains(&"azure"));
        assert!(!provider_ids(&report.ready_providers).contains(&"azure"));
    }

    #[test]
    fn local_provider_not_auto_selected_when_openai_api_key_present() {
        // When an auth-requiring provider is ready, it should take precedence
        // over the always-ready local fallback.
        let resolver = ProviderResolver::builtin();

        let report = resolver
            .resolve_with_env(
                &AgentSettings::default(),
                &StoredCredentials::default(),
                [("OPENAI_API_KEY", "sk-test-key".to_string())],
            )
            .expect("auto-selection with openai key should succeed");

        assert_eq!(
            report.provider.as_deref(),
            Some("openai"),
            "openai (with API key) must beat local in auto-selection"
        );
    }

    #[test]
    fn ollama_host_to_openai_base_conversion() {
        assert_eq!(
            ollama_host_to_openai_base("http://localhost:11434"),
            "http://localhost:11434/v1"
        );
        assert_eq!(
            ollama_host_to_openai_base("http://localhost:11434/"),
            "http://localhost:11434/v1"
        );
        assert_eq!(
            ollama_host_to_openai_base("http://localhost:11434/v1"),
            "http://localhost:11434/v1"
        );
        assert_eq!(
            ollama_host_to_openai_base("http://localhost:11434/v1/"),
            "http://localhost:11434/v1"
        );
    }

    // ── provider-matrix-tests ─────────────────────────────────────────────────
    //
    // Each entry drives auto-provider selection from a single injected env var.
    // The resolver must pick exactly the documented provider when that variable
    // is the only key present (all other providers remain unconfigured).
    //
    // Providers that require multiple env vars (Bedrock, Vertex, Azure) are
    // covered by dedicated execution tests below rather than the matrix sweep.

    /// Verifies that every single-env-var API-key provider auto-selects itself
    /// when its advertised env var is the sole key present in the environment.
    #[test]
    fn provider_matrix_every_api_key_env_var_auto_selects_correct_provider() {
        // (env_var, dummy_value, expected_provider_id)
        // The dummy value is arbitrary — we only need the key to be non-empty
        // so the resolver considers auth ready.
        let cases: &[(&str, &str, &str)] = &[
            ("HYPER_API_KEY", "test-hyper-key", "hyper"),
            ("ANTHROPIC_API_KEY", "test-ant-key", "anthropic"),
            ("OPENAI_API_KEY", "test-oai-key", "openai"),
            ("VERCEL_API_KEY", "test-vercel-key", "vercel"),
            ("GEMINI_API_KEY", "test-gemini-key", "gemini"),
            ("SYNTHETIC_API_KEY", "test-syn-key", "synthetic"),
            ("ZAI_API_KEY", "test-zai-key", "zai"),
            ("MINIMAX_API_KEY", "test-mm-key", "minimax"),
            ("HF_TOKEN", "test-hf-token", "huggingface"),
            ("CEREBRAS_API_KEY", "test-cbr-key", "cerebras"),
            ("OPENROUTER_API_KEY", "test-or-key", "openrouter"),
            ("IONET_API_KEY", "test-ionet-key", "ionet"),
            ("GROQ_API_KEY", "test-groq-key", "groq"),
            ("AVIAN_API_KEY", "test-avian-key", "avian"),
            ("OPENCODE_API_KEY", "test-opencode-key", "opencode"),
        ];

        let resolver = ProviderResolver::builtin();

        for &(env_var, key_value, expected_provider) in cases {
            let report = resolver
                .resolve_with_env(
                    &AgentSettings::default(),
                    &StoredCredentials::default(),
                    // Inject only this one key — all other providers stay unauthenticated.
                    [(env_var, key_value.to_string())],
                )
                .unwrap_or_else(|e| panic!("resolve_with_env failed for env var `{env_var}`: {e}"));

            assert_eq!(
                report.provider.as_deref(),
                Some(expected_provider),
                "env var `{env_var}` should auto-select provider `{expected_provider}`"
            );
            assert_eq!(
                report.readiness,
                ProviderReadiness::Ready,
                "provider `{expected_provider}` must be Ready when `{env_var}` is set"
            );
            assert_eq!(
                report.auth.source_label(),
                Some("environment"),
                "auth for `{expected_provider}` must be sourced from environment"
            );
        }
    }

    /// Azure requires two env vars (endpoint + key); verify readiness when both are present.
    #[test]
    fn azure_readiness_requires_api_key_env_var() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("azure".into()),
            ..AgentSettings::default()
        };

        // Without key — must be MissingAuth.
        let missing = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
            )
            .expect("azure resolve without key should succeed");
        assert_eq!(
            missing.readiness,
            ProviderReadiness::MissingAuth,
            "azure should be MissingAuth when AZURE_OPENAI_API_KEY is absent"
        );

        // With key — must be Ready.
        let ready = resolver
            .resolve_with_env(
                &settings,
                &StoredCredentials::default(),
                [("AZURE_OPENAI_API_KEY", "azure-test-key".to_string())],
            )
            .expect("azure resolve with key should succeed");
        assert_eq!(
            ready.readiness,
            ProviderReadiness::Ready,
            "azure should be Ready when AZURE_OPENAI_API_KEY is present"
        );
        assert_eq!(ready.auth.source_label(), Some("environment"));
    }

    /// The `AZURE_OPENAI_API_ENDPOINT` env var is the only way to supply
    /// Azure's api_base at runtime (the descriptor has `api_base: None`).
    #[test]
    fn azure_resolves_api_base_from_endpoint_env_var() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("azure".into()),
            ..AgentSettings::default()
        };

        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [
                    (
                        "AZURE_OPENAI_API_ENDPOINT",
                        "https://my-azure.openai.azure.com".to_string(),
                    ),
                    ("AZURE_OPENAI_API_KEY", "azure-test-key".to_string()),
                ],
                &ProviderSelection::default(),
            )
            .expect("azure should resolve api_base from AZURE_OPENAI_API_ENDPOINT");

        assert_eq!(
            resolved.api_base(),
            "https://my-azure.openai.azure.com",
            "api_base must come from AZURE_OPENAI_API_ENDPOINT"
        );
        assert_eq!(resolved.provider_id(), "azure");
        assert_eq!(resolved.auth_source(), Some(AuthSource::Environment));
    }

    /// Azure fails to produce an execution when `AZURE_OPENAI_API_ENDPOINT` is
    /// absent and no settings override fills the gap.
    #[test]
    fn azure_execution_fails_without_endpoint_env_var() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("azure".into()),
            ..AgentSettings::default()
        };

        let error = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                // Provide the key but not the endpoint.
                [("AZURE_OPENAI_API_KEY", "azure-test-key".to_string())],
                &ProviderSelection::default(),
            )
            .expect_err("azure without endpoint env var must fail");

        let message = error.to_string();
        assert!(
            message.contains("AZURE_OPENAI_API_ENDPOINT"),
            "error should mention AZURE_OPENAI_API_ENDPOINT; got: {message}"
        );
    }

    /// Bedrock can source the AWS region from `AWS_DEFAULT_REGION` as an
    /// alternative to `AWS_REGION`.
    #[test]
    fn bedrock_region_sourced_from_aws_default_region_env_var() {
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
                    ("AWS_SECRET_ACCESS_KEY", "SECRET".to_string()),
                    // Use the fallback env var, not AWS_REGION.
                    ("AWS_DEFAULT_REGION", "ap-southeast-1".to_string()),
                ],
                &ProviderSelection::default(),
            )
            .expect("bedrock should resolve with AWS_DEFAULT_REGION");

        let creds = resolved.aws_credentials().expect("aws credentials");
        assert_eq!(
            creds.region, "ap-southeast-1",
            "region should be sourced from AWS_DEFAULT_REGION"
        );
    }

    /// Vertex AI resolves when a service-account key file path is supplied via
    /// `GOOGLE_APPLICATION_CREDENTIALS` instead of a pre-obtained bearer token.
    #[test]
    fn vertex_resolves_from_google_application_credentials_without_bearer_token() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("vertex".into()),
            ..AgentSettings::default()
        };

        let resolved = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [
                    ("VERTEXAI_PROJECT", "my-gcp-project".to_string()),
                    ("VERTEXAI_LOCATION", "europe-west4".to_string()),
                    // No GOOGLE_BEARER_TOKEN; supply a credentials file path instead.
                    (
                        "GOOGLE_APPLICATION_CREDENTIALS",
                        "/etc/gcp/service-account.json".to_string(),
                    ),
                ],
                &ProviderSelection::default(),
            )
            .expect("vertex should resolve from GOOGLE_APPLICATION_CREDENTIALS");

        assert_eq!(resolved.provider_id(), "vertex");
        assert_eq!(
            resolved.auth_state().status,
            wonder_of_u_core::AuthStatus::Ready
        );
        // No pre-obtained token — gcp_access_token() must return None.
        assert!(
            resolved.gcp_access_token().is_none(),
            "access_token must be None when only GOOGLE_APPLICATION_CREDENTIALS is set"
        );
        let (project, location) = resolved
            .gcp_project_location()
            .expect("gcp project and location");
        assert_eq!(project, "my-gcp-project");
        assert_eq!(location, "europe-west4");
    }

    /// Vertex requires project + location in addition to at least one of
    /// GOOGLE_BEARER_TOKEN / GOOGLE_APPLICATION_CREDENTIALS; missing all three
    /// must fail.
    #[test]
    fn vertex_fails_without_credential_env_vars() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("vertex".into()),
            ..AgentSettings::default()
        };

        let error = resolver
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                // Provide project + location but no credentials source.
                [
                    ("VERTEXAI_PROJECT", "my-project".to_string()),
                    ("VERTEXAI_LOCATION", "us-central1".to_string()),
                ],
                &ProviderSelection::default(),
            )
            .expect_err("vertex without credentials must fail");

        let message = error.to_string();
        assert!(
            message.contains("GOOGLE_APPLICATION_CREDENTIALS"),
            "error should mention GOOGLE_APPLICATION_CREDENTIALS; got: {message}"
        );
    }

    /// Verify that the synthetic provider is registered with the correct env var,
    /// protocol, and auth kind following the same pattern as other gateways.
    #[test]
    fn synthetic_provider_registered_with_expected_metadata() {
        let registry = ProviderRegistry::builtin();
        let synthetic = registry
            .get("synthetic")
            .expect("synthetic provider must be registered");

        assert_eq!(synthetic.display_name, "Synthetic AI");
        assert_eq!(
            synthetic.auth_kind,
            AuthMaterialKind::ApiKey,
            "synthetic must use ApiKey auth"
        );
        assert_eq!(
            synthetic.api_key_env.as_deref(),
            Some("SYNTHETIC_API_KEY"),
            "synthetic must read auth from SYNTHETIC_API_KEY"
        );
        assert_eq!(
            synthetic.wire_protocol,
            WireProtocol::OpenAiCompat,
            "synthetic must use OpenAiCompat protocol"
        );
        assert!(
            synthetic.api_base.is_some(),
            "synthetic must declare an api_base"
        );
        assert!(
            !synthetic.strict_model_validation,
            "synthetic must not enforce strict model validation"
        );
    }

    /// Verify the local model env/base resolution priority chain in a single
    /// parameterised sweep.  Each row isolates one source to confirm the
    /// documented priority:
    ///   WONDER_OF_U_LOCAL_API_BASE > OLLAMA_HOST > LMSTUDIO_API_BASE > descriptor default
    #[test]
    fn local_model_api_base_resolution_priority_matrix() {
        let resolver = ProviderResolver::builtin();
        let settings = AgentSettings {
            selected_provider: Some("local".into()),
            ..AgentSettings::default()
        };

        type EnvPairs<'a> = &'a [(&'a str, &'a str)];
        type LocalBaseCase<'a> = (EnvPairs<'a>, &'a str, &'a str);

        // (env vars to inject, expected api_base, description)
        let cases: &[LocalBaseCase<'_>] = &[
            (
                &[("WONDER_OF_U_LOCAL_API_BASE", "http://primary:9000/v1")],
                "http://primary:9000/v1",
                "WONDER_OF_U_LOCAL_API_BASE should win",
            ),
            (
                &[("OLLAMA_HOST", "http://gpu-box:11434")],
                "http://gpu-box:11434/v1",
                "OLLAMA_HOST without /v1 suffix should gain /v1",
            ),
            (
                &[("OLLAMA_HOST", "http://gpu-box:11434/v1")],
                "http://gpu-box:11434/v1",
                "OLLAMA_HOST already ending in /v1 must not duplicate suffix",
            ),
            (
                &[("LMSTUDIO_API_BASE", "http://localhost:1234/v1")],
                "http://localhost:1234/v1",
                "LMSTUDIO_API_BASE should be used when OLLAMA_HOST absent",
            ),
            (
                &[],
                DEFAULT_LOCAL_API_BASE,
                "descriptor default used when no env vars set",
            ),
            (
                &[
                    ("WONDER_OF_U_LOCAL_API_BASE", "http://primary:9999/v1"),
                    ("OLLAMA_HOST", "http://secondary:11434"),
                    ("LMSTUDIO_API_BASE", "http://tertiary:1234/v1"),
                ],
                "http://primary:9999/v1",
                "WONDER_OF_U_LOCAL_API_BASE must beat OLLAMA_HOST and LMSTUDIO_API_BASE",
            ),
            (
                &[
                    ("OLLAMA_HOST", "http://secondary:11434"),
                    ("LMSTUDIO_API_BASE", "http://tertiary:1234/v1"),
                ],
                "http://secondary:11434/v1",
                "OLLAMA_HOST must beat LMSTUDIO_API_BASE",
            ),
        ];

        for &(env_pairs, expected_base, description) in cases {
            let resolved = resolver
                .resolve_execution_with_env(
                    &settings,
                    &StoredCredentials::default(),
                    env_pairs
                        .iter()
                        .map(|&(k, v)| (k, v.to_string()))
                        .collect::<Vec<_>>(),
                    &ProviderSelection::default(),
                )
                .unwrap_or_else(|e| panic!("local resolve failed ({description}): {e}"));

            assert_eq!(resolved.api_base(), expected_base, "case: {description}");
        }
    }
}
