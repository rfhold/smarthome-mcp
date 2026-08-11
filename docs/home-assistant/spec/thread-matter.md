# Thread and Matter Tools

## Purpose

This specification defines the complete Thread and Matter progressive tool surface. It owns action catalogs, inputs, outputs, authorization, safety properties, and exclusions.

## Tool Catalog

| Tool | MCP annotations | Actions |
| --- | --- | --- |
| `thread_query` | `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: true` | `network.list`, `router.discover`, `readiness.get` |
| `thread_exec` | `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: true` | `network.set_preferred`, `router.set_preferred` |
| `matter_query` | `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: true` | `readiness.get`, `device.list`, `device.diagnostics`, `device.ping` |
| `matter_exec` | `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: true` | `device.interview` |

Each input schema rejects unknown fields. Empty-input actions accept only an empty object.

## Fixed Upstream Catalog

The server opens `/api/websocket` at one configured Home Assistant origin. It authenticates with the server-owned Home Assistant token.

| Action | Fixed Home Assistant commands |
| --- | --- |
| Thread `network.list` | `thread/list_datasets` |
| Thread `router.discover` | `thread/discover_routers`, then `unsubscribe_events` |
| Thread `readiness.get` | `thread/list_datasets`, `thread/discover_routers`, then `unsubscribe_events` |
| Thread `network.set_preferred` | `thread/set_preferred_dataset` |
| Thread `router.set_preferred` | `thread/set_preferred_border_agent` |
| Matter `readiness.get` and `device.list` | `config/device_registry/list`, then `config/area_registry/list` |
| Matter `device.diagnostics` | `config/device_registry/list`, then `matter/node_diagnostics` |
| Matter `device.ping` | `config/device_registry/list`, then `matter/ping_node` |
| Matter `device.interview` | `config/device_registry/list`, then `matter/interview_node` |

Callers cannot select an origin, credential, command type, path, method, header, registry, or arbitrary command data.

## Thread Contract

### `network.list`

The action accepts no fields. It returns `action: "network.list"` and `networks`, sorted by `dataset_id`.

Each network contains only `channel`, `created`, `dataset_id`, `extended_pan_id`, `network_name`, `pan_id`, `preferred`, `preferred_border_agent_id`, `preferred_extended_address`, and `source`. The server rejects more than 100 networks, duplicate dataset IDs, malformed fields, or oversized fields.

The output never contains an operational dataset TLV or Thread credentials.

### `router.discover`

| Input | Contract |
| --- | --- |
| `duration_seconds` | Optional integer from 1 through 10; default 3. |

The action returns `action`, the effective `duration_seconds`, and `routers`. The server tracks events for that duration, keys routers by event key, applies removals, and returns at most 100 routers in key order.

Each router contains only `key`, `instance_name`, `addresses`, `border_agent_id`, `brand`, `extended_address`, `extended_pan_id`, `model_name`, `network_name`, `server`, `thread_version`, `unconfigured`, and `vendor_name`. Each router has at most 16 validated IP addresses. The server sorts and deduplicates those addresses.

### Thread Projection Limits

| Values | Maximum bytes |
| --- | --- |
| Dataset ID, preferred border agent ID, router key, router border agent ID | 255 |
| Network name, source, instance name, model name, server, vendor name | 256 |
| Created value, extended PAN ID, PAN ID, preferred extended address, router address, brand, extended address, Thread version | 64 |

`channel` must fit an unsigned 16-bit integer. Each required text value must not be empty.

### `readiness.get`

The action accepts no fields. It lists stored networks and performs a three-second router discovery.

The output contains only `action`, `datasets_exist`, `dataset_count`, `preferred_dataset_count`, `preferred_dataset_id`, `routers_discovered`, `router_count`, `router_matches_preferred_network`, and `issues`. The issue catalog is `no_datasets`, `no_preferred_dataset`, `multiple_preferred_datasets`, `no_routers_discovered`, and `no_router_matches_preferred_network`.

The router match compares the extended PAN ID or network name against the first preferred network in dataset ID order.

The action reports observed registry and discovery facts. It does not claim Matter server, Bluetooth, or device reachability status.

### Preferred Selections

`network.set_preferred` requires `dataset_id`. It returns `action`, `dataset_id`, and `success: true` after Home Assistant accepts the command.

`router.set_preferred` requires `dataset_id` and `extended_address`; `border_agent_id` is optional and nullable. An omitted value is sent to Home Assistant as null. It returns `action`, `dataset_id`, and `success: true` after Home Assistant accepts the command.

Each identifier must contain 1 through 255 ASCII bytes. Allowed bytes are letters, digits, `-`, `_`, `:`, and `.`.

The server forwards only these validated identifiers. Home Assistant decides whether each referenced network or router exists and accepts the selection.

## Matter Contract

