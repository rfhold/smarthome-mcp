# Architecture Overview

`smarthome-mcp` is a hosted MCP resource server and bounded capability adapter. It authenticates MCP clients through its own OAuth issuer and authenticates to one fixed Home Assistant origin with a server-owned token.

## Components

| Component | Responsibility |
| --- | --- |
| Axum host | Health, readiness, OAuth/OIDC, MCP routing, and graceful shutdown. |
| Kuri `mcp` | Streamable HTTP, progressive tools, hosted OAuth, OIDC resource-owner flow, and PostgreSQL persistence. |
| `Services` | Concrete composition root for integration clients. |
| Home Assistant integration | Input validation, Assist exposure checks, fixed REST reads and common-control service calls, registry reads, result validation, safe errors, and bounded telemetry. |
| PostgreSQL | Durable OAuth, OIDC, and wrapped signing-key state. |
| Authentik | Browser identity only; its tokens never authorize `/mcp`. |

## Request Flow

1. Kuri validates a locally issued token bound to `/mcp` and scope `mcp:use`.
2. The progressive server validates the selected `home_assistant_query` or `home_assistant_exec` action.
3. The integration acquires one of four non-waiting operation permits.
4. It opens a bounded Home Assistant WebSocket, authenticates, and requests `homeassistant/expose_entity/list`.
5. It permits only entities explicitly marked `conversation: true`.
6. A query calls fixed upstream reads and returns bounded normalized data or one validated camera image. An execution action maps to one fixed Home Assistant service POST with bounded server-constructed JSON, discards the bounded upstream result, and returns minimal output. `device.list` retains its authenticated socket for fixed registry enrichment after it selects exposed current states.

The caller cannot select an upstream origin, path, credential, header, Home Assistant domain or service, HTTP method, or arbitrary service data. Controls outside the fixed [common-control catalog](../home-assistant/common-controls.md) and arbitrary HTTP proxy behavior remain outside this architecture.

`GET /health` reports process health. `GET /ready` performs bounded PostgreSQL and signing-key readiness checks; it does not contact Home Assistant or Authentik.
