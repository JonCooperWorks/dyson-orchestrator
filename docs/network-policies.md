# Network Policies

Every hired Dyson instance gets an egress profile. Swarm resolves that policy
at hire/restore time and pushes it into Cube and the host-side egress proxy.

## Profiles

| Profile | Meaning |
|---|---|
| `nolocalnet` | Public internet allowed; private/link-local/loopback/reserved ranges blocked |
| `open` | Public internet plus internal/LAN access |
| `airgap` | Only the swarm `/llm` proxy is reachable |
| `allowlist` | `/llm` proxy plus listed CIDRs/hostnames |
| `denylist` | Public internet minus default denies and listed CIDRs/hostnames |

## Enforcement Layers

### In Cube

Swarm resolves the configured profile into the `allowOut` / `denyOut` shapes
Cube expects. That covers direct egress policy inside the sandbox.

### On the host

For HTTP/S traffic, the sandbox also uses:

- `HTTP_PROXY=http://169.254.68.5:3128`
- `HTTPS_PROXY=http://169.254.68.5:3128`

The host-side `dyson-egress-proxy` looks up the sandbox by source IP and
applies the compiled policy from `/run/dyson-egress/policies.json`.

Unknown source IPs fail closed.

## Hostname Resolution

Allowlist and denylist entries may be hostnames. Swarm resolves them at hire
time and stores both:

- the raw operator entry
- the resolved CIDR set enforced by the sandbox

The trade-off is DNS staleness: if a hostname’s IPs rotate later, the running
instance keeps the original resolution until it is rehired/restored.

## Live Policy Changes

Cube does not expose an in-place patch for the egress maps, so “change network
access” is implemented as snapshot -> restore successor -> destroy old
sandbox.

The important user-visible effect:

- workspace state survives
- instance identity stays logically the same in the swarm model
- the underlying sandbox is rebuilt with the new policy

## Special Requirement

`airgap` and `allowlist` need a reachable host-side `/llm` proxy address, so
`cube_facing_addr` must be configured.
