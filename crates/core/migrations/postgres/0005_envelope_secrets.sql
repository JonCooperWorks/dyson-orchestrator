-- Postgres twin of migrations/sqlite/0005_envelope_secrets.sql.
-- See the sqlite version for the design rationale.
--
-- Differences from the sqlite version:
--  * BIGINT for unix-epoch timestamps (Pg needs the right type from the
--    start; sqlite is liberal).
--
-- KEEP THIS SCHEMA IN LOCKSTEP WITH migrations/sqlite/0005_envelope_secrets.sql.

CREATE TABLE user_secrets (
  user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  ciphertext  TEXT NOT NULL,
  created_at  BIGINT NOT NULL,
  updated_at  BIGINT NOT NULL,
  PRIMARY KEY (user_id, name)
);

CREATE INDEX idx_user_secrets_user ON user_secrets(user_id);

CREATE TABLE system_secrets (
  name        TEXT PRIMARY KEY,
  ciphertext  TEXT NOT NULL,
  created_at  BIGINT NOT NULL,
  updated_at  BIGINT NOT NULL
);

-- Wipe + recreate instance_secrets with ciphertext column.  See the
-- sqlite twin for why dropping the old plaintext rows is safe.
DROP TABLE IF EXISTS instance_secrets;

CREATE TABLE instance_secrets (
  instance_id TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  ciphertext  TEXT NOT NULL,
  created_at  BIGINT NOT NULL,
  updated_at  BIGINT NOT NULL,
  PRIMARY KEY (instance_id, name)
);

CREATE INDEX idx_instance_secrets_instance ON instance_secrets(instance_id);
