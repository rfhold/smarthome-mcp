# Camera Snapshot

`camera.snapshot` accepts exactly one syntactically valid `camera.*` entity ID. Unknown fields and every other entity domain produce `invalid_arguments` before concurrency admission or network contact.

The action performs a fresh Assist exposure lookup for every invocation. It retrieves no image unless that exact entity has `conversation: true`. The [exposure and data-safety contract](../../architecture/exposure-data-safety.md) defines fail-closed authorization and privacy behavior.

After authorization, the action calls only fixed `GET /api/camera_proxy/{entity_id}` with the server-owned bearer credential. It never calls a Home Assistant service. In particular, it never invokes Home Assistant's file-writing `camera.snapshot` service. It also never accepts an upstream origin, path, query, header, or proxy target from the caller.

The response must use exactly `image/jpeg`, `image/png`, or `image/webp`. The decoded bytes must match the declared format signature and must not exceed 4 MiB. A MIME or signature mismatch produces `invalid_response`; an oversized image produces `response_too_large`.

The MCP result contains short text, one image content block with canonical padded standard Base64, and bounded metadata. Metadata does not duplicate image bytes. The decoded image limit leaves the complete result below Kuri Agent's 8 MiB MCP transport limit. The shared 2 MiB normalized JSON output limit remains unchanged.

The action uses the existing `home_assistant_query` tool and `mcp:use` OAuth scope. It uses the [shared four-permit admission and 20-second deadline](common.md).
