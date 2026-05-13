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
use sqlx::{PgPool, Postgres, QueryBuilder, Row};

use crate::db::pg::map_sqlx;
use crate::error::StoreError;
use crate::traits::{
    AdminAuditEntry, AdminAuditStore, AuditEntry, AuditStore, LlmToolCallEntry, LlmToolCallFilters,
    LlmToolCallRow, LlmToolCallStatusFilter, LlmToolCallStore, McpAuditEntry, McpAuditStore,
};

#[derive(Debug, Clone)]
pub struct PgAuditStore {
    pool: PgPool,
}

impl PgAuditStore {
    pub fn new(pool: PgPool) -> Self {
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
pub struct PgMcpAuditStore {
    pool: PgPool,
}

impl PgMcpAuditStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Clone)]
pub struct PgAdminAuditStore {
    pool: PgPool,
}

impl PgAdminAuditStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Clone)]
pub struct PgLlmToolCallStore {
    pool: PgPool,
}

impl PgLlmToolCallStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NoopMcpAuditStore;

#[async_trait]
impl AuditStore for PgAuditStore {
    async fn insert(&self, entry: &AuditEntry) -> Result<i64, StoreError> {
        // SQLite's `INTEGER PRIMARY KEY AUTOINCREMENT` exposes the
        // newly-assigned id via `last_insert_rowid()`; we round-trip
        // it through a single `RETURNING id` for portability.
        let row = sqlx::query(
            "INSERT INTO llm_audit \
             (owner_id, instance_id, provider, model, prompt_tokens, output_tokens, status_code, duration_ms, occurred_at, key_source, completed) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
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
             FROM llm_audit WHERE owner_id = $1 AND occurred_at >= $2",
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
        sqlx::query("UPDATE llm_audit SET output_tokens = $1, completed = 1 WHERE id = $2")
            .bind(output_tokens)
            .bind(audit_id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }
}

#[async_trait]
impl McpAuditStore for PgMcpAuditStore {
    async fn insert(&self, entry: &McpAuditEntry) -> Result<i64, StoreError> {
        let row = sqlx::query(
            "INSERT INTO mcp_audit \
             (owner_id, instance_id, server_name, tool, status, duration_ms, ts, completed) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
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
        sqlx::query(
            "UPDATE mcp_audit SET status = $1, duration_ms = $2, completed = 1 WHERE id = $3",
        )
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
impl LlmToolCallStore for PgLlmToolCallStore {
    async fn insert_call(&self, entry: &LlmToolCallEntry) -> Result<i64, StoreError> {
        let row = sqlx::query(
            "INSERT INTO llm_tool_call \
             (llm_audit_id, owner_id, instance_id, tool_use_id, tool_name, mcp_server, input_sealed, called_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
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
             SET result_sealed = $1, is_error = $2, resulted_at = $3 \
             WHERE id = ( \
               SELECT id FROM llm_tool_call \
               WHERE tool_use_id = $4 AND result_sealed IS NULL \
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
             WHERE c.owner_id = $1 AND c.instance_id = $2 AND c.id > $3 \
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
               WHERE owner_id = $1 AND instance_id = $2 AND server_name = $3 AND tool = $4 \
               ORDER BY ABS(ts - $5) ASC, id DESC LIMIT 1 \
             ) \
             WHERE id = $6 AND mcp_audit_id IS NULL \
               AND EXISTS ( \
                 SELECT 1 FROM mcp_audit \
                 WHERE owner_id = $7 AND instance_id = $8 AND server_name = $9 AND tool = $10 \
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
}

fn tool_call_select_builder<'a>() -> QueryBuilder<'a, Postgres> {
    QueryBuilder::new(
        "SELECT c.id, c.llm_audit_id, c.owner_id, c.instance_id, c.tool_use_id, c.tool_name, \
                c.mcp_server, c.input_sealed, c.result_sealed, c.is_error, c.called_at, \
                c.resulted_at, c.mcp_audit_id, m.status AS mcp_status, m.duration_ms AS mcp_duration_ms \
         FROM llm_tool_call c \
         LEFT JOIN mcp_audit m ON m.id = c.mcp_audit_id",
    )
}

fn append_tool_call_filters<'a>(
    q: &mut QueryBuilder<'a, Postgres>,
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

fn row_to_tool_call(row: &sqlx::postgres::PgRow) -> Result<LlmToolCallRow, StoreError> {
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

#[async_trait]
impl AdminAuditStore for PgAdminAuditStore {
    async fn insert(&self, entry: &AdminAuditEntry) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO admin_audit \
             (actor_subject, action, target_user, params_hash, ts) \
             VALUES ($1, $2, $3, $4, $5)",
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

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::*;
    use crate::network_policy::NetworkPolicy;
    use crate::traits::{InstanceRow, InstanceStatus, InstanceStore};

    fn unique(prefix: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{prefix}-{nanos}")
    }

    async fn fixture(name: &str) -> Option<(PgPool, PgLlmToolCallStore, String, String)> {
        let url = match std::env::var("PG_TEST_URL") {
            Ok(url) if !url.trim().is_empty() => url,
            _ => return None,
        };
        let pool = crate::db::pg::open(&url).await.unwrap();
        let owner = unique(&format!("owner-{name}"));
        let instance = unique(&format!("inst-{name}"));
        sqlx::query(
            "INSERT INTO users (id, subject, status, created_at, activated_at) \
             VALUES ($1, $2, 'active', 0, 0)",
        )
        .bind(&owner)
        .bind(format!("subject-{owner}"))
        .execute(&pool)
        .await
        .unwrap();
        let instances = crate::db::pg::instances::PgInstanceStore::new(
            pool.clone(),
            crate::db::sqlite::test_system_cipher(),
        );
        instances
            .create(InstanceRow {
                id: instance.clone(),
                owner_id: owner.clone(),
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
        Some((pool.clone(), PgLlmToolCallStore::new(pool), owner, instance))
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

    #[tokio::test]
    async fn pg_tool_call_insert_attach_and_filter_round_trip() {
        let Some((_pool, store, owner, instance)) = fixture("round").await else {
            return;
        };
        let id = store
            .insert_call(&tool_call(&owner, &instance, "use-1", "bash", None, 100))
            .await
            .unwrap();
        assert!(
            store
                .attach_result("use-1", br#"{"ok":true}"#, false, 150)
                .await
                .unwrap()
        );
        assert!(
            !store
                .attach_result("use-1", br#"{"ok":false}"#, true, 151)
                .await
                .unwrap()
        );

        let rows = store
            .list(
                &owner,
                &instance,
                LlmToolCallFilters {
                    tool: Some("bash"),
                    status: LlmToolCallStatusFilter::Ok,
                    server: None,
                },
                None,
                100,
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(
            rows[0].result_sealed.as_deref(),
            Some(br#"{"ok":true}"#.as_slice())
        );
        assert_eq!(rows[0].is_error, Some(false));
    }

    #[tokio::test]
    async fn pg_tool_call_stream_after_and_mcp_link() {
        let Some((pool, store, owner, instance)) = fixture("stream").await else {
            return;
        };
        let mcp = PgMcpAuditStore::new(pool);
        let mcp_id = mcp
            .insert(&McpAuditEntry {
                owner_id: owner.clone(),
                instance_id: instance.clone(),
                server_name: "github".into(),
                tool: Some("create_issue".into()),
                status: 200,
                duration_ms: 42,
                ts: 101,
                completed: true,
            })
            .await
            .unwrap();
        let native = store
            .insert_call(&tool_call(&owner, &instance, "use-1", "bash", None, 100))
            .await
            .unwrap();
        let call_id = store
            .insert_call(&tool_call(
                &owner,
                &instance,
                "use-2",
                "mcp__github__create_issue",
                Some("github"),
                102,
            ))
            .await
            .unwrap();
        assert!(
            store
                .link_mcp_audit(call_id, &owner, &instance, "github", "create_issue", 102)
                .await
                .unwrap()
        );

        let rows = store.stream_after(&owner, &instance, native).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, call_id);
        assert_eq!(rows[0].mcp_audit_id, Some(mcp_id));
    }
}
