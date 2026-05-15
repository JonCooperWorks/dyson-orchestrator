//! Admin endpoint for manually re-pushing the DB-backed egress policy map.

use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router, extract::State};
use serde::Serialize;

use crate::http::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/admin/egress/resync", post(resync))
        .with_state(state)
}

#[derive(Serialize)]
struct ResyncResponse {
    ok: bool,
}

async fn resync(State(state): State<AppState>) -> Result<Json<ResyncResponse>, StatusCode> {
    state
        .egress_sync
        .refresh()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(ResyncResponse { ok: true }))
}
