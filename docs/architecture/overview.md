# Architecture Overview

`smarthome-mcp` is a hosted MCP resource server and bounded capability adapter. It authenticates MCP clients through its own OAuth issuer and authenticates to one fixed Home Assistant origin with a server-owned token.

## Components

| Component | Responsibility |
| --- | --- |
| Axum host | Health, readiness, OAuth/OIDC, MCP routing, and graceful shutdown. |
| Kuri `mcp` | Streamable HTTP, progressive tools, hosted OAuth, OIDC resource-owner flow, and PostgreSQL persistence. |
| `Services` | Concrete composition root for integration clients. |
| Home Assistant integration | Input validation, entity exposure checks, fixed REST and WebSocket commands, registry reads, result projection, safe errors, and bounded telemetry. |
| PostgreSQL | Durable OAuth, OIDC, and wrapped signing-key state. |
| Authentik | Browser identity only; its tokens never authorize `/mcp`. |

## Request Flow

1. Kuri validates a locally issued token bound to `/mcp` and scope `mcp:use`.
2. The progressive server validates the selected entity, Thread, or Matter tool action.
3. The integration acquires one of four non-waiting operation permits.
4. It opens a bounded authenticated Home Assistant connection for the selected fixed operation.
5. Entity actions refresh Assist exposure and permit only entities marked `conversation: true`.
6. Matter device actions refresh the device registry and require an exact Matter device match.
7. Thread and Matter actions use only their fixed [WebSocket command catalog](../home-assistant/spec/thread-matter.md).
8. Queries return projected bounded data. Execution actions discard upstream details and return minimal output.

The caller cannot select an upstream origin, path, credential, header, command, Home Assistant service, HTTP method, or arbitrary data. Controls outside the fixed [common-control catalog](../home-assistant/common-controls.md), Thread and Matter exclusions, and arbitrary proxy behavior remain outside this architecture.

`GET /health` reports process health. `GET /ready` performs bounded PostgreSQL and signing-key readiness checks; it does not contact Home Assistant or Authentik.
