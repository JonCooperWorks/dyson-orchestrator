//! `llm_audit` row access.
//!
//! Plain functions, like the snapshots module — there is no `AuditStore`
//! trait in the brief; the proxy writes audit rows directly.

use sqlx::{Row, SqlitePool};

use crate::error::StoreError;

fn map_sqlx(e: sqlx::Error) -> StoreError {
    match e {
        sqlx::Error::RowNotFound => StoreError::NotFound,
        other => StoreError::Io(other.to_string()),
    }
}

#[derive(Debug, Clone)]
pub struct AuditRow {
    pub instance_id: String,
    pub provider: String,
    pub model: Option<String>,
    pub prompt_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub status_code: i64,
    pub duration_ms: i64,
    pub occurred_at: i64,
}

pub async fn insert(pool: &SqlitePool, row: &AuditRow) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO llm_audit \
         (instance_id, provider, model, prompt_tokens, output_tokens, status_code, duration_ms, occurred_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.instance_id)
    .bind(&row.provider)
    .bind(&row.model)
    .bind(row.prompt_tokens)
    .bind(row.output_tokens)
    .bind(row.status_code)
    .bind(row.duration_ms)
    .bind(row.occurred_at)
    .execute(pool)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

/// Sum of prompt_tokens + output_tokens for the given instance over the past
/// 24 hours. NULL token counts contribute 0.
pub async fn daily_tokens(
    pool: &SqlitePool,
    instance_id: &str,
    now: i64,
) -> Result<u64, StoreError> {
    let since = now - 86_400;
    let row = sqlx::query(
        "SELECT COALESCE(SUM(COALESCE(prompt_tokens,0) + COALESCE(output_tokens,0)), 0) AS total \
         FROM llm_audit WHERE instance_id = ? AND occurred_at >= ?",
    )
    .bind(instance_id)
    .bind(since)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)?;
    let total: i64 = row.try_get("total").map_err(map_sqlx)?;
    Ok(total.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    fn r(instance: &str, when: i64, prompt: i64, output: i64) -> AuditRow {
        AuditRow {
            instance_id: instance.into(),
            provider: "openai".into(),
            model: Some("gpt-4o".into()),
            prompt_tokens: Some(prompt),
            output_tokens: Some(output),
            status_code: 200,
            duration_ms: 100,
            occurred_at: when,
        }
    }

    #[tokio::test]
    async fn daily_tokens_sums_window() {
        let pool = open_in_memory().await.unwrap();
        let now = 1_000_000;
        // Inside window (within last 24h).
        insert(&pool, &r("i1", now - 100, 100, 50)).await.unwrap();
        insert(&pool, &r("i1", now - 1000, 200, 100)).await.unwrap();
        // Outside window.
        insert(&pool, &r("i1", now - 86_500, 999, 999)).await.unwrap();
        // Other instance — not counted.
        insert(&pool, &r("i2", now - 100, 9999, 9999)).await.unwrap();

        let total = daily_tokens(&pool, "i1", now).await.unwrap();
        assert_eq!(total, 100 + 50 + 200 + 100);
    }

    #[tokio::test]
    async fn daily_tokens_handles_null_columns() {
        let pool = open_in_memory().await.unwrap();
        let now = 1_000_000;
        let mut row = r("i1", now - 10, 50, 25);
        row.prompt_tokens = None;
        insert(&pool, &row).await.unwrap();
        // Only output_tokens contributes.
        assert_eq!(daily_tokens(&pool, "i1", now).await.unwrap(), 25);
    }

    #[tokio::test]
    async fn daily_tokens_zero_on_no_rows() {
        let pool = open_in_memory().await.unwrap();
        assert_eq!(daily_tokens(&pool, "i1", 1).await.unwrap(), 0);
    }
}
