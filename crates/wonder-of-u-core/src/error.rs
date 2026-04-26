use thiserror::Error;

/// Shared result type for wonder-of-u crates.
pub type Result<T> = std::result::Result<T, WonderError>;

/// Errors that cross crate boundaries.
#[derive(Debug, Error)]
pub enum WonderError {
    #[error("validation failed: {0}")]
    Validation(String),

    #[error("{resource} not found: {id}")]
    NotFound { resource: String, id: String },

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl WonderError {
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }

    pub fn not_found(resource: impl Into<String>, id: impl Into<String>) -> Self {
        Self::NotFound {
            resource: resource.into(),
            id: id.into(),
        }
    }

    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::PermissionDenied(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}
