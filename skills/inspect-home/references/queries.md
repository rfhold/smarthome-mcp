# Queries

Read current data with MCP resources. Call `query` for temporal history with `action`, action-specific `input`, and optional jq-compatible `filter`. Use discovery for exact templates, fields, and bounds; unknown wrapper fields fail. Filters are not an authorization boundary.

| Action | Use |
| --- | --- |
| `smarthome://entities` | Bounded normalized Assist-exposed entities with canonical links. |
| `smarthome://devices` | Exposed states grouped by device and area; follow entity-anchored group links. |
| `smarthome://states/{entity_id}` | One currently exposed entity, using a returned state link. |
| `history.get` | Read minimal significant history for up to 10 IDs over at most 24 hours; arbitrary attributes are excluded. |
| `smarthome://cameras/{entity_id}` | One exposed camera's current frame as an MCP resource blob. |

Use returned exact IDs. Missing, revoked, or indeterminate exposure is a denial, not an invitation to use another route. Camera images can reveal private surroundings; request them only when needed and do not log or unnecessarily repeat them. A snapshot is an observation, not a continuous feed. A successful history or state read is not proof of future state.

The fixed Home Assistant upstream is not caller-selectable. There is no arbitrary HTTP, WebSocket, registry, attribute, or service proxy. Native stored configurations and blueprint YAML use a separate administrator exception, not these entity-read rules; load `author-home-config` for them. Keep entity IDs, names, state values, images, and raw upstream errors out of telemetry.
