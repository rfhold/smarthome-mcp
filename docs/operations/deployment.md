# Deployment

## Status

Preview was first deployed from commit `cb02f24` on 2026-08-10 by PipelineRun `smarthome-mcp-preview-r8hf7`. All seven pipeline tasks succeeded. The resulting Deployment was ready at one replica, CloudNativePG reported a healthy cluster, and the HTTPRoute reported accepted and resolved references. That evidence predates the private component deployment contract. Production declarations and a release pipeline exist; they do not establish production deployment or live compatibility evidence.

Bounded public smoke checks returned HTTP 200 from `/health` and `/ready`. An unauthenticated MCP initialize request returned HTTP 401 with the scope configured before this local contract change. Both OAuth metadata documents returned the configured preview resource and issuer. No live validation of `mcp:use`, an authenticated query or control invocation, the authoring and evidence actions, private component deployment, browser OIDC flow, telemetry-backend delivery, backup restore, or production deployment is claimed.

## Stack Targets

| Stack | Namespace | Hostname |
| --- | --- | --- |
| Preview | `smarthome-mcp-preview` | `preview-smarthome-mcp.holdenitdown.net` |
| Production | `smarthome-mcp` | `smarthome-mcp.holdenitdown.net` |

`infra/pulumi/` defines both stacks with Pulumi TypeScript and Bun. Stack files contain no image, Home Assistant token, or other deployment secret.

Both stacks declare the administrator-confirmed SSH target `172.16.1.10:2200`, username `root`, config root `/homeassistant`, and egress CIDR `172.16.1.10/32`. This does not install the component or authorize a Home Assistant restart. Preview trusts only `https://preview-waltr.holdenitdown.net` for private CIMD; production trusts only `https://waltr.holdenitdown.net`. Neither stack adds a broader network exception.

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

Cargo uses CLI Git for private Waltr tag `v0.3.1` and its exact lockfile SHA. BuildKit supplies Git configuration and credentials through secret mounts; those credentials must not enter layers or logs.

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

The service workflows use `pipelinesascode.tekton.dev/task` to import and inline only their referenced shared Tasks from immutable Homelab commit `db8891ad3b9467db8ff15d80bb931e01c2af2e0d`. Installing a cluster Task does not automatically resolve a named `taskRef` during Pipelines as Code processing. Shared Task updates require an explicit consumer pin update and separately approved Task installation before execution; source or cluster availability alone does not prove run health.

`.tekton/smarthome-mcp-preview.yaml` targets `main` push and incoming events. It has one implemented preview path.

The pipeline:

1. checks out the exact requested revision;
2. scans tracked build and deployment inputs for private key patterns;
3. invokes `service-build-v1` independently for amd64 and arm64 after the scan, with native builders and a read-only Secret workspace for private Git credentials;
4. passes the immutable build results, ordered amd64 then arm64, to `service-manifest-v1` for metadata verification and the joined index; and
5. passes that immutable index to `service-pulumi-up-v1` for a frozen dependency install and direct `pulumi up --stack preview --yes --skip-preview` using the previously seeded protected Stash outputs.

The builds publish `<full-SHA>-amd64` and `<full-SHA>-arm64` aliases; the joined index uses `<full-SHA>`, without a `preview-` prefix. Deployment uses only the immutable result. There are no test, lint, or separate Pulumi preview tasks in the deployment DAG; local test suites remain available.

The four shared Tasks (`service-build-v1`, `service-manifest-v1`, `service-promote-v1`, and `service-pulumi-up-v1`) must be installed in `pipelines-as-code` from the homelab repository through a separately approved installation before these workflows can run. These declarations do not install them.

The Pulumi Task requires `pulumi-credentials`, `authentik-credentials`, and `tekton-cluster-kubeconfig` in the PipelineRun namespace. Authentik is enabled; Grafana and OpenBao are disabled. It uses the existing non-admin pipeline identity and does not require Home Assistant credentials there.

A failed or cancelled PipelineRun does not prove rollback or the absence of external effects. Completed architecture, commit, or release image publications and Pulumi changes can remain. Remote BuildKit work can continue or finish after cancellation; confirm its status separately. Before an authorized retry, inspect registry aliases and immutable digests with target-specific read authority. Separately inspect remote BuildKit status and Pulumi stack pending operations and current resources under appropriate authority. Retries, deployments, and rollbacks each require appropriate target-specific authority; the workflow promises no automatic rollback.

## Production Release

`.tekton/smarthome-mcp-release.yaml` handles push events for stable tags `vMAJOR.MINOR.PATCH`, without leading zeroes, prereleases, or build metadata. Release tags must be OpenPGP-signed annotated tags. The pipeline checks the tag commit against the event SHA and detached checkout, verifies its signature using only the provisioned trust keyring, requires ancestry on fetched `main`, and checks the tag version against `Cargo.toml`, `pyproject.toml`, and `custom_components/smarthome_mcp/manifest.json`. A private-material scan gates promotion as well.

