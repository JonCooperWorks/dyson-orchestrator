-- Postgres twin of migrations/sqlite/0006_user_openrouter.sql.
-- See the sqlite version for the design rationale.
--
-- Differences from the sqlite version:
--  * DOUBLE PRECISION instead of REAL for the USD limit so usage rollups
--    don't lose precision under aggregation.
--
-- KEEP THIS SCHEMA IN LOCKSTEP WITH migrations/sqlite/0006_user_openrouter.sql.

ALTER TABLE users ADD COLUMN openrouter_key_id TEXT;
ALTER TABLE users ADD COLUMN openrouter_key_limit_usd DOUBLE PRECISION NOT NULL DEFAULT 10.0;

CREATE INDEX idx_users_openrouter_key_id ON users(openrouter_key_id)
  WHERE openrouter_key_id IS NOT NULL;
