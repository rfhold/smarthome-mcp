# Controls

Call `execute` with `action` and `input`. Every action here requires exactly one `entity_id` matching the action's domain. The live schema is authoritative; wrappers reject unknown fields. There is no batching, toggle, arbitrary service data, or caller-selected service.

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
