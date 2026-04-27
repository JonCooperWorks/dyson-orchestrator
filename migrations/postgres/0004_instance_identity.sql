-- Twin of migrations/sqlite/0004_instance_identity.sql; kept in
-- lockstep so the postgres backend (gated behind the `postgres`
-- cargo feature) sees the same schema.  See the sqlite file for the
-- design rationale.
ALTER TABLE instances ADD COLUMN name TEXT NOT NULL DEFAULT '';
ALTER TABLE instances ADD COLUMN task TEXT NOT NULL DEFAULT '';
