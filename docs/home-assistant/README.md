# Home Assistant

The progressive `home_assistant_query` tool is read-only, non-destructive, idempotent, and open-world. It has five actions and always applies the current Assist exposure policy before reading entity data or camera images. Root help lists namespaces; namespace help such as `help.camera` lists its actions and schemas.

| Specification | Covers |
| --- | --- |
| [Shared contract](spec/common.md) | Authentication, exposure, limits, normalization, and errors. |
| [List entities](spec/list-entities.md) | Search, domain filters, ordering, and limits. |
| [List devices](spec/list-devices.md) | Exposure-filtered current states grouped by device and effective area. |
| [Get states](spec/get-states.md) | Explicit current-state reads. |
| [Get history](spec/get-history.md) | Bounded minimal significant history. |
| [Camera snapshot](spec/camera-snapshot.md) | One exposure-authorized, validated camera image from a fixed read-only endpoint. |
