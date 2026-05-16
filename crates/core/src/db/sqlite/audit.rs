//! Audit-row CRUD. Backs the [`AuditStore`] trait. The `subject` parameter
//! to `daily_tokens` is opaque — `instance_id` today, `owner_id` after
//! phase 6 — so the trait shape doesn't change when budgets become per-user.
//!
//! # Two-step write (D1)
//!
//! Streaming LLM calls are now logged in two phases:
//!   1. `insert` — runs before the upstream body is consumed; stamps
//!      `completed = false` and returns the row id.
//!   2. `update_completion` — runs after the upstream body has fully
//!      streamed; stamps `completed = true` and the final `output_tokens`.
//!
//! A crash mid-stream therefore leaves a forensic row marked
//! `completed = 0`, distinguishable from rows that finished cleanly.
//! Daily-token rollups sum both prompt and output regardless of the
//! `completed` flag — partial usage still counts toward the cap so a
//! crashing tenant can't run a token-exfil loop.

use async_trait::async_trait;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};

use crate::db::sqlite::map_sqlx;
use crate::envelope::{KmsScope, SecretAccessOperation, SecretAccessReason, SecretAccessResult};
use crate::error::StoreError;
use crate::traits::{
    AdminAuditEntry, AdminAuditStore, AuditEntry, AuditStore, LlmToolCallEntry, LlmToolCallFilters,
    LlmToolCallRow, LlmToolCallStatusFilter, LlmToolCallStore, McpAuditEntry, McpAuditStore,
    SecretAccessAuditEntry, SecretAccessAuditFilter, SecretAccessAuditPage, SecretAccessAuditStore,
};

#[derive(Debug, Clone)]
pub struct SqliteAuditStore {
    pool: SqlitePool,
}

impl SqliteAuditStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Pricing is intentionally not implemented. Kept as a well-typed
    /// entry point so a future pricing layer can land without re-plumbing
    /// every call site.
    #[allow(dead_code, clippy::unused_async)]
    pub async fn monthly_usd(&self, _owner_id: &str, _now: i64) -> Result<f64, StoreError> {
        Ok(0.0)
    }
}

#[derive(Debug, Clone)]
pub struct SqliteMcpAuditStore {
    pool: SqlitePool,
}

impl SqliteMcpAuditStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Clone)]
pub struct SqliteAdminAuditStore {
    pool: SqlitePool,
}

impl SqliteAdminAuditStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Clone)]
pub struct SqliteSecretAccessAuditStore {
    pool: SqlitePool,
}

impl SqliteSecretAccessAuditStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Clone)]
pub struct SqliteLlmToolCallStore {
    pool: SqlitePool,
}

impl SqliteLlmToolCallStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NoopMcpAuditStore;

#[async_trait]
impl AuditStore for SqliteAuditStore {
    async fn insert(&self, entry: &AuditEntry) -> Result<i64, StoreError> {
        // SQLite's `INTEGER PRIMARY KEY AUTOINCREMENT` exposes the
        // newly-assigned id via `last_insert_rowid()`; we round-trip
        // it through a single `RETURNING id` for portability.
        let row = sqlx::query(
            "INSERT INTO llm_audit \
             (owner_id, instance_id, provider, model, prompt_tokens, output_tokens, status_code, duration_ms, occurred_at, key_source, completed) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING id",
        )
        .bind(&entry.owner_id)
        .bind(&entry.instance_id)
        .bind(&entry.provider)
        .bind(&entry.model)
        .bind(entry.prompt_tokens)
        .bind(entry.output_tokens)
        .bind(entry.status_code)
        .bind(entry.duration_ms)
        .bind(entry.occurred_at)
        .bind(&entry.key_source)
        .bind(i64::from(entry.completed))
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let id: i64 = row.try_get("id").map_err(map_sqlx)?;
        Ok(id)
    }

    /// Sums tokens *per-owner* over the past 24h. Per-user budgets hold
    /// across all of a tenant's instances. Both prompt and output
    /// tokens count — usage from a streamed-but-incomplete row still
    /// pushes toward the cap.
    async fn daily_tokens(&self, owner_id: &str, now: i64) -> Result<u64, StoreError> {
        let since = now - 86_400;
        let row = sqlx::query(
            "SELECT COALESCE(SUM(COALESCE(prompt_tokens,0) + COALESCE(output_tokens,0)), 0) AS total \
             FROM llm_audit WHERE owner_id = ? AND occurred_at >= ?",
        )
        .bind(owner_id)
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let total: i64 = row.try_get("total").map_err(map_sqlx)?;
        Ok(u64::try_from(total.max(0)).unwrap_or(0))
    }

