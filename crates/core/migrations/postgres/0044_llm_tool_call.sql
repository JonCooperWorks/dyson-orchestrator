-- Postgres twin of migrations/sqlite/0044_llm_tool_call.sql.
--
-- Rollback: DROP INDEX idx_llm_tool_call_use_id; DROP INDEX idx_llm_tool_call_owner_called; DROP TABLE llm_tool_call;
-- this loses only LLM tool-call audit history and does not mutate live instance state.

CREATE TABLE IF NOT EXISTS llm_tool_call (
  id              BIGSERIAL PRIMARY KEY,
  llm_audit_id    BIGINT REFERENCES llm_audit(id),
  owner_id        TEXT NOT NULL REFERENCES users(id),
  instance_id     TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
  tool_use_id     TEXT NOT NULL,
  tool_name       TEXT NOT NULL,
  mcp_server      TEXT,
  input_sealed    BYTEA,
  result_sealed   BYTEA,
  is_error        BIGINT CHECK (is_error IS NULL OR is_error IN (0, 1)),
  called_at       BIGINT NOT NULL,
  resulted_at     BIGINT,
  mcp_audit_id    BIGINT REFERENCES mcp_audit(id)
);

CREATE INDEX idx_llm_tool_call_owner_called
  ON llm_tool_call(owner_id, instance_id, called_at);

CREATE INDEX idx_llm_tool_call_use_id
  ON llm_tool_call(tool_use_id);
