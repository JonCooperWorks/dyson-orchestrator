expl# Audit

Swarm keeps forensic audit state on the host, not inside the cube. Sandboxes
are disposable; audit rows live with the durable swarm state and survive
instance rotation, recreate, and cube replacement.

## Audit Surfaces

The main audit surfaces are:

- `llm_audit`: one row per proxied LLM request, including provider, model,
  status, token counts when available, and completion state.
- `mcp_audit`: one row per MCP proxy transport call, including owner,
  instance, server, tool, status, duration, and completion state.
- `llm_tool_call`: one row per model-emitted tool call, with the sealed
  tool input and the sealed result attached after the following turn.
- `secret_access_audit`: one row per audited local KMS secret access operation,
  including scope, owner/instance attribution, key metadata, result, and
  redacted error details.

Do not fold these tables together. They answer related but different
questions: "which model call happened?", "which MCP transport call happened?",
"which tool did the model ask the agent to run, with what input/result?", and
"which secret material did the local KMS open, seal, rewrap, rotate, or delete?"

## LLM Tool-Call Audit

`llm_tool_call` captures every `tool_use -> tool_result` pair that passes
through the LLM proxy:

- native tools such as `bash` and editor tools
- MCP-routed tools named as `mcp__{server}__{tool}`
- Anthropic SSE `tool_use` blocks
- OpenAI/OpenRouter SSE `choices[].delta.tool_calls[]` function calls

The call side is inserted while streaming the model response. The result side
is attached from the next request body when Dyson sends the prior
`tool_result` back to the model. Until the result arrives, `resulted_at` and
`is_error` are null.

Payload size is capped before sealing. Oversized input or result JSON is
replaced with a sealed JSON marker containing `_truncated: true` and a UTF-8
prefix. There is no plaintext payload search index.

## Encryption Model

Tool-call input and result payloads are sealed with the owner's age identity,
using the same envelope pattern as user secrets. Metadata stays plaintext:

- owner id
- instance id
- tool use id
- tool name
- MCP server name when parsed from `mcp__...`
- timestamps
- status/error bit
- foreign keys to `llm_audit` and best-effort `mcp_audit`

This is IDOR, SQL injection, and database-exfiltration protection. It is not
operator protection: a host operator with access to the per-owner age identity
can decrypt payloads through the normal server-side paths.

## KMS Secret Access Audit

`secret_access_audit` is the local KMS audit trail. It records successful and
failed secret access operations without storing plaintext secret values.

Audited operations are `encrypt`, `decrypt`, `rewrap`, `rotate`, and `delete`.
Rows include:

- `actor_kind` and `actor_id`: the caller identity such as `runtime`,
  `system`, `operator_cli`, or `test`. Runtime actor ids identify the runtime
  path and are not ownership authority.
- `reason`: the code path, such as `LlmProviderProxy`, `McpProxyForward`,
  `RuntimeConfigurePush`, `StateReplay`, `ArtefactRead`, `Migration`, or
  `OperatorCli`.
- `scope`: the KMS scope, such as `runtime_token`, `system_secret`,
  `system_configure`, `user_secret`, `user_api_key`, `user_profile`,
  `state_file`, `artefact`, `webhook_delivery`, or `llm_tool_call`.
- `owner_id`, `instance_id`, and `secret_name`: attribution copied from the
  KMS context. `secret_name` is the logical name already known to the caller,
  for example `proxy_token:<provider>`; it is not a secret value.
- `key_id` and `key_version`: envelope key metadata when available.
- `result`, `error_class`, and `error_message`: success/failure status with a
  short redacted error message.

Owner attribution must come from durable ownership state, not from the runtime
actor. Runtime proxy tokens are instance-scoped, so new `runtime_token` rows
carry the owning `instances.owner_id` whenever the token belongs to an
instance. Older no-owner runtime-token envelopes remain readable: swarm first
tries the owner-aware KMS context, falls back to the legacy no-owner context
only on a context mismatch, and lazily rewraps successful legacy opens under the
owner-aware context.

Migration `0050_secret_access_audit_owner_backfill` fills missing
`secret_access_audit.owner_id` values from `instances.owner_id` when an audit
row has a non-empty `instance_id` that still matches an `instances.id`. It is
idempotent. It does not invent an owner for system-only rows with no instance,
or for rows whose instance no longer exists.

