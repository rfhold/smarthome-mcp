# Home Assistant Component Deployment Contract

## Status

The repository implements this contract and covers it with local Rust tests. No recorded evidence proves a live SSH/SFTP connection, Home Assistant filesystem mutation, component load, restart, setup, or external operation.

## Action And Schema

`home_assistant_exec` action `smarthome_mcp.deploy` accepts only this closed input:

```json
{"confirm": true}
```

The Boolean must be exactly `true`; unknown fields, strings, and false values fail before network contact. The result contains only `action`, `operation`, `changed`, optional `previous_version`, `installed_version`, and `restart_required`. Operation is `install`, `update`, or `noop`. A changed result sets `restart_required: true`; a no-op sets it to false.

## Fixed Authority

The caller cannot select a host, port, username, config root, path, repository, version, bytes, SSH command, or credential. The preview declaration fixes `172.16.1.10:2200`, user `root`, and config root `/homeassistant`. This canonical physical directory avoids trusting the `/config` symlink while preserving the deployer's symlink rejection. Component files are exact compile-time bytes from `custom_components/smarthome_mcp/`; their manifest version must equal the MCP package version, currently `0.2.0`.

The native Rust client accepts only an independently seeded Ed25519 host public key, compares it before password authentication, and then requests only the SFTP subsystem. It does not use a shell, TOFU, an SSH agent, ambient SSH configuration, caller input, or a generic filesystem interface.

Endpoint-wide `mcp:use` authorizes this fixed machine-filesystem mutation. It has no separate deployment scope and does not use Assist exposure.

## Reconciliation

The deployer inspects only bounded regular files and directories under `/homeassistant/custom_components/smarthome_mcp` and `/homeassistant/.smarthome_mcp-deploy`. Implementation-managed directories must use mode `0755`, component files mode `0644`, and lock, claim, and journal files mode `0600`. It rejects symlinks, special nodes, invalid names, unexpected modes, excessive depth, entries, file size, or total size.

| Existing state | Result |
| --- | --- |
| Component absent | Stage and install embedded `0.2.0`. |
| Recognized lower SemVer | Stage and update to embedded `0.2.0`; report the prior version. |
| Version `0.2.0` with exact files and SHA-256 content | No-op without mutation. |
| Version `0.2.0` with any drift | Reject as unsafe. |
| Newer version, invalid version, or unrecognized tree | Reject as unsafe; never downgrade or overwrite. |

## Transaction And Recovery

One non-waiting local permit prevents concurrent deploys in a process. An owner-bearing exclusive remote lock coordinates other processes. The deployer verifies ownership before every remote mutation and unlock. Stale takeover first claims the observed lock through an atomic rename and fails closed on ambiguous lock or claim state. A valid lock or claim is stale only after five minutes, while one transaction has a four-minute deadline. Staging uses exclusive file creation, fixed modes, bounded readback, and SHA-256 comparison against every embedded file.

An update retains one recognized lower-version backup. Every present backup is validated before recovery mutation. Commit renames active to backup, then staging to active. If the second rename fails, rollback restores the backup and removes bounded staging state. A small journal lets the next invocation reconcile supported interrupted install or update states before inspecting the installed version. Failed journal or owned-lock cleanup returns a retryable `deployment_cleanup_incomplete` error instead of success. Unknown or ambiguous state fails closed rather than deleting arbitrary content.

The transaction runs in a spawned task holding the local permit. Cancelling the MCP request does not abandon in-progress work. The internal four-minute deadline cancels the transaction, attempts an owner-checked unlock, and leaves journaled state for bounded reconciliation when needed. Deployment telemetry remains bounded; payloads, paths, credentials, host-key material, component bytes, hashes, lock owners, and raw SSH/SFTP errors are excluded.

## Restart And Setup Boundaries

Deployment changes files only. It never restarts Home Assistant, invokes `smarthome_mcp.setup`, creates a config entry, or claims that Home Assistant loaded the component. Operators must invoke restart and setup separately under their own authority. Follow the [component deployment runbook](../../operations/component-deployment.md).
