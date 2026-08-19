# Component Deployment Operations

## Status

The implementation, local tests, Pulumi declarations, and embedded-source checks establish repository-local behavior only. No recorded evidence proves live SSH/SFTP connectivity, host-key correctness, Home Assistant filesystem replacement, restart, component load, setup, or recovery on the target.

## Prerequisites

Preview is declared with the exact target `172.16.1.10:2200`, user `root`, config root `/config`, and egress CIDR `172.16.1.10/32`. The target must provide SFTP over SSH, permit that account to inspect and replace the bounded component and transaction directories, and support same-filesystem rename semantics.

Obtain the Ed25519 host public key through an independent trusted channel before deployment. Do not learn or accept it from the deployment connection. The password must not contain control characters, and the public-key file must contain an `ssh-ed25519` key in the accepted public-key format.

## Pulumi Stash Bootstrap

Pulumi declares protected Stashes for `HOME_ASSISTANT_SSH_PASSWORD` and `HOME_ASSISTANT_SSH_HOST_PUBLIC_KEY`. Their secret inputs intentionally enter encrypted Pulumi state. A full update fails closed while either Stash contains its unseeded marker.

Bootstrap is a separate, authorized targeted Pulumi operation that supplies each value through the matching process environment variable. The repository does not implement one canonical seed command, so this runbook does not invent one. Review the planned target and protected-resource behavior before applying it.

Normal updates read the protected Stash outputs and create dedicated Kubernetes Secret `smarthome-mcp-home-assistant-ssh`. The workload mounts only `password` and `host_public_key` from that Secret read-only. Do not place either value in stack YAML, command arguments, logs, evidence, or the general application Secret.

Production intentionally lacks the required SSH target configuration and remains fail-closed. Do not copy preview values or infer a production target.

## Deployment Sequence

1. Confirm an authorized preview deployment has current protected Stash values, the dedicated Secret mount, and exact `/32` egress.
2. Independently verify the target's current Ed25519 host public key and expected SFTP service.
3. Confirm the intended image reports an MCP version matching its Cargo package version and embeds the same manifest version. Rust and Python tests enforce this invariant before image construction.
4. Invoke `home_assistant_exec` action `smarthome_mcp.deploy` with `{"confirm":true}`.
5. Record only the bounded operation, changed flag, previous version when present, installed version, restart requirement, and safe error code.
6. If changed, invoke `home_assistant.restart` separately with `{"confirm":true}` and wait for Home Assistant readiness.
7. Invoke `smarthome_mcp.setup` separately after Home Assistant has loaded the component.
8. Verify the config entry and administrator-only `smarthome_mcp/blueprint/get` command with a non-sensitive fixture.

An install or update does not imply restart or setup. A successful restart does not prove setup or command registration. A successful setup does not prove blueprint behavior.

## Failure And Recovery

| Symptom | Response |
| --- | --- |
| `host_key_mismatch` | Stop. Re-establish target identity independently; never accept a key from the failed connection. |
| `authentication_failed` | Check the protected Stash and target account without printing either credential. |
| `capacity_exhausted` | Another local or remote deployment owns the lock. Allow active work to finish; retry after the five-minute stale threshold only when no deployment is running. |
| `unsafe_deployment_state` | Stop manual mutation. Inspect only the bounded component and transaction locations through a separately authorized operator channel. Preserve evidence without file contents. |
| `deployment_verification_failed` | Do not restart. Treat the staged bytes or readback as untrusted and investigate the image and target storage. |
| `deployment_rollback_failed` | Do not restart or setup. Escalate for authorized recovery of active, backup, staging, journal, and lock state. |
| `deployment_cleanup_incomplete` | Do not restart based on this response. The file commit may have completed, but journal or owned-lock cleanup did not. Wait for target stability and invoke deploy again to reconcile. |
| Request cancellation or timeout | Do not assume the transaction stopped. Wait for bounded telemetry and target stability, then invoke deploy again to reconcile supported journal state. |

The deployer retains at most one recognized lower-version backup and automatically rolls back a failed update commit when possible. Do not manually restore, delete, or rename transaction paths without explicit target-specific authority and a recovery plan.

## Evidence Boundary

Local Rust tests cover MCP discovery metadata, embedded source bytes and paths, embedded-version validation, install, exact no-op, lower-version update, equal-version drift rejection, newer-version rejection, staging verification, owner-checked stale-lock and claim behavior, managed-state modes, interrupted-state reconciliation, cleanup failures, rollback, transaction timeout, and cancellation continuity. Local Python tests require the Cargo, Python project, and integration manifest versions to agree and use root SemVer.

These checks do not establish live SSH negotiation, SFTP server semantics, network policy enforcement, credentials, filesystem permissions, Home Assistant loading, restart, setup, or recovery. Record those only after separately authorized disposable or preview exercises. Never retain passwords, host-key material, component contents, hashes tied to private state, paths beyond the fixed contract, or raw SSH/SFTP responses.
