# Testing

## Status

The local Rust and Pulumi suites pass under Rust 1.96 and Bun. They provide unit, generated MCP transport, normalization, configuration, hosted-auth seam, and mocked infrastructure evidence. The preview pipeline has also built and verified the multi-architecture image, applied the Pulumi stack, reached a ready workload and healthy PostgreSQL cluster, and passed public health, readiness, MCP challenge, and metadata smoke checks. This evidence does not prove an authenticated Home Assistant action, browser OIDC flow, telemetry-backend delivery, backup restoration, or production behavior.

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

Run `git diff --check` before finalizing changes.

## Current Coverage

| Layer | Implemented evidence |
| --- | --- |
| Configuration and host | Strict environment parsing, Home Assistant origin policy, keyring parsing, health, readiness, and cancellation-safe HTTP metrics. |
| OAuth and OIDC seams | Exact `mcp:use` resource consent, `openid profile email` configuration, and stable issuer-plus-subject principal mapping. |
| MCP contract | Progressive discovery, dotted actions, legacy-action rejection, separate query and execution tools, annotations, schema validation, synchronized JSON text and structured query results, specialized camera and execution results, and safe semantic errors. |
| Home Assistant inputs | Entity IDs, domain/search limits, entity and device-list limits, entity counts, RFC3339 history ranges, defaults, read limits, and bounded common-control values. |
| Home Assistant normalization | Approved state metadata only; device and effective-area enrichment is bounded; arbitrary attributes and raw registry data are omitted. |
| Home Assistant telemetry | Fixed action/outcome labels and cancellation-safe counters. |
| Pulumi | Immutable images, HTTPS origins, wrapping-key policy, Home Assistant process-secret propagation, Authentik, CNPG/backups, workload hardening, egress, Service, route, and safe outputs. |

## Home Assistant Contract

Tests must cover the [Home Assistant specifications](../home-assistant/README.md):

- fresh WebSocket authentication and exposure lookup for every entity-targeted invocation;
- exact exposure command ID and type;
- explicit `conversation: true` authorization only;
- fail-closed handling for false, absent, malformed, unauthorized, and unavailable exposure results;
- fixed REST methods and paths with bearer-header-only credentials;
- deterministic list filtering, sorting, limits, and truncation;
- selected-entity-only registry enrichment, device-area grouping, area overrides, standalone retention, and registry failure handling;
- all-or-nothing authorization before per-entity state reads;
- fixed minimal, no-attributes, significant-only history parameters;
- approved normalized fields and bounded history points;
- `camera.snapshot` input shape, `camera.*` restriction, and unknown-field rejection;
- fresh exact exposure authorization before every camera image request;
- fixed camera proxy GET routing and bearer authentication, with no service call or caller-selected upstream data;
- exact JPEG, PNG, and WebP MIME values, matching signatures, standard Base64, and decoded and transport bounds;
- camera success results with short text, one image block, bounded metadata, and no duplicate image bytes;
- camera error mapping, timeouts, cancellation, capacity exhaustion, and permit release;
- progressive camera discovery, namespace help, tool annotations, and response filtering;
- camera privacy across errors, logs, traces, and metrics, including absence of identifiers, image data, paths, MIME, authentication data, headers, bodies, and raw errors;
- separate `home_assistant_exec` discovery and annotations with read-only false, destructive true, idempotent false, and open-world true;
- the complete [common-control action catalog](../home-assistant/common-controls.md), matching-domain single-entity targeting, unknown-field rejection, and every numeric boundary;
- fresh exact `conversation: true` exposure before every common-control mutation, with no service POST on denied or malformed exposure;
- fixed action-to-service POST mapping and bounded constructed JSON, with no caller-selected service, domain, path, method, headers, origin, or arbitrary data;
- bounded upstream result consumption and minimal output without upstream wrappers, state, context, or service-response data;
- rejection of batching, toggle, confirmations, presets, sources, modes, colors, templates, and unapproved actions;
- `scene.upsert` and `automation.upsert` closed wrappers, safe bounded single-segment keys, object shape, encoded-byte, nesting-depth, and transport bounds;
- acceptance of an absent top-level `id` or a string `id` equal to `config_key`, with rejection of every other value or type;
- complete native scene and automation JSON forwarding to only the fixed admin config POSTs, with intentional matching-key replacement;
- upserts and config reads without exposure lookup or recursive reference authorization, plus rejection of deletes and caller-selected routing;
- bounded editor-managed scene and automation discovery from `/api/states`, safe-ID filtering, case-insensitive search, deterministic sort, limit, count, truncation, and omission of YAML-only entries and arbitrary attributes;
- exact fixed config GET routing, complete bounded native object output, and rejection of malformed, oversized, or excessively nested responses;
- minimal upsert acknowledgment that does not claim active operation after Home Assistant schedules its asynchronous reload;
- `automation.validate` routing only to WebSocket `validate_config`, bounded projected acceptance, and no stored-config read;
- `automation.traces` routing only to WebSocket `trace/list` with fixed `automation` domain and one validated item ID;
- bounded trace summaries with only run ID, time, duration, state, execution status, not-triggered status, and safe error presence or category;
- exclusion of raw errors, last steps, config, variables, state or service data, contexts, full traces, `trace/get`, debugging, subscriptions, and script execution;
- clear result semantics: validation reflects current acceptance, traces describe past runs, and neither guarantees future behavior;
- absence of config keys, entity IDs, names, item IDs, native config, trace data, and raw errors from telemetry and safe errors;
- endpoint-wide `mcp:use` authority for all six tools, including Thread selection and Matter interview without a separate per-tool grant;
- redirect denial, body/frame/URL/time/concurrency bounds, cancellation, and permit release; and
- stable source-free error codes without URLs, credentials, bodies, entity IDs, or state data.