    async fn update_completion(
        &self,
        audit_id: i64,
        output_tokens: Option<i64>,
    ) -> Result<(), StoreError> {
        // Idempotent: a re-stamp matches the same row and writes the
        // same values.  No `revoked_at IS NULL`-style guard because
        // the row is keyed on its primary id and there's no harm in
        // overwriting with a more accurate token count if the proxy
        // happens to call us twice.
        sqlx::query("UPDATE llm_audit SET output_tokens = ?, completed = 1 WHERE id = ?")
            .bind(output_tokens)
            .bind(audit_id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }
}

#[async_trait]
impl McpAuditStore for SqliteMcpAuditStore {
    async fn insert(&self, entry: &McpAuditEntry) -> Result<i64, StoreError> {
        let row = sqlx::query(
            "INSERT INTO mcp_audit \
             (owner_id, instance_id, server_name, tool, status, duration_ms, ts, completed) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING id",
        )
        .bind(&entry.owner_id)
        .bind(&entry.instance_id)
        .bind(&entry.server_name)
        .bind(&entry.tool)
        .bind(entry.status)
        .bind(entry.duration_ms)
        .bind(entry.ts)
        .bind(i64::from(entry.completed))
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let id: i64 = row.try_get("id").map_err(map_sqlx)?;
        Ok(id)
    }

    async fn update_status(
        &self,
        audit_id: i64,
        status: i64,
        duration_ms: i64,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE mcp_audit SET status = ?, duration_ms = ?, completed = 1 WHERE id = ?")
            .bind(status)
            .bind(duration_ms)
            .bind(audit_id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }
}

#[async_trait]
impl McpAuditStore for NoopMcpAuditStore {
    async fn insert(&self, _entry: &McpAuditEntry) -> Result<i64, StoreError> {
        Ok(0)
    }

    async fn update_status(
        &self,
        _audit_id: i64,
        _status: i64,
        _duration_ms: i64,
    ) -> Result<(), StoreError> {
        Ok(())
    }
}

#[async_trait]
impl SecretAccessAuditStore for SqliteSecretAccessAuditStore {
    async fn insert(&self, entry: &SecretAccessAuditEntry) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO secret_access_audit \
             (timestamp, actor_kind, actor_id, reason, operation, scope, owner_id, instance_id, secret_name, \
              key_id, key_version, result, error_class, error_message) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(entry.timestamp)
        .bind(&entry.actor_kind)
        .bind(&entry.actor_id)
        .bind(entry.reason.as_str())
        .bind(entry.operation.as_str())
        .bind(entry.scope.as_str())
        .bind(&entry.owner_id)
        .bind(&entry.instance_id)
        .bind(&entry.secret_name)
        .bind(&entry.key_id)
        .bind(entry.key_version.map(i64::from))
        .bind(entry.result.as_str())
        .bind(&entry.error_class)
        .bind(&entry.error_message)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn list(
        &self,
        filter: SecretAccessAuditFilter,
    ) -> Result<SecretAccessAuditPage, StoreError> {
        let limit = filter.limit.clamp(1, 500);
        let fetch_limit = limit + 1;
        let mut q: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
            "SELECT timestamp, actor_kind, actor_id, reason, operation, scope, owner_id, instance_id, \
                    secret_name, key_id, key_version, result, error_class, error_message \
             FROM secret_access_audit WHERE 1 = 1",
        );
        if let Some(scope) = filter.scope {
            q.push(" AND scope = ");
            q.push_bind(scope.as_str());
        }
        if let Some(owner_id) = filter.owner_id.as_deref().filter(|s| !s.is_empty()) {
            q.push(" AND owner_id = ");
            q.push_bind(owner_id);
        }
        if let Some(instance_id) = filter.instance_id.as_deref().filter(|s| !s.is_empty()) {
            q.push(" AND instance_id = ");
            q.push_bind(instance_id);
        }
        if let Some(secret_name) = filter.secret_name.as_deref().filter(|s| !s.is_empty()) {
            q.push(" AND secret_name = ");
            q.push_bind(secret_name);
        }
        if let Some(operation) = filter.operation {
            q.push(" AND operation = ");
            q.push_bind(operation.as_str());
        }
        if let Some(result) = filter.result {
            q.push(" AND result = ");
            q.push_bind(result.as_str());
        }
        if let Some(reason) = filter.reason {
            q.push(" AND reason = ");
            q.push_bind(reason.as_str());
        }
        if let Some(since) = filter.since {
            q.push(" AND timestamp >= ");
            q.push_bind(since);
        }
        if let Some(until) = filter.until {
            q.push(" AND timestamp <= ");
            q.push_bind(until);
        }
        q.push(" ORDER BY timestamp DESC LIMIT ");
        q.push_bind(i64::from(fetch_limit));
        q.push(" OFFSET ");
        q.push_bind(i64::from(filter.offset));

        let rows = q.build().fetch_all(&self.pool).await.map_err(map_sqlx)?;
        let has_next = rows.len() > limit as usize;
        let items = rows
            .into_iter()
            .take(limit as usize)
            .map(row_to_secret_access)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SecretAccessAuditPage {
            items,
            next_offset: has_next.then_some(filter.offset + limit),
        })
    }
}

