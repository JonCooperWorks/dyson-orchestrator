-- Postgres twin of migrations/sqlite/0012_audit_completion.sql.
-- See the sqlite version for the design rationale.
--
-- Differences from the sqlite version:
--  * BOOLEAN proper instead of INTEGER 0/1.

ALTER TABLE llm_audit ADD COLUMN completed BOOLEAN NOT NULL DEFAULT TRUE;
