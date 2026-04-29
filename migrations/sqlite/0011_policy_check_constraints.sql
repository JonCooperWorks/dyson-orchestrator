-- Belt-and-braces non-negativity guards for the budget columns on
-- `user_policies`.  Application code already clamps reads/writes
-- (db/policies.rs:62-64), but a CHECK constraint at the DB layer
-- closes any path that bypasses the store (raw SQL in tests, future
-- admin tools, manual repair scripts).  Per the security review (G3)
-- a negative budget would invert the meaning of the cap.
--
-- SQLite doesn't support `ALTER TABLE ADD CONSTRAINT`, so we recreate
-- the table.  All columns + the PRIMARY KEY + FK to users are
-- preserved verbatim from migrations/sqlite/0002_multitenant.sql.
-- There are no secondary indexes on user_policies to recreate.
--
-- Note: `instance_policies` was dropped in migrations/sqlite/0002_multitenant.sql
-- when budgets moved per-user, so only `user_policies` needs the
-- guard today.

CREATE TABLE user_policies_new (
  user_id            TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  allowed_providers  TEXT NOT NULL,
  allowed_models     TEXT NOT NULL,
  daily_token_budget INTEGER CHECK (daily_token_budget IS NULL OR daily_token_budget >= 0),
  monthly_usd_budget REAL    CHECK (monthly_usd_budget IS NULL OR monthly_usd_budget >= 0),
  rps_limit          INTEGER
);

INSERT INTO user_policies_new
  (user_id, allowed_providers, allowed_models, daily_token_budget, monthly_usd_budget, rps_limit)
  SELECT user_id, allowed_providers, allowed_models, daily_token_budget, monthly_usd_budget, rps_limit
  FROM user_policies;

DROP TABLE user_policies;
ALTER TABLE user_policies_new RENAME TO user_policies;
