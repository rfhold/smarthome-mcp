# Home Assistant Shared Contract

The tool names are `home_assistant_query` and `home_assistant_exec`. Generated progressive help and schemas own action discovery. `home_assistant_query` retains its read-only contract. `home_assistant_exec` is limited to the [common-control contract](../common-controls.md). Every external input rejects unknown fields; semantic validation completes before concurrency admission or network contact.

One invocation acquires one of four non-waiting permits and has a 20-second end-to-end deadline. The deadline covers WebSocket authentication and commands, exposure authorization, REST response reading, validation, and output construction. URLs are limited to 8 KiB, REST bodies to 4 MiB, and WebSocket messages and frames to 1 MiB. Normalized JSON output remains limited to 2 MiB, and history output remains limited to 2,000 state points. The [camera snapshot contract](camera-snapshot.md) owns its image and transport limits.

The server uses a fixed origin and credential. It disables REST redirects and environment proxies and never returns upstream wrappers or error bodies. Execution actions read and discard bounded upstream results and return only a minimal result.

Successful unfiltered `device.list`, `entity.list`, `state.get`, and `history.get` results return the complete normalized object as serialized JSON text and as the same object in `structuredContent`. Progressive filters keep text and structured content synchronized. Camera results retain their specialized image response, and execution actions retain their minimal acknowledgment response.

Semantic failures use `structuredContent.error` with stable `code`, `message`, and `retryable` fields. Codes are `invalid_arguments`, `capacity_exhausted`, `timeout`, `home_assistant_unauthorized`, `not_allowed`, `entity_not_found`, `request_rejected`, `upstream_unavailable`, `response_too_large`, and `invalid_response`. All errors follow the [exposure and data-safety privacy contract](../../architecture/exposure-data-safety.md).
