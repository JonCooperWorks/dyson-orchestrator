//! Skill marketplace catalog routes.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};

use crate::http::AppState;
use crate::skill_marketplace::SkillMarketplaceError;

const STATE_TOKEN_PREFIX: &str = "st_";

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/skill-marketplaces", get(list_sources))
        .route("/v1/skill-marketplaces/skills", get(list_skills))
        .route(
            "/v1/skill-marketplaces/:marketplace/skills/:skill",
            get(skill_detail),
        )
        .route(
            "/v1/skill-marketplaces/:marketplace/skills/:skill/content",
            get(skill_content),
        )
        .with_state(state)
}

pub fn internal_router(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/internal/skill-marketplaces",
            get(internal_list_sources),
        )
        .route(
            "/v1/internal/skill-marketplaces/skills",
            get(internal_list_skills),
        )
        .route(
            "/v1/internal/skill-marketplaces/:marketplace/skills/:skill",
            get(internal_skill_detail),
        )
        .route(
            "/v1/internal/skill-marketplaces/:marketplace/skills/:skill/content",
            get(internal_skill_content),
        )
        .with_state(state)
}

async fn list_sources(State(state): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "sources": state.skill_marketplace.source_views(),
    }))
}

async fn list_skills(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.skill_marketplace.catalog().await)
}

async fn skill_detail(
    State(state): State<AppState>,
    Path((marketplace, skill)): Path<(String, String)>,
) -> impl IntoResponse {
    json_result(state.skill_marketplace.detail(&marketplace, &skill).await)
}

async fn skill_content(
    State(state): State<AppState>,
    Path((marketplace, skill)): Path<(String, String)>,
) -> impl IntoResponse {
    json_result(state.skill_marketplace.content(&marketplace, &skill).await)
}

async fn internal_list_sources(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    match authorize_state_token(&state, &headers).await {
        Ok(()) => Json(serde_json::json!({
            "sources": state.skill_marketplace.source_views(),
        }))
        .into_response(),
        Err(status) => status.into_response(),
    }
}

async fn internal_list_skills(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    match authorize_state_token(&state, &headers).await {
        Ok(()) => Json(state.skill_marketplace.catalog().await).into_response(),
        Err(status) => status.into_response(),
    }
}

async fn internal_skill_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((marketplace, skill)): Path<(String, String)>,
) -> impl IntoResponse {
    match authorize_state_token(&state, &headers).await {
        Ok(()) => json_result(state.skill_marketplace.detail(&marketplace, &skill).await),
        Err(status) => status.into_response(),
    }
}

async fn internal_skill_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((marketplace, skill)): Path<(String, String)>,
) -> impl IntoResponse {
    match authorize_state_token(&state, &headers).await {
        Ok(()) => json_result(state.skill_marketplace.content(&marketplace, &skill).await),
        Err(status) => status.into_response(),
    }
}

async fn authorize_state_token(state: &AppState, headers: &HeaderMap) -> Result<(), StatusCode> {
    let bearer = match extract_bearer(headers) {
        Some(b) if b.starts_with(STATE_TOKEN_PREFIX) => b.to_owned(),
        _ => return Err(StatusCode::UNAUTHORIZED),
    };
    let token_record = match state.tokens.resolve(&bearer).await {
        Ok(Some(r)) => r,
        Ok(None) => return Err(StatusCode::UNAUTHORIZED),
        Err(e) => {
            tracing::warn!(error = %e, "skill marketplace: token resolve failed");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    if token_record.provider != crate::db::tokens::STATE_SYNC_PROVIDER {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .trim();
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn json_result<T: serde::Serialize>(
    result: Result<T, SkillMarketplaceError>,
) -> axum::response::Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(err) => (
            status_for_error(&err),
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

fn status_for_error(err: &SkillMarketplaceError) -> StatusCode {
    match err {
        SkillMarketplaceError::MarketplaceNotFound(_)
        | SkillMarketplaceError::SkillNotFound { .. } => StatusCode::NOT_FOUND,
        SkillMarketplaceError::Invalid(_) => StatusCode::BAD_REQUEST,
        SkillMarketplaceError::Io(_) | SkillMarketplaceError::Http(_) => StatusCode::BAD_GATEWAY,
    }
}