#[async_trait]
impl LlmToolCallStore for SqliteLlmToolCallStore {
    async fn insert_call(&self, entry: &LlmToolCallEntry) -> Result<i64, StoreError> {
        let row = sqlx::query(
            "INSERT INTO llm_tool_call \
             (llm_audit_id, owner_id, instance_id, tool_use_id, tool_name, mcp_server, input_sealed, called_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING id",
        )
        .bind(entry.llm_audit_id)
        .bind(&entry.owner_id)
        .bind(&entry.instance_id)
        .bind(&entry.tool_use_id)
        .bind(&entry.tool_name)
        .bind(&entry.mcp_server)
        .bind(&entry.input_sealed)
        .bind(entry.called_at)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let id: i64 = row.try_get("id").map_err(map_sqlx)?;
        Ok(id)
    }

    async fn attach_result(
        &self,
        tool_use_id: &str,
        result_sealed: &[u8],
        is_error: bool,
        resulted_at: i64,
    ) -> Result<bool, StoreError> {
        let result = sqlx::query(
            "UPDATE llm_tool_call \
             SET result_sealed = ?, is_error = ?, resulted_at = ? \
             WHERE id = ( \
               SELECT id FROM llm_tool_call \
               WHERE tool_use_id = ? AND result_sealed IS NULL \
               ORDER BY id DESC LIMIT 1 \
             )",
        )
        .bind(result_sealed)
        .bind(i64::from(is_error))
        .bind(resulted_at)
        .bind(tool_use_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(result.rows_affected() > 0)
    }

    async fn list(
        &self,
        owner_id: &str,
        instance_id: &str,
        filters: LlmToolCallFilters<'_>,
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<LlmToolCallRow>, StoreError> {
        let mut q = tool_call_select_builder();
        q.push(" WHERE c.owner_id = ");
        q.push_bind(owner_id);
        q.push(" AND c.instance_id = ");
        q.push_bind(instance_id);
        append_tool_call_filters(&mut q, filters, before);
        q.push(" ORDER BY c.id DESC LIMIT ");
        q.push_bind(i64::from(limit));
        let rows = q.build().fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(row_to_tool_call).collect()
    }

    async fn stream_after(
        &self,
        owner_id: &str,
        instance_id: &str,
        cursor_id: i64,
    ) -> Result<Vec<LlmToolCallRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT c.id, c.llm_audit_id, c.owner_id, c.instance_id, c.tool_use_id, c.tool_name, \
                    c.mcp_server, c.input_sealed, c.result_sealed, c.is_error, c.called_at, \
                    c.resulted_at, c.mcp_audit_id, m.status AS mcp_status, m.duration_ms AS mcp_duration_ms \
             FROM llm_tool_call c \
             LEFT JOIN mcp_audit m ON m.id = c.mcp_audit_id \
             WHERE c.owner_id = ? AND c.instance_id = ? AND c.id > ? \
             ORDER BY c.id ASC LIMIT 500",
        )
        .bind(owner_id)
        .bind(instance_id)
        .bind(cursor_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.iter().map(row_to_tool_call).collect()
    }

    async fn link_mcp_audit(
        &self,
        tool_call_id: i64,
        owner_id: &str,
        instance_id: &str,
        server_name: &str,
        tool_name: &str,
        called_at: i64,
    ) -> Result<bool, StoreError> {
        let result = sqlx::query(
            "UPDATE llm_tool_call \
             SET mcp_audit_id = ( \
               SELECT id FROM mcp_audit \
               WHERE owner_id = ? AND instance_id = ? AND server_name = ? AND tool = ? \
               ORDER BY ABS(ts - ?) ASC, id DESC LIMIT 1 \
             ) \
             WHERE id = ? AND mcp_audit_id IS NULL \
               AND EXISTS ( \
                 SELECT 1 FROM mcp_audit \
                 WHERE owner_id = ? AND instance_id = ? AND server_name = ? AND tool = ? \
               )",
        )
        .bind(owner_id)
        .bind(instance_id)
        .bind(server_name)
        .bind(tool_name)
        .bind(called_at)
        .bind(tool_call_id)
        .bind(owner_id)
        .bind(instance_id)
        .bind(server_name)
        .bind(tool_name)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(result.rows_affected() > 0)
    }

    async fn rewrap_input(
        &self,
        id: i64,
        previous_input_sealed: &[u8],
        input_sealed: &[u8],
    ) -> Result<bool, StoreError> {
        let result = sqlx::query(
            "UPDATE llm_tool_call \
             SET input_sealed = ? \
             WHERE id = ? AND input_sealed = ?",
        )
        .bind(input_sealed)
        .bind(id)
        .bind(previous_input_sealed)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(result.rows_affected() > 0)
    }

    async fn rewrap_result(
        &self,
        id: i64,
        previous_result_sealed: &[u8],
        result_sealed: &[u8],
    ) -> Result<bool, StoreError> {
        let result = sqlx::query(
            "UPDATE llm_tool_call \
             SET result_sealed = ? \
             WHERE id = ? AND result_sealed = ?",
        )
        .bind(result_sealed)
        .bind(id)
        .bind(previous_result_sealed)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(result.rows_affected() > 0)
    }
}

fn tool_call_select_builder<'a>() -> QueryBuilder<'a, Sqlite> {
    QueryBuilder::new(
        "SELECT c.id, c.llm_audit_id, c.owner_id, c.instance_id, c.tool_use_id, c.tool_name, \
                c.mcp_server, c.input_sealed, c.result_sealed, c.is_error, c.called_at, \
                c.resulted_at, c.mcp_audit_id, m.status AS mcp_status, m.duration_ms AS mcp_duration_ms \
         FROM llm_tool_call c \
         LEFT JOIN mcp_audit m ON m.id = c.mcp_audit_id",
    )
}

fn append_tool_call_filters<'a>(
    q: &mut QueryBuilder<'a, Sqlite>,
    filters: LlmToolCallFilters<'a>,
    before: Option<i64>,
) {
    if let Some(tool) = filters.tool.filter(|s| !s.is_empty()) {
        q.push(" AND c.tool_name = ");
        q.push_bind(tool);
    }
    match filters.status {
        LlmToolCallStatusFilter::All => {}
        LlmToolCallStatusFilter::Ok => {
            q.push(" AND c.is_error = 0");
        }
        LlmToolCallStatusFilter::Err => {
            q.push(" AND c.is_error = 1");
        }
    }
    if let Some(server) = filters.server.filter(|s| !s.is_empty()) {
        q.push(" AND c.mcp_server = ");
        q.push_bind(server);
    }
    if let Some(before) = before {
        q.push(" AND c.id < ");
        q.push_bind(before);
    }
}

