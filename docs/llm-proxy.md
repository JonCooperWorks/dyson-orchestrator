# LLM Proxy

The LLM proxy lives under `/llm/*` and is the main app-layer outbound path
for Dyson instances.

## Request Path

1. the Dyson sandbox sends a request to `SWARM_PROXY_URL`
2. swarm authenticates the per-instance `pt_...` bearer
3. swarm loads the instance row and tenant policy
4. swarm chooses an upstream key source
5. swarm strips or rewrites headers as needed for the provider
6. swarm forwards the request upstream and streams the response back
7. swarm records an audit row

Relevant code:

- [proxy/http.rs](../crates/swarm/src/proxy/http.rs)
- [proxy/mod.rs](../crates/swarm/src/proxy/mod.rs)
- [policy.rs](../crates/core/src/policy.rs)

## Key Resolution Order

Provider-specific details vary, but the broad model is:

- BYOK when the user supplied one
- provider-specific minted key when available (OpenRouter)
- configured platform key
- BYO upstream when the user selected that flow

OpenRouter is the only provider today with per-user lazy minting through an
operator provisioning key.

## Policy Enforcement

The proxy applies:

- allowed provider checks
- allowed model checks
- daily token budgets
- monthly USD budget placeholder gate
- request-per-second limits

Budgeting is keyed by owner, not instance, so a user’s total usage still
rolls up cleanly across rotated/restored instances.

## Auditing

Audit rows track:

- owner and instance
- provider and model
- prompt/output token counts
- completion status
- key source (`platform`, `byok`, `or_minted`)

That is what powers both operator reporting and per-user limit enforcement.

## Why This Is Separate From the Egress Proxy

The egress proxy enforces network policy at the tunnel/request-destination
layer. The LLM proxy enforces application policy:

- credential selection
- provider/model allowlists
- budgets
- usage accounting
- provider-specific request shaping

Those are different jobs, which is why the LLM proxy remains a dedicated
surface instead of being folded into the generic HTTP CONNECT proxy.
