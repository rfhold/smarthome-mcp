# Testing

## Status

The local Rust and Pulumi suites pass under Rust 1.96 and Bun. They provide unit, generated MCP transport, normalization, configuration, hosted-auth seam, component deployment, and mocked infrastructure evidence. Recorded preview evidence covers an earlier multi-architecture image, Pulumi apply, ready workload, healthy PostgreSQL cluster, and public health, readiness, MCP challenge, and metadata smoke checks.

Local implementation and tests cover blueprint and component deployment schemas, routing, bounds, state checks, privacy, custom integration behavior, embedded source bytes, shared versions, and transaction recovery. They do not prove a live SSH/SFTP connection, Home Assistant compatibility, component installation or load, network-policy enforcement, restart, setup, or external behavior.

## Commands

Run from the repository root:

```bash
cargo +1.96.0 fmt --all -- --check
cargo +1.96.0 check --locked --all-targets --all-features
cargo +1.96.0 clippy --locked --all-targets --all-features -- -D warnings
cargo +1.96.0 test --locked --all-features
```

Run infrastructure checks from `infra/pulumi`:

```bash
bun install --frozen-lockfile
bun run build
bun test
```

Run custom-integration checks from the repository root:

```bash
uv sync --frozen
uv run ruff format --check custom_components tests
uv run ruff check custom_components tests
uv run mypy
uv run pytest
```

The Rust suite checks the MCP discovery metadata version against the Cargo package version and validates the embedded integration manifest and exact embedded file set against the repository component tree. The Python suite parses `Cargo.toml`, `pyproject.toml`, and the integration manifest, requires root SemVer, and rejects version drift. Locked Cargo commands enforce `Cargo.toml` and `Cargo.lock` consistency. Run `git diff --check` before finalizing changes.

## Current Coverage

| Layer | Implemented evidence |
| --- | --- |
| Configuration and host | Strict environment parsing, Home Assistant origin and SSH configuration, keyring parsing, health, readiness, and cancellation-safe HTTP metrics. |
| OAuth and OIDC seams | Exact `mcp:use` resource consent, `openid profile email` configuration, and stable issuer-plus-subject principal mapping. |
| MCP contract | Six-tool progressive discovery, dotted actions, legacy-action rejection, separate query and execution tools, annotations, schemas, synchronized query results, specialized camera results, and safe semantic errors. |
| Home Assistant | Exposure-gated entity operations, bounded normalization, authoring and evidence actions, blueprint operations, setup, restart, and privacy controls. |
| Component deployment | Closed confirmation schema, embedded version match, strict host-key parsing, install/update/no-op decisions, drift and downgrade rejection, bounded inspection, staging readback, lock handling, two-rename update, rollback, journal reconciliation, and cancellation continuity. |
| Embedded component | Exact repository source bytes and path set, manifest domain, and manifest version matching the Cargo package version. Python tests independently reject root-package version drift. |
| Pulumi | Protected credential Stashes, dedicated read-only SSH Secret, exact preview `/32` egress, immutable images, runtime resources, and fail-closed unseeded values. |

## Home Assistant Contract

Tests must cover the [Home Assistant specifications](../home-assistant/README.md), including:

- fresh WebSocket authentication and exact `conversation: true` exposure for every entity-targeted invocation;
- fail-closed handling for absent, false, malformed, unauthorized, or unavailable exposure;
- fixed REST and WebSocket routing with no caller-selected origin, credential, service, method, path, header, or command;
- bounded deterministic list, state, history, device, camera, config, validation, trace, blueprint, Thread, and Matter projections;
- the complete [common-control action catalog](../home-assistant/common-controls.md) and numeric boundaries;
- eight fixed [authoring and evidence actions](../home-assistant/spec/authoring-evidence.md), including complete authorized config reads and bounded projected traces;
- fixed blueprint list, semantic get, replace-save, substitution, setup, and separately confirmed restart behavior;
- six-tool progressive discovery and endpoint-wide `mcp:use` authority without a separate management scope;
- redirect denial, body/frame/URL/time/concurrency bounds, cancellation, and permit release; and
- stable source-free errors without credentials, household values, identifiers, paths, bodies, or raw upstream errors.

Mock transport coverage proves repository behavior without contacting a live household. Live tests must use deliberately exposed non-sensitive fixtures and must never retain raw responses as shared artifacts.

## Component Deployment Coverage

The local suite and required live evidence are governed by the [component deployment contract](../home-assistant/spec/component-deployment.md):

- `smarthome_mcp.deploy` accepts only exact `confirm: true` and returns bounded operation metadata;
- component files and manifest version equal package version `0.2.0` at build time;
- the caller cannot select a host, port, user, path, repository, version, bytes, credential, or command;
- Ed25519 host identity is pinned before password authentication and only SFTP is requested;
- inspection rejects unknown nodes, modes, names, depth, entry count, file size, total size, and ambiguous transaction state;
- absent installs, recognized lower SemVer updates, exact-content equal-version no-ops, and equal drift, newer, invalid, and downgrade cases fail as specified;
- staging uses exclusive writes, bounded readback, exact file sets, and SHA-256 verification;
- one local permit and one remote lock prevent unsafe concurrency;
- update retains one backup and uses active-to-backup then staging-to-active renames;
- failed commit rolls back, supported interrupted state reconciles, and ambiguous state fails closed;
- cancellation does not abandon in-progress commit or rollback work;
- deploy never restarts Home Assistant or invokes setup; and
- telemetry excludes target details, paths, credentials, host-key material, component bytes, hashes, and raw errors.

Local tests do not prove the independently seeded key, target identity, SSH negotiation, SFTP implementation, filesystem permissions, rename durability, network path, or Home Assistant component behavior. An authorized disposable or preview exercise must record exact MCP, component, and Home Assistant versions and only bounded outcomes.

## Hosted Access Contract

Tests must cover the [access contract](../architecture/access-authentication.md), including exact `/mcp` resource binding, `mcp:use`, Bearer challenges, direct Authentik-token rejection, DCR, hardened CIMD, loopback redirects, PKCE, OIDC transaction single use, refresh rotation, and wrapped ES256 signing material.

Negative tests must verify that errors, logs, redirects, traces, and MCP content contain no credential or household data.

## Required External Evidence

| Layer | Required evidence |
| --- | --- |
| PostgreSQL | Embedded migrations, expiry, one-shot state, replay prevention, refresh rotation, and encrypted signing-key persistence against a disposable database. |
| Authentik | Discovery, browser login, callback validation, and local token issuance. |
| Kuri client | DCR, CIMD, loopback authorization, refresh, exact resource binding, and discovery and invocation of all six tools. |
| Home Assistant | Version-pinned blueprint model, config flow, restart, custom integration load, and current controlled-operation evidence. |
| SSH/SFTP | Independently verified Ed25519 host identity, password authentication, SFTP-only operation, bounded filesystem transaction, rollback, and recovery. |
| Container | Multi-architecture build, UID, revision label, embedded component, private-material inspection, and startup with controlled dependencies. |
| Preview | Authorized browser login, authenticated `/mcp`, bounded Home Assistant, deployment, Thread, and Matter operations, telemetry, probes, and secret non-disclosure. |

Production additionally requires target-specific SSH configuration, image and stack review, credential and wrapping-key rotation exercises, backup restoration, and recovery objectives. Production is currently unconfigured and no external component deployment evidence is claimed.
