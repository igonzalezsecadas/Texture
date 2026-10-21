//! Server-rendered web UI. minijinja templates plus htmx; no JavaScript build
//! step and no client-side state machine.

pub mod defs;
pub mod filters;
pub mod handlers;
pub mod render;

use crate::config::Config;
use crate::storage::Db;
use axum::extract::DefaultBodyLimit;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{extract::Path, http::header, http::StatusCode, Router};
use std::sync::Arc;

/// Shared application state for the web layer and the import status endpoint.
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub cfg: Arc<Config>,
    pub importer: ImporterHandle,
}

/// A snapshot of importer activity, shared between the background task and the
/// import-status screen.
#[derive(Clone, Default)]
pub struct ImporterHandle {
    inner: Arc<std::sync::RwLock<ImporterSnapshot>>,
}

#[derive(Clone, Default)]
pub struct ImporterSnapshot {
    pub running: bool,
    pub kind: String,
    pub progress: crate::importer::ImportProgress,
    pub last_error: Option<String>,
}

impl ImporterHandle {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn snapshot(&self) -> ImporterSnapshot {
        self.inner.read().expect("importer state poisoned").clone()
    }
    pub fn set(&self, snap: ImporterSnapshot) {
        *self.inner.write().expect("importer state poisoned") = snap;
    }
    pub fn update(&self, f: impl FnOnce(&mut ImporterSnapshot)) {
        f(&mut self.inner.write().expect("importer state poisoned"));
    }
}

/// Errors are rendered as a plain-text response with the right status, which is
/// enough for a localhost-only single-user tool and keeps the handler code free
/// of template plumbing.
impl IntoResponse for handlers::WebError {
    fn into_response(self) -> Response {
        (self.0, self.1).into_response()
    }
}

impl std::fmt::Display for handlers::WebError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.1)
    }
}

impl std::fmt::Debug for handlers::WebError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WebError({}, {})", self.0, self.1)
    }
}

impl std::error::Error for handlers::WebError {}

// Static assets are embedded, exactly like the templates, so the container never
// depends on a runtime asset path.
const APP_CSS: &str = include_str!("static/app.css");
const APP_JS: &str = include_str!("static/app.js");
const HTMX_JS: &str = include_str!("static/htmx.min.js");
const UPLOT_JS: &str = include_str!("static/uPlot.iife.min.js");
const UPLOT_CSS: &str = include_str!("static/uPlot.min.css");
const HTMX_LICENSE: &str = include_str!("static/htmx.LICENSE.txt");
const UPLOT_LICENSE: &str = include_str!("static/uPlot.LICENSE.txt");

async fn static_asset(Path(name): Path<String>) -> Response {
    let (content_type, body): (&str, &str) = match name.as_str() {
        "app.css" => ("text/css; charset=utf-8", APP_CSS),
        "app.js" => ("application/javascript; charset=utf-8", APP_JS),
        "htmx.min.js" => ("application/javascript; charset=utf-8", HTMX_JS),
        "uPlot.iife.min.js" => ("application/javascript; charset=utf-8", UPLOT_JS),
        "uPlot.min.css" => ("text/css; charset=utf-8", UPLOT_CSS),
        "htmx.LICENSE.txt" => ("text/plain; charset=utf-8", HTMX_LICENSE),
        "uPlot.LICENSE.txt" => ("text/plain; charset=utf-8", UPLOT_LICENSE),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}

/// Build the router without binding a socket (used by tests).
///
/// `GET` routes are reader-only; the writes are settings, quarantine retry, and
/// hand-history upload, which redirect back or render their own summary.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(handlers::dashboard))
        .route("/preflop", get(handlers::preflop))
        .route("/postflop", get(handlers::postflop))
        .route("/position", get(handlers::position))
        .route("/sessions", get(handlers::sessions))
        .route("/hands", get(handlers::hands))
        .route("/hands/:id", get(handlers::hand_detail))
        .route("/import", get(handlers::import_screen))
        .route(
            "/import/upload",
            post(handlers::upload_hands)
                .layer(DefaultBodyLimit::max(handlers::MAX_UPLOAD_BYTES)),
        )
        .route("/quarantine/:id/retry", post(handlers::retry_quarantine))
        .route("/settings", get(handlers::settings))
        .route("/settings/profiles", post(handlers::create_profile))
        .route("/settings/profiles/:id/players", post(handlers::add_profile_player))
        .route("/settings/profiles/:id/activate", post(handlers::activate_profile))
        .route("/settings/session-gap", post(handlers::set_session_gap))
        .route("/settings/snapshot", post(handlers::take_snapshot_now))
        .route("/export/aggregate.csv", get(handlers::export_aggregate_csv))
        .route("/export/hands.csv", get(handlers::export_hands_csv))
        .route("/healthz", get(handlers::healthz))
        .route("/static/:asset", get(static_asset))
        .with_state(state)
}

/// Bind and serve until `shutdown` flips to `true`.
///
/// The importer handle is the process-wide one the watcher publishes to, so the
/// import screen shows live progress rather than a private idle snapshot.
pub async fn serve(
    db: Db,
    cfg: Config,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let state = AppState {
        db,
        cfg: Arc::new(cfg.clone()),
        importer: crate::importer::shared_importer_handle().clone(),
    };
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(cfg.bind_addr).await?;
    tracing::info!(addr = %cfg.bind_addr, "web ui listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown.changed().await;
        })
        .await?;
    Ok(())
}
