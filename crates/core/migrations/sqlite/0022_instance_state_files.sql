CREATE TABLE IF NOT EXISTS instance_state_files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    instance_id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    namespace TEXT NOT NULL,
    path TEXT NOT NULL,
    mime TEXT,
    bytes INTEGER NOT NULL DEFAULT 0,
    body_path TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    synced_at INTEGER NOT NULL,
    deleted_at INTEGER,
    UNIQUE(instance_id, namespace, path)
);

CREATE INDEX IF NOT EXISTS idx_instance_state_files_instance
    ON instance_state_files(instance_id, namespace);

CREATE INDEX IF NOT EXISTS idx_instance_state_files_owner
    ON instance_state_files(owner_id, instance_id);
