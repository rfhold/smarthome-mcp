# Home Assistant Shared Contract

The tool name is `home_assistant_query`. Generated progressive help and schemas own action discovery. Every external input rejects unknown fields; semantic validation completes before concurrency admission or network contact.

One invocation acquires one of four non-waiting permits and has a 20-second end-to-end deadline. The deadline covers WebSocket authentication and commands, exposure authorization, REST response reading, validation, and output construction. URLs are limited to 8 KiB, REST bodies to 4 MiB, and WebSocket messages and frames to 1 MiB. Normalized JSON output remains limited to 2 MiB, and history output remains limited to 2,000 state points. The [camera snapshot contract](camera-snapshot.md) owns its image and transport limits.

The server uses a fixed origin and credential. It disables REST redirects and environment proxies and never returns upstream wrappers or error bodies.

Semantic failures use `structuredContent.error` with stable `code`, `message`, and `retryable` fields. Codes are `invalid_arguments`, `capacity_exhausted`, `timeout`, `home_assistant_unauthorized`, `not_allowed`, `entity_not_found`, `request_rejected`, `upstream_unavailable`, `response_too_large`, and `invalid_response`. All errors follow the [exposure and data-safety privacy contract](../../architecture/exposure-data-safety.md).
