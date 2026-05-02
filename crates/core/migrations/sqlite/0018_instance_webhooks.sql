-- Per-instance webhook tasks.
--
-- A "task" (UI label) / "webhook" (code label) is a named handler
-- exposed off `/webhooks/<instance_id>/<name>` that, when called and
-- signature-verified, kicks off a fresh agent conversation seeded
-- with the operator-authored `description` plus the inbound payload.
--
-- Identity collision note: `instances.task` (singular) is the
-- IDENTITY.md mission brief.  This new table is an unrelated noun;
-- keeping the column names distinct avoids confusion in queries.
--
-- `secret_name` points into `instance_secrets` — we don't duplicate
-- secret storage, the signing key reuses the same owner-sealed
-- envelope every other per-instance secret uses.  Convention: rows
-- written by the webhooks layer are named `_webhook_<webhook_name>`
-- (leading underscore = managed, the SPA hides them in the regular
-- secrets panel).

CREATE TABLE instance_webhooks (
  instance_id TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  auth_scheme TEXT NOT NULL,
  secret_name TEXT,
  enabled     INTEGER NOT NULL DEFAULT 1,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  PRIMARY KEY (instance_id, name)
);

CREATE INDEX instance_webhooks_enabled_idx
  ON instance_webhooks(instance_id, enabled);

-- Delivery audit log.  Metadata only — we deliberately do NOT persist
-- request bodies (they may carry payloads the agent then acts on, and
-- we don't want a second copy at rest).  Rows are written in every
-- terminal arm of `verify_and_dispatch` — verify-fail, dispatch-fail,
-- and dispatch-ok — so a failed signature shows up here too.
--
-- `signature_ok=0` + `status_code=401` is the verify-fail shape;
-- `signature_ok=1` + `status_code=2xx` is the happy path.

CREATE TABLE webhook_deliveries (
  id            TEXT PRIMARY KEY,
  instance_id   TEXT NOT NULL,
  webhook_name  TEXT NOT NULL,
  fired_at      INTEGER NOT NULL,
  status_code   INTEGER NOT NULL,
  latency_ms    INTEGER NOT NULL,
  request_id    TEXT,
  signature_ok  INTEGER NOT NULL,
  error         TEXT,
  FOREIGN KEY (instance_id, webhook_name)
    REFERENCES instance_webhooks(instance_id, name) ON DELETE CASCADE
);

CREATE INDEX webhook_deliveries_lookup_idx
  ON webhook_deliveries(instance_id, webhook_name, fired_at DESC);
