# Deployment

## Status

Preview was first deployed from commit `cb02f24` on 2026-08-10 by PipelineRun `smarthome-mcp-preview-r8hf7`. All seven pipeline tasks succeeded. The resulting Deployment was ready at one replica, CloudNativePG reported a healthy cluster, and the HTTPRoute reported accepted and resolved references. That evidence predates the private component deployment contract. Production remains intentionally unconfigured and fail-closed.

Bounded public smoke checks returned HTTP 200 from `/health` and `/ready`. An unauthenticated MCP initialize request returned HTTP 401 with the scope configured before this local contract change. Both OAuth metadata documents returned the configured preview resource and issuer. No live validation of `mcp:use`, an authenticated query or control invocation, the authoring and evidence actions, private component deployment, browser OIDC flow, telemetry-backend delivery, backup restore, or production deployment is claimed.

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
- a versioned 32-byte OAuth wrapping-key Secret, an application Secret, and a dedicated Home Assistant SSH Secret;
- a hardened one-replica `Recreate` Deployment;
- an egress NetworkPolicy and ClusterIP Service; and
- an HTTPRoute to `ingress/default-gateway` with request timeout `0s`.

The Deployment runs as UID and GID 65532. It disables service-account token mounts, privilege escalation, writable root filesystems, and Linux capabilities. It declares startup, readiness, and liveness probes, explicit resource limits, bounded temporary storage, and Stakater Reloader annotations.

`GET /health` reports process health. `GET /ready` performs bounded PostgreSQL and signing-key checks; it does not contact Authentik or Home Assistant.

## Container Image

`Dockerfile` uses Rust 1.96 and a Debian bookworm runtime. The final image contains the optimized application binary and CA certificates, runs as UID/GID 65532, and has OCI source and revision labels. Kubernetes owns health checks.

Cargo uses CLI Git for the exact private Kuri revision. BuildKit supplies Git configuration and credentials through secret mounts; those credentials must not enter layers or logs.

## Credential Boundaries

Bootstrap supplies `HOME_ASSISTANT_URL`, `HOME_ASSISTANT_TOKEN`, `HOME_ASSISTANT_SSH_PASSWORD`, and `HOME_ASSISTANT_SSH_HOST_PUBLIC_KEY` as process environment values to separately authorized targeted Pulumi Stash updates. All inputs are wrapped with `pulumi.secret` and intentionally enter encrypted Pulumi state. Normal previews and updates use protected Stash outputs; they do not need the process environment values. No value is stored in stack YAML or plaintext state. An unseeded Stash fails closed. The repository does not implement a canonical Stash seed command.

Initial stack bootstrap reads the existing Waltr inputs:

- ConfigMap `homeassistant-component-config`, key `HOME_ASSISTANT_URL`;
- Secret `homeassistant-component-secret`, key `token`.

Seed the URL and token for each configured stack from its matching Waltr namespace before the first full update. Seed the SSH password and independently obtained Ed25519 host public key through separate protected bootstrap inputs. These are operator bootstrap steps, not PipelineRun dependencies. The preview pipeline reads only persisted protected Stash outputs and performs no cross-namespace credential access.

The Home Assistant client sends the token only as a REST bearer credential and as the WebSocket authentication message. Redirects and environment proxies are disabled for REST. The deployment client reads the password and pinned host public key from a dedicated read-only Secret mount, verifies the Ed25519 key before password authentication, and requests only SFTP. Tool input cannot select an origin, host, port, path, header, command, or credential.

The service issues local ES256 access tokens and uses Authentik only for browser identity. The browser requests `openid profile email` through provisioned Authentik mappings. `/mcp` accepts only locally issued tokens bound to the exact resource and `mcp:use` scope. PostgreSQL stores generic OAuth/OIDC state and wrapped signing material; the wrapping-key file uses a separate Secret and read-only mount.

Runtime surfaces follow the [exposure and data-safety contract](../architecture/exposure-data-safety.md). MCP results expose only the authorized bounded action payload.

## Preview Pipeline

`.tekton/smarthome-mcp-preview.yaml` targets `main` push and incoming events. It has one implemented preview path.

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

The implemented runtime contract has six progressive tools. The Home Assistant tools include [blueprint and lifecycle actions](../home-assistant/spec/blueprints.md) and [private component deployment](../home-assistant/spec/component-deployment.md). Local code and tests cover these additions; no live SSH/SFTP deployment, installation, compatibility, or external-operation evidence exists.

Arbitrary service calls, native Home Assistant MCP bridging, and arbitrary HTTP, WebSocket, SSH, or filesystem access are not capabilities. Config deletes, generic config routing, full traces, blueprint delete or import, script blueprints, trigger subscriptions, caller-selected deployment targets, shell commands, and implicit restart remain excluded. Endpoint-wide `mcp:use` authorizes all six tools; no separate management scope exists.

Camera, control, authoring, evidence, component deployment, Thread, and Matter support requires no additional OAuth grant. The custom integration requires file deployment, a separate Home Assistant restart, and one config entry. Follow the [component deployment runbook](component-deployment.md). Internal Home Assistant API and live SSH/SFTP support require disposable-version evidence. The recorded preview evidence predates these implemented contracts and does not prove their deployment.

## Availability and Data

The declared topology uses one replica and does not provide service-level high availability. Preview declares 14-day backup retention and 10 GiB database storage; production declares 30 days and 20 GiB. Both declare a daily backup schedule and S3-compatible endpoint. Restore exercises and recovery objectives remain unverified.

## Approval Boundaries

Stack initialization, Stash bootstrap, Pulumi preview, deployment, cluster mutation, pipeline execution, component deployment, restart, setup, commit, and push each require explicit authority. Production operations remain outside the initial implementation.
