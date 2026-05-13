//! SQLite-backed public publication gate for agent-authored skills.

use async_trait::async_trait;
use sqlx::{Row, SqlitePool};

use crate::db::sqlite::map_sqlx;
use crate::error::StoreError;
use crate::now_secs;
use crate::traits::{
    AgentSkillPublicationRow, AgentSkillPublicationSpec, AgentSkillPublicationStore,
};

#[derive(Debug, Clone)]
pub struct SqlxAgentSkillPublicationStore {
    pool: SqlitePool,
}

impl SqlxAgentSkillPublicationStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AgentSkillPublicationStore for SqlxAgentSkillPublicationStore {
    async fn publish(
        &self,
        spec: AgentSkillPublicationSpec<'_>,
    ) -> Result<AgentSkillPublicationRow, StoreError> {
        let now = now_secs();
        sqlx::query(
            "INSERT INTO agent_skill_publications \
             (instance_id, owner_id, skill, published_by, published_at, revoked_at) \
             VALUES (?, ?, ?, ?, ?, NULL) \
             ON CONFLICT(instance_id, skill) DO UPDATE SET \
               owner_id = excluded.owner_id, \
               published_by = excluded.published_by, \
               published_at = excluded.published_at, \
               revoked_at = NULL",
        )
        .bind(spec.instance_id)
        .bind(spec.owner_id)
        .bind(spec.skill)
        .bind(spec.published_by)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        self.find(spec.instance_id, spec.skill)
            .await?
            .ok_or_else(|| StoreError::Io("agent skill publication vanished after publish".into()))
    }

    async fn revoke(&self, instance_id: &str, skill: &str) -> Result<bool, StoreError> {
        let now = now_secs();
        let result = sqlx::query(
            "UPDATE agent_skill_publications \
             SET revoked_at = ? \
             WHERE instance_id = ? AND skill = ? AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(instance_id)
        .bind(skill)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(result.rows_affected() > 0)
    }

    async fn find(
        &self,
        instance_id: &str,
        skill: &str,
    ) -> Result<Option<AgentSkillPublicationRow>, StoreError> {
        let row = sqlx::query(
            "SELECT instance_id, owner_id, skill, published_by, published_at, revoked_at \
             FROM agent_skill_publications \
             WHERE instance_id = ? AND skill = ?",
        )
        .bind(instance_id)
        .bind(skill)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        row.map(row_to_publication).transpose()
    }

    async fn list_public(&self) -> Result<Vec<AgentSkillPublicationRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT instance_id, owner_id, skill, published_by, published_at, revoked_at \
             FROM agent_skill_publications \
             WHERE revoked_at IS NULL \
             ORDER BY published_at DESC, instance_id, skill",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.into_iter().map(row_to_publication).collect()
    }

    async fn list_for_instance(
        &self,
        instance_id: &str,
    ) -> Result<Vec<AgentSkillPublicationRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT instance_id, owner_id, skill, published_by, published_at, revoked_at \
             FROM agent_skill_publications \
             WHERE instance_id = ? \
             ORDER BY skill",
        )
        .bind(instance_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.into_iter().map(row_to_publication).collect()
    }
}

fn row_to_publication(
    row: sqlx::sqlite::SqliteRow,
) -> Result<AgentSkillPublicationRow, StoreError> {
    Ok(AgentSkillPublicationRow {
        instance_id: row.try_get("instance_id").map_err(map_sqlx)?,
        owner_id: row.try_get("owner_id").map_err(map_sqlx)?,
        skill: row.try_get("skill").map_err(map_sqlx)?,
        published_by: row.try_get("published_by").map_err(map_sqlx)?,
        published_at: row.try_get("published_at").map_err(map_sqlx)?,
        revoked_at: row.try_get("revoked_at").map_err(map_sqlx)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sqlite::open_in_memory;

    #[tokio::test]
    async fn publish_revoke_and_republish_round_trip() {
        let pool = open_in_memory().await.unwrap();
        sqlx::query(
             "INSERT INTO users \
             (id, subject, email, display_name, status, created_at, activated_at, last_seen_at, openrouter_key_id, openrouter_key_limit_usd, email_ciphertext) \
             VALUES ('owner-1', 'sub-owner-1', NULL, NULL, 'active', 1, 1, NULL, NULL, 0, NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO instances \
             (id, owner_id, cube_sandbox_id, template_id, status, bearer_token, pinned, expires_at, last_active_at, last_probe_at, last_probe_status, created_at, destroyed_at, network_policy_kind, network_policy_entries, network_policy_cidrs, models, tools, name, task, rotated_to, state_generation) \
             VALUES ('inst-1', 'owner-1', NULL, 'tpl', 'live', 'sealed:v1:test', 0, NULL, 1, NULL, NULL, 1, NULL, 'nolocalnet', '', '', '[]', '[]', '', '', NULL, 'gen')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let store = SqlxAgentSkillPublicationStore::new(pool);

        let row = store
            .publish(AgentSkillPublicationSpec {
                instance_id: "inst-1",
                owner_id: "owner-1",
                skill: "debug-logs",
                published_by: "owner-1",
            })
            .await
            .unwrap();
        assert_eq!(row.skill, "debug-logs");
        assert!(row.revoked_at.is_none());
        assert_eq!(store.list_public().await.unwrap().len(), 1);

        assert!(store.revoke("inst-1", "debug-logs").await.unwrap());
        assert!(store.list_public().await.unwrap().is_empty());
        assert!(!store.revoke("inst-1", "debug-logs").await.unwrap());

        let republished = store
            .publish(AgentSkillPublicationSpec {
                instance_id: "inst-1",
                owner_id: "owner-1",
                skill: "debug-logs",
                published_by: "admin-1",
            })
            .await
            .unwrap();
        assert_eq!(republished.published_by, "admin-1");
        assert!(republished.revoked_at.is_none());
    }
}
