# Architecture Overview

`smarthome-mcp` is a hosted MCP resource server and bounded capability adapter. It authenticates MCP clients through its own OAuth issuer and authenticates to one fixed Home Assistant origin with a server-owned token.

## Components

| Component | Responsibility |
| --- | --- |
| Axum host | Health, readiness, OAuth/OIDC, MCP routing, and graceful shutdown. |
| Kuri `mcp` | Streamable HTTP, progressive tools, hosted OAuth, OIDC resource-owner flow, and PostgreSQL persistence. |
| `Services` | Concrete composition root for integration clients. |
| Home Assistant integration | Input validation, entity exposure checks, fixed REST and WebSocket operations, authoring, evidence projection, safe errors, and bounded telemetry. |
| PostgreSQL | Durable OAuth, OIDC, and wrapped signing-key state. |
| Authentik | Browser identity only; its tokens never authorize `/mcp`. |

## Request Flow

1. Kuri validates a locally issued token bound to `/mcp` and scope `mcp:use`.
2. The progressive server validates the selected Home Assistant, Thread, or Matter tool action.
3. The integration acquires one of four non-waiting operation permits.
4. It opens a bounded authenticated Home Assistant connection for the selected fixed operation.
5. Entity operations refresh Assist exposure and permit only entities marked `conversation: true`.
6. Scene and automation authoring and evidence actions use the [administrator-token exception](../home-assistant/spec/authoring-evidence.md).
7. Matter device actions refresh the device registry and require an exact Matter device match.
8. Thread and Matter actions use only their fixed [WebSocket command catalog](../home-assistant/spec/thread-matter.md).
9. Queries return projected bounded data. Execution actions discard upstream details and return minimal output.

The caller cannot select an upstream origin, credential, header, generic command, Home Assistant service, or HTTP method. A validated config key selects only the final segment of one fixed scene or automation config path. Native JSON is accepted only by the four fixed [authoring and evidence actions](../home-assistant/spec/authoring-evidence.md). Controls outside the fixed [common-control catalog](../home-assistant/common-controls.md), authoring and evidence contract, and Thread and Matter contract remain excluded.

`GET /health` reports process health. `GET /ready` performs bounded PostgreSQL and signing-key readiness checks; it does not contact Home Assistant or Authentik.
