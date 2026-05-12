-- Postgres twin of migrations/sqlite/0026_mcp_docker_catalog.sql.
-- See the sqlite version for design rationale.

CREATE TABLE IF NOT EXISTS mcp_docker_catalog (
  id            BIGSERIAL PRIMARY KEY,
  server_name   TEXT   NOT NULL UNIQUE,
  display_name  TEXT   NOT NULL,
  category      TEXT   NOT NULL DEFAULT 'other',
  description   TEXT   NOT NULL DEFAULT '',
  image         TEXT   NOT NULL,
  command       TEXT[] NOT NULL,
  env           JSONB  NOT NULL DEFAULT '{}',
  placeholder_values JSONB NOT NULL DEFAULT '{}',
  created_at    BIGINT NOT NULL,
  updated_at    BIGINT NOT NULL
);
