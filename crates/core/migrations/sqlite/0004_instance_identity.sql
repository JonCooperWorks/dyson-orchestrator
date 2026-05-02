-- Employee-shaped instances: each Dyson now carries a human-readable
-- name and a free-text task / mission statement.  Both survive
-- snapshot+restore and are seeded into the sandbox env on create as
-- WARDEN_NAME and WARDEN_TASK.  Per the design: warden seeds these
-- on first boot; the agent owns its identity from then on (writes
-- SOUL.md and friends), so subsequent edits in warden don't
-- propagate to a running sandbox without an explicit re-onboard.
--
-- Defaults are empty strings so existing rows migrate cleanly without
-- a backfill pass — they'll show as anonymous instances in the UI
-- until renamed.
ALTER TABLE instances ADD COLUMN name TEXT NOT NULL DEFAULT '';
ALTER TABLE instances ADD COLUMN task TEXT NOT NULL DEFAULT '';
