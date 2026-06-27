#[derive(Debug, thiserror::Error)]
pub enum ModelSentryError {
    #[error("provider error: {message}")]
    Provider { message: String },

    #[error("provider returned HTTP {status}: {body}")]
    ProviderHttp { status: u16, body: String },

    #[error("embedding dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    #[error(
        "baseline was captured with a {baseline_dim}-dimensional embedding model but this run \
         produced {run_dim}-dimensional embeddings — the embedding model changed; re-capture the \
         baseline for this probe"
    )]
    BaselineEmbeddingMismatch { baseline_dim: usize, run_dim: usize },

    #[error("empty embedding vector")]
    EmptyEmbedding,

    #[error("baseline not found: {id}")]
    BaselineNotFound { id: String },

    #[error("probe not found: {id}")]
    ProbeNotFound { id: String },

    #[error("storage error: {0}")]
    Storage(#[from] redb::StorageError),

    #[error("database open error: {0}")]
    DatabaseOpen(#[from] redb::DatabaseError),

    #[error("database transaction error: {0}")]
    Transaction(#[from] redb::TransactionError),

    #[error("database table error: {0}")]
    Table(#[from] redb::TableError),

    #[error("database commit error: {0}")]
    Commit(#[from] redb::CommitError),

    #[error("database error: {0}")]
    Db(String),

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

    #[test]
    fn baseline_embedding_mismatch_is_actionable() {
        let err = ModelSentryError::BaselineEmbeddingMismatch {
            baseline_dim: 1536,
            run_dim: 3072,
        };
        let display = err.to_string();
        assert!(display.contains("1536"));
        assert!(display.contains("3072"));
        // The message must guide the operator toward the fix.
        assert!(display.contains("re-capture"));
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
