# Resource-First MCP Interface

The authenticated `/mcp` endpoint always exposes `create`, `edit`, `query`, and `execute`, dynamic `smarthome://` resources, and unchanged immutable `skill://` resources. Private generated handlers retain legacy names for adapter dispatch and tests, not public tools or compatibility aliases. This document owns public routing. Domain documents describe adapter validation, normalization, and upstream behavior, not additional public capabilities.

## Resources

`resources/list` discovers seven bounded JSON catalogs:

| URI | Content |
| --- | --- |
| `smarthome://entities` | Fresh Assist-exposed normalized entities, canonical item, state, and camera links. |
| `smarthome://devices` | Exposed states grouped by device and effective area, linked using the group's first entity ID. |
| `smarthome://scenes` | Editor-managed scene metadata and config links. |
| `smarthome://automations` | Editor-managed automation metadata, config links, and trace links. |
| `smarthome://blueprints` | Automation blueprint metadata and semantic YAML links. |
| `smarthome://thread/networks` | Stored Thread datasets, excluding operational TLVs. |
| `smarthome://matter/devices` | Registered Matter device projections. |

Catalogs request 100 entries (Thread uses its existing fixed bound). Device limits count exposed entities before grouping. Existing `truncated` and `total` fields remain when supplied; no pagination or subscription is advertised. Device, Thread, and Matter item reads require membership in a freshly read bounded catalog. Native config/blueprint reads use the validated canonical identifier directly with the administrator-only boundary; catalog discovery is not a complete inventory.

Every dynamic text catalog/item response enforces a 2 MiB bound after link enrichment, pretty serialization, resource wrapping, JSON escaping, and resource metadata. Oversized responses fail with a small generic error, not partial content. Native text views retain their additional 256 KiB limit.

Templates are `smarthome://entities/{entity_id}`, `smarthome://states/{entity_id}`, `smarthome://devices/{entity_id}`, `smarthome://cameras/{entity_id}`, `smarthome://scenes/{config_key}`, `smarthome://automations/{config_key}`, `smarthome://automations/{config_key}/traces`, `smarthome://blueprints/{path}`, `smarthome://thread/networks/{dataset_id}`, and `smarthome://matter/devices/{device_id}`.

Follow returned links. Identifiers occupy one URI segment: encode every UTF-8 byte except ASCII letters, digits, `-`, `_`, `.`, and `~` using uppercase percent escapes. Blueprint slashes are encoded, for example `smarthome://blueprints/vendor%2Fmotion.yaml`. Noncanonical escaping, raw separators inside identifiers, query strings, fragments, malformed UTF-8, empty identifiers, controls, and encoded identifiers exceeding 768 bytes are rejected. Existing domain validation applies after decoding.

Entity/state/camera reads perform a fresh exact Assist exposure check. Device reads refresh exposure through grouping. Earlier discovery is never authorization for a later read. Administrator config, blueprint, Thread, and Matter resources retain endpoint-wide `mcp:use` and upstream credential authority, without recursive entity-reference authorization. Membership observation and content retrieval are not an atomic upstream snapshot.

Camera contents use MCP resource `blob` with standard base64 and validated MIME type. Scene/automation reads use native config GET and deterministic pretty JSON; blueprint reads use the existing semantic YAML getter. These are semantic views, not byte-identical source. `_meta.revision` is `sha256:` followed by the lowercase SHA-256 digest of the exact displayed UTF-8 text. `_meta.editable: true`, `_meta.concurrency: 'single-writer'`, and `_meta.revision_check: 'best-effort'` describe local observational checks, not native compare-and-swap. Item text views reject output above 256 KiB. These views may contain sensitive values.

## Tools

`query` and `execute` retain the closed `{action, input?, filter?}` wrapper and existing action-specific validation. Optional jq filtering is output convenience, never authorization.

`query` has seven actions: `history.get`, `automation.validate`, `thread.router.discover`, `thread.readiness.get`, `matter.readiness.get`, `matter.device.diagnostics`, and `matter.device.ping`. Static/current reads are resources, not query aliases. Temporal and computed results retain their existing limited meaning.

`execute` retains all fixed physical controls under domain action names, including `scene.activate` and `media_player.stop`. It also includes `smarthome_mcp.deploy`, `smarthome_mcp.setup`, `home_assistant.restart`, `thread.network.set_preferred`, `thread.router.set_preferred`, and `matter.device.interview`. Selection, interview, restart, and setup are commands, not text edits. Exact confirmations, fixed targets, host trust, and separate deploy/restart/setup decisions remain unchanged. A command acknowledgment is not verified physical state, reload completion, or readiness. Unconfirmed, cancelled, or timed-out writes can have unknown outcomes and must not be automatically repeated.

## Single-Writer Authoring

Discovery always exposes `create` and `edit`. They use existing native config GET/POST and blueprint list/save APIs; blueprint item reads require only the existing semantic blueprint reader component. No Core patch, authoring capability probe, or extra component command is required. No `destroy` tool exists. Legacy config upsert/save/from-blueprint action names remain excluded from public dispatch.

`create` accepts the closed `{action, input}` wrapper. Actions are `scene.create`, `automation.create`, and `blueprint.create`. Scene/automation input is `{config_key, text}` with complete JSON object text; blueprint input is `{path, text}` with complete semantic YAML. Text is at most 256 KiB. Two native absence observations precede the write. Config creation requires native GET 404; blueprint creation checks the complete native list including invalid entries and uses `allow_override: false`. Other read errors fail closed. Observed existing identifiers return `already_exists`. Config POST remains a native upsert and can overwrite a concurrent writer between preflight and persistence: this is not atomic create-only.

`edit` accepts direct closed `{uri, expected_revision, edits}` arguments, not an action/input/filter envelope. It allows 1 through 256 ordered local text operations: `{operation: 'replace', old_text, new_text}` or `{operation: 'insert', text, placement: 'start'|'end'|'before'|'after', anchor?}`. Replacement requires a unique nonempty match including overlaps; before/after require a unique nonempty anchor; start/end forbid an anchor even when null. Each intermediate and final text is bounded to 256 KiB. Text no-ops and JSON/YAML-semantic net-zero changes fail. The wrapper reads fresh native text, compares its digest, applies all edits and validates the candidate locally, then re-reads and compares again. Local blueprint parsing requires one mapping document, rejects syntax/duplicate-key errors, and preserves `!input` tags. Native owners still validate their complete domain schemas before persistence. A failed ordered edit sequence never writes. No native transaction or lock spans this sequence. See [simple authoring](simple-authoring.md) for validation limits.

Mutation responses include `accepted: true`, `reload_complete: false`, and the single-writer/best-effort metadata. Native persistence acknowledgment is not reload completion, activation, or future execution evidence. Validation errors are generic and do not echo private input. Conflicts and existing identifiers fail before writing. Once dispatch may have happened, timeout, cancellation, connection failure, unexpected errors, or malformed acknowledgments return nonretryable `mutation_outcome_unknown`. The service never retries writes; read before making a new explicitly authorized decision. Cancellation is not rollback.

Local mocks do not establish live compatibility, integration installation, external operations, or deployment. The published immutable Kuri MCP dependency remains unchanged.
