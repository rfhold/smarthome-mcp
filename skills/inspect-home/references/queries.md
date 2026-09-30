# Queries

Call `home_assistant_query` with `action`, action-specific `input`, and optional jq-compatible `filter`. Use the live schema for exact fields, defaults, and bounds; unknown wrapper fields fail. Filters keep JSON text and structured content synchronized, but are not an authorization boundary.

| Action | Use |
| --- | --- |
| `entity.list` | Search or filter normalized Assist-exposed entities by domain with a bounded limit. |
| `device.list` | Group exposed states by device and effective area; standalone entities remain identifiable. |
| `state.get` | Read up to 25 explicit entity IDs, all currently exposed. |
| `history.get` | Read minimal significant history for up to 10 IDs over at most 24 hours; arbitrary attributes are excluded. |
| `camera.snapshot` | Read one exposed camera's current frame as an MCP image with bounded metadata. |

Use returned exact IDs. Missing, revoked, or indeterminate exposure is a denial, not an invitation to use another route. Camera images can reveal private surroundings; request them only when needed and do not log or unnecessarily repeat them. A snapshot is an observation, not a continuous feed. A successful history or state read is not proof of future state.

The fixed Home Assistant upstream is not caller-selectable. There is no arbitrary HTTP, WebSocket, registry, attribute, or service proxy. Native stored configurations and blueprint YAML use a separate administrator exception, not these entity-read rules; load `author-home-config` for them. Keep entity IDs, names, state values, images, and raw upstream errors out of telemetry.