Admin-only listing is exposed at:

```text
GET /v1/admin/kms/audit
```

Supported filters are `scope`, `owner_id`, `instance_id`, `secret_name`,
`operation`, `result`, `reason`, `since`, `until`, `limit`, and `offset`.

Host-side verification should aggregate metadata only:

```sh
sudo sqlite3 /var/lib/dyson-swarm/state.db \
  "SELECT scope, COUNT(*) AS total, SUM(CASE WHEN owner_id IS NULL OR owner_id = '' THEN 1 ELSE 0 END) AS missing_owner FROM secret_access_audit GROUP BY scope ORDER BY total DESC;"
```

After the owner backfill, this query should return no rows for still-linked
instance-scoped audit entries:

```sh
sudo sqlite3 /var/lib/dyson-swarm/state.db \
  "SELECT scope, COUNT(*) FROM secret_access_audit AS saa WHERE (owner_id IS NULL OR owner_id = '') AND instance_id IS NOT NULL AND instance_id != '' AND EXISTS (SELECT 1 FROM instances AS i WHERE i.id = saa.instance_id) GROUP BY scope ORDER BY scope;"
```

## MCP Cross-Linking

For tool names that start with `mcp__`, swarm parses the server and tool name
and tries to link the `llm_tool_call` row to the matching `mcp_audit` row.

The link is best effort. The LLM tool-call row and MCP transport row are
written from different request paths, so swarm retries the link briefly and
does not fail user traffic if no transport row is found.

## API

Tenant-authenticated per-instance routes:

```text
GET /v1/instances/:id/audit/tool-calls
GET /v1/instances/:id/audit/tool-calls/export
GET /v1/instances/:id/audit/tool-calls/facets
GET /v1/instances/:id/audit/tool-calls/stream
```

Supported list/export query parameters:

- `tool=<name>`
- `status=all|ok|err`
- `server=<mcp_server>`
- `q=<substring>`
- `before=<row_id>`
- `limit=<n>`; default 100, max 500

The server enforces instance ownership before reading rows. Decryption happens
server-side; the browser and API client receive plaintext JSON only after auth
and ownership checks pass.

`/stream` is SSE. It sends the last 50 matching rows in follow order, then
polls for new rows every second and emits `event: tool_call`. It sends a
heartbeat comment every 15 seconds.

`/export` returns NDJSON for the current filters. It is intentionally explicit
and uses the same decrypted row shape as the list endpoint.

`/facets` returns instance-wide distinct tool names and MCP server names. The
Activity UI uses it to populate searchable filter suggestions even when the
current `status`, `tool`, `server`, or payload search filter has no matching
rows.

## Web UI

The instance detail page has an Activity tab. It shows a live timeline of tool
calls with searchable filters for tool, status, MCP server, and decrypted
payload search. Filter no-match states keep the controls visible; only a truly
empty audit history shows the first-run empty state.

Rows show call time, tool name, duration when paired, status, and a short input
preview. Opening a row shows the full decrypted input/result JSON plus MCP
transport status and duration when a matching `mcp_audit` row was linked.

The UI keeps only a bounded in-memory tail. It is an operator/user visibility
surface, not the retention policy.

## What This Does Not Capture

This audit is specifically for tools invoked through model tool-use protocol
messages. It does not capture:

- arbitrary shell commands run outside an LLM `tool_use`
- cross-instance rollups
- retention or TTL decisions
- blind indexes or searchable encrypted payload columns

Those can be added later without changing the `llm_tool_call` contract.

## Operational Checks

After deploying a change in this area:

1. Run `cargo test -p dyson-swarm recording_body::tests`.
2. Run `cargo test -p dyson-swarm http::tests::tool_call_audit`.
3. Run the web tests for `activity.test.jsx` or `npm run build` in
   `crates/swarm/src/http/web`.
4. Redeploy swarm and verify:

```sh
curl -fsS -H "Authorization: Bearer $SWARM_API_KEY" \
  "http://$DYSON_CUBE_GATEWAY_IP:$DYSON_SWARM_PORT/v1/instances/$INSTANCE_ID/audit/tool-calls?limit=5"
```

An idle instance may legitimately return an empty `items` array. A forced or
real model tool call should create a row within a few seconds, and the next
request carrying the matching `tool_result` should attach the result side.
