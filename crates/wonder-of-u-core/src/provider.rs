use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMaterialKind {
    #[default]
    None,
    ApiKey,
    OAuth,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    NotRequired,
    Missing,
    Ready,
    Pending,
}

impl Default for AuthStatus {
    fn default() -> Self {
        Self::NotRequired
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthSource {
    Environment,
    CredentialsFile,
    Settings,
    Interactive,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuthState {
    pub kind: AuthMaterialKind,
    pub status: AuthStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<AuthSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl AuthState {
    #[must_use]
    pub fn not_required() -> Self {
        Self {
            kind: AuthMaterialKind::None,
            status: AuthStatus::NotRequired,
            source: None,
            detail: None,
        }
    }

    #[must_use]
    pub fn missing(kind: AuthMaterialKind) -> Self {
        Self {
            kind,
            status: AuthStatus::Missing,
            source: None,
            detail: None,
        }
    }

    #[must_use]
    pub fn ready(kind: AuthMaterialKind, source: AuthSource) -> Self {
        Self {
            kind,
            status: AuthStatus::Ready,
            source: Some(source),
            detail: None,
        }
    }

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

    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self.status, AuthStatus::NotRequired | AuthStatus::Ready)
    }

    #[must_use]
    pub const fn status_label(&self) -> &'static str {
        match self.status {
            AuthStatus::NotRequired => "not_required",
            AuthStatus::Missing => "missing",
            AuthStatus::Ready => "ready",
            AuthStatus::Pending => "pending",
        }
    }

    #[must_use]
    pub const fn kind_label(&self) -> &'static str {
        match self.kind {
            AuthMaterialKind::None => "none",
            AuthMaterialKind::ApiKey => "api_key",
            AuthMaterialKind::OAuth => "oauth",
        }
    }

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderReadiness {
    #[default]
    Unconfigured,
    MissingAuth,
    Ready,
}

impl ProviderReadiness {
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
