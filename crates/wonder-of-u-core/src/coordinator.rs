use serde::{Deserialize, Serialize};

/// Minimal coordinator mode metadata used by local orchestration paths.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinatorMode {
    #[default]
    Direct,
    Local,
    Cloud,
    External,
}

impl CoordinatorMode {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Local => "local",
            Self::Cloud => "cloud",
            Self::External => "external",
        }
    }
}

/// Support level for coordinator surfaces that do not have a real backend yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinatorSupport {
    LocalOnly,
    Unsupported,
    Deferred,
}

impl CoordinatorSupport {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LocalOnly => "local_only",
            Self::Unsupported => "unsupported",
            Self::Deferred => "deferred",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CoordinatorSurfaceStatus {
    pub surface: String,
    pub support: CoordinatorSupport,
    pub reason: String,
}

impl CoordinatorSurfaceStatus {
    #[must_use]
    pub fn local_only(surface: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            surface: surface.into(),
            support: CoordinatorSupport::LocalOnly,
            reason: reason.into(),
        }
    }

    #[must_use]
    pub fn unsupported(surface: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            surface: surface.into(),
            support: CoordinatorSupport::Unsupported,
            reason: reason.into(),
        }
    }

    #[must_use]
    pub fn deferred(surface: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            surface: surface.into(),
            support: CoordinatorSupport::Deferred,
            reason: reason.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CoordinatorState {
    pub mode: CoordinatorMode,
    pub task_dispatch: CoordinatorSurfaceStatus,
    pub cloud_queries: CoordinatorSurfaceStatus,
    pub external_backend: CoordinatorSurfaceStatus,
}

impl CoordinatorState {
    #[must_use]
    pub fn for_mode(mode: CoordinatorMode) -> Self {
        Self {
            mode,
            task_dispatch: CoordinatorSurfaceStatus::local_only(
                "task_dispatch",
                "worker/task coordination stays local to the current runtime",
            ),
            cloud_queries: CoordinatorSurfaceStatus::unsupported(
                "cloud_queries",
                "cloud coordinator queries are unsupported in this Rust port",
            ),
            external_backend: CoordinatorSurfaceStatus::deferred(
                "external_backend",
                "external coordinator backends are deferred until a concrete remote service exists",
            ),
        }
    }
}

impl Default for CoordinatorState {
    fn default() -> Self {
        Self::for_mode(CoordinatorMode::Direct)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinator_state_marks_cloud_and_external_surfaces_without_backends() {
        let cloud = CoordinatorState::for_mode(CoordinatorMode::Cloud);
        assert_eq!(cloud.mode, CoordinatorMode::Cloud);
        assert_eq!(cloud.cloud_queries.support, CoordinatorSupport::Unsupported);
        assert!(
            cloud
                .cloud_queries
                .reason
                .contains("unsupported in this Rust port")
        );

        let external = CoordinatorState::for_mode(CoordinatorMode::External);
        assert_eq!(
            external.external_backend.support,
            CoordinatorSupport::Deferred
        );
        assert!(
            external
                .external_backend
                .reason
                .contains("deferred until a concrete remote service exists")
        );
    }
}
