-- Rename the Docker MCP template metadata column from the old generic
-- to the new semantic name.  Nothing structural changes — just the
-- column label the service layer writes/reads.

ALTER TABLE mcp_docker_catalog
  RENAME COLUMN template_metadata TO placeholder_values;
