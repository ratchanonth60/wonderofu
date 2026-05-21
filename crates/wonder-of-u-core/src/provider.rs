use serde::{Deserialize, Serialize};
/// Enumerates auth material kind
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMaterialKind {
    /// Represents none
    #[default]
    None,
    /// Represents api key
    ApiKey,
    /// Represents o auth
    OAuth,
    /// Represents AWS Signature Version 4 (used by Amazon Bedrock)
    AwsSigV4,
    /// Short-lived bearer token sourced from `AWS_BEARER_TOKEN_BEDROCK`.
    ///
    /// Highest-priority resolution path for Bedrock; no signing required.
    AwsBearer,
    /// Named AWS profile resolved from `~/.aws/credentials` (or
    /// `AWS_SHARED_CREDENTIALS_FILE`).  Falls back to this when neither a
    /// bearer token nor static SigV4 env vars are present.
    AwsProfile,
    /// Google Cloud OAuth 2 access token for Vertex AI.
    ///
    /// Requires `VERTEXAI_PROJECT`, `VERTEXAI_LOCATION`, and
    /// `GOOGLE_APPLICATION_CREDENTIALS` to be set.
    GcpOAuth2,
}
/// Enumerates auth status
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    /// Represents not required
    #[default]
    NotRequired,
    /// Represents missing
    Missing,
    /// Represents ready
    Ready,
    /// Represents pending
    Pending,
}
/// Enumerates auth source
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthSource {
    /// Represents environment
    Environment,
    /// Represents credentials file
    CredentialsFile,
    /// Represents settings
    Settings,
    /// Represents interactive
    Interactive,
}
/// Represents auth state
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuthState {
    /// Stores the kind
    pub kind: AuthMaterialKind,
    /// Stores the status
    pub status: AuthStatus,
    /// Stores the source
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<AuthSource>,
    /// Stores the detail
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl AuthState {
    /// Handles not required
    #[must_use]
    pub fn not_required() -> Self {
        Self {
            kind: AuthMaterialKind::None,
            status: AuthStatus::NotRequired,
            source: None,
            detail: None,
        }
    }
    /// Handles missing
    #[must_use]
    pub fn missing(kind: AuthMaterialKind) -> Self {
        Self {
            kind,
            status: AuthStatus::Missing,
            source: None,
            detail: None,
        }
    }
    /// Handles ready
    #[must_use]
    pub fn ready(kind: AuthMaterialKind, source: AuthSource) -> Self {
        Self {
            kind,
            status: AuthStatus::Ready,
            source: Some(source),
            detail: None,
        }
    }
    /// Handles pending
    #[must_use]
    pub fn pending(
        kind: AuthMaterialKind,
        source: Option<AuthSource>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            status: AuthStatus::Pending,
            source,
            detail: Some(detail.into()),
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self.status, AuthStatus::NotRequired | AuthStatus::Ready)
    }
    /// Constant fn
    #[must_use]
    pub const fn status_label(&self) -> &'static str {
        match self.status {
            AuthStatus::NotRequired => "not_required",
            AuthStatus::Missing => "missing",
            AuthStatus::Ready => "ready",
            AuthStatus::Pending => "pending",
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn kind_label(&self) -> &'static str {
        match self.kind {
            AuthMaterialKind::None => "none",
            AuthMaterialKind::ApiKey => "api_key",
            AuthMaterialKind::OAuth => "oauth",
            AuthMaterialKind::AwsSigV4 => "aws_sig_v4",
            AuthMaterialKind::AwsBearer => "aws_bearer",
            AuthMaterialKind::AwsProfile => "aws_profile",
            AuthMaterialKind::GcpOAuth2 => "gcp_oauth2",
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn source_label(&self) -> Option<&'static str> {
        match self.source {
            Some(AuthSource::Environment) => Some("environment"),
            Some(AuthSource::CredentialsFile) => Some("credentials_file"),
            Some(AuthSource::Settings) => Some("settings"),
            Some(AuthSource::Interactive) => Some("interactive"),
            None => None,
        }
    }
}
/// Enumerates provider readiness
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderReadiness {
    /// Represents unconfigured
    #[default]
    Unconfigured,
    /// Represents missing auth
    MissingAuth,
    /// Represents ready
    Ready,
}

impl ProviderReadiness {
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unconfigured => "unconfigured",
            Self::MissingAuth => "missing_auth",
            Self::Ready => "ready",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_state_labels_are_stable() {
        let auth = AuthState::ready(AuthMaterialKind::ApiKey, AuthSource::Environment);

        assert_eq!(auth.kind_label(), "api_key");
        assert_eq!(auth.status_label(), "ready");
        assert_eq!(auth.source_label(), Some("environment"));
    }

    #[test]
    fn all_auth_material_kind_labels_are_distinct_and_stable() {
        // Ensures that every variant has a unique, stable serialisation label.
        let cases = [
            (AuthMaterialKind::None, "none"),
            (AuthMaterialKind::ApiKey, "api_key"),
            (AuthMaterialKind::OAuth, "oauth"),
            (AuthMaterialKind::AwsSigV4, "aws_sig_v4"),
            (AuthMaterialKind::AwsBearer, "aws_bearer"),
            (AuthMaterialKind::AwsProfile, "aws_profile"),
            (AuthMaterialKind::GcpOAuth2, "gcp_oauth2"),
        ];
        let labels: Vec<_> = cases.iter().map(|(_, label)| *label).collect();
        // All labels must be unique.
        let unique: std::collections::HashSet<_> = labels.iter().copied().collect();
        assert_eq!(unique.len(), labels.len(), "duplicate kind labels detected");

        for (kind, expected_label) in &cases {
            let auth = AuthState::missing(*kind);
            assert_eq!(auth.kind_label(), *expected_label, "mismatch for {kind:?}");
        }
    }
}
