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
}
