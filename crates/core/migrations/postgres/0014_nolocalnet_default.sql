-- Postgres twin of migrations/sqlite/0014_nolocalnet_default.sql.
-- See the sqlite version for the design rationale: this rewrites
-- every existing 'open' instance row to 'nolocalnet' to close the
-- cloud-metadata egress vector flagged in the security review (A1).

UPDATE instances SET network_policy_kind = 'nolocalnet' WHERE network_policy_kind = 'open';
