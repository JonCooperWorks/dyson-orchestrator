-- Postgres twin of migrations/sqlite/0013_snapshot_hash.sql.
-- See the sqlite version for the design rationale.

ALTER TABLE snapshots ADD COLUMN content_hash TEXT;
