# Get States

Public current-state reads use `smarthome://states/{entity_id}` under the [resource-first contract](resource-first.md). The batch action below is a private adapter, not a query alias.

`state.get` accepts 1 through 25 unique, syntactically valid entity IDs. All IDs must be present in the fresh explicit Assist exposure set before any state endpoint is called.

The service calls fixed `GET /api/states/{entity_id}` endpoints and returns `action` plus normalized `entities`. Missing entities produce `entity_not_found`; unexposed entities produce `not_allowed` without a REST state read.
