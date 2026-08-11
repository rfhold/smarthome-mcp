# Home Assistant

The Home Assistant integration has two progressive tools. `home_assistant_query` remains read-only, non-destructive, idempotent, and open-world, with its existing five actions. `home_assistant_exec` provides only the approved common controls and is read-only false, destructive, non-idempotent, and open-world. Both tools apply the current Assist exposure policy before accessing an entity. Root help lists namespaces; namespace help lists actions and schemas.

| Document | Covers |
| --- | --- |
| [Shared contract](spec/common.md) | Tool boundaries, authentication, exposure, limits, normalization, and errors. |
| [Common controls](common-controls.md) | The complete execution action catalog, inputs, fixed service mapping, and exclusions. |
| [List entities](spec/list-entities.md) | Search, domain filters, ordering, and limits. |
| [List devices](spec/list-devices.md) | Exposure-filtered current states grouped by device and effective area. |
| [Get states](spec/get-states.md) | Explicit current-state reads. |
| [Get history](spec/get-history.md) | Bounded minimal significant history. |
| [Camera snapshot](spec/camera-snapshot.md) | One exposure-authorized, validated camera image from a fixed read-only endpoint. |
