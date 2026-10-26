//! Errors reported by the library.

/// A failure to prepare a model or transform a document.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Filesystem or stream error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Malformed metadata.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Invalid user input or configuration.
    #[error("{0}")]
    InvalidInput(String),
    /// Native runtime or model failure.
    #[error("inference failed: {0}")]
    Inference(String),
    /// Model download or validation failure.
    #[error("model setup failed: {0}")]
    Model(String),
    /// Unsupported or malformed Word document.
    #[error("DOCX processing failed: {0}")]
    Document(String),
    /// Cancellation requested between chunks.
    #[error("cancelled")]
    Cancelled,
}

/// A library result.
pub type Result<T> = std::result::Result<T, Error>;
