CREATE TABLE instance_channels (
  id INTEGER PRIMARY KEY,
  instance_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  handle TEXT NOT NULL,
  secret_name TEXT NOT NULL,
  webhook_secret_name TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  last_inbound_at INTEGER,
  created_at INTEGER NOT NULL,
  UNIQUE(instance_id, kind),
  FOREIGN KEY(instance_id) REFERENCES instances(id) ON DELETE CASCADE
);

CREATE INDEX idx_instance_channels_instance
  ON instance_channels(instance_id);

CREATE TABLE instance_channel_deliveries (
  id INTEGER PRIMARY KEY,
  instance_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  received_at INTEGER NOT NULL,
  status INTEGER NOT NULL,
  preview TEXT NOT NULL,
  FOREIGN KEY(instance_id) REFERENCES instances(id) ON DELETE CASCADE
);

CREATE INDEX idx_instance_channel_deliveries_recent
  ON instance_channel_deliveries(instance_id, kind, received_at DESC, id DESC);
