# Architecture Overview

`smarthome-mcp` is a hosted MCP resource server and bounded capability adapter. It authenticates MCP clients through its own OAuth issuer and authenticates to one fixed Home Assistant origin with a server-owned token.

## Components

| Component | Responsibility |
| --- | --- |
| Axum host | Health, readiness, OAuth/OIDC, MCP routing, and graceful shutdown. |
| Kuri `mcp` | Streamable HTTP, progressive tools, hosted OAuth, OIDC resource-owner flow, and PostgreSQL persistence. |
| `Services` | Concrete composition root for integration clients. |
| Home Assistant integration | Input validation, entity exposure checks, fixed REST, WebSocket, and SFTP operations, authoring, evidence projection, safe errors, and bounded telemetry. |
| `smarthome_mcp` custom integration | One config entry and one administrator-only semantic blueprint read command. |
| Component deployer | Embedded component bytes, pinned native SSH/SFTP transport, and bounded transactional replacement. |
| PostgreSQL | Durable OAuth, OIDC, and wrapped signing-key state. |
| Authentik | Browser identity only; its tokens never authorize `/mcp`. |

## Request Flow

1. Kuri validates a locally issued token bound to `/mcp` and scope `mcp:use`.
2. The progressive server validates the selected Home Assistant, Thread, or Matter tool action.
3. A Home Assistant HTTP or WebSocket operation acquires one of four non-waiting operation permits and opens a bounded authenticated connection.
4. Component deployment instead acquires its separate one-operation permit and uses only the fixed native SFTP transaction.
5. Entity operations refresh Assist exposure and permit only entities marked `conversation: true`.
6. Scene and automation authoring, config-read, and evidence actions use the [administrator-token exception](../home-assistant/spec/authoring-evidence.md).
7. Matter device actions refresh the device registry and require an exact Matter device match.
8. Thread and Matter actions use only their fixed [WebSocket command catalog](../home-assistant/spec/thread-matter.md).
9. Blueprint actions use only native list, save, substitute, fixed config, and custom semantic-read operations.
10. Component deployment uses only the fixed private target and embedded bytes.
11. Queries return projected bounded data. Execution actions discard upstream details and return minimal output.

The caller cannot select an upstream origin, credential, header, generic command, Home Assistant service, HTTP method, deployment host, filesystem path, repository, version, or component bytes. A validated config key selects only the final segment of one fixed scene or automation config path. Native JSON follows the eight fixed [authoring and evidence actions](../home-assistant/spec/authoring-evidence.md). Blueprint JSON and YAML follow the [blueprint contract](../home-assistant/spec/blueprints.md). Private replacement follows the [component deployment contract](../home-assistant/spec/component-deployment.md). All contracts exclude generic routes and proxies.

`GET /health` reports process health. `GET /ready` performs bounded PostgreSQL and signing-key readiness checks; it does not contact Home Assistant or Authentik.
