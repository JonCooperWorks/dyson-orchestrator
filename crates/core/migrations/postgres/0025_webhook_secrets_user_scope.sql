-- Postgres twin of migrations/sqlite/0025_webhook_secrets_user_scope.sql.
-- See the sqlite version for design rationale.

ALTER TABLE instance_webhooks
  ADD COLUMN secret_name TEXT;
