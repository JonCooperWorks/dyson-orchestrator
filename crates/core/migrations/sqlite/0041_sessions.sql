-- Opaque SPA sessions for dyson_swarm_session cookies.
--
-- Rollback: DROP INDEX idx_sessions_user; DROP TABLE sessions;

CREATE TABLE sessions (
  id            TEXT PRIMARY KEY,
  user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at    INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL,
  revoked_at    INTEGER
);

CREATE INDEX idx_sessions_user ON sessions(user_id);
