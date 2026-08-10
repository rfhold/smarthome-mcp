# Get History

`get_history` accepts 1 through 10 unique entity IDs, a required RFC3339 `start`, and an optional RFC3339 `end` that defaults to the current time. The interval must be positive and no longer than 24 hours.

After fresh exposure authorization, the service calls `GET /api/history/period/{start}` with the entity filter, end time, `minimal_response`, `no_attributes`, and `significant_changes_only`. It returns at most 2,000 points across all entities and sets `truncated` when additional points existed.
