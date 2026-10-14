//! Server-rendered web UI. minijinja templates plus htmx; no JavaScript build
//! step and no client-side state machine.

use crate::config::Config;
use crate::storage::Db;
use axum::Router;
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

/// Build the router without binding a socket (used by tests).
pub fn router(state: AppState) -> Router {
    let _ = state;
    unimplemented!("web::router")
}

/// Bind and serve until `shutdown` flips to `true`.
pub async fn serve(
    db: Db,
    cfg: Config,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let _ = (db, cfg, shutdown);
    unimplemented!("web::serve")
}
