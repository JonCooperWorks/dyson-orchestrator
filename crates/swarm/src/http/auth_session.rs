//! `POST /auth/session` — server-stamped HttpOnly cookie for Dyson subdomains.

use axum::extract::{Json, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Router};
use serde::Deserialize;

use crate::auth::user::{is_session_id, read_session_cookie};
use crate::auth::{
    CallerIdentity, SESSION_COOKIE_NAME, UserAuthState, extract_bearer, user_middleware,
};
use crate::traits::SessionRow;

use super::{AppState, store_err_to_status};

#[derive(Debug, Deserialize)]
struct SessionBody {
    #[serde(default)]
    expires_at: Option<i64>,
}

pub fn router(state: AppState, user_auth: UserAuthState) -> Router {
    Router::new()
        .route(
            "/auth/session",
            post(create)
                .layer(middleware::from_fn_with_state(user_auth, user_middleware))
                .delete(clear),
        )
        .with_state(state)
}

async fn create(
    State(state): State<AppState>,
    Extension(caller): Extension<CallerIdentity>,
    headers: HeaderMap,
    body: Option<Json<SessionBody>>,
) -> Response {
    if extract_bearer(&headers).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let now = crate::now_secs();
    let session_id = mint_session_id();
    let row = SessionRow {
        id: session_id.clone(),
        user_id: caller.user_id,
        created_at: now,
        last_seen_at: now,
        revoked_at: None,
    };
    if let Err(err) = state.sessions.insert(&row).await {
        tracing::warn!(error = %err, "session create failed");
        return store_err_to_status(err).into_response();
    }
    let mut resp = StatusCode::NO_CONTENT.into_response();
    let cookie = build_cookie(
        &state,
        &session_id,
        body.and_then(|Json(b)| b.expires_at),
        false,
    );
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        resp.headers_mut().insert(header::SET_COOKIE, value);
    }
    resp
}

async fn clear(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(session_id) = read_session_cookie(&headers)
        && is_session_id(&session_id)
        && let Err(err) = state.sessions.revoke(&session_id, crate::now_secs()).await
    {
        tracing::warn!(error = %err, "session revoke failed");
        return store_err_to_status(err).into_response();
    }
    let mut resp = StatusCode::NO_CONTENT.into_response();
    let cookie = build_cookie(&state, "", None, true);
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        resp.headers_mut().insert(header::SET_COOKIE, value);
    }
    resp
}

fn build_cookie(state: &AppState, token: &str, expires_at: Option<i64>, clear: bool) -> String {
    let mut parts = vec![
        format!("{SESSION_COOKIE_NAME}={token}"),
        "Path=/".to_owned(),
        "SameSite=Strict".to_owned(),
        "HttpOnly".to_owned(),
    ];
    if state.hostname.is_some() {
        parts.push("Secure".to_owned());
    }
    if let Some(domain) = cookie_domain(state.hostname.as_deref()) {
        parts.push(format!("Domain={domain}"));
    }
    if clear {
        parts.push("Max-Age=0".to_owned());
        parts.push("Expires=Thu, 01 Jan 1970 00:00:00 GMT".to_owned());
    } else if let Some(exp) = expires_at {
        let max_age = exp.saturating_sub(crate::now_secs()).max(0);
        parts.push(format!("Max-Age={max_age}"));
    }
    parts.join("; ")
}

fn mint_session_id() -> String {
    format!("ses_{}", uuid::Uuid::new_v4().simple())
}

fn cookie_domain(host: Option<&str>) -> Option<String> {
    let host = host?.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty()
        || !host.contains('.')
        || host.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    {
        return None;
    }
    Some(host)
}
