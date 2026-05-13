-- Postgres twin of migrations/sqlite/0043_agent_skill_publications.sql.
-- See the sqlite version for design rationale.

CREATE TABLE IF NOT EXISTS agent_skill_publications (
  instance_id  TEXT   NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
  owner_id     TEXT   NOT NULL,
  skill        TEXT   NOT NULL,
  published_by TEXT   NOT NULL,
  published_at BIGINT NOT NULL,
  revoked_at   BIGINT,
  PRIMARY KEY (instance_id, skill)
);

CREATE INDEX IF NOT EXISTS idx_agent_skill_publications_public
  ON agent_skill_publications(revoked_at, instance_id, skill);

CREATE INDEX IF NOT EXISTS idx_agent_skill_publications_owner
  ON agent_skill_publications(owner_id, revoked_at, instance_id, skill);
