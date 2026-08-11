# Home Assistant

The Home Assistant integration has six progressive tools. `home_assistant_query` and `home_assistant_exec` apply fresh Assist exposure authorization to entity operations. The Thread and Matter tools use separate fixed WebSocket command and registry boundaries. Root help lists namespaces; namespace help lists actions and schemas.

| Document | Covers |
| --- | --- |
| [Shared contract](spec/common.md) | Tool boundaries, authentication, exposure, limits, normalization, and errors. |
| [Common controls](common-controls.md) | The complete execution action catalog, inputs, fixed service mapping, and exclusions. |
| [Thread and Matter](spec/thread-matter.md) | Complete Thread and Matter catalogs, schemas, projections, authorization, safety, and exclusions. |
| [List entities](spec/list-entities.md) | Search, domain filters, ordering, and limits. |
| [List devices](spec/list-devices.md) | Exposure-filtered current states grouped by device and effective area. |
| [Get states](spec/get-states.md) | Explicit current-state reads. |
| [Get history](spec/get-history.md) | Bounded minimal significant history. |
| [Camera snapshot](spec/camera-snapshot.md) | One exposure-authorized, validated camera image from a fixed read-only endpoint. |
