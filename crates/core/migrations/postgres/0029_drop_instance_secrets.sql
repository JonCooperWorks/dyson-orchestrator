-- Per-instance sandbox secrets are no longer a surface.
-- Instance-level config is sealed inside system_secrets;
-- per-user secrets go through user_secrets.
DROP TABLE IF EXISTS instance_secrets;
