//! HTTP server — `AppState`, Axum router, and `run()` entry point.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Response, StatusCode, header};
use axum::middleware::{self, Next};
use axum::{BoxError, Json, Router, routing::get};
use modelsentry_common::{
    config::AppConfig,
    constants::header as app_header,
    error::{ModelSentryError, Result},
};
use modelsentry_core::{alert::AlertEngine, drift::calculator::DriftCalculator};
use modelsentry_store::AppStore;
use tower::ServiceBuilder;
use tower_http::{cors::CorsLayer, limit::RequestBodyLimitLayer, trace::TraceLayer};

use crate::constants::server::{MAX_BODY_BYTES, RATE_LIMIT_BURST, RATE_LIMIT_REPLENISH_SECS};
use crate::{routes, vault::Vault};

/// `GET /health` — lightweight liveness probe for the daemon.
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "service": "modelsentry-daemon" }))
}

/// Shared application state injected into every route handler.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<AppStore>,
    pub vault: Arc<Vault>,
    pub calculator: Arc<DriftCalculator>,
    pub alert_engine: Arc<AlertEngine>,
    pub config: Arc<AppConfig>,
}

/// Build the Axum router with all routes and middleware.
#[allow(clippy::needless_pass_by_value)]
pub fn build_router(state: AppState) -> Router {
    let config = Arc::clone(&state.config);
    let api = routes::router(state.clone());

    // Apply auth middleware to the API sub-router when enabled.
    let api = if config.auth.enabled {
        let keys: Arc<Vec<String>> = Arc::new(config.auth.api_keys.clone());
        api.layer(middleware::from_fn_with_state(keys, auth_middleware))
    } else {
        api
    };

    // Build CORS layer from config.
    let cors = if config.server.cors_origin == "*" {
        CorsLayer::permissive()
    } else if let Ok(origin) = config.server.cors_origin.parse::<HeaderValue>() {
        CorsLayer::new()
            .allow_origin(origin)
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::DELETE,
            ])
            .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
    } else {
        tracing::warn!(
            origin = %config.server.cors_origin,
            "invalid CORS origin in config, falling back to restrictive"
        );
        CorsLayer::new()
    };

    // API-only server: the dashboard is served separately (the Node container in
    // docker-compose). Non-`/api`, non-`/health` paths return 404.
    Router::new()
        .nest("/api", api)
        .route("/health", get(health))
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
                .layer(axum::error_handling::HandleErrorLayer::new(
                    |_: BoxError| async { StatusCode::REQUEST_TIMEOUT },
                ))
                .layer(tower::timeout::TimeoutLayer::new(Duration::from_secs(
                    config.server.timeout_secs,
                )))
                .layer(cors),
        )
}

