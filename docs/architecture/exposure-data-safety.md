# Exposure and Data Safety

## Authorization

Every entity-targeted read or control performs a fresh `homeassistant/expose_entity/list` lookup over `/api/websocket`. An entity is readable or controllable only when its exposure object contains `conversation: true`.

The service fails closed when exposure is absent, false, malformed, unavailable, or returned with an unexpected command ID or message type. It does not infer Home Assistant defaults and does not retain an exposure cache after the invocation.

Eight scene and automation actions intentionally bypass Assist exposure. Endpoint-wide `mcp:use` plus the configured Home Assistant administrator token authorizes them. The service does not recursively authorize references in native config. The [authoring and evidence contract](../home-assistant/spec/authoring-evidence.md) defines this authority expansion.

Blueprint, lifecycle, and component deployment actions use the same administrator exception. They add blueprint reads and writes, automation creation, config-flow setup, fixed machine-filesystem mutation, and confirmed restart. They do not recursively authorize blueprint inputs or entity references. The [blueprint](../home-assistant/spec/blueprints.md) and [component deployment](../home-assistant/spec/component-deployment.md) contracts define these risks.

Thread operations have no entity target, so Assist exposure does not authorize them. Matter device actions refresh the device registry and fail closed unless the exact device has a `matter` identifier. The [Thread and Matter contract](../home-assistant/spec/thread-matter.md) defines this separate authorization boundary.

## Data Projection

Current-state entity results may contain only entity ID, domain, state, friendly name, device class, unit of measurement, last changed, and last updated. `home_assistant_query` action `device.list` may additionally place selected states in groups with a bounded device display name and bounded effective area name. Config lists project only safe config key, entity ID, and name; exact config gets return the explicitly authorized complete bounded native object. History contains entity ID, state, and last changed. `camera.snapshot` returns one validated image plus short text and bounded metadata. Automation traces return only bounded status summaries. Authorized blueprint reads can return bounded semantic YAML. Deployment returns only bounded operation metadata. Other execution actions discard upstream results and return minimal output.

That entity `device.list` action requests entity registry entries only for exposed states selected by its entity limit. Device and area registries enrich those selected entities but cannot create output. Its projection excludes device IDs, area IDs, manufacturer, model, identifiers, labels, registry settings, raw registry objects, and hidden or registry-only entities.

Entity IDs, config keys, automation item IDs, friendly names, states, attributes, native config, blueprint YAML and inputs, component bytes, deployment paths, hashes, search terms, result data, trace data, camera images or Base64, upstream paths, MIME values, and the Home Assistant origin must not appear in logs, traces, safe errors, or metric labels. Authorization material, SSH credentials and host keys, headers, bodies, and raw errors must also remain absent. Authorized MCP clients receive requested data only through bounded tool results.

Thread and Matter results also use strict projections. Thread network output excludes operational dataset TLVs and credentials. Matter interview output excludes the complete upstream response.

## Upstream Boundary

The origin and bearer token come from runtime configuration. Redirects and environment proxies are disabled for REST. HTTPS is required unless an operator explicitly enables internal HTTP. Requests use fixed API paths, bounded URL/body/frame sizes, one end-to-end deadline, and non-waiting concurrency admission.

The server exposes projected queries, fixed [common controls](../home-assistant/common-controls.md), eight fixed [authoring and evidence actions](../home-assistant/spec/authoring-evidence.md), fixed [blueprint](../home-assistant/spec/blueprints.md) and [component deployment](../home-assistant/spec/component-deployment.md) actions, and fixed [Thread and Matter commands](../home-assistant/spec/thread-matter.md). Arbitrary services, generic WebSocket commands, generic filesystem access, native MCP bridging, caller-selected routes, blueprint import or delete, and script blueprints remain excluded.
