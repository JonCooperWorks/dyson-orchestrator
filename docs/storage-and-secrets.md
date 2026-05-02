# Storage and Secrets

Swarm keeps long-lived state outside the sandbox so sandboxes can be rotated
and rebuilt safely.

## Main Persistence Layers

### SQLite

The SQLite database holds metadata and indexes:

- users
- instances
- user API keys
- proxy tokens
- snapshots
- user policies
- webhooks and webhook deliveries
- share rows and share-access audit
- artefact cache metadata
- mirrored state-file metadata
- LLM audit rows

### Local filesystem cache

The backup/local cache directory holds:

- snapshot bundles
- cached artefact bodies
- mirrored state-file bodies

Bodies are encrypted before disk where the service expects secrecy.

## Secret Scopes

Swarm uses age envelope encryption with two broad scopes:

### Per-user secrets

Encrypted under the user’s own age key:

- MCP server entries
- user BYOK values
- OpenRouter minted per-user keys
- any other user-owned sensitive blobs

### System secrets

Encrypted under the host/system age key:

- provider API keys
- OpenRouter provisioning key
- other host-operator credentials

## Why the DB Does Not Store Plaintext Tokens

User API keys and proxy tokens are stored sealed or hashed/selectable in ways
that avoid plaintext-at-rest:

- user API keys: sealed ciphertext + lookup prefix
- proxy tokens: sealed token + SHA-256 lookup key

That keeps the DB useful for lookup and revocation without turning it into a
plaintext secret dump.

## Artefact Cache and State Mirror

Two swarm-side stores matter operationally:

- artefact cache: keeps shared/UI-visible artefacts available even when the
  sandbox has been rebuilt
- state-file mirror: keeps selected workspace/chat state available for restore
  and rotation flows

These are what let rotation and restore preserve useful state while still
treating the sandbox itself as disposable.
