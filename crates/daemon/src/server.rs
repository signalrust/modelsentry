//! HTTP server — `AppState`, Axum router, and `run()` entry point.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::Path;
use axum::http::{HeaderValue, Response, StatusCode, header};
use axum::{BoxError, Router, routing::get};
use include_dir::{Dir, include_dir};
use modelsentry_common::{
    config::AppConfig,
    error::{ModelSentryError, Result},
};
use modelsentry_core::{alert::AlertEngine, drift::calculator::DriftCalculator};
use modelsentry_store::AppStore;
use tower::ServiceBuilder;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{routes, scheduler::ProviderRegistry, vault::Vault};

/// Compiled-in copy of the `SvelteKit` static build output.
///
/// In development, `web/build/` may be empty or absent; the handler returns
/// 404s for asset requests which is fine because Vite's dev server handles them.
///
/// In a release build the CI workflow runs `npm run build` before `cargo build`
/// so the directory is fully populated and embedded.
static WEB_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../web/build");

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

/// Serve a file from the embedded `WEB_DIR`, falling back to `index.html`
/// for unrecognised paths (`SvelteKit` client-side routing).
async fn serve_static(Path(path): Path<String>) -> Response<Body> {
    // Strip leading slash if present
    let rel = path.trim_start_matches('/');
    serve_file(rel)
}

async fn serve_index() -> Response<Body> {
    serve_file("index.html")
}

fn serve_file(rel: &str) -> Response<Body> {
    let file = WEB_DIR
        .get_file(rel)
        .or_else(|| WEB_DIR.get_file("index.html"));
    if let Some(f) = file {
        let mime = mime_guess::from_path(f.path())
            .first_raw()
            .unwrap_or("application/octet-stream");
        let mut resp = Response::new(Body::from(f.contents()));
        resp.headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
        resp
    } else {
        let mut resp = Response::new(Body::from("Not Found"));
        *resp.status_mut() = StatusCode::NOT_FOUND;
        resp
    }
}

/// Build the Axum router with all routes and middleware.
pub fn build_router(state: AppState) -> Router {
    let api = routes::router(state);

    Router::new()
        .nest("/api", api)
        // Serve the embedded SvelteKit dashboard for all non-API requests
        .route("/", get(serve_index))
        .route("/{*path}", get(serve_static))
        .layer(
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
