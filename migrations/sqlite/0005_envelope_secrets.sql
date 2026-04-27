-- Envelope-encrypted secrets at rest.
--
-- Three tables, all storing age-armored ciphertexts (`TEXT` because the
-- armored form is ASCII).  Plaintext lives only in process memory after
-- a `CipherDirectory::for_user(id).open(...)`; sqlite never sees it.
--
-- 1. `user_secrets`  — per-user opaque blobs, encrypted with the user's
--    own age key (SqlxUserSecretStore routes via CipherDirectory::for_user).
-- 2. `system_secrets` — global blobs (provider api_keys, OpenRouter
--    provisioning key), encrypted with the SYSTEM_KEY_ID cipher.
-- 3. `instance_secrets` — DROPPED + RECREATED with the new shape.
--    The wipe is intentional and confirmed by the operator: the old
--    table held plaintext values; we don't carry them across.  Operators
--    re-add via the SPA secrets panel.

CREATE TABLE user_secrets (
  user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  ciphertext  TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  PRIMARY KEY (user_id, name)
);

CREATE INDEX idx_user_secrets_user ON user_secrets(user_id);

CREATE TABLE system_secrets (
  name        TEXT PRIMARY KEY,
  ciphertext  TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);

-- Wipe + recreate instance_secrets with ciphertext column.  The old
-- (instance_id, name, value, created_at) shape was plaintext — confirmed
-- safe to drop because there are no production rows worth preserving
-- (single user, will re-add via SPA).  New rows are encrypted with the
-- instance OWNER's age key (per-user-key model: a stolen instance row
-- doesn't leak secrets without the owner's key file).
DROP TABLE IF EXISTS instance_secrets;

CREATE TABLE instance_secrets (
  instance_id TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  ciphertext  TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  PRIMARY KEY (instance_id, name)
);

CREATE INDEX idx_instance_secrets_instance ON instance_secrets(instance_id);
