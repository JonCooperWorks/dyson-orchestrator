//! Admin API for KMS secret access audit events.

use std::collections::HashMap;

use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::http::AppState;
use crate::traits::{SecretAccessAuditEntry, SecretAccessAuditFilter};

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/admin/kms/audit", get(list))
        .with_state(state)
}

#[derive(Debug, Serialize)]
struct KmsAuditResponse {
    items: Vec<KmsAuditRow>,
    limit: u32,
    offset: u32,
    next_offset: Option<u32>,
}

#[derive(Debug, Serialize)]
struct KmsAuditRow {
    timestamp: i64,
    actor_kind: String,
    actor_id: Option<String>,
    reason: String,
    operation: String,
    scope: String,
    owner_id: Option<String>,
    instance_id: Option<String>,
    secret_name: Option<String>,
    key_id: Option<String>,
    key_version: Option<u32>,
    result: String,
    error_class: Option<String>,
    error_message: Option<String>,
}

async fn list(
    State(state): State<AppState>,
    uri: Uri,
) -> Result<Json<KmsAuditResponse>, StatusCode> {
    let params = parse_query(uri.query().unwrap_or(""));
    let limit = parse_u32(&params, "limit")
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT);
    let offset = parse_u32(&params, "offset").unwrap_or(0);
    let filter = SecretAccessAuditFilter {
        scope: params
            .get("scope")
            .and_then(|s| crate::envelope::KmsScope::parse(s)),
        owner_id: params.get("owner_id").filter(|s| !s.is_empty()).cloned(),
        instance_id: params.get("instance_id").filter(|s| !s.is_empty()).cloned(),
        secret_name: params.get("secret_name").filter(|s| !s.is_empty()).cloned(),
        operation: params
            .get("operation")
            .and_then(|s| crate::envelope::SecretAccessOperation::parse(s)),
        result: params
            .get("result")
            .and_then(|s| crate::envelope::SecretAccessResult::parse(s)),
        reason: params
            .get("reason")
            .and_then(|s| crate::envelope::SecretAccessReason::parse(s)),
        since: parse_i64(&params, "since"),
        until: parse_i64(&params, "until"),
        limit,
        offset,
    };
    let page = state
        .secret_access_audit
        .list(filter)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(KmsAuditResponse {
        items: page.items.into_iter().map(KmsAuditRow::from).collect(),
        limit,
        offset,
        next_offset: page.next_offset,
    }))
}

fn parse_u32(params: &HashMap<String, String>, name: &str) -> Option<u32> {
    params.get(name).and_then(|s| s.parse().ok())
}

fn parse_i64(params: &HashMap<String, String>, name: &str) -> Option<i64> {
    params.get(name).and_then(|s| s.parse().ok())
}

fn parse_query(s: &str) -> HashMap<String, String> {
    s.split('&')
        .filter(|p| !p.is_empty())
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.to_owned(), v.to_owned()))
        })
        .collect()
}

impl From<SecretAccessAuditEntry> for KmsAuditRow {
    fn from(entry: SecretAccessAuditEntry) -> Self {
        Self {
            timestamp: entry.timestamp,
            actor_kind: entry.actor_kind,
            actor_id: entry.actor_id,
            reason: entry.reason.as_str().to_owned(),
            operation: entry.operation.as_str().to_owned(),
            scope: entry.scope.as_str().to_owned(),
            owner_id: entry.owner_id,
            instance_id: entry.instance_id,
            secret_name: entry.secret_name,
            key_id: entry.key_id,
            key_version: entry.key_version,
            result: entry.result.as_str().to_owned(),
            error_class: entry.error_class,
            error_message: entry.error_message,
        }
    }
}