### `readiness.get`

The action accepts no fields. It returns `action`, `device_registry_responsive: true`, `matter_device_count`, and an empty `issues` list.

This action reports registry responsiveness and the observed Matter device count only. It does not claim network, Thread, Bluetooth, server, or device readiness.

### `device.list`

| Input | Contract |
| --- | --- |
| `limit` | Optional integer from 1 through 100; default 50. |

The action selects device registry entries whose `identifiers` contain a tuple with first value `matter`. It sorts devices by `device_id` and applies the limit.

The output contains `action`, `devices`, `total`, and `truncated`. Each device contains only `device_id`, `name`, `manufacturer`, `model`, `area_id`, and `area_name`. The user-defined name takes precedence over the registry name.

The server rejects more than 10,000 Matter device entries.

Registry device and area IDs have a 255-byte limit. Device, manufacturer, model, and area names have a 256-byte limit.

### Device Authorization

`device.diagnostics`, `device.ping`, and `device.interview` require one `device_id`. The identifier follows the same 1-through-255-byte syntax as Thread identifiers.

Before each device command, the server refreshes `config/device_registry/list`. The server fails with `not_matter_device` unless exactly one matching entry has a `matter` identifier.

Assist entity exposure does not authorize Thread or Matter operations. These operations have no entity target and use the fixed command and registry checks in this specification.

### `device.diagnostics`

The action returns `action`, `device_id`, and a projected `result`. The result contains only these fields:

| Field | Contract |
| --- | --- |
| `node_id` | Unsigned integer. |
| `network_type` | `thread`, `wifi`, `ethernet`, or `unknown`. |
| `node_type` | `end_device`, `sleepy_end_device`, `routing_end_device`, `bridge`, or `unknown`. |
| `network_name` | Optional bounded text. |
| `ip_addresses` | At most 16 validated IP addresses. |
| `mac_address` | Optional bounded text. |
| `available` | Boolean. |
| `active_fabrics` | At most 16 projected fabric records. |
| `active_fabric_index` | Unsigned integer. |

Each fabric record contains only `fabric_id`, `vendor_id`, `fabric_index`, `fabric_label`, and `vendor_name`. The first three fields are unsigned integers. The last two fields are optional bounded text.

Diagnostic network names, fabric labels, and fabric vendor names have a 256-byte limit. Diagnostic IP and MAC addresses have a 64-byte limit.

### `device.ping`

The upstream response must map at most 16 IP addresses to Boolean reachability values. Each key must be an IPv4 address, an IPv6 address, or a scoped IPv6 address.

A scope has 1 through 32 ASCII bytes. Allowed scope bytes are letters, digits, `_`, `-`, and `.`.

The action returns `action`, `device_id`, and `result.addresses`. Each address entry contains only `address` and `reachable`; entries are sorted by address.

### `device.interview`

The server discards the complete Home Assistant response. It returns only `action: "device.interview"`, `device_id`, and `success: true`.

## Authorization And Trust Decision

The `/mcp` endpoint requires the endpoint-wide `mcp:use` OAuth scope. That scope authorizes all six progressive tools.

Kuri's current server context provides no per-tool diagnostic or administrator scope. A valid `mcp:use` token therefore grants access to Thread selection and Matter interview actions.

This design is a current security limitation and trust decision. Per-tool privilege separation requires a separate OAuth or endpoint architecture change.

The Home Assistant service token must support administrator-only Thread commands and Matter interview. Home Assistant can permit Matter diagnostics and ping without administrator privileges, but this deployment shares one credential across all commands.

## Shared Safety Limits

Every operation uses one of four non-waiting permits and one 20-second end-to-end deadline. WebSocket frames and messages have a 1 MiB limit. Normalized JSON output has a 2 MiB limit.

The [shared contract](common.md) defines safe errors and the rest of the limits. The [data-safety contract](../../architecture/exposure-data-safety.md) defines telemetry and disclosure rules.

## Exclusions

The tools do not expose arbitrary Home Assistant WebSocket commands or caller-selected registries. They do not retrieve or return operational dataset TLVs or handle Thread credentials.

The Thread tools do not add, import, replace, or delete datasets. The Matter tools do not commission devices, remove fabrics, or open commissioning windows.

## Implementation References

- [`SmarthomeMcp` progressive tool declarations and action handlers](../../../src/mcp.rs)
- [`actions::thread` input schemas and validation](../../../src/integrations/home_assistant/actions/thread.rs)
- [`actions::matter` input schemas and validation](../../../src/integrations/home_assistant/actions/matter.rs)
- [`HomeAssistantClient` Thread and Matter methods, projections, and tests](../../../src/integrations/home_assistant/client.rs)
- [`Error::NotMatterDevice` and safe tool errors](../../../src/integrations/home_assistant/error.rs)
