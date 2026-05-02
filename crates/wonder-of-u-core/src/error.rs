use thiserror::Error;

/// Shared result type for wonder-of-u crates.
pub type Result<T> = std::result::Result<T, WonderError>;

/// Errors that cross crate boundaries.
#[derive(Debug, Error)]
pub enum WonderError {
    /// Represents validation
    #[error("validation failed: {0}")]
    Validation(String),
    /// Represents not found
    #[error("{resource} not found: {id}")]
    NotFound {
        /// Stores the resource
        resource: String,
        /// Stores the id
        id: String,
    },
    /// Represents permission denied
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// Represents internal
    #[error("internal error: {0}")]
    Internal(String),
    /// Represents io
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Represents json
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl WonderError {
    /// Handles validation
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }

    /// Handles not found
    pub fn not_found(resource: impl Into<String>, id: impl Into<String>) -> Self {
        Self::NotFound {
            resource: resource.into(),
            id: id.into(),
        }
    }

    /// Handles permission denied
    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::PermissionDenied(message.into())
    }

    /// Handles internal
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}
