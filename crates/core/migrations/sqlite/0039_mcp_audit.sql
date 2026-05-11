-- MCP proxy forwards carry paid upstream access and need the same forensic
-- shape as llm_audit: insert before forwarding, stamp completion after the
-- response status is known.
--
-- Rollback: DROP INDEX idx_mcp_audit_owner_server_ts; DROP TABLE mcp_audit;
-- this loses only MCP audit history and does not mutate live instance state.

CREATE TABLE IF NOT EXISTS mcp_audit (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  owner_id      TEXT NOT NULL REFERENCES users(id),
  instance_id   TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
  server_name   TEXT NOT NULL,
  tool          TEXT,
  status        INTEGER NOT NULL,
  duration_ms   INTEGER NOT NULL,
  ts            INTEGER NOT NULL,
  completed     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_mcp_audit_owner_server_ts
  ON mcp_audit(owner_id, server_name, ts);
