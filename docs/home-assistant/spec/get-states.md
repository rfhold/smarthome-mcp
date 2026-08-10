# Get States

`get_states` accepts 1 through 25 unique, syntactically valid entity IDs. All IDs must be present in the fresh explicit Assist exposure set before any state endpoint is called.

The service calls fixed `GET /api/states/{entity_id}` endpoints and returns `action` plus normalized `entities`. Missing entities produce `entity_not_found`; unexposed entities produce `not_allowed` without a REST state read.
