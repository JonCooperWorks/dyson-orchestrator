CREATE TABLE instance_channels (
  id BIGSERIAL PRIMARY KEY,
  instance_id TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  handle TEXT NOT NULL,
  secret_name TEXT NOT NULL,
  webhook_secret_name TEXT NOT NULL,
  enabled BOOLEAN NOT NULL DEFAULT TRUE,
  last_inbound_at BIGINT,
  created_at BIGINT NOT NULL,
  UNIQUE(instance_id, kind)
);

CREATE INDEX idx_instance_channels_instance
  ON instance_channels(instance_id);

CREATE TABLE instance_channel_deliveries (
  id BIGSERIAL PRIMARY KEY,
  instance_id TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  received_at BIGINT NOT NULL,
  status INTEGER NOT NULL,
  preview TEXT NOT NULL
);

CREATE INDEX idx_instance_channel_deliveries_recent
  ON instance_channel_deliveries(instance_id, kind, received_at DESC, id DESC);
