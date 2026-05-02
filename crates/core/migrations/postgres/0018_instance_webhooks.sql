-- Postgres twin of migrations/sqlite/0018_instance_webhooks.sql.
-- See the sqlite version for design rationale.
--
-- Differences from the sqlite version:
--  * BIGINT for unix-epoch timestamps and integer status/latency
--    where Postgres needs the right type from the start.
--  * BOOLEAN in place of INTEGER for `enabled` / `signature_ok`.
--
-- KEEP THIS SCHEMA IN LOCKSTEP WITH migrations/sqlite/0018_instance_webhooks.sql.

CREATE TABLE instance_webhooks (
  instance_id TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  auth_scheme TEXT NOT NULL,
  secret_name TEXT,
  enabled     BOOLEAN NOT NULL DEFAULT TRUE,
  created_at  BIGINT NOT NULL,
  updated_at  BIGINT NOT NULL,
  PRIMARY KEY (instance_id, name)
);

CREATE INDEX instance_webhooks_enabled_idx
  ON instance_webhooks(instance_id, enabled);

CREATE TABLE webhook_deliveries (
  id            TEXT PRIMARY KEY,
  instance_id   TEXT NOT NULL,
  webhook_name  TEXT NOT NULL,
  fired_at      BIGINT NOT NULL,
  status_code   INTEGER NOT NULL,
  latency_ms    INTEGER NOT NULL,
  request_id    TEXT,
  signature_ok  BOOLEAN NOT NULL,
  error         TEXT,
  FOREIGN KEY (instance_id, webhook_name)
    REFERENCES instance_webhooks(instance_id, name) ON DELETE CASCADE
);

CREATE INDEX webhook_deliveries_lookup_idx
  ON webhook_deliveries(instance_id, webhook_name, fired_at DESC);
