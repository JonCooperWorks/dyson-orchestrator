UPDATE secret_access_audit
SET owner_id = (
  SELECT instances.owner_id
  FROM instances
  WHERE instances.id = secret_access_audit.instance_id
)
WHERE (owner_id IS NULL OR owner_id = '')
  AND instance_id IS NOT NULL
  AND instance_id != ''
  AND EXISTS (
    SELECT 1
    FROM instances
    WHERE instances.id = secret_access_audit.instance_id
  );
