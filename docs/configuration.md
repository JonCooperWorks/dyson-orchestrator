# Configuration

Swarm reads a single TOML file, typically
`/etc/dyson-swarm/config.toml`. The example file in the repo is
[config.example.toml](../config.example.toml).

## Required Basics

- `bind`: host:port for the Axum server
- `db_path`: SQLite database path
- `[cube]`: Cube API URL, API key, and sandbox domain
- `[backup]`: backup sink selection and local cache directory

## Important Optional Fields

### `hostname`

Public apex hostname, for example `swarm.example.com`.

When set:

- each Dyson is reachable at `<instance_id>.<hostname>`
- the SPA's `open` link points there
- share pages live on `share.<hostname>`
- MCP OAuth can build public callback URLs

When unset:

- the REST API and SPA still work
- host-based per-Dyson browsing is disabled
- MCP OAuth start will fail with a clear message because no callback URL can be built

### `cube_facing_addr`

Host address the sandbox should use to reach swarm’s `/llm` proxy, usually
something like `192.168.0.1:8080`.

This is separate from `hostname` because the sandbox may not be able to hairpin
through the host’s public address cleanly.

### `default_template_id`

Default Cube template id for new hires and the reference point for startup
binary rotation.

### `cube_profiles`

Named Cube-template choices surfaced to the SPA. These are operator UX
metadata around pre-registered templates.

## Auth Configuration

### `[oidc]`

Controls backend JWT verification and the SPA browser login flow.

- `issuer`
- `audience`
- optional `jwks_url`
- `jwks_ttl_seconds`
- optional `spa_client_id`
- optional `spa_scopes`
- optional `[oidc.roles]` block for admin-role gating

If `[oidc]` is omitted, swarm can still be used through opaque user API keys,
but browser login and admin-role-based access are unavailable.

## Provider Configuration

`[providers.<name>]` configures upstream LLM providers. In production, the
recommended source of truth for `api_key` is `system_secrets` using the name:

`provider.<name>.api_key`

The TOML value remains as a fallback for local development or unmigrated hosts.

## OpenRouter Provisioning

`[openrouter]` is optional. When configured, swarm can lazily mint a
per-user OpenRouter key on first use instead of sending all users through one
shared global OpenRouter key.

Recommended secret source:

`openrouter.provisioning_key`

set via:

```sh
swarmctl secrets system-set --stdin openrouter.provisioning_key
```

## BYO Upstreams

`[byo]` controls whether users may point a provider at their own
OpenAI-compatible upstream.

- `enabled`
- `allow_internal`

`allow_internal = true` is an explicit operator opt-in and expands the SSRF
surface by design. Use it only when private-fabric targets are an intended
feature.

## Backups

`[backup]` selects the sink:

- `local`
- `s3`

`[backup.s3]` can be populated inline, but the long-term preferred posture is
to keep credentials in `system_secrets` instead of plaintext TOML.
