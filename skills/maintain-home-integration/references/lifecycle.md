# Integration Lifecycle

| Action | Decision and input |
| --- | --- |
| `smarthome_mcp.deploy` | Fixed private component replacement on the server-configured SSH/SFTP target; exact boolean `confirm: true`. |
| `home_assistant.restart` | Full Home Assistant availability impact; separately authorized exact boolean `confirm: true`. |
| `smarthome_mcp.setup` | Start only this integration's config flow if absent; empty input and separate setup authority. |

Read the current input schemas; do not send credentials, paths, hosts, commands, or payloads. Deployment uses the binary's embedded component, fixed directory, pinned host trust, staging verification, and transactional replacement. It is not a generic filesystem or SSH proxy. Exact existing bytes can be a no-op; equal-version drift and newer remote versions are rejected. Do not respond to rejection by changing trust or overwriting unrelated files.

The safe deploy result reports operation, changed state, installed version, and restart requirement, not connection secrets or paths. Deploy does not restart or set up Home Assistant. A restart acknowledgment does not establish readiness or integration loading. Setup does not install files or restart; an existing entry returns idempotent success. Installation and setup compatibility require separate disposable-target evidence; local tests do not prove live support.

These operations use endpoint-wide `mcp:use` and server-owned deployment/administrator credentials, not Assist exposure. There is no separate administrator OAuth scope. An API confirmation field is not a substitute for user authority for the exact external action. Keep host identities, credentials, source, config, and raw errors out of telemetry. Never retrieve or embed runtime secrets in skill material.
