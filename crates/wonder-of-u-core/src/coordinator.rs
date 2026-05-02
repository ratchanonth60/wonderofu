use serde::{Deserialize, Serialize};

/// Minimal coordinator mode metadata used by local orchestration paths.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinatorMode {
    /// Represents direct
    #[default]
    Direct,
    /// Represents local
    Local,
    /// Represents cloud
    Cloud,
    /// Represents external
    External,
}

impl CoordinatorMode {
    /// Constant fn
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
    /// Represents local only
    LocalOnly,
    /// Represents unsupported
    Unsupported,
    /// Represents deferred
    Deferred,
}

impl CoordinatorSupport {
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LocalOnly => "local_only",
            Self::Unsupported => "unsupported",
            Self::Deferred => "deferred",
        }
    }
}
/// Represents coordinator surface status
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CoordinatorSurfaceStatus {
    /// Stores the surface
    pub surface: String,
    /// Stores the support
    pub support: CoordinatorSupport,
    /// Stores the reason
    pub reason: String,
}

impl CoordinatorSurfaceStatus {
    /// Handles local only
    #[must_use]
    pub fn local_only(surface: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            surface: surface.into(),
            support: CoordinatorSupport::LocalOnly,
            reason: reason.into(),
        }
    }
    /// Handles unsupported
    #[must_use]
    pub fn unsupported(surface: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            surface: surface.into(),
            support: CoordinatorSupport::Unsupported,
            reason: reason.into(),
        }
    }
    /// Handles deferred
    #[must_use]
    pub fn deferred(surface: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            surface: surface.into(),
            support: CoordinatorSupport::Deferred,
            reason: reason.into(),
        }
    }
}
/// Represents coordinator state
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CoordinatorState {
    /// Stores the mode
    pub mode: CoordinatorMode,
    /// Stores the task dispatch
    pub task_dispatch: CoordinatorSurfaceStatus,
    /// Stores the cloud queries
    pub cloud_queries: CoordinatorSurfaceStatus,
    /// Stores the external backend
    pub external_backend: CoordinatorSurfaceStatus,
}

impl CoordinatorState {
    /// Handles for mode
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
