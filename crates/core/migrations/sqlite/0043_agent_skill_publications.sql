-- Explicit public opt-in for skills learned or authored inside an agent.
--
-- Agent skill bodies can contain environment-specific instructions or
-- credential hints, so mirrored workspace skills must not be projected into
-- the public marketplace unless a user/admin publishes them deliberately.

CREATE TABLE IF NOT EXISTS agent_skill_publications (
  instance_id  TEXT    NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
  owner_id     TEXT    NOT NULL,
  skill        TEXT    NOT NULL,
  published_by TEXT    NOT NULL,
  published_at INTEGER NOT NULL,
  revoked_at   INTEGER,
  PRIMARY KEY (instance_id, skill)
);

CREATE INDEX IF NOT EXISTS idx_agent_skill_publications_public
  ON agent_skill_publications(revoked_at, instance_id, skill);

CREATE INDEX IF NOT EXISTS idx_agent_skill_publications_owner
  ON agent_skill_publications(owner_id, revoked_at, instance_id, skill);
