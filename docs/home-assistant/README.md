# Home Assistant

The public runtime has `create`, `edit`, `query`, and `execute`, dynamic `smarthome://` resources, and immutable Skills. Entity operations apply fresh Assist exposure authorization; administrator capabilities retain endpoint-wide `mcp:use`. Creation and text editing use existing native APIs with best-effort checks under a single-writer assumption. Live compatibility, installation, deployment, and external behavior remain unverified. The resource-first contract owns public routing; domain documents retain private adapter details.

| Document | Covers |
| --- | --- |
| [Resource-first interface](spec/resource-first.md) | Current tools, catalogs, templates, canonical links, and single-writer authoring. |
| [Simple authoring](spec/simple-authoring.md) | Existing native endpoints, best-effort checks, limits, and uncertain outcomes. |
| [Shared contract](spec/common.md) | Tool boundaries, authentication, exposure, limits, normalization, and errors. |
| [MCP Skills](spec/skills.md) | Immutable authored catalog, extension discovery, exact resource manifests, help cutover, and authority boundaries. |
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
