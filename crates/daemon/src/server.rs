//! HTTP server — `AppState`, Axum router, and `run()` entry point.

use std::sync::Arc;
use std::time::Duration;

use axum::BoxError;
use axum::Router;
use axum::http::StatusCode;
use modelsentry_common::{
    config::AppConfig,
    error::{ModelSentryError, Result},
};
use modelsentry_core::{alert::AlertEngine, drift::calculator::DriftCalculator};
use modelsentry_store::AppStore;
use tower::ServiceBuilder;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{routes, scheduler::ProviderRegistry, vault::Vault};

/// Shared application state injected into every route handler.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<AppStore>,
    pub vault: Arc<Vault>,
    pub providers: Arc<ProviderRegistry>,
    pub calculator: Arc<DriftCalculator>,
    pub alert_engine: Arc<AlertEngine>,
    pub config: Arc<AppConfig>,
}

/// Build the Axum router with all routes and middleware.
pub fn build_router(state: AppState) -> Router {
    let api = routes::router(state);

    Router::new().nest("/api", api).layer(
        ServiceBuilder::new()
            .layer(TraceLayer::new_for_http())
            .layer(axum::error_handling::HandleErrorLayer::new(
                |_: BoxError| async { StatusCode::REQUEST_TIMEOUT },
            ))
            .layer(tower::timeout::TimeoutLayer::new(Duration::from_secs(30)))
            .layer(CorsLayer::permissive()),
    )
}

/// Start the HTTP server and block until it terminates.
///
/// # Errors
///
/// Returns an error if the listener cannot be bound or the server encounters
/// a fatal error.
pub async fn run(config: &AppConfig, state: AppState) -> Result<()> {
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener =
        tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| ModelSentryError::Config {
                message: format!("cannot bind to {addr}: {e}"),
            })?;

    tracing::info!("listening on {addr}");
    let router = build_router(state);
    axum::serve(listener, router)
        .await
        .map_err(|e| ModelSentryError::Config {
            message: format!("server error: {e}"),
        })
}
