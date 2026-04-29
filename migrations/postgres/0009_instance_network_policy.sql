-- Twin of migrations/sqlite/0009_instance_network_policy.sql; see
-- that file for the design rationale.  Kept in lockstep so the
-- postgres backend (gated behind the `postgres` cargo feature) sees
-- the same schema.
ALTER TABLE instances ADD COLUMN network_policy_kind     TEXT NOT NULL DEFAULT 'open';
ALTER TABLE instances ADD COLUMN network_policy_entries  TEXT NOT NULL DEFAULT '';
ALTER TABLE instances ADD COLUMN network_policy_cidrs    TEXT NOT NULL DEFAULT '';
