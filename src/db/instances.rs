use async_trait::async_trait;
use sqlx::{Row, SqlitePool};

use crate::error::StoreError;
use crate::traits::{
    InstanceRow, InstanceStatus, InstanceStore, ListFilter, ProbeResult,
};

#[derive(Debug, Clone)]
pub struct SqlxInstanceStore {
    pool: SqlitePool,
}

impl SqlxInstanceStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn map_sqlx(e: sqlx::Error) -> StoreError {
    match e {
        sqlx::Error::RowNotFound => StoreError::NotFound,
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            StoreError::Constraint(db.to_string())
        }
        other => StoreError::Io(other.to_string()),
    }
}

fn row_to_instance(row: &sqlx::sqlite::SqliteRow) -> Result<InstanceRow, StoreError> {
    let status_text: String = row.try_get("status").map_err(map_sqlx)?;
    let status = InstanceStatus::parse(&status_text)
        .ok_or_else(|| StoreError::Malformed(format!("status={status_text}")))?;
    let pinned_int: i64 = row.try_get("pinned").map_err(map_sqlx)?;
    let probe_text: Option<String> = row.try_get("last_probe_status").map_err(map_sqlx)?;
    let last_probe_status = match probe_text {
        Some(t) => Some(
            serde_json::from_str::<ProbeResult>(&t)
                .map_err(|e| StoreError::Malformed(format!("last_probe_status: {e}")))?,
        ),
        None => None,
    };
    Ok(InstanceRow {
        id: row.try_get("id").map_err(map_sqlx)?,
        cube_sandbox_id: row.try_get("cube_sandbox_id").map_err(map_sqlx)?,
        template_id: row.try_get("template_id").map_err(map_sqlx)?,
        status,
        bearer_token: row.try_get("bearer_token").map_err(map_sqlx)?,
        pinned: pinned_int != 0,
        expires_at: row.try_get("expires_at").map_err(map_sqlx)?,
        last_active_at: row.try_get("last_active_at").map_err(map_sqlx)?,
        last_probe_at: row.try_get("last_probe_at").map_err(map_sqlx)?,
        last_probe_status,
        created_at: row.try_get("created_at").map_err(map_sqlx)?,
        destroyed_at: row.try_get("destroyed_at").map_err(map_sqlx)?,
    })
}

