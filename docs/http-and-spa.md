# HTTP and SPA

Swarm serves both the JSON API and the embedded React app from the same Axum
process.

## Route Tiers

At a high level:

- `/healthz` — unauthenticated
- `/auth/config` — unauthenticated auth-mode descriptor for the SPA
- `/v1/admin/*` — authenticated, then admin-role gated
- `/v1/*` tenant routes — authenticated as a user
- `/llm/*` — authenticated by per-instance proxy bearer
- `/mcp/*` agent proxy — authenticated by per-instance proxy bearer
- `/mcp/oauth/callback` — public callback surface
- `share.<hostname>` — public anonymous share read path
- `<instance_id>.<hostname>` — host-based reverse proxy to a live Dyson
- `/` and asset paths — embedded SPA bundle

Relevant code:

- [http/mod.rs](../crates/swarm/src/http/mod.rs)
- [dyson_proxy.rs](../crates/swarm/src/http/dyson_proxy.rs)
- [share_public.rs](../crates/swarm/src/http/share_public.rs)
- [static_assets.rs](../crates/swarm/src/http/static_assets.rs)

## SPA Shape

The web UI lives under:

`crates/swarm/src/http/web/`

It is a Vite + React app built into the Rust binary.

Main areas:

- instances list/detail/edit
- artefacts and shares
- tasks/webhooks and delivery audit
- MCP server management
- BYOK management
- admin user management

## Why Hash Routing

The SPA uses hash-based routing so IdP redirects and host-based subdomain
flows do not need server-side route rewriting for every deep link.

That keeps the embedded-static-asset story simple while still allowing:

- deep-linked instance views
- OAuth browser round-trips
- mobile/desktop navigation parity
