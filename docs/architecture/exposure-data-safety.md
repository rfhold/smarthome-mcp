# Exposure and Data Safety

## Authorization

Every action performs a fresh `homeassistant/expose_entity/list` lookup over `/api/websocket`. An entity is readable only when its exposure object contains `conversation: true`.

The service fails closed when exposure is absent, false, malformed, unavailable, or returned with an unexpected command ID or message type. It does not infer Home Assistant defaults and does not retain an exposure cache after the invocation.

## Data Projection

Current-state results may contain only entity ID, domain, state, friendly name, device class, unit of measurement, last changed, and last updated. History contains entity ID, state, and last changed. Arbitrary attributes and Home Assistant context IDs are discarded.

Entity IDs, friendly names, states, attributes, and the Home Assistant origin must not appear in logs, traces, or metric labels. They remain tool result data visible to the authorized MCP client.

## Upstream Boundary

The origin and bearer token come from runtime configuration. Redirects and environment proxies are disabled for REST. HTTPS is required unless an operator explicitly enables internal HTTP. Requests use fixed API paths, bounded URL/body/frame sizes, one end-to-end deadline, and non-waiting concurrency admission.

The initial server is read-only. Home Assistant service calls, controls, templates, native MCP bridging, subscriptions, and registry APIs are excluded.
