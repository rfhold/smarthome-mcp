# Home Assistant

The progressive `home_assistant_query` tool is read-only, non-destructive, idempotent, and open-world. It has three actions and always applies the current Assist exposure policy before reading entity data.

| Specification | Covers |
| --- | --- |
| [Shared contract](spec/common.md) | Authentication, exposure, limits, normalization, and errors. |
| [List entities](spec/list-entities.md) | Search, domain filters, ordering, and limits. |
| [Get states](spec/get-states.md) | Explicit current-state reads. |
| [Get history](spec/get-history.md) | Bounded minimal significant history. |