fn row_to_tool_call(row: &sqlx::sqlite::SqliteRow) -> Result<LlmToolCallRow, StoreError> {
    let is_error: Option<i64> = row.try_get("is_error").map_err(map_sqlx)?;
    Ok(LlmToolCallRow {
        id: row.try_get("id").map_err(map_sqlx)?,
        llm_audit_id: row.try_get("llm_audit_id").map_err(map_sqlx)?,
        owner_id: row.try_get("owner_id").map_err(map_sqlx)?,
        instance_id: row.try_get("instance_id").map_err(map_sqlx)?,
        tool_use_id: row.try_get("tool_use_id").map_err(map_sqlx)?,
        tool_name: row.try_get("tool_name").map_err(map_sqlx)?,
        mcp_server: row.try_get("mcp_server").map_err(map_sqlx)?,
        input_sealed: row.try_get("input_sealed").map_err(map_sqlx)?,
        result_sealed: row.try_get("result_sealed").map_err(map_sqlx)?,
        is_error: is_error.map(|v| v != 0),
        called_at: row.try_get("called_at").map_err(map_sqlx)?,
        resulted_at: row.try_get("resulted_at").map_err(map_sqlx)?,
        mcp_audit_id: row.try_get("mcp_audit_id").map_err(map_sqlx)?,
        mcp_status: row.try_get("mcp_status").map_err(map_sqlx)?,
        mcp_duration_ms: row.try_get("mcp_duration_ms").map_err(map_sqlx)?,
    })
}

fn row_to_secret_access(
    row: sqlx::sqlite::SqliteRow,
) -> Result<SecretAccessAuditEntry, StoreError> {
    let reason: String = row.try_get("reason").map_err(map_sqlx)?;
    let operation: String = row.try_get("operation").map_err(map_sqlx)?;
    let scope: String = row.try_get("scope").map_err(map_sqlx)?;
    let result: String = row.try_get("result").map_err(map_sqlx)?;
    let key_version: Option<i64> = row.try_get("key_version").map_err(map_sqlx)?;
    Ok(SecretAccessAuditEntry {
        timestamp: row.try_get("timestamp").map_err(map_sqlx)?,
        actor_kind: row.try_get("actor_kind").map_err(map_sqlx)?,
        actor_id: row.try_get("actor_id").map_err(map_sqlx)?,
        reason: SecretAccessReason::parse(&reason)
            .ok_or_else(|| StoreError::Malformed(format!("unknown kms audit reason {reason:?}")))?,
        operation: SecretAccessOperation::parse(&operation).ok_or_else(|| {
            StoreError::Malformed(format!("unknown kms audit operation {operation:?}"))
        })?,
        scope: KmsScope::parse(&scope)
            .ok_or_else(|| StoreError::Malformed(format!("unknown kms audit scope {scope:?}")))?,
        owner_id: row.try_get("owner_id").map_err(map_sqlx)?,
        instance_id: row.try_get("instance_id").map_err(map_sqlx)?,
        secret_name: row.try_get("secret_name").map_err(map_sqlx)?,
        key_id: row.try_get("key_id").map_err(map_sqlx)?,
        key_version: key_version.and_then(|v| u32::try_from(v).ok()),
        result: SecretAccessResult::parse(&result)
            .ok_or_else(|| StoreError::Malformed(format!("unknown kms audit result {result:?}")))?,
        error_class: row.try_get("error_class").map_err(map_sqlx)?,
        error_message: row.try_get("error_message").map_err(map_sqlx)?,
    })
}

