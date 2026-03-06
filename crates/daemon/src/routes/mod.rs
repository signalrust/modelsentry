//! Route modules and shared `AppError` type.

use axum::{
    Json, Router,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

use modelsentry_common::error::ModelSentryError;

use crate::server::AppState;

pub mod alerts;
pub mod baselines;
pub mod probes;
pub mod runs;
pub mod vault;

/// Assemble all route groups under a shared `AppState`.
pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(probes::router())
        .merge(baselines::router())
        .merge(runs::router())
        .merge(alerts::router())
        .merge(vault::router())
        .with_state(state)
}

// ---------------------------------------------------------------------------
// AppError — converts ModelSentryError into HTTP responses
// ---------------------------------------------------------------------------

/// Axum-compatible error wrapper.
#[derive(Debug)]
pub struct AppError(ModelSentryError);

impl From<ModelSentryError> for AppError {
    fn from(e: ModelSentryError) -> Self {
        Self(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self.0 {
            ModelSentryError::ProbeNotFound { .. } => (StatusCode::NOT_FOUND, self.0.to_string()),
            ModelSentryError::BaselineNotFound { .. } => {
                (StatusCode::NOT_FOUND, self.0.to_string())
            }
            ModelSentryError::Config { .. } => {
                (StatusCode::UNPROCESSABLE_ENTITY, self.0.to_string())
            }
            ModelSentryError::Vault { .. } => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string())
            }
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".to_string(),
            ),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
