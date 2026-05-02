-- Per-snapshot content hash for tamper detection (A3).  Computed
-- (e.g. SHA-256 over the on-disk archive) by the snapshot service
-- when a row is inserted or after the bytes are written; verified at
-- restore-time to detect a mutated path.  Nullable so existing rows
-- migrate cleanly — the hash is filled lazily on next snapshot or via
-- a one-shot rehash sweep.

ALTER TABLE snapshots ADD COLUMN content_hash TEXT;
