# List Entities

`list_entities` reads `GET /api/states`, discards entities not explicitly exposed to the conversation assistant, normalizes approved fields, filters, sorts by entity ID, and applies the limit.

| Input | Contract |
| --- | --- |
| `query` | Optional case-insensitive substring over entity ID and friendly name; at most 128 bytes. |
| `domains` | Optional list of at most 20 lowercase Home Assistant domain names. |
| `limit` | Optional integer from 1 through 100; default 50. |

The result contains `action`, `entities`, and `truncated`.