Mock transport coverage should prove these properties without contacting a live household. Live tests must use deliberately exposed non-sensitive fixtures and must never retain raw responses as shared artifacts.

## Hosted Access Contract

Tests must cover the [access contract](../architecture/access-authentication.md), including exact `/mcp` resource binding, `mcp:use`, Bearer challenges, direct Authentik-token rejection, DCR, hardened CIMD, loopback redirects, PKCE, OIDC transaction single use, refresh rotation, and wrapped ES256 signing material.

Negative tests must verify that errors, logs, redirects, traces, and MCP content contain no credential or household data.

## Required External Evidence

Before preview acceptance, record separate evidence for:

| Layer | Required evidence |
| --- | --- |
| PostgreSQL | Embedded migrations, expiry, one-shot state, replay prevention, refresh rotation, and encrypted signing-key persistence against a disposable database. |
| Authentik | Discovery, browser login, callback validation, and local token issuance. |
| Kuri client | DCR, CIMD, loopback authorization, refresh, exact resource binding, and discovery and invocation of all six progressive tools. |
| Home Assistant | Controlled exposure, bounded reads, fixed service calls, editor-managed scene and automation discovery and complete reads, replacement, asynchronous reload behavior, `validate_config`, projected `trace/list`, Thread selection, and projected Matter operations for non-sensitive fixtures. |
| Container | Multi-architecture build, UID, revision label, private-material inspection, and startup with controlled dependencies. |
| Preview | Authorized browser login, authenticated `/mcp`, bounded HA, Thread, and Matter operations, telemetry, probes, and secret non-disclosure. |

The authoring and evidence tests must use a disposable Home Assistant instance and its configured administrator token. Record the tested Home Assistant version. Prove that non-admin credentials fail and that excluded APIs remain unreachable. Do not retain native config, full traces, or raw errors as evidence.

Production additionally requires image and stack review, credential and wrapping-key rotation exercises, backup restoration, and recovery objectives. No external evidence is currently claimed.
