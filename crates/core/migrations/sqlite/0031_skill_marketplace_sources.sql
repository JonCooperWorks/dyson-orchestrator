-- DB-backed skill marketplace source registry.
--
-- Marketplace indexes are discovery metadata; installed skill bodies still
-- live in each Dyson workspace and are mirrored by the existing state-files
-- flow.  Keeping sources in SQLite lets operators add, disable, or repair
-- marketplace feeds without redeploying swarm config.

CREATE TABLE IF NOT EXISTS skill_marketplace_sources (
  id                   TEXT    PRIMARY KEY,
  source_type          TEXT    NOT NULL CHECK (source_type IN ('file', 'http')),
  location             TEXT    NOT NULL,
  enabled              INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  created_at           INTEGER NOT NULL,
  updated_at           INTEGER NOT NULL,
  deleted_at           INTEGER,
  last_fetch_at        INTEGER,
  last_success_at      INTEGER,
  last_error           TEXT
);

CREATE INDEX IF NOT EXISTS idx_skill_marketplace_sources_visible
  ON skill_marketplace_sources(deleted_at, enabled, id);
