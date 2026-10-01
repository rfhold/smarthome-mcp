# Networks And Devices

| Tool | Actions and inputs |
| --- | --- |
| Resources | `smarthome://thread/networks` and `smarthome://matter/devices`; follow returned canonical item links. |
| `query` | `thread.readiness.get` and `matter.readiness.get`: empty input; `thread.router.discover`: optional `duration_seconds`, 1-10, default 3; `matter.device.diagnostics` and `matter.device.ping`: exact `device_id`. |
| `execute` | `thread.network.set_preferred`: exact `dataset_id`; `thread.router.set_preferred`: exact `dataset_id` and `extended_address`, optional nullable `border_agent_id`; `matter.device.interview`: exact `device_id`. |

Use current closed schemas and optional query filters, not removed help actions. Identifiers allow 1-255 ASCII bytes of letters, digits, hyphen, underscore, colon, and dot. Read current networks and routers before selecting; Home Assistant decides whether identifiers exist. Omitted `border_agent_id` is sent as null. Preferred selection and interview require explicit user confirmation for the exact target and effect; their API schemas do not have a `confirm` field, so do not invent one.

Thread readiness reports stored datasets and observed router discovery only, not Matter server, Bluetooth, or device reachability. Matter readiness reports device-registry responsiveness and count only, not network or device readiness. Ping reports observed IP reachability. Diagnostics return a strict bounded node/fabric projection, not raw diagnostics. Each Matter device command refreshes the registry and rejects targets not uniquely identified as Matter devices.

Endpoint-wide `mcp:use` authorizes all these tools without per-tool separation. The shared Home Assistant token must support administrator-only Thread commands and Matter interview; diagnostics and ping can be permitted without administrator privilege upstream, but this deployment shares one token. Confirm supported Home Assistant behavior independently; mocks do not prove release compatibility. Do not turn an unavailable API into arbitrary WebSocket access.

Thread network output excludes operational dataset TLVs and credentials. No dataset add/import/replace/delete, Matter commissioning, fabric removal, or commissioning-window action exists. Keep network names, addresses, IDs, diagnostic values, and raw errors out of telemetry. Minimal mutation acknowledgments show command acceptance, not complete network health.
