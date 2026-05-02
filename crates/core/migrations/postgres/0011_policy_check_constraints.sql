-- Belt-and-braces non-negativity guards for the budget columns on
-- `user_policies`.  Application code already clamps reads/writes
-- (db/policies.rs:62-64), but a CHECK constraint at the DB layer
-- closes any path that bypasses the store (raw SQL in tests, future
-- admin tools, manual repair scripts).  Per the security review (G3)
-- a negative budget would invert the meaning of the cap.
--
-- Note: `instance_policies` was dropped in migrations/postgres/0002_multitenant.sql
-- when budgets moved per-user, so only `user_policies` needs the
-- guard today.

ALTER TABLE user_policies
  ADD CONSTRAINT user_policies_daily_token_budget_nonneg
  CHECK (daily_token_budget IS NULL OR daily_token_budget >= 0);

ALTER TABLE user_policies
  ADD CONSTRAINT user_policies_monthly_usd_budget_nonneg
  CHECK (monthly_usd_budget IS NULL OR monthly_usd_budget >= 0);
