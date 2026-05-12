-- Postgres twin of migrations/sqlite/0028_webhook_signature_header.sql.
-- See the sqlite version for design rationale.

ALTER TABLE instance_webhooks
  ADD COLUMN signature_header TEXT NOT NULL DEFAULT 'x-swarm-signature';
