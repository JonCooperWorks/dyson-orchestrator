use async_trait::async_trait;
use sqlx::{Row, SqlitePool};

use crate::error::StoreError;
use crate::traits::SecretStore;

#[derive(Debug, Clone)]
pub struct SqlxSecretStore {
    pool: SqlitePool,
}

impl SqlxSecretStore {
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

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[async_trait]
impl SecretStore for SqlxSecretStore {
    async fn put(&self, instance_id: &str, name: &str, value: &str) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO instance_secrets (instance_id, name, value, created_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(instance_id, name) DO UPDATE SET value = excluded.value, created_at = excluded.created_at",
        )
        .bind(instance_id)
        .bind(name)
        .bind(value)
        .bind(now_secs())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn delete(&self, instance_id: &str, name: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM instance_secrets WHERE instance_id = ? AND name = ?")
            .bind(instance_id)
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }

    async fn list(&self, instance_id: &str) -> Result<Vec<(String, String)>, StoreError> {
        let rows = sqlx::query(
            "SELECT name, value FROM instance_secrets WHERE instance_id = ? ORDER BY name",
        )
        .bind(instance_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.iter()
            .map(|r| {
                let n: String = r.try_get("name").map_err(map_sqlx)?;
                let v: String = r.try_get("value").map_err(map_sqlx)?;
                Ok((n, v))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::db::instances::SqlxInstanceStore;
    use crate::traits::{InstanceRow, InstanceStatus, InstanceStore};

    async fn seed_instance(pool: sqlx::SqlitePool, id: &str) {
        let store = SqlxInstanceStore::new(pool);
        store
            .create(InstanceRow {
                id: id.into(),
                owner_id: "legacy".into(),
            name: String::new(),
            task: String::new(),
                cube_sandbox_id: None,
                template_id: "t".into(),
                status: InstanceStatus::Live,
                bearer_token: "b".into(),
                pinned: false,
                expires_at: None,
                last_active_at: 0,
                last_probe_at: None,
                last_probe_status: None,
                created_at: 0,
                destroyed_at: None,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn put_list_delete_round_trip() {
        let pool = open_in_memory().await.unwrap();
        seed_instance(pool.clone(), "i1").await;
        let s = SqlxSecretStore::new(pool);
        s.put("i1", "GITHUB_TOKEN", "ghp_xxx").await.unwrap();
        s.put("i1", "OPENAI_KEY", "sk_xxx").await.unwrap();

        let listed = s.list("i1").await.unwrap();
        assert_eq!(
            listed,
            vec![
                ("GITHUB_TOKEN".to_string(), "ghp_xxx".to_string()),
                ("OPENAI_KEY".to_string(), "sk_xxx".to_string()),
            ]
        );

        s.delete("i1", "GITHUB_TOKEN").await.unwrap();
        let after = s.list("i1").await.unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].0, "OPENAI_KEY");
    }

    #[tokio::test]
    async fn put_overwrites_existing() {
        let pool = open_in_memory().await.unwrap();
        seed_instance(pool.clone(), "i1").await;
        let s = SqlxSecretStore::new(pool);
        s.put("i1", "K", "v1").await.unwrap();
        s.put("i1", "K", "v2").await.unwrap();
        let listed = s.list("i1").await.unwrap();
        assert_eq!(listed, vec![("K".to_string(), "v2".to_string())]);
    }

    #[tokio::test]
    async fn isolation_between_instances() {
        let pool = open_in_memory().await.unwrap();
        seed_instance(pool.clone(), "i1").await;
        seed_instance(pool.clone(), "i2").await;
        let s = SqlxSecretStore::new(pool);
        s.put("i1", "K", "one").await.unwrap();
        s.put("i2", "K", "two").await.unwrap();
        assert_eq!(s.list("i1").await.unwrap()[0].1, "one");
        assert_eq!(s.list("i2").await.unwrap()[0].1, "two");
    }
}
