//! HTTP server assembly.
//!
//! Each sub-module exports a `router(state)` factory; this module composes
//! them and wraps `/v1/*` in the admin-bearer middleware. Step 11 adds
//! `/healthz` and graceful shutdown; step 14 mounts the `/llm/` proxy.

pub mod instances;
pub mod secrets;
pub mod snapshots;

use std::sync::Arc;

use axum::{middleware, Router};

use crate::auth::{admin_bearer, AuthState};
use crate::instance::InstanceService;
use crate::secrets::SecretsService;
use crate::snapshot::SnapshotService;

/// Shared state handed to every route handler. Cheap to clone — every field
/// is an `Arc` or scalar `String`.
#[derive(Clone)]
pub struct AppState {
    pub secrets: Arc<SecretsService>,
    pub instances: Arc<InstanceService>,
    pub snapshots: Arc<SnapshotService>,
    pub sandbox_domain: String,
}

/// Build the public `Router`. All `/v1/*` routes share the admin-bearer
/// middleware. The `auth` argument decides whether the middleware enforces a
/// token or runs in `--dangerous-no-auth` pass-through mode.
pub fn router(state: AppState, auth: AuthState) -> Router {
    Router::new()
        .merge(instances::router(state.clone()))
        .merge(snapshots::router(state.clone()))
        .merge(secrets::router(state))
        .layer(middleware::from_fn_with_state(auth, admin_bearer))
}
