//! Snapshot CRUD. There is no `SnapshotStore` trait — the brief lists
//! `SqlxInstanceStore`, `SqlxSecretStore`, and `SqlxTokenStore` as the
//! store impls, and snapshot rows are managed via these plain functions
//! by the snapshot module (step 8) and the BackupSink impls.

use sqlx::{Row, SqlitePool};

use crate::error::StoreError;
use crate::traits::{SnapshotKind, SnapshotRow};

fn map_sqlx(e: sqlx::Error) -> StoreError {
    match e {
        sqlx::Error::RowNotFound => StoreError::NotFound,
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            StoreError::Constraint(db.to_string())
        }
        other => StoreError::Io(other.to_string()),
    }
}

fn row_to_snapshot(row: &sqlx::sqlite::SqliteRow) -> Result<SnapshotRow, StoreError> {
    let kind_text: String = row.try_get("kind").map_err(map_sqlx)?;
    let kind = SnapshotKind::parse(&kind_text)
        .ok_or_else(|| StoreError::Malformed(format!("kind={kind_text}")))?;
    Ok(SnapshotRow {
        id: row.try_get("id").map_err(map_sqlx)?,
        source_instance_id: row.try_get("source_instance_id").map_err(map_sqlx)?,
        parent_snapshot_id: row.try_get("parent_snapshot_id").map_err(map_sqlx)?,
        kind,
        path: row.try_get("path").map_err(map_sqlx)?,
        host_ip: row.try_get("host_ip").map_err(map_sqlx)?,
        remote_uri: row.try_get("remote_uri").map_err(map_sqlx)?,
        size_bytes: row.try_get("size_bytes").map_err(map_sqlx)?,
        created_at: row.try_get("created_at").map_err(map_sqlx)?,
        deleted_at: row.try_get("deleted_at").map_err(map_sqlx)?,
    })
}

pub async fn insert(pool: &SqlitePool, row: &SnapshotRow) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO snapshots \
         (id, source_instance_id, parent_snapshot_id, kind, path, host_ip, remote_uri, size_bytes, created_at, deleted_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.source_instance_id)
    .bind(&row.parent_snapshot_id)
    .bind(row.kind.as_str())
    .bind(&row.path)
    .bind(&row.host_ip)
    .bind(&row.remote_uri)
    .bind(row.size_bytes)
    .bind(row.created_at)
    .bind(row.deleted_at)
    .execute(pool)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<SnapshotRow>, StoreError> {
    let row = sqlx::query("SELECT * FROM snapshots WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(map_sqlx)?;
    match row {
        Some(r) => Ok(Some(row_to_snapshot(&r)?)),
        None => Ok(None),
    }
}

pub async fn list_for_instance(
    pool: &SqlitePool,
    instance_id: &str,
) -> Result<Vec<SnapshotRow>, StoreError> {
    let rows = sqlx::query(
        "SELECT * FROM snapshots WHERE source_instance_id = ? AND deleted_at IS NULL ORDER BY created_at DESC",
    )
    .bind(instance_id)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;
    rows.iter().map(row_to_snapshot).collect()
}

pub async fn update_remote_uri(
    pool: &SqlitePool,
    id: &str,
    uri: &str,
) -> Result<(), StoreError> {
    let r = sqlx::query("UPDATE snapshots SET remote_uri = ? WHERE id = ?")
        .bind(uri)
        .bind(id)
        .execute(pool)
        .await
        .map_err(map_sqlx)?;
    if r.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

pub async fn update_path(pool: &SqlitePool, id: &str, path: &str) -> Result<(), StoreError> {
    let r = sqlx::query("UPDATE snapshots SET path = ? WHERE id = ?")
        .bind(path)
        .bind(id)
        .execute(pool)
        .await
        .map_err(map_sqlx)?;
    if r.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

pub async fn mark_deleted(pool: &SqlitePool, id: &str, when: i64) -> Result<(), StoreError> {
    let r = sqlx::query("UPDATE snapshots SET deleted_at = ? WHERE id = ?")
        .bind(when)
        .bind(id)
        .execute(pool)
        .await
        .map_err(map_sqlx)?;
    if r.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::instances::SqlxInstanceStore;
    use crate::db::open_in_memory;
    use crate::traits::{InstanceRow, InstanceStatus, InstanceStore};

    async fn seed(pool: &SqlitePool, id: &str) {
        let store = SqlxInstanceStore::new(pool.clone());
        store
            .create(InstanceRow {
                id: id.into(),
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

    fn snap(id: &str, parent: Option<&str>, source: &str) -> SnapshotRow {
        SnapshotRow {
            id: id.into(),
            source_instance_id: source.into(),
            parent_snapshot_id: parent.map(String::from),
            kind: SnapshotKind::Manual,
            path: format!("/var/snaps/{id}"),
            host_ip: "10.0.0.1".into(),
            remote_uri: None,
            size_bytes: Some(1234),
            created_at: 100,
            deleted_at: None,
        }
    }

    #[tokio::test]
    async fn insert_get_with_parent_and_remote_uri() {
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        insert(&pool, &snap("s1", None, "i1")).await.unwrap();
        insert(&pool, &snap("s2", Some("s1"), "i1")).await.unwrap();
        update_remote_uri(&pool, "s2", "s3://bucket/key/s2/")
            .await
            .unwrap();
        let g = get(&pool, "s2").await.unwrap().unwrap();
        assert_eq!(g.parent_snapshot_id.as_deref(), Some("s1"));
        assert_eq!(g.remote_uri.as_deref(), Some("s3://bucket/key/s2/"));
        assert_eq!(g.kind, SnapshotKind::Manual);
    }

    #[tokio::test]
    async fn list_excludes_deleted() {
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        insert(&pool, &snap("s1", None, "i1")).await.unwrap();
        insert(&pool, &snap("s2", None, "i1")).await.unwrap();
        mark_deleted(&pool, "s1", 200).await.unwrap();
        let listed = list_for_instance(&pool, "i1").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "s2");
    }

    #[tokio::test]
    async fn update_path_after_pull() {
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        insert(&pool, &snap("s1", None, "i1")).await.unwrap();
        update_path(&pool, "s1", "/var/cache/s1").await.unwrap();
        let g = get(&pool, "s1").await.unwrap().unwrap();
        assert_eq!(g.path, "/var/cache/s1");
    }

    #[tokio::test]
    async fn kind_round_trip() {
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        let mut s = snap("s1", None, "i1");
        s.kind = SnapshotKind::Backup;
        insert(&pool, &s).await.unwrap();
        let g = get(&pool, "s1").await.unwrap().unwrap();
        assert_eq!(g.kind, SnapshotKind::Backup);
    }
}
