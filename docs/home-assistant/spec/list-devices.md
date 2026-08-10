# List Devices

`device.list` returns current normalized states grouped by Home Assistant device and effective area. Root `help` advertises the `device` namespace; `help.device` describes this action and its schema.

| Input | Contract |
| --- | --- |
| `limit` | Optional integer from 1 through 100; default 100. It counts exposed current-state entities before grouping, not device groups. |

The action performs a fresh exposure lookup, reads fixed `GET /api/states`, keeps only exact `conversation: true` entities, sorts by entity ID, and applies the entity limit. It then requests entity registry entries only for those selected IDs and reads device and area registries on the same authenticated WebSocket. Registry-only data cannot create output.

Entity registry `area_id` overrides device `area_id`. Entities without a device remain in separate single-entity groups and may receive area enrichment. Device names prefer a non-empty `name_by_user`, then a non-empty `name`. Accepted registry references are at most 255 bytes, and returned names are at most 256 bytes. Missing registry references omit metadata without dropping the exposed entity.

Groups are sorted by area, device name, and first entity ID; entities are sorted by entity ID. The result contains `action`, `devices`, and `truncated`. Each non-empty group contains optional `name`, optional `area`, and `entities` using the shared normalized current-state projection.

The privacy projection never returns device IDs, area IDs, manufacturer, model, identifiers, labels, registry settings, arbitrary state attributes, or raw registry objects.
