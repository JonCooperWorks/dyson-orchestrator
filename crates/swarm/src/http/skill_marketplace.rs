//! Skill marketplace catalog routes.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;

use crate::http::AppState;
use crate::skill_marketplace::{SkillMarketplaceError, SkillMarketplaceSourceConfig};

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

pub fn admin_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/admin/skill-marketplaces", get(admin_list_sources))
        .route(
            "/v1/admin/skill-marketplaces/:marketplace",
            put(admin_put_source).delete(admin_delete_source),
        )
        .with_state(state)
}

async fn list_sources(State(state): State<AppState>) -> impl IntoResponse {
    match state.skill_marketplace.source_views().await {
        Ok(sources) => Json(serde_json::json!({ "sources": sources })).into_response(),
        Err(err) => error_response(err),
    }
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
        Ok(()) => match state.skill_marketplace.source_views().await {
            Ok(sources) => Json(serde_json::json!({ "sources": sources })).into_response(),
            Err(err) => error_response(err),
        },
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

#[derive(Deserialize)]
struct AdminPutSkillMarketplaceSourceBody {
    source_type: String,
    location: String,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

fn default_enabled() -> bool {
    true
}

async fn admin_list_sources(State(state): State<AppState>) -> impl IntoResponse {
    match state.skill_marketplace.admin_source_views().await {
        Ok(sources) => Json(serde_json::json!({ "sources": sources })).into_response(),
        Err(err) => error_response(err),
    }
}

async fn admin_put_source(
    State(state): State<AppState>,
    Path(marketplace): Path<String>,
    Json(body): Json<AdminPutSkillMarketplaceSourceBody>,
) -> impl IntoResponse {
    let enabled = body.enabled;
    let source = match source_from_admin_body(marketplace, body) {
        Ok(source) => source,
        Err(err) => return error_response(err),
    };
    match state.skill_marketplace.upsert_source(source, enabled).await {
        Ok(source) => Json(source).into_response(),
        Err(err) => error_response(err),
    }
}

async fn admin_delete_source(
    State(state): State<AppState>,
    Path(marketplace): Path<String>,
) -> impl IntoResponse {
    match state.skill_marketplace.delete_source(&marketplace).await {
        Ok(deleted) => Json(serde_json::json!({ "ok": true, "deleted": deleted })).into_response(),
        Err(err) => error_response(err),
    }
}

fn source_from_admin_body(
    id: String,
    body: AdminPutSkillMarketplaceSourceBody,
) -> Result<SkillMarketplaceSourceConfig, SkillMarketplaceError> {
    let source = match body.source_type.trim().to_ascii_lowercase().as_str() {
        "file" => SkillMarketplaceSourceConfig::File {
            id,
            path: body.location.into(),
        },
        "http" => SkillMarketplaceSourceConfig::Http {
            id,
            url: body.location,
        },
        other => {
            return Err(SkillMarketplaceError::Invalid(format!(
                "unsupported marketplace source_type {other:?}"
            )));
        }
    };
    crate::skill_marketplace::validate_marketplace_source_config(&source)?;
    Ok(source)
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
        Err(err) => error_response(err),
    }
}

fn error_response(err: SkillMarketplaceError) -> axum::response::Response {
    (
        status_for_error(&err),
        Json(serde_json::json!({ "error": err.to_string() })),
    )
        .into_response()
}

fn status_for_error(err: &SkillMarketplaceError) -> StatusCode {
    match err {
        SkillMarketplaceError::MarketplaceNotFound(_)
        | SkillMarketplaceError::SkillNotFound { .. } => StatusCode::NOT_FOUND,
        SkillMarketplaceError::Invalid(_) => StatusCode::BAD_REQUEST,
        SkillMarketplaceError::Io(_) | SkillMarketplaceError::Http(_) => StatusCode::BAD_GATEWAY,
        SkillMarketplaceError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
