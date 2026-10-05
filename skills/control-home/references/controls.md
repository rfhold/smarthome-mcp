# Controls

Call `execute` with `action` and `input`. Every entity action in the table requires exactly one `entity_id` matching the action's domain. The live schema is authoritative; wrappers reject unknown fields. There is no batching, toggle, arbitrary service data, or caller-selected service.

| Actions | Additional input |
| --- | --- |
| `scene.activate` | None; activation is separate from config replacement. |
| `light.turn_on`, `light.turn_off` | On accepts optional `brightness_pct`, 0-100. |
| `switch.turn_on`, `switch.turn_off` | None. |
| `fan.turn_on`, `fan.turn_off`, `fan.set_percentage` | Percentage requires `percentage`, 0-100. |
| `cover.open`, `cover.close`, `cover.stop`, `cover.set_position` | Position requires `position`, 0-100. |
| `climate.turn_on`, `climate.turn_off`, `climate.set_temperature` | Temperature requires finite `temperature`, -273.15 to 1000. |
| `media_player.turn_on`, `media_player.turn_off`, `media_player.play`, `media_player.pause`, `media_player.stop`, `media_player.volume_set` | Volume requires finite `volume_level`, 0-1. |
| `lock.lock`, `lock.unlock` | None. |

Home Assistant decides device-specific capability and behavior; the numeric API bounds are not a recommendation for safe temperatures or positions. Stop on denied exposure or invalid targets. Do not bypass by authoring a native config.

Controls discard upstream service responses and return a minimal success projection. Query a fresh state separately when verification matters. Endpoint-wide `mcp:use` has no per-tool scope separation; skills and annotations do not add it. Stored config writes, integration maintenance, and Thread/Matter mutations are different workflows. No removed `help` action or alias is available.

## Live Automation Trigger

`automation.trigger` is distinct from entity controls and config authoring. Resolve the exact `config_key` through `smarthome://automations`, read `smarthome://automations/<config_key>`, review the native text and `_meta.revision`, and obtain explicit authority for live execution. Send `config_key`, that `expected_revision`, and `confirm: true`; `skip_conditions` defaults to `false` and may be set to `true` only when explicitly requested. The service checks the native config ID, current revision, and exact unique config-to-entity mapping before dispatch. It resolves the entity internally; no caller `entity_id`, synthetic `trigger`, `variables`, or dry-run is supported.

Clients with endpoint-wide `mcp:use` can manually run any resolvable editor-managed automation with confirmation and revision using the configured administrator Home Assistant token. No separate per-automation approval configuration or per-action permission system exists. This broader live-execution authority permits native nested effects on entities not exposed to Assist; nested actions are not recursively exposure-authorized. Review the current native action sequence and establish user authority for those effects. Never use authoring or automation execution to circumvent denied controls. Revision checks are best-effort, not atomic, and do not prove runtime reload completion. Manual triggering does not recreate `trigger.to_state`; trigger-dependent conditions or actions may skip or fail.

For an existing `evening_lights` automation, use the actual revision from its resource:

```json
{"action":"automation.trigger","input":{"config_key":"evening_lights","expected_revision":"sha256:<64 lowercase hex characters from _meta.revision>","confirm":true}}
```

Report only dispatch acknowledgment. `completion_verified` and `effects_verified` remain false, and `trace_correlation` is unsupported. Independent trace resources are not evidence that their newest trace belongs to this request. After post-dispatch timeout, disconnect, cancellation, or errors, the handler classifies the outcome as nonretryable `mutation_outcome_unknown`. The MCP transport can suppress the response on cancellation. An absent response does not prove that upstream execution stopped and does not authorize a retry. No rollback, upstream cancellation, or automatic retry is provided. Observe independently and obtain a new exact decision before another dispatch.
