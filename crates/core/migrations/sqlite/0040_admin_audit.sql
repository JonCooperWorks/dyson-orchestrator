-- Admin user mutations must leave a forensic row before state changes are
-- applied.  params_hash stores a SHA-256 digest of canonical route params;
-- plaintext request parameters and secrets never land in this table.
--
-- Rollback: DROP INDEX idx_admin_audit_target_ts; DROP TABLE admin_audit;
-- this loses only admin audit history and does not mutate live user state.

CREATE TABLE IF NOT EXISTS admin_audit (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  actor_subject  TEXT NOT NULL,
  action         TEXT NOT NULL,
  target_user    TEXT NOT NULL,
  params_hash    TEXT NOT NULL,
  ts             INTEGER NOT NULL
);

CREATE INDEX idx_admin_audit_target_ts
  ON admin_audit(target_user, ts);
