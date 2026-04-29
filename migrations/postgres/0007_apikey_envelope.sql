-- Postgres twin of migrations/sqlite/0007_apikey_envelope.sql.
-- See the sqlite version for the design rationale.
--
-- Differences from the sqlite version:
--  * BIGINT for unix-epoch timestamps.
--
-- KEEP THIS SCHEMA IN LOCKSTEP WITH migrations/sqlite/0007_apikey_envelope.sql.

DROP INDEX IF EXISTS idx_user_api_keys_user;
DROP TABLE user_api_keys;

CREATE TABLE user_api_keys (
  id          TEXT PRIMARY KEY,
  user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  prefix      TEXT NOT NULL,        -- first 8 hex chars of the random part
  ciphertext  TEXT NOT NULL,        -- age-armored sealed token (per-user key)
  label       TEXT,
  created_at  BIGINT NOT NULL,
  revoked_at  BIGINT
);

CREATE INDEX idx_user_api_keys_prefix ON user_api_keys(prefix);
CREATE INDEX idx_user_api_keys_user ON user_api_keys(user_id);
