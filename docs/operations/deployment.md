# Deployment

## Status

The repository declares container, Pulumi, and preview-pipeline foundations. This new repository has no committed or deployed revision yet. Production remains declaration-only.

No commit, push, pipeline execution, Pulumi preview, or deployment is implied by these files. Each external action requires separate target-specific approval.

## Stack Targets

| Stack | Namespace | Hostname |
| --- | --- | --- |
| Preview | `smarthome-mcp-preview` | `preview-smarthome-mcp.holdenitdown.net` |
| Production | `smarthome-mcp` | `smarthome-mcp.holdenitdown.net` |

`infra/pulumi/` defines both stacks with Pulumi TypeScript and Bun. Stack files contain no image, Home Assistant token, or other deployment secret.

## Declared Resources

An approved stack apply creates:

- the target Namespace;
- an ObjectBucketClaim for database backups;
- a one-instance CloudNativePG Cluster, Database, and ScheduledBackup;
- an Authentik confidential browser application and RSA signing certificate;
- a versioned 32-byte OAuth wrapping-key Secret and an application Secret;
- a hardened one-replica `Recreate` Deployment;
- an egress NetworkPolicy and ClusterIP Service; and
- an HTTPRoute to `ingress/default-gateway` with request timeout `0s`.

The Deployment runs as UID and GID 65532. It disables service-account token mounts, privilege escalation, writable root filesystems, and Linux capabilities. It declares startup, readiness, and liveness probes, explicit resource limits, bounded temporary storage, and Stakater Reloader annotations.

`GET /health` reports process health. `GET /ready` performs bounded PostgreSQL and signing-key checks; it does not contact Authentik or Home Assistant.

## Container Image

`Dockerfile` uses Rust 1.96 and a Debian bookworm runtime. The final image contains the release binary and CA certificates, runs as UID/GID 65532, and has OCI source and revision labels. Kubernetes owns health checks.

Cargo uses CLI Git for the exact private Kuri revision. BuildKit supplies Git configuration and credentials through secret mounts; those credentials must not enter layers or logs.

## Credential Boundaries

Bootstrap supplies `HOME_ASSISTANT_URL` and `HOME_ASSISTANT_TOKEN` as process environment values to a targeted Pulumi update of `pulumi:index:Stash::home-assistant-url` and `pulumi:index:Stash::home-assistant-token`. Both inputs are wrapped with `pulumi.secret`. Normal previews and updates use the immutable protected stash outputs to create the service-local Kubernetes Secret containing the corresponding `SMARTHOME_MCP_` runtime variables; they do not need either process environment value. Neither value is stored in stack YAML or plaintext state. An unseeded stack fails closed.

Initial stack bootstrap reads the existing Waltr inputs:

- ConfigMap `homeassistant-component-config`, key `HOME_ASSISTANT_URL`;
- Secret `homeassistant-component-secret`, key `token`.

Seed each stack from its matching Waltr namespace before the first full update. This is an operator bootstrap step, not a PipelineRun dependency. The preview pipeline reads only the persisted protected Stash outputs and performs no cross-namespace credential access.

The Home Assistant client sends the token only as a REST bearer credential and as the WebSocket authentication message. Redirects and environment proxies are disabled for REST. Tool input cannot select the origin, path, headers, or credential.

The service issues local ES256 access tokens and uses Authentik only for browser identity. `/mcp` accepts only locally issued tokens bound to the exact resource and `mcp:read` scope. PostgreSQL stores generic OAuth/OIDC state and wrapped signing material; the wrapping-key file uses a separate Secret and read-only mount.

Runtime logs, MCP results, redirects, image layers, Pulumi outputs, and health responses must not expose credentials or Home Assistant household data.

## Preview Pipeline

`.tekton/smarthome-mcp-preview.yaml` targets `main` push and incoming events. It has one preview path and no release path.

The pipeline:

1. checks out the exact requested revision;
2. scans tracked build and deployment inputs for private key patterns;
3. builds amd64 and arm64 images with BuildKit secret mounts;
4. creates a multi-architecture manifest;
5. resolves an immutable digest and verifies runtime UID and OCI revision; and
6. runs `pulumi preview` and `pulumi up` for `preview` using the previously seeded protected Stash outputs.

The Pulumi step requires `pulumi-credentials`, `authentik-credentials`, and `tekton-cluster-kubeconfig` in the PipelineRun namespace. It does not require Home Assistant credentials there.

## Runtime Contract

The runtime serves stateless MCP Streamable HTTP revision `2026-07-28` at exact resource `/mcp`. It supports DCR, hardened CIMD, and native loopback clients through PostgreSQL-backed OAuth state.

One progressive read-only tool, `home_assistant_query`, exposes `list_entities`, `get_states`, and `get_history`. Each action performs a fresh Assist exposure lookup over Home Assistant WebSocket and permits only explicit `conversation: true` entries before using fixed REST endpoints. See the [Home Assistant specifications](../home-assistant/README.md).

Controls, service calls, native Home Assistant MCP bridging, and arbitrary HTTP access are not deployed capabilities.

## Availability and Data

The declared topology uses one replica and does not provide service-level high availability. Preview declares 14-day backup retention and 10 GiB database storage; production declares 30 days and 20 GiB. Both declare a daily backup schedule and S3-compatible endpoint. Restore exercises and recovery objectives remain unverified.

## Approval Boundaries

Stack initialization, Pulumi preview, deployment, cluster mutation, pipeline execution, commit, push, and release each require explicit authority. Production operations remain outside the initial implementation.