#[async_trait]
impl InstanceStore for SqlxInstanceStore {
    async fn create(&self, row: InstanceRow) -> Result<(), StoreError> {
        let probe_json = match &row.last_probe_status {
            Some(p) => Some(serde_json::to_string(p).map_err(|e| StoreError::Io(e.to_string()))?),
            None => None,
        };
        sqlx::query(
            "INSERT INTO instances \
             (id, cube_sandbox_id, template_id, status, bearer_token, pinned, expires_at, \
              last_active_at, last_probe_at, last_probe_status, created_at, destroyed_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&row.cube_sandbox_id)
        .bind(&row.template_id)
        .bind(row.status.as_str())
        .bind(&row.bearer_token)
        .bind(row.pinned as i64)
        .bind(row.expires_at)
        .bind(row.last_active_at)
        .bind(row.last_probe_at)
        .bind(probe_json)
        .bind(row.created_at)
        .bind(row.destroyed_at)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<InstanceRow>, StoreError> {
        let row = sqlx::query("SELECT * FROM instances WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        match row {
            Some(r) => Ok(Some(row_to_instance(&r)?)),
            None => Ok(None),
        }
    }

    async fn list(&self, filter: ListFilter) -> Result<Vec<InstanceRow>, StoreError> {
        let status_filter: Option<String> = filter.status.map(|s| s.as_str().to_owned());
        let rows = sqlx::query(
            "SELECT * FROM instances \
             WHERE (?1 IS NULL OR status = ?1) \
               AND (?2 = 1 OR status != 'destroyed') \
             ORDER BY created_at DESC",
        )
        .bind(status_filter)
        .bind(filter.include_destroyed as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.iter().map(row_to_instance).collect()
    }

    async fn update_status(&self, id: &str, status: InstanceStatus) -> Result<(), StoreError> {
        let now = now_secs();
        let result = sqlx::query(
            "UPDATE instances SET status = ?1, \
                                  destroyed_at = CASE WHEN ?1 = 'destroyed' THEN ?2 ELSE destroyed_at END \
             WHERE id = ?3",
        )
        .bind(status.as_str())
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    async fn touch(&self, id: &str) -> Result<(), StoreError> {
        let result = sqlx::query("UPDATE instances SET last_active_at = ? WHERE id = ?")
            .bind(now_secs())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    async fn pin(&self, id: &str, pinned: bool, ttl: Option<i64>) -> Result<(), StoreError> {
        let expires_at = if pinned {
            None
        } else {
            ttl.map(|t| now_secs() + t)
        };
        let result = sqlx::query(
            "UPDATE instances SET pinned = ?1, expires_at = ?2 WHERE id = ?3",
        )
        .bind(pinned as i64)
        .bind(expires_at)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    async fn record_probe(&self, id: &str, status: ProbeResult) -> Result<(), StoreError> {
        let json = serde_json::to_string(&status).map_err(|e| StoreError::Io(e.to_string()))?;
        let result = sqlx::query(
            "UPDATE instances SET last_probe_at = ?, last_probe_status = ? WHERE id = ?",
        )
        .bind(now_secs())
        .bind(json)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    async fn expired(&self, now: i64) -> Result<Vec<InstanceRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT * FROM instances \
             WHERE pinned = 0 \
               AND expires_at IS NOT NULL \
               AND expires_at < ? \
               AND status != 'destroyed'",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.iter().map(row_to_instance).collect()
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    fn sample(id: &str) -> InstanceRow {
        InstanceRow {
            id: id.to_owned(),
            cube_sandbox_id: Some(format!("sb-{id}")),
            template_id: "tpl-1".into(),
            status: InstanceStatus::Live,
            bearer_token: format!("tok-{id}"),
            pinned: false,
            expires_at: Some(1000),
            last_active_at: 100,
            last_probe_at: None,
            last_probe_status: None,
            created_at: 50,
            destroyed_at: None,
        }
    }

    #[tokio::test]
    async fn create_get_round_trip() {
        let pool = open_in_memory().await.unwrap();
        let store = SqlxInstanceStore::new(pool);
        store.create(sample("a")).await.unwrap();
        let got = store.get("a").await.unwrap().expect("present");
        assert_eq!(got.id, "a");
        assert_eq!(got.status, InstanceStatus::Live);
        assert_eq!(got.cube_sandbox_id.as_deref(), Some("sb-a"));
        assert!(!got.pinned);
        assert!(store.get("missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn update_status_destroys() {
        let pool = open_in_memory().await.unwrap();
        let store = SqlxInstanceStore::new(pool);
        store.create(sample("a")).await.unwrap();
        store
            .update_status("a", InstanceStatus::Destroyed)
            .await
            .unwrap();
        let got = store.get("a").await.unwrap().unwrap();
        assert_eq!(got.status, InstanceStatus::Destroyed);
        assert!(got.destroyed_at.is_some());
    }

    #[tokio::test]
    async fn pin_clears_expiry_unpin_sets_it() {
        let pool = open_in_memory().await.unwrap();
        let store = SqlxInstanceStore::new(pool);
        store.create(sample("a")).await.unwrap();
        store.pin("a", true, None).await.unwrap();
        let pinned = store.get("a").await.unwrap().unwrap();
        assert!(pinned.pinned);
        assert!(pinned.expires_at.is_none());

        store.pin("a", false, Some(60)).await.unwrap();
        let unpinned = store.get("a").await.unwrap().unwrap();
        assert!(!unpinned.pinned);
        assert!(unpinned.expires_at.is_some());
    }

    #[tokio::test]
    async fn record_probe_round_trips_through_json() {
        let pool = open_in_memory().await.unwrap();
        let store = SqlxInstanceStore::new(pool);
        store.create(sample("a")).await.unwrap();
        store
            .record_probe(
                "a",
                ProbeResult::Degraded {
                    reason: "slow".into(),
                },
            )
            .await
            .unwrap();
        let got = store.get("a").await.unwrap().unwrap();
        match got.last_probe_status {
            Some(ProbeResult::Degraded { reason }) => assert_eq!(reason, "slow"),
            other => panic!("unexpected {other:?}"),
        }
        assert!(got.last_probe_at.is_some());
    }

    #[tokio::test]
    async fn expired_excludes_pinned_and_destroyed() {
        let pool = open_in_memory().await.unwrap();
        let store = SqlxInstanceStore::new(pool);
        let mut a = sample("a");
        a.expires_at = Some(50);
        store.create(a).await.unwrap();

        let mut b = sample("b");
        b.expires_at = Some(50);
        b.pinned = true;
        store.create(b).await.unwrap();

        let mut c = sample("c");
        c.expires_at = Some(50);
        c.status = InstanceStatus::Destroyed;
        store.create(c).await.unwrap();

        let mut d = sample("d");
        d.expires_at = Some(2000);
        store.create(d).await.unwrap();

        let exp = store.expired(100).await.unwrap();
        let ids: Vec<_> = exp.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["a"]);
    }

    #[tokio::test]
    async fn list_filters_destroyed_by_default() {
        let pool = open_in_memory().await.unwrap();
        let store = SqlxInstanceStore::new(pool);
        store.create(sample("a")).await.unwrap();
        let mut b = sample("b");
        b.status = InstanceStatus::Destroyed;
        store.create(b).await.unwrap();

        let live_only = store.list(ListFilter::default()).await.unwrap();
        assert_eq!(live_only.len(), 1);
        assert_eq!(live_only[0].id, "a");

        let all = store
            .list(ListFilter {
                status: None,
                include_destroyed: true,
            })
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn touch_updates_last_active() {
        let pool = open_in_memory().await.unwrap();
        let store = SqlxInstanceStore::new(pool);
        store.create(sample("a")).await.unwrap();
        let before = store.get("a").await.unwrap().unwrap().last_active_at;
        // touch sets to now, which is far larger than 100
        store.touch("a").await.unwrap();
        let after = store.get("a").await.unwrap().unwrap().last_active_at;
        assert!(after > before);
    }
}
