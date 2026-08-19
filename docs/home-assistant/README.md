# Home Assistant

The current runtime has six progressive tools. Entity operations apply fresh Assist exposure authorization. Authoring, blueprint, component deployment, setup, restart, Thread, and Matter administrator actions use endpoint-wide `mcp:use`. Local code and tests cover these contracts. Live Home Assistant compatibility, SSH/SFTP deployment, installation, and external behavior remain unverified.

| Document | Covers |
| --- | --- |
| [Shared contract](spec/common.md) | Tool boundaries, authentication, exposure, limits, normalization, and errors. |
| [Common controls](common-controls.md) | The complete execution action catalog, inputs, fixed service mapping, and exclusions. |
| [Authoring and evidence](spec/authoring-evidence.md) | Scene and automation discovery, exact native config reads, upserts, validation, projected traces, authority, and live evidence requirements. |
| [Blueprints](spec/blueprints.md) | Custom integration, blueprint actions, setup, restart, bounds, authority, and compatibility. |
| [Component deployment](spec/component-deployment.md) | Fixed private SFTP deployment, reconciliation, transaction, authority, privacy, and restart boundaries. |
| [Thread and Matter](spec/thread-matter.md) | Complete Thread and Matter catalogs, schemas, projections, authorization, safety, and exclusions. |
| [List entities](spec/list-entities.md) | Search, domain filters, ordering, and limits. |
| [List devices](spec/list-devices.md) | Exposure-filtered current states grouped by device and effective area. |
| [Get states](spec/get-states.md) | Explicit current-state reads. |
| [Get history](spec/get-history.md) | Bounded minimal significant history. |
| [Camera snapshot](spec/camera-snapshot.md) | One exposure-authorized, validated camera image from a fixed read-only endpoint. |
