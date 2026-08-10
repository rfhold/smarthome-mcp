# Architecture Overview

`smarthome-mcp` is a hosted MCP resource server and bounded capability adapter. It authenticates MCP clients through its own OAuth issuer and authenticates to one fixed Home Assistant origin with a server-owned token.

## Components

| Component | Responsibility |
| --- | --- |
| Axum host | Health, readiness, OAuth/OIDC, MCP routing, and graceful shutdown. |
| Kuri `mcp` | Streamable HTTP, progressive tools, hosted OAuth, OIDC resource-owner flow, and PostgreSQL persistence. |
| `Services` | Concrete composition root for integration clients. |
| Home Assistant integration | Input validation, Assist exposure checks, REST reads, normalization, safe errors, and bounded telemetry. |
| PostgreSQL | Durable OAuth, OIDC, and wrapped signing-key state. |
| Authentik | Browser identity only; its tokens never authorize `/mcp`. |

## Request Flow

1. Kuri validates a locally issued token bound to `/mcp` and scope `mcp:use`.
2. The progressive server validates the selected `home_assistant_query` action.
3. The integration acquires one of four non-waiting operation permits.
4. It opens a bounded Home Assistant WebSocket, authenticates, and requests `homeassistant/expose_entity/list`.
5. It permits only entities explicitly marked `conversation: true`.
6. It calls a fixed REST endpoint and returns a bounded normalized result.

The caller cannot select an upstream origin, path, credential, header, or Home Assistant service. Control and arbitrary HTTP proxy behavior are outside this architecture.

`GET /health` reports process health. `GET /ready` performs bounded PostgreSQL and signing-key readiness checks; it does not contact Home Assistant or Authentik.
