-- Per-instance forensic log for model-driven tool calls. Payloads are
-- owner-sealed before they enter this table; metadata stays plaintext so
-- operators can page, filter, and correlate without decrypting every row.
--
-- Rollback: DROP INDEX idx_llm_tool_call_use_id; DROP INDEX idx_llm_tool_call_owner_called; DROP TABLE llm_tool_call;
-- this loses only LLM tool-call audit history and does not mutate live instance state.

CREATE TABLE IF NOT EXISTS llm_tool_call (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  llm_audit_id    INTEGER REFERENCES llm_audit(id),
  owner_id        TEXT NOT NULL REFERENCES users(id),
  instance_id     TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
  tool_use_id     TEXT NOT NULL,
  tool_name       TEXT NOT NULL,
  mcp_server      TEXT,
  input_sealed    BLOB,
  result_sealed   BLOB,
  is_error        INTEGER CHECK (is_error IS NULL OR is_error IN (0, 1)),
  called_at       INTEGER NOT NULL,
  resulted_at     INTEGER,
  mcp_audit_id    INTEGER REFERENCES mcp_audit(id)
);

CREATE INDEX idx_llm_tool_call_owner_called
  ON llm_tool_call(owner_id, instance_id, called_at);

CREATE INDEX idx_llm_tool_call_use_id
  ON llm_tool_call(tool_use_id);