#[async_trait]
impl AdminAuditStore for SqliteAdminAuditStore {
    async fn insert(&self, entry: &AdminAuditEntry) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO admin_audit \
             (actor_subject, action, target_user, params_hash, ts) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&entry.actor_subject)
        .bind(&entry.action)
        .bind(&entry.target_user)
        .bind(&entry.params_hash)
        .bind(entry.ts)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sqlite::open_in_memory;
    use crate::network_policy::NetworkPolicy;
    use crate::traits::{InstanceRow, InstanceStatus, InstanceStore};

    fn r(owner: &str, instance: &str, when: i64, prompt: i64, output: i64) -> AuditEntry {
        AuditEntry {
            owner_id: owner.into(),
            instance_id: instance.into(),
            provider: "openai".into(),
            model: Some("gpt-4o".into()),
            prompt_tokens: Some(prompt),
            output_tokens: Some(output),
            status_code: 200,
            duration_ms: 100,
            occurred_at: when,
            key_source: "platform".into(),
            completed: true,
        }
    }

    fn tool_call(
        owner: &str,
        instance: &str,
        use_id: &str,
        tool: &str,
        server: Option<&str>,
        called_at: i64,
    ) -> LlmToolCallEntry {
        LlmToolCallEntry {
            llm_audit_id: None,
            owner_id: owner.into(),
            instance_id: instance.into(),
            tool_use_id: use_id.into(),
            tool_name: tool.into(),
            mcp_server: server.map(str::to_owned),
            input_sealed: Some(format!(r#"{{"tool":"{tool}"}}"#).into_bytes()),
            called_at,
        }
    }

    async fn seed_owner_instance(pool: &SqlitePool, owner: &str, instance: &str) {
        sqlx::query(
            "INSERT INTO users (id, subject, status, created_at, activated_at) \
             VALUES (?, ?, 'active', 0, 0)",
        )
        .bind(owner)
        .bind(format!("subject-{owner}"))
        .execute(pool)
        .await
        .unwrap();
        let instances = crate::db::sqlite::instances::SqlxInstanceStore::new(
            pool.clone(),
            crate::db::sqlite::test_system_cipher(),
        );
        instances
            .create(InstanceRow {
                id: instance.into(),
                owner_id: owner.into(),
                name: "agent".into(),
                task: "task".into(),
                cube_sandbox_id: Some("cube-1".into()),
                state_generation: "gen-1".into(),
                template_id: "tmpl".into(),
                status: InstanceStatus::Live,
                bearer_token: "bearer".into(),
                pinned: false,
                expires_at: None,
                last_active_at: 0,
                last_probe_at: None,
                last_probe_status: None,
                created_at: 0,
                destroyed_at: None,
                rotated_to: None,
                network_policy: NetworkPolicy::NoLocalNet,
                network_policy_cidrs: Vec::new(),
                models: Vec::new(),
                tools: Vec::new(),
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn daily_tokens_sums_window_per_owner() {
        let pool = open_in_memory().await.unwrap();
        let store = SqliteAuditStore::new(pool);
        let now = 1_000_000;
        // Owner u1, two different instances — both should count.
        store
            .insert(&r("u1", "i-a", now - 100, 100, 50))
            .await
            .unwrap();
        store
            .insert(&r("u1", "i-b", now - 1000, 200, 100))
            .await
            .unwrap();
        // Outside window.
        store
            .insert(&r("u1", "i-a", now - 86_500, 999, 999))
            .await
            .unwrap();
        // Different owner — not counted.
        store
            .insert(&r("u2", "i-c", now - 100, 9999, 9999))
            .await
            .unwrap();

        let total = store.daily_tokens("u1", now).await.unwrap();
        assert_eq!(total, 100 + 50 + 200 + 100);
    }

    #[tokio::test]
    async fn daily_tokens_handles_null_columns() {
        let pool = open_in_memory().await.unwrap();
        let store = SqliteAuditStore::new(pool);
        let now = 1_000_000;
        let mut row = r("u1", "i1", now - 10, 50, 25);
        row.prompt_tokens = None;
        store.insert(&row).await.unwrap();
        assert_eq!(store.daily_tokens("u1", now).await.unwrap(), 25);
    }

    #[tokio::test]
    async fn daily_tokens_zero_on_no_rows() {
        let pool = open_in_memory().await.unwrap();
        let store = SqliteAuditStore::new(pool);
        assert_eq!(store.daily_tokens("i1", 1).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn daily_tokens_sums_prompt_and_output_columns() {
        // Regression guard: the budget rollup must include *both*
        // `prompt_tokens` and `output_tokens`.  Earlier shape only
        // summed prompt; if the proxy ever logs a row with prompt=0
        // and output>0 the cap must still tick over.
        let pool = open_in_memory().await.unwrap();
        let store = SqliteAuditStore::new(pool);
        let now = 1_000_000;
        // Only-output and only-prompt rows both contribute.
        let mut only_output = r("u1", "i1", now - 10, 0, 700);
        only_output.prompt_tokens = None;
        store.insert(&only_output).await.unwrap();
        let mut only_prompt = r("u1", "i1", now - 20, 300, 0);
        only_prompt.output_tokens = None;
        store.insert(&only_prompt).await.unwrap();

        assert_eq!(store.daily_tokens("u1", now).await.unwrap(), 700 + 300);
    }

    #[tokio::test]
    async fn update_completion_stamps_tokens_and_completed() {
        let pool = open_in_memory().await.unwrap();
        let store = SqliteAuditStore::new(pool.clone());
        let now = 1_000_000;
        // Insert as completed = false, output_tokens = None — the
        // proxy's up-front shape.
        let mut entry = r("u1", "i1", now - 5, 100, 0);
        entry.completed = false;
        entry.output_tokens = None;
        let id = store.insert(&entry).await.unwrap();

        // Pre-update: completed = 0, output_tokens IS NULL.
        let row = sqlx::query("SELECT completed, output_tokens FROM llm_audit WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let completed: i64 = row.try_get("completed").unwrap();
        let output: Option<i64> = row.try_get("output_tokens").unwrap();
        assert_eq!(completed, 0);
        assert!(output.is_none());

        // Stamp completion.
        store.update_completion(id, Some(450)).await.unwrap();

        let row = sqlx::query("SELECT completed, output_tokens FROM llm_audit WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let completed: i64 = row.try_get("completed").unwrap();
        let output: Option<i64> = row.try_get("output_tokens").unwrap();
        assert_eq!(completed, 1);
        assert_eq!(output, Some(450));

        // Daily-tokens now sees the updated output count.
        // prompt(100) + output(450) = 550.
        assert_eq!(store.daily_tokens("u1", now).await.unwrap(), 550);
    }

    #[tokio::test]
    async fn update_completion_idempotent() {
        // Re-stamping is harmless — same row, same values.
        let pool = open_in_memory().await.unwrap();
        let store = SqliteAuditStore::new(pool);
        let now = 1_000_000;
        let mut entry = r("u1", "i1", now - 5, 50, 0);
        entry.completed = false;
        entry.output_tokens = None;
        let id = store.insert(&entry).await.unwrap();
        store.update_completion(id, Some(10)).await.unwrap();
        store.update_completion(id, Some(10)).await.unwrap();
        // No panic, no state divergence.
        assert_eq!(store.daily_tokens("u1", now).await.unwrap(), 50 + 10);
    }

    #[tokio::test]
    async fn monthly_usd_is_zero_pricing_disabled() {
        // Pricing is intentionally not implemented (demo deployment);
        // the entry point exists so a future pricing layer can land
        // without re-plumbing call sites.
        let pool = open_in_memory().await.unwrap();
        let store = SqliteAuditStore::new(pool);
        // float exact-compare is intentional: this is a hard-coded 0.0
        // and we want to catch any accidental change to the no-op return.
        let usd = store.monthly_usd("u1", 0).await.unwrap();
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(usd, 0.0);
        }
    }

    #[tokio::test]
    async fn tool_call_insert_and_list_round_trip() {
        let pool = open_in_memory().await.unwrap();
        seed_owner_instance(&pool, "owner-a", "inst-a").await;
        let store = SqliteLlmToolCallStore::new(pool);

        let id = store
            .insert_call(&tool_call("owner-a", "inst-a", "use-1", "bash", None, 100))
            .await
            .unwrap();

        let rows = store
            .list(
                "owner-a",
                "inst-a",
                LlmToolCallFilters::default(),
                None,
                100,
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].tool_use_id, "use-1");
        assert_eq!(rows[0].tool_name, "bash");
        assert_eq!(
            rows[0].input_sealed.as_deref(),
            Some(br#"{"tool":"bash"}"#.as_slice())
        );
        assert!(rows[0].result_sealed.is_none());
    }

    #[tokio::test]
    async fn tool_call_attach_result_pairs_once() {
        let pool = open_in_memory().await.unwrap();
        seed_owner_instance(&pool, "owner-a", "inst-a").await;
        let store = SqliteLlmToolCallStore::new(pool);
        store
            .insert_call(&tool_call("owner-a", "inst-a", "use-1", "bash", None, 100))
            .await
            .unwrap();

        assert!(
            store
                .attach_result("use-1", br#"{"ok":false}"#, true, 125)
                .await
                .unwrap()
        );
        assert!(
            !store
                .attach_result("use-1", br#"{"ok":true}"#, false, 130)
                .await
                .unwrap()
        );

        let rows = store
            .list(
                "owner-a",
                "inst-a",
                LlmToolCallFilters::default(),
                None,
                100,
            )
            .await
            .unwrap();
        assert_eq!(rows[0].is_error, Some(true));
        assert_eq!(rows[0].resulted_at, Some(125));
        assert_eq!(
            rows[0].result_sealed.as_deref(),
            Some(br#"{"ok":false}"#.as_slice())
        );
    }

    #[tokio::test]
    async fn tool_call_list_filters_and_paginates() {
        let pool = open_in_memory().await.unwrap();
        seed_owner_instance(&pool, "owner-a", "inst-a").await;
        let store = SqliteLlmToolCallStore::new(pool);

        let bash = store
            .insert_call(&tool_call(
                "owner-a", "inst-a", "use-bash", "bash", None, 10,
            ))
            .await
            .unwrap();
        let gh_ok = store
            .insert_call(&tool_call(
                "owner-a",
                "inst-a",
                "use-gh-ok",
                "mcp__github__create_issue",
                Some("github"),
                20,
            ))
            .await
            .unwrap();
        let gh_err = store
            .insert_call(&tool_call(
                "owner-a",
                "inst-a",
                "use-gh-err",
                "mcp__github__close_issue",
                Some("github"),
                30,
            ))
            .await
            .unwrap();
        let _native_err = store
            .insert_call(&tool_call(
                "owner-a",
                "inst-a",
                "use-native-err",
                "edit_file",
                None,
                40,
            ))
            .await
            .unwrap();
        store
            .attach_result("use-gh-ok", br#"{"ok":true}"#, false, 25)
            .await
            .unwrap();
        store
            .attach_result("use-gh-err", br#"{"ok":false}"#, true, 35)
            .await
            .unwrap();
        store
            .attach_result("use-native-err", br#"{"ok":false}"#, true, 45)
            .await
            .unwrap();

        let github_rows = store
            .list(
                "owner-a",
                "inst-a",
                LlmToolCallFilters {
                    server: Some("github"),
                    ..LlmToolCallFilters::default()
                },
                None,
                100,
            )
            .await
            .unwrap();
        assert_eq!(
            github_rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![gh_err, gh_ok]
        );

        let errors = store
            .list(
                "owner-a",
                "inst-a",
                LlmToolCallFilters {
                    status: LlmToolCallStatusFilter::Err,
                    ..LlmToolCallFilters::default()
                },
                None,
                100,
            )
            .await
            .unwrap();
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().all(|r| r.is_error == Some(true)));

        let one_tool = store
            .list(
                "owner-a",
                "inst-a",
                LlmToolCallFilters {
                    tool: Some("bash"),
                    ..LlmToolCallFilters::default()
                },
                None,
                100,
            )
            .await
            .unwrap();
        assert_eq!(
            one_tool.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![bash]
        );

        let page = store
            .list("owner-a", "inst-a", LlmToolCallFilters::default(), None, 2)
            .await
            .unwrap();
        assert_eq!(page.len(), 2);
        let next = store
            .list(
                "owner-a",
                "inst-a",
                LlmToolCallFilters::default(),
                Some(page[1].id),
                10,
            )
            .await
            .unwrap();
        assert_eq!(
            next.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![gh_ok, bash]
        );
    }

    #[tokio::test]
    async fn tool_call_stream_after_and_mcp_link() {
        let pool = open_in_memory().await.unwrap();
        seed_owner_instance(&pool, "owner-a", "inst-a").await;
        let mcp = SqliteMcpAuditStore::new(pool.clone());
        let mcp_id = mcp
            .insert(&McpAuditEntry {
                owner_id: "owner-a".into(),
                instance_id: "inst-a".into(),
                server_name: "github".into(),
                tool: Some("create_issue".into()),
                status: 200,
                duration_ms: 42,
                ts: 101,
                completed: true,
            })
            .await
            .unwrap();
        let store = SqliteLlmToolCallStore::new(pool);
        let native = store
            .insert_call(&tool_call("owner-a", "inst-a", "use-1", "bash", None, 100))
            .await
            .unwrap();
        let call_id = store
            .insert_call(&tool_call(
                "owner-a",
                "inst-a",
                "use-2",
                "mcp__github__create_issue",
                Some("github"),
                102,
            ))
            .await
            .unwrap();
        assert!(
            store
                .link_mcp_audit(call_id, "owner-a", "inst-a", "github", "create_issue", 102)
                .await
                .unwrap()
        );

        let rows = store
            .stream_after("owner-a", "inst-a", native)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, call_id);
        assert_eq!(rows[0].mcp_audit_id, Some(mcp_id));
        assert_eq!(rows[0].mcp_status, Some(200));
        assert_eq!(rows[0].mcp_duration_ms, Some(42));
    }

    #[tokio::test]
    async fn secret_access_audit_lists_newest_first_and_filters() {
        let pool = open_in_memory().await.unwrap();
        let store = SqliteSecretAccessAuditStore::new(pool);
        let first = SecretAccessAuditEntry {
            timestamp: 10,
            actor_kind: "runtime".into(),
            actor_id: Some("inst-a".into()),
            reason: SecretAccessReason::LlmProviderProxy,
            operation: SecretAccessOperation::Decrypt,
            scope: KmsScope::RuntimeToken,
            owner_id: None,
            instance_id: Some("inst-a".into()),
            secret_name: Some("proxy_token:*".into()),
            key_id: Some("system/provider".into()),
            key_version: Some(1),
            result: SecretAccessResult::Success,
            error_class: None,
            error_message: None,
        };
        let second = SecretAccessAuditEntry {
            timestamp: 20,
            result: SecretAccessResult::Failure,
            error_class: Some("EnvelopeError".into()),
            error_message: Some("redacted".into()),
            ..first.clone()
        };
        store.insert(&first).await.unwrap();
        store.insert(&second).await.unwrap();

        let page = store
            .list(SecretAccessAuditFilter {
                scope: Some(KmsScope::RuntimeToken),
                limit: 1,
                offset: 0,
                ..SecretAccessAuditFilter::default()
            })
            .await
            .unwrap();
        assert_eq!(page.items, vec![second]);
        assert_eq!(page.next_offset, Some(1));
    }

    async fn apply_secret_access_owner_backfill(pool: &SqlitePool) -> u64 {
        let migration_sql =
            include_str!("../../../migrations/sqlite/0050_secret_access_audit_owner_backfill.sql")
                .lines()
                .filter(|line| !line.trim_start().starts_with("--"))
                .collect::<Vec<_>>()
                .join("\n");
        let mut affected = 0;
        for statement in migration_sql.split(';') {
            let statement = statement.trim();
            if !statement.is_empty() {
                affected += sqlx::query(statement)
                    .execute(pool)
                    .await
                    .unwrap()
                    .rows_affected();
            }
        }
        affected
    }

    #[tokio::test]
    async fn secret_access_audit_owner_backfill_is_idempotent_and_instance_scoped() {
        let pool = open_in_memory().await.unwrap();
        seed_owner_instance(&pool, "owner-a", "inst-a").await;
        let store = SqliteSecretAccessAuditStore::new(pool.clone());
        let instance_missing_owner = SecretAccessAuditEntry {
            timestamp: 10,
            actor_kind: "runtime".into(),
            actor_id: Some("inst-a".into()),
            reason: SecretAccessReason::LlmProviderProxy,
            operation: SecretAccessOperation::Decrypt,
            scope: KmsScope::RuntimeToken,
            owner_id: None,
            instance_id: Some("inst-a".into()),
            secret_name: Some("proxy_token:*".into()),
            key_id: Some("system/runtime_tokens".into()),
            key_version: Some(1),
            result: SecretAccessResult::Success,
            error_class: None,
            error_message: None,
        };
        let system_only = SecretAccessAuditEntry {
            timestamp: 20,
            actor_kind: "system".into(),
            actor_id: Some("bootstrap".into()),
            reason: SecretAccessReason::SystemSecretBootstrap,
            operation: SecretAccessOperation::Decrypt,
            scope: KmsScope::SystemSecret,
            owner_id: None,
            instance_id: None,
            secret_name: Some("provider".into()),
            key_id: Some("system/provider".into()),
            key_version: Some(1),
            result: SecretAccessResult::Success,
            error_class: None,
            error_message: None,
        };
        let instance_empty_owner = SecretAccessAuditEntry {
            timestamp: 30,
            owner_id: Some(String::new()),
            ..instance_missing_owner.clone()
        };
        let missing_instance = SecretAccessAuditEntry {
            timestamp: 40,
            instance_id: Some("missing-inst".into()),
            ..instance_missing_owner.clone()
        };
        store.insert(&instance_missing_owner).await.unwrap();
        store.insert(&system_only).await.unwrap();
        store.insert(&instance_empty_owner).await.unwrap();
        store.insert(&missing_instance).await.unwrap();

        assert_eq!(apply_secret_access_owner_backfill(&pool).await, 2);
        assert_eq!(apply_secret_access_owner_backfill(&pool).await, 0);

        let rows: Vec<(i64, Option<String>)> = sqlx::query_as(
            "SELECT timestamp, owner_id FROM secret_access_audit ORDER BY timestamp",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                (10, Some("owner-a".to_owned())),
                (20, None),
                (30, Some("owner-a".to_owned())),
                (40, None),
            ]
        );
    }
}
