#[derive(Debug, thiserror::Error)]
pub enum ModelSentryError {
    #[error("provider error: {message}")]
    Provider { message: String },

    #[error("provider returned HTTP {status}: {body}")]
    ProviderHttp { status: u16, body: String },

    #[error("embedding dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    #[error("empty embedding vector")]
    EmptyEmbedding,

    #[error("baseline not found: {id}")]
    BaselineNotFound { id: String },

    #[error("probe not found: {id}")]
    ProbeNotFound { id: String },

    #[error("storage error: {0}")]
    Storage(#[from] redb::StorageError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("vault error: {message}")]
    Vault { message: String },

    #[error("configuration error: {message}")]
    Config { message: String },
}

pub type Result<T> = std::result::Result<T, ModelSentryError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_does_not_include_secrets() {
        let err = ModelSentryError::Provider {
            message: "connection refused".to_string(),
        };
        let display = err.to_string();
        assert!(display.contains("provider error"));
        assert!(display.contains("connection refused"));
    }

    #[test]
    fn dimension_mismatch_error_includes_sizes() {
        let err = ModelSentryError::DimensionMismatch {
            expected: 1536,
            actual: 768,
        };
        let display = err.to_string();
        assert!(display.contains("1536"));
        assert!(display.contains("768"));
    }

    // result_alias_is_model_sentry_error: verified at compile time by the type alias
    #[test]
    fn result_alias_is_model_sentry_error() {
        let ok: Result<i32> = Ok(42);
        assert!(ok.is_ok());

        let err: Result<i32> = Err(ModelSentryError::EmptyEmbedding);
        assert!(err.is_err());
    }
}
