CREATE TABLE secret_access_audit (
  timestamp     INTEGER NOT NULL,
  actor_kind    TEXT NOT NULL,
  actor_id      TEXT,
  reason        TEXT NOT NULL,
  operation     TEXT NOT NULL,
  scope         TEXT NOT NULL,
  owner_id      TEXT,
  instance_id   TEXT,
  secret_name   TEXT,
  key_id        TEXT,
  key_version   INTEGER,
  result        TEXT NOT NULL,
  error_class   TEXT,
  error_message TEXT
);

CREATE INDEX secret_access_audit_ts_idx
  ON secret_access_audit(timestamp);

CREATE INDEX secret_access_audit_scope_idx
  ON secret_access_audit(scope, owner_id, instance_id);
