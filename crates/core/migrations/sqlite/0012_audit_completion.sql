-- Mark llm_audit rows as completed once the upstream call has fully
-- streamed (D1).  The proxy now writes the row up-front with
-- `completed = 0` (so a crash mid-stream still leaves a forensic
-- trail) and stamps `completed = 1` + the final `output_tokens`
-- count via `update_completion` once the body finishes.
--
-- `output_tokens` already exists from migrations/sqlite/0001_init.sql;
-- not redeclared here.  No USD column — pricing is intentionally
-- omitted for this demo deployment, so monthly_usd_budget enforcement
-- is a no-op in db/audit.rs::monthly_usd.

ALTER TABLE llm_audit ADD COLUMN completed INTEGER NOT NULL DEFAULT 1;