The shared `service-promote-v1` Task requires the existing `cr.holdenitdown.net/rfhold/smarthome-mcp:<full-SHA>` multi-architecture index and the validated stable tag. It resolves that image once to a digest, verifies Linux amd64 and arm64 configurations, runtime user `65532:65532`, and the exact OCI revision on both platforms. It never rebuilds or substitutes another preview. An absent preview, malformed digest, or failed verification stops the release.

The `vMAJOR.MINOR.PATCH` image alias must be absent or already point to that same digest. Only an explicit registry `MANIFEST_UNKNOWN` response permits creating the alias; authorization, transport, generic HTTP 404, and other errors fail closed. An existing different digest fails. The pipeline copies from the immutable digest and verifies the resulting alias, then passes the shared promotion result map (`image`) containing only `repository@sha256:...` to `service-pulumi-up-v1` for direct `prod` update with `--skip-preview`. CI does not run a separate Pulumi preview. Concurrent writers are not locked by this check: operators must serialize releases and protect release aliases from overwrite.

Prerequisites in the PipelineRun namespace:

- The normal Pipelines-as-Code `git_auth_secret`, mounted read-only for checkout, and existing registry credentials permitting preview reads and release-alias writes.
- Secret `smarthome-mcp-release-trusted-signers`, key `signing-key.asc`, containing only approved OpenPGP public signing keys. Missing trust material fails closed; private signing keys never enter CI.
- Existing `pulumi-credentials`, `authentik-credentials`, and `tekton-cluster-kubeconfig`.
- The `prod` stack and its protected Home Assistant URL, token, SSH password, and independently verified Ed25519 host-key Stash outputs, seeded through a separately authorized operator bootstrap before running a release. No credential is copied from preview by the pipeline.
- A successful preview build for the exact release commit, retained at its SHA alias, and availability of the pinned third-party CI images. Only maintained CI images under `cr.holdenitdown.net/rfhold/` use `latest` with `imagePullPolicy: Always`. The general CI image must supply Bun `1.3.5` and Pulumi `v3.253.0`, as required by the release checks. Application promotion and deployment still use verified immutable digests.

Prepare the aligned version change on `main`, wait for its preview pipeline to succeed, then create and publish a signed annotated stable tag only with explicit tag/push authority. Pipeline execution authorizes service deployment, not component installation, Home Assistant restart, setup, or device actions. Recovery retries the same tag and immutable digest; a different digest at an existing release alias requires investigation rather than overwrite.

Run `bun test ./.tekton/release.test.ts` from the repository root for local YAML, caller wiring, gate, version-alignment, and shared-Task promotion regressions with a mocked registry. The tests read `service-promote-v1.yaml` from the sibling `homelab/src/components/tekton-tasks` checkout by default; set `SHARED_TASKS_DIR` to another reviewed local Task directory if needed. These tests perform no registry or production actions and do not prove live signature tooling, registry access, or deployment readiness.

## Runtime Contract

The runtime serves stateless MCP Streamable HTTP revision `2026-07-28` at exact resource `/mcp`. It supports DCR, hardened CIMD, and native loopback clients through PostgreSQL-backed OAuth state.

The implemented public runtime has resource-first reads and [create/edit/query/execute tools](../home-assistant/spec/resource-first.md). Authoring uses existing native APIs under a single-writer assumption, without a Core patch. Lifecycle commands and [private component deployment](../home-assistant/spec/component-deployment.md) retain separate authority. Local code and tests do not establish live SSH/SFTP deployment, installation, compatibility, or external-operation evidence.

Arbitrary service calls, native Home Assistant MCP bridging, and arbitrary HTTP, WebSocket, SSH, or filesystem access are not capabilities. Config deletes, generic config routing, full traces, blueprint delete or import, script blueprints, trigger subscriptions, caller-selected deployment targets, shell commands, and implicit restart remain excluded. Endpoint-wide `mcp:use` authorizes both tools and resources; no separate management scope exists.

Camera, control, authoring, evidence, component deployment, Thread, and Matter support requires no additional OAuth grant. The custom integration requires file deployment, a separate Home Assistant restart, and one config entry. Follow the [component deployment runbook](component-deployment.md). Internal Home Assistant API and live SSH/SFTP support require disposable-version evidence. The recorded preview evidence predates these implemented contracts and does not prove their deployment.

## Availability and Data

The declared topology uses one replica and does not provide service-level high availability. Preview declares 14-day backup retention and 10 GiB database storage; production declares 30 days and 20 GiB. Both declare a daily backup schedule and S3-compatible endpoint. Restore exercises and recovery objectives remain unverified.

## Approval Boundaries

Stack initialization, Stash bootstrap, Pulumi preview, deployment, cluster mutation, pipeline execution, component deployment, restart, setup, commit, tag creation, and push each require explicit authority. Release declarations alone grant none of those permissions.
