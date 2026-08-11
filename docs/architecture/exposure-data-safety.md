# Exposure and Data Safety

## Authorization

Every action performs a fresh `homeassistant/expose_entity/list` lookup over `/api/websocket`. An entity is readable or controllable only when its exposure object contains `conversation: true`.

The service fails closed when exposure is absent, false, malformed, unavailable, or returned with an unexpected command ID or message type. It does not infer Home Assistant defaults and does not retain an exposure cache after the invocation.

## Data Projection

Current-state results may contain only entity ID, domain, state, friendly name, device class, unit of measurement, last changed, and last updated. `device.list` may additionally place selected states in groups with a bounded device display name and bounded effective area name. History contains entity ID, state, and last changed. `camera.snapshot` returns one validated image plus short text and bounded metadata. Execution actions discard upstream service results and return minimal output. Arbitrary attributes and Home Assistant context IDs are discarded.

`device.list` requests entity registry entries only for exposed states selected by its entity limit. Device and area registries enrich those selected entities but cannot create output. Device IDs, area IDs, manufacturer, model, identifiers, labels, registry settings, raw registry objects, and hidden or registry-only entities are never serialized.

Entity IDs, friendly names, states, attributes, camera images or Base64, upstream paths, MIME values, and the Home Assistant origin must not appear in logs, traces, errors, or metric labels. Authorization material, headers, bodies, and raw errors must also remain absent. Authorized MCP clients receive household data only through the bounded tool result.

## Upstream Boundary

The origin and bearer token come from runtime configuration. Redirects and environment proxies are disabled for REST. HTTPS is required unless an operator explicitly enables internal HTTP. Requests use fixed API paths, bounded URL/body/frame sizes, one end-to-end deadline, and non-waiting concurrency admission.

The server exposes read-only queries and only the fixed [common-control service calls](../home-assistant/common-controls.md). Each control targets one matching-domain entity through fixed POST routing and bounded server-constructed JSON after fresh exact exposure authorization. Arbitrary service calls, templates, native MCP bridging, subscriptions, and caller-selected registry access remain excluded. `device.list` uses fixed registry commands only for bounded server-owned enrichment. The MCP action named `camera.snapshot` uses only the camera proxy GET endpoint; it never invokes Home Assistant's file-writing service with the same name.
