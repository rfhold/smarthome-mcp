# Home Assistant Common Controls

`home_assistant_exec` is the only execution tool. Its MCP annotations are `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, and `openWorldHint: true`. The existing `mcp:use` scope authorizes this tool as well as `home_assistant_query`; there is no per-tool OAuth scope.

## Actions

Every action requires exactly one `entity_id` in the matching domain. Only the listed additional input is accepted.

| Action | Additional input | Fixed POST endpoint |
| --- | --- | --- |
| `scene.activate` | None. | `/api/services/scene/turn_on` |
| `light.turn_on` | Optional `brightness_pct` from 0 through 100, inclusive. | `/api/services/light/turn_on` |
| `light.turn_off` | None. | `/api/services/light/turn_off` |
| `switch.turn_on` | None. | `/api/services/switch/turn_on` |
| `switch.turn_off` | None. | `/api/services/switch/turn_off` |
| `fan.turn_on` | None. | `/api/services/fan/turn_on` |
| `fan.turn_off` | None. | `/api/services/fan/turn_off` |
| `fan.set_percentage` | `percentage` from 0 through 100, inclusive. | `/api/services/fan/set_percentage` |
| `cover.open` | None. | `/api/services/cover/open_cover` |
| `cover.close` | None. | `/api/services/cover/close_cover` |
| `cover.stop` | None. | `/api/services/cover/stop_cover` |
| `cover.set_position` | `position` from 0 through 100, inclusive. | `/api/services/cover/set_cover_position` |
| `climate.turn_on` | None. | `/api/services/climate/turn_on` |
| `climate.turn_off` | None. | `/api/services/climate/turn_off` |
| `climate.set_temperature` | `temperature`, finite number from -273.15 through 1000, inclusive. | `/api/services/climate/set_temperature` |
| `media_player.turn_on` | None. | `/api/services/media_player/turn_on` |
| `media_player.turn_off` | None. | `/api/services/media_player/turn_off` |
| `media_player.play` | None. | `/api/services/media_player/media_play` |
| `media_player.pause` | None. | `/api/services/media_player/media_pause` |
| `media_player.stop` | None. | `/api/services/media_player/media_stop` |
| `media_player.volume_set` | `volume_level`, finite number from 0.0 through 1.0, inclusive. | `/api/services/media_player/volume_set` |
| `lock.lock` | None. | `/api/services/lock/lock` |
| `lock.unlock` | None. | `/api/services/lock/unlock` |

Unknown fields, invalid values, and a domain mismatch fail validation. Batching and multi-entity targets are not supported.

## Authorization And Routing

Every invocation performs a fresh `homeassistant/expose_entity/list` lookup. The exact target must have `conversation: true` before any mutation. The service then maps the selected action server-side to fixed `POST /api/services/{domain}/{service}` routing and constructs a bounded JSON body from only the validated entity ID and listed value. Callers cannot provide a service, domain, path, method, headers, origin, or arbitrary service data.

The server reads and discards bounded upstream results and returns a minimal result rather than Home Assistant state, context, or service-response data. The shared admission, deadline, size, privacy, and error contracts are defined in [the shared contract](spec/common.md) and [Exposure and Data Safety](../architecture/exposure-data-safety.md).

## Exclusions

The tool does not provide arbitrary services, batching, toggle actions, confirmations, presets, sources, modes, colors, or templates. `lock.unlock` and `cover.open` deliberately require no confirmation beyond exact Assist exposure and a valid token with `mcp:use`.
