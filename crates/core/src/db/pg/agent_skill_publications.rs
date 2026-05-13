//! Postgres-backed public publication gate for agent-authored skills.

use async_trait::async_trait;
use sqlx::{PgPool, Row};

use crate::db::pg::map_sqlx;
use crate::error::StoreError;
use crate::now_secs;
use crate::traits::{
    AgentSkillPublicationRow, AgentSkillPublicationSpec, AgentSkillPublicationStore,
};

#[derive(Debug, Clone)]
pub struct PgAgentSkillPublicationStore {
    pool: PgPool,
}

impl PgAgentSkillPublicationStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AgentSkillPublicationStore for PgAgentSkillPublicationStore {
    async fn publish(
        &self,
        spec: AgentSkillPublicationSpec<'_>,
    ) -> Result<AgentSkillPublicationRow, StoreError> {
        let now = now_secs();
        sqlx::query(
            "INSERT INTO agent_skill_publications \
             (instance_id, owner_id, skill, published_by, published_at, revoked_at) \
             VALUES ($1, $2, $3, $4, $5, NULL) \
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
             SET revoked_at = $1 \
             WHERE instance_id = $2 AND skill = $3 AND revoked_at IS NULL",
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
             WHERE instance_id = $1 AND skill = $2",
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
             WHERE instance_id = $1 \
             ORDER BY skill",
        )
        .bind(instance_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.into_iter().map(row_to_publication).collect()
    }
}

fn row_to_publication(row: sqlx::postgres::PgRow) -> Result<AgentSkillPublicationRow, StoreError> {
    Ok(AgentSkillPublicationRow {
        instance_id: row.try_get("instance_id").map_err(map_sqlx)?,
        owner_id: row.try_get("owner_id").map_err(map_sqlx)?,
        skill: row.try_get("skill").map_err(map_sqlx)?,
        published_by: row.try_get("published_by").map_err(map_sqlx)?,
        published_at: row.try_get("published_at").map_err(map_sqlx)?,
        revoked_at: row.try_get("revoked_at").map_err(map_sqlx)?,
    })
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::*;

    async fn pg_pool() -> Option<PgPool> {
        let url = std::env::var("PG_TEST_URL").ok()?;
        let pool = crate::db::pg::open(&url).await.ok()?;
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let instance_id = format!("pg-pub-{suffix}");
        sqlx::query(
            "INSERT INTO users (id, subject, email, display_name, status, created_at, activated_at, last_seen_at, openrouter_key_id, openrouter_key_limit_usd, email_ciphertext) \
             VALUES ($1, $2, NULL, NULL, 'active', 1, 1, NULL, NULL, 0, NULL) \
             ON CONFLICT(id) DO NOTHING",
        )
        .bind("pg-owner")
        .bind(format!("sub-{suffix}"))
        .execute(&pool)
        .await
        .ok()?;
        sqlx::query(
            "INSERT INTO instances \
             (id, owner_id, cube_sandbox_id, template_id, status, bearer_token, pinned, expires_at, last_active_at, last_probe_at, last_probe_status, created_at, destroyed_at, network_policy_kind, network_policy_entries, network_policy_cidrs, models, tools, name, task, rotated_to, state_generation) \
             VALUES ($1, 'pg-owner', NULL, 'tpl', 'live', 'sealed:v1:test', 0, NULL, 1, NULL, NULL, 1, NULL, 'nolocalnet', '', '', '[]', '[]', '', '', NULL, 'gen')",
        )
        .bind(instance_id)
        .execute(&pool)
        .await
        .ok()?;
        Some(pool)
    }

    #[tokio::test]
    async fn publish_revoke_and_republish_round_trip() {
        let Some(pool) = pg_pool().await else {
            eprintln!("skipping postgres agent skill publication test; PG_TEST_URL unset");
            return;
        };
        let instance_id: String = sqlx::query_scalar(
            "SELECT id FROM instances WHERE id LIKE 'pg-pub-%' ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let store = PgAgentSkillPublicationStore::new(pool);

        let row = store
            .publish(AgentSkillPublicationSpec {
                instance_id: &instance_id,
                owner_id: "pg-owner",
                skill: "debug-logs",
                published_by: "pg-owner",
            })
            .await
            .unwrap();
        assert_eq!(row.skill, "debug-logs");
        assert!(store.revoke(&instance_id, "debug-logs").await.unwrap());
        assert!(
            store
                .publish(AgentSkillPublicationSpec {
                    instance_id: &instance_id,
                    owner_id: "pg-owner",
                    skill: "debug-logs",
                    published_by: "pg-admin",
                })
                .await
                .unwrap()
                .revoked_at
                .is_none()
        );
    }
}
