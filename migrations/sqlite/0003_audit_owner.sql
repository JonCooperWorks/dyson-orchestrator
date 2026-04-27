-- Per-user budget rollups need to sum llm_audit rows by owner, not by
-- instance, so daily/monthly limits hold across all of a tenant's instances.
-- Adding owner_id directly avoids a JOIN on the hot path of every LLM call.

ALTER TABLE llm_audit ADD COLUMN owner_id TEXT NOT NULL DEFAULT 'legacy';

CREATE INDEX idx_llm_audit_owner ON llm_audit(owner_id, occurred_at);
