-- Multi-tenancy.
--
-- The original schema treated every API caller as god-mode. This migration
-- introduces:
--   * `users` — the tenant identity (sourced from OIDC `sub`)
--   * `user_api_keys` — opaque bearer fallback for CI/admin paths that can't
--     do an OIDC flow
--   * `owner_id` columns on `instances` and `snapshots` so every row belongs
--     to exactly one user
--   * `user_policies` replacing `instance_policies` because budgets and
--     allowlists are scoped per-user, not per-instance
--
-- Existing rows (none in production yet — this lands before the multi-tenant
-- build ships) are migrated to a synthetic `legacy` user so foreign keys
-- hold; admins must reassign before activating real tenants.

CREATE TABLE users (
  id              TEXT PRIMARY KEY,           -- internal id (uuid)
  subject         TEXT NOT NULL UNIQUE,        -- OIDC `sub` claim or admin-issued key id
  email           TEXT,
  display_name    TEXT,
  status          TEXT NOT NULL DEFAULT 'inactive', -- 'inactive' | 'active' | 'suspended'
  created_at      INTEGER NOT NULL,
  activated_at    INTEGER,
  last_seen_at    INTEGER
);

CREATE INDEX idx_users_subject ON users(subject);
CREATE INDEX idx_users_status ON users(status);

CREATE TABLE user_api_keys (
  token           TEXT PRIMARY KEY,
  user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  label           TEXT,
  created_at      INTEGER NOT NULL,
  revoked_at      INTEGER
);

CREATE INDEX idx_user_api_keys_user ON user_api_keys(user_id);

-- Seed a `legacy` user so existing instances/snapshots can have a non-null
-- owner_id during the migration. Admins reassign before activating real
-- tenants.
INSERT INTO users (id, subject, email, display_name, status, created_at, activated_at)
  VALUES ('legacy', 'legacy', NULL, 'Legacy (pre-tenancy rows)', 'suspended',
          strftime('%s','now'), strftime('%s','now'));

-- Add owner_id to instances. SQLite doesn't support ADD COLUMN with FK to
-- another table after the table exists in one statement, so we backfill via
-- a default literal and then validate at the application layer.
ALTER TABLE instances ADD COLUMN owner_id TEXT NOT NULL DEFAULT 'legacy'
  REFERENCES users(id);

ALTER TABLE snapshots ADD COLUMN owner_id TEXT NOT NULL DEFAULT 'legacy'
  REFERENCES users(id);

CREATE INDEX idx_instances_owner ON instances(owner_id);
CREATE INDEX idx_snapshots_owner ON snapshots(owner_id);

-- Replace instance_policies with user_policies. Drop the old table — no
-- production data existed before this migration.
DROP TABLE instance_policies;

CREATE TABLE user_policies (
  user_id            TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  allowed_providers  TEXT NOT NULL,
  allowed_models     TEXT NOT NULL,
  daily_token_budget INTEGER,
  monthly_usd_budget REAL,
  rps_limit          INTEGER
);
