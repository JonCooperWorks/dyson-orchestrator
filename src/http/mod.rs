//! HTTP server assembly.
//!
//! At this point only the secrets routes are wired. Step 7 brings instance
//! routes + admin-bearer middleware; Step 8 adds snapshots; Step 11 stitches
//! the full router together with healthz, graceful shutdown, and the proxy
//! mount. Each sub-module exports a `router(state)` function so the top-level
//! assembly is just a couple of `.merge()` calls.

pub mod secrets;

use std::sync::Arc;

use axum::Router;

use crate::secrets::SecretsService;

/// Shared state handed to every route handler.
///
/// Cheap to clone — every field is an `Arc`. Components are added as the
/// remaining steps land.
#[derive(Clone)]
pub struct AppState {
    pub secrets: Arc<SecretsService>,
}

/// Build the public `Router`. Routes that need admin auth will be wrapped by
/// the middleware introduced in step 7.
pub fn router(state: AppState) -> Router {
    Router::new().merge(secrets::router(state.clone()))
}