/// Middleware that checks for a valid API key in `Authorization: Bearer <key>`
/// or `X-Api-Key: <key>` headers.
async fn auth_middleware(
    State(api_keys): State<Arc<Vec<String>>>,
    request: Request,
    next: Next,
) -> std::result::Result<Response<Body>, StatusCode> {
    let headers = request.headers();

    // Check Authorization: Bearer <token>
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);

    // Fall back to X-Api-Key header
    let token = token.or_else(|| {
        headers
            .get(app_header::API_KEY)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    });

    match token {
        Some(t) if token_matches(&api_keys, &t) => Ok(next.run(request).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Check `candidate` against every configured key in constant time.
///
/// Uses `subtle::ConstantTimeEq` and always compares against all keys (no
/// early return) so the response latency does not leak how many leading bytes
/// of a key were guessed — closing the timing side-channel that a plain
/// `Vec::contains` / `==` comparison would open.
fn token_matches(api_keys: &[String], candidate: &str) -> bool {
    use subtle::ConstantTimeEq;

    let candidate = candidate.as_bytes();
    let mut matched = subtle::Choice::from(0u8);
    for key in api_keys {
        matched |= key.as_bytes().ct_eq(candidate);
    }
    matched.into()
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

    // Per-IP rate limiting (keyed by peer address). Applied only here on the
    // real serve path — `build_router` stays free of it so in-process tests
    // (which have no socket / ConnectInfo) are unaffected.
    let governor_conf = Arc::new(
        tower_governor::governor::GovernorConfigBuilder::default()
            .per_second(RATE_LIMIT_REPLENISH_SECS)
            .burst_size(RATE_LIMIT_BURST)
            .finish()
            .ok_or_else(|| ModelSentryError::Config {
                message: "invalid rate-limit configuration".to_string(),
            })?,
    );
    let router = build_router(state).layer(tower_governor::GovernorLayer {
        config: governor_conf,
    });

    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .map_err(|e| ModelSentryError::Config {
        message: format!("server error: {e}"),
    })
}

/// Resolve when the process receives a shutdown signal — `Ctrl+C` on every
/// platform, or `SIGTERM` on Unix (the signal an orchestrator sends to stop a
/// container). Drives Axum's graceful shutdown so in-flight requests finish
/// before the listener closes. If a handler cannot be installed the future
/// simply never resolves, leaving the server running (fail-safe).
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!("failed to install Ctrl+C handler: {e}");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                tracing::error!("failed to install SIGTERM handler: {e}");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum_test::TestServer;
    use std::sync::Arc;

    use modelsentry_common::config::{
        AlertsConfig, AuthConfig, DatabaseConfig, ProvidersConfig, SchedulerConfig, ServerConfig,
        VaultConfig,
    };
    use modelsentry_core::{
        alert::AlertEngine,
        drift::{assessment::AssessmentConfig, calculator::DriftCalculator},
    };
    use modelsentry_store::AppStore;

    use super::*;

    fn open_store() -> (tempfile::TempDir, Arc<AppStore>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        (dir, Arc::new(AppStore::open(&path).unwrap()))
    }

    fn make_config(auth_enabled: bool, api_keys: Vec<String>) -> AppConfig {
        AppConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 7740,
                timeout_secs: 30,
                cors_origin: "http://localhost:5173".to_string(),
            },
            vault: VaultConfig {
                path: std::path::PathBuf::from("/tmp/vault.age"),
            },
            database: DatabaseConfig {
                path: std::path::PathBuf::from("/tmp/modelsentry.db"),
            },
            scheduler: SchedulerConfig {
                default_interval_minutes: 60,
                max_concurrent_runs: 8,
            },
            alerts: AlertsConfig::default(),
            providers: ProvidersConfig::default(),
            auth: AuthConfig {
                enabled: auth_enabled,
                api_keys,
            },
        }
    }

    fn test_server(config: AppConfig) -> (tempfile::TempDir, tempfile::TempDir, TestServer) {
        let (db_dir, store) = open_store();
        let vault_dir = tempfile::tempdir().unwrap();
        let vault_path = vault_dir.path().join("v.age");
        let state = AppState {
            store,
            vault: Arc::new(
                crate::vault::Vault::create(
                    &vault_path,
                    secrecy::SecretString::new("test".to_string().into()),
                )
                .unwrap(),
            ),
            calculator: Arc::new(DriftCalculator::new(AssessmentConfig::default())),
            alert_engine: Arc::new(AlertEngine::default()),
            config: Arc::new(config),
        };
        let router = build_router(state);
        (db_dir, vault_dir, TestServer::new(router))
    }

    #[tokio::test]
    async fn auth_disabled_allows_unauthenticated_requests() {
        let config = make_config(false, vec![]);
        let (_db, _vault, server) = test_server(config);
        let resp = server.get("/api/probes").await;
        resp.assert_status_ok();
    }

    #[tokio::test]
    async fn oversized_request_body_is_rejected() {
        let config = make_config(false, vec![]);
        let (_db, _vault, server) = test_server(config);
        // 2 MiB > the 1 MiB MAX_BODY_BYTES limit. Send it as JSON so the body
        // is actually read; the limit aborts the read and axum maps the
        // length-limit error to 413 Payload Too Large.
        let big = axum::body::Bytes::from(vec![b'x'; 2 * 1024 * 1024]);
        let resp = server
            .post("/api/probes")
            .content_type("application/json")
            .bytes(big)
            .await;
        resp.assert_status(StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn auth_enabled_rejects_unauthenticated_requests() {
        let config = make_config(true, vec!["secret-key".to_string()]);
        let (_db, _vault, server) = test_server(config);
        let resp = server.get("/api/probes").await;
        resp.assert_status(StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn auth_enabled_accepts_valid_bearer_token() {
        let config = make_config(true, vec!["secret-key".to_string()]);
        let (_db, _vault, server) = test_server(config);
        let resp = server
            .get("/api/probes")
            .add_header(
                axum::http::header::AUTHORIZATION,
                "Bearer secret-key"
                    .parse::<axum::http::HeaderValue>()
                    .unwrap(),
            )
            .await;
        resp.assert_status_ok();
    }

    #[tokio::test]
    async fn auth_enabled_accepts_valid_x_api_key() {
        let config = make_config(true, vec!["secret-key".to_string()]);
        let (_db, _vault, server) = test_server(config);
        let resp = server
            .get("/api/probes")
            .add_header(
                axum::http::HeaderName::from_static(app_header::API_KEY),
                "secret-key".parse::<axum::http::HeaderValue>().unwrap(),
            )
            .await;
        resp.assert_status_ok();
    }

    #[tokio::test]
    async fn auth_enabled_rejects_invalid_key() {
        let config = make_config(true, vec!["secret-key".to_string()]);
        let (_db, _vault, server) = test_server(config);
        let resp = server
            .get("/api/probes")
            .add_header(
                axum::http::header::AUTHORIZATION,
                "Bearer wrong-key"
                    .parse::<axum::http::HeaderValue>()
                    .unwrap(),
            )
            .await;
        resp.assert_status(StatusCode::UNAUTHORIZED);
    }
}
