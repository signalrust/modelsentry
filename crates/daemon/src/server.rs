//! HTTP server — `AppState`, Axum router, and `run()` entry point.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, Response, StatusCode, header};
use axum::middleware::{self, Next};
use axum::{BoxError, Json, Router, routing::get};
use include_dir::{Dir, include_dir};
use modelsentry_common::{
    config::AppConfig,
    error::{ModelSentryError, Result},
};
use modelsentry_core::{alert::AlertEngine, drift::calculator::DriftCalculator};
use modelsentry_store::AppStore;
use tower::ServiceBuilder;
use tower_http::{cors::CorsLayer, limit::RequestBodyLimitLayer, trace::TraceLayer};

use crate::{routes, scheduler::ProviderRegistry, vault::Vault};

/// Maximum accepted request body. Probe/alert payloads are small; anything
/// larger is rejected with `413 Payload Too Large` to bound memory use.
const MAX_BODY_BYTES: usize = 1024 * 1024; // 1 MiB

/// Per-client rate limit (applied in `run()` keyed by peer IP): the bucket
/// holds up to `RATE_LIMIT_BURST` requests and refills one token per second,
/// so a steady stream is allowed but credential brute-forcing is throttled.
const RATE_LIMIT_BURST: u32 = 100;
const RATE_LIMIT_REPLENISH_SECS: u64 = 1;

/// `GET /health` — lightweight liveness probe for the daemon.
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "service": "modelsentry-daemon" }))
}

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

    Router::new()
        .nest("/api", api)
        .route("/health", get(health))
        // Serve the embedded SvelteKit dashboard for all non-API requests
        .route("/", get(serve_index))
        .route("/{*path}", get(serve_static))
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
            .get("x-api-key")
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
    .await
    .map_err(|e| ModelSentryError::Config {
        message: format!("server error: {e}"),
    })
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
    use modelsentry_core::{alert::AlertEngine, drift::calculator::DriftCalculator};
    use modelsentry_store::AppStore;

    use super::*;
    use crate::scheduler::new_registry;

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
            },
            alerts: AlertsConfig {
                drift_threshold_kl: 0.5,
                drift_threshold_cos: 0.5,
                allow_private_webhook_targets: false,
            },
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
            providers: Arc::new(new_registry()),
            calculator: Arc::new(DriftCalculator::new(0.5, 0.5).unwrap()),
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
                axum::http::HeaderName::from_static("x-api-key"),
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
