# Testing

Swarm’s tests are layered. The goal is to catch mistakes close to the seam
they affect while still keeping at least one realistic end-to-end path.

## Main Layers

### Unit tests

Mostly in:

- `crates/core/src/*`
- `crates/swarm/src/*`
- web client/component tests under `crates/swarm/src/http/web/src`

These lock down:

- store behaviour
- auth edge cases
- token resolution
- network policy resolution
- OpenRouter minting logic
- MCP OAuth discovery and error handling
- UI route and component behaviour

### Integration tests

Main server-side suites:

- [e2e_mock_cube.rs](../crates/swarm/tests/e2e_mock_cube.rs)
- [integration_extra.rs](../crates/swarm/tests/integration_extra.rs)
- [share_flow.rs](../crates/swarm/tests/share_flow.rs)

These stand up swarm with in-process mocks and verify realistic multi-step
flows such as hire/snapshot/restore, tenancy isolation, OpenRouter lazy mint,
and public artefact sharing.

### External-service integration

[backup_s3_minio.rs](../crates/swarm/tests/backup_s3_minio.rs) exercises the
S3 backup sink against MinIO when the relevant env vars are provided.

### Web build verification

The embedded SPA uses:

```sh
npm test
npm run build
```

The production build runs the test suite before bundling.

## Useful Commands

```sh
cargo test -p dyson-swarm-core
cargo test -p dyson-swarm
cd crates/swarm/src/http/web && npm test
cd crates/swarm/src/http/web && npm run build
```

## What To Update When You Change Behaviour

- route or auth behaviour: add/adjust Rust integration coverage
- storage or token logic: add/adjust core tests
- UI behaviour or copy that matters operationally: add/adjust web tests
- provider- or protocol-specific regressions: add focused tests near that seam
