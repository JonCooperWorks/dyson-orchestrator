//! Internal state-file ingest — `POST /v1/internal/state/file`.
//!
//! Dyson calls this from its swarm-mode background worker to mirror
//! selected workspace files back to the parent swarm. Auth is a
//! per-instance `st_<32hex>` bearer minted by swarm and scoped through
//! the same token resolver as `/llm` and artefact ingest. The resolved
//! token's `instance_id` is authoritative; the request body cannot
//! write into another instance or another tenant's owner cipher.

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::Deserialize;

use crate::http::AppState;
use crate::state_files::StateFileMeta;

pub const MAX_STATE_BODY: usize = 25 * 1024 * 1024;

const STATE_TOKEN_PREFIX: &str = "st_";

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/internal/state/file", post(ingest_file))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct StateFileRequest {
    namespace: String,
    path: String,
    #[serde(default)]
    mime: Option<String>,
    /// Plaintext seconds-since-epoch from dyson's clock. Cosmetic for
    /// restore ordering; swarm's `synced_at` records receipt time.
    updated_at: i64,
    #[serde(default)]
    deleted: bool,
    #[serde(default)]
    body_b64: Option<String>,
}

async fn ingest_file(State(state): State<AppState>, req: Request<Body>) -> StatusCode {
    let bearer = match extract_bearer(&req) {
        Some(b) if b.starts_with(STATE_TOKEN_PREFIX) => b.to_owned(),
        _ => return StatusCode::UNAUTHORIZED,
    };
    let token_record = match state.tokens.resolve(&bearer).await {
        Ok(Some(r)) => r,
        Ok(None) => return StatusCode::UNAUTHORIZED,
        Err(e) => {
            tracing::warn!(error = %e, "state ingest: token resolve failed");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };
    if token_record.provider != crate::db::tokens::STATE_SYNC_PROVIDER {
        return StatusCode::UNAUTHORIZED;
    }
    let instance_id = token_record.instance_id;

    let instance = match state.instances.get_unscoped(&instance_id).await {
        Ok(r) => r,
        Err(crate::error::SwarmError::NotFound) => return StatusCode::NOT_FOUND,
        Err(e) => {
            tracing::warn!(
                error = %e,
                instance = %instance_id,
                "state ingest: instance lookup failed"
            );
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };

    let body_bytes = match axum::body::to_bytes(req.into_body(), MAX_STATE_BODY).await {
        Ok(b) => b,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE,
    };
    let body: StateFileRequest = match serde_json::from_slice(&body_bytes) {
        Ok(b) => b,
        Err(e) => {
            tracing::debug!(error = %e, "state ingest: malformed JSON envelope");
            return StatusCode::BAD_REQUEST;
        }
    };

    let meta = StateFileMeta {
        instance_id: &instance_id,
        owner_id: &instance.owner_id,
        namespace: &body.namespace,
        path: &body.path,
        mime: body.mime.as_deref(),
        updated_at: body.updated_at,
    };

    let result = if body.deleted {
        state.state_files.tombstone(meta).await
    } else {
        let encoded = match body.body_b64.as_deref() {
            Some(s) => s,
            None => return StatusCode::BAD_REQUEST,
        };
        let decoded = match B64.decode(encoded) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(error = %e, "state ingest: malformed base64 body");
                return StatusCode::BAD_REQUEST;
            }
        };
        state.state_files.ingest(meta, &decoded).await
    };

    match result {
        Ok(_) => StatusCode::NO_CONTENT,
        Err(crate::state_files::StateFileError::Invalid(e)) => {
            tracing::debug!(error = %e, "state ingest: rejected file");
            StatusCode::BAD_REQUEST
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                instance = %instance_id,
                namespace = %body.namespace,
                path = %body.path,
                "state ingest: write failed",
            );
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

fn extract_bearer(req: &Request<Body>) -> Option<&str> {
    let raw = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .trim();
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}
